//! THROWAWAY #48: bounded per-attachment delivery that never blocks the pump.
//!
//! Restoration bytes are an ordered prefix that is never dropped; live output
//! queued behind it is bounded. Overflow, a stall, or an unfinished
//! restoration at its deadline disconnects only that attachment.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeliveryBudget {
    pub chunk_bytes: usize,
    pub queue_bytes: usize,
    pub no_progress: Duration,
    pub restoration_deadline: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disconnect {
    QueueOverflow,
    NoProgress,
    RestorationDeadline,
}

pub struct Outbound {
    budget: DeliveryBudget,
    restoration: Vec<u8>,
    restoration_until: Option<Instant>,
    live: VecDeque<u8>,
    sent: usize,
    waiting_since: Option<Instant>,
}

impl Outbound {
    pub fn live(budget: DeliveryBudget, _now: Instant) -> Self {
        Self {
            budget,
            restoration: Vec::new(),
            restoration_until: None,
            live: VecDeque::new(),
            sent: 0,
            waiting_since: None,
        }
    }

    pub fn restoring(budget: DeliveryBudget, restoration: Vec<u8>, now: Instant) -> Self {
        Self {
            restoration_until: Some(now + budget.restoration_deadline),
            waiting_since: (!restoration.is_empty()).then_some(now),
            restoration,
            ..Self::live(budget, now)
        }
    }

    /// Never blocks: either the bytes are queued whole or the attachment
    /// must be disconnected.
    pub fn publish(&mut self, bytes: &[u8], now: Instant) -> Result<(), Disconnect> {
        if self.live.len() + bytes.len() > self.budget.queue_bytes {
            return Err(Disconnect::QueueOverflow);
        }
        if !bytes.is_empty() && self.next_chunk().is_none() {
            self.waiting_since = Some(now);
        }
        self.live.extend(bytes);
        Ok(())
    }

    pub fn restoring_now(&self) -> bool {
        self.sent < self.restoration.len()
    }

    pub fn next_chunk(&mut self) -> Option<&[u8]> {
        let chunk = if self.restoring_now() {
            let end = self
                .restoration
                .len()
                .min(self.sent + self.budget.chunk_bytes);
            &self.restoration[self.sent..end]
        } else {
            let live = self.live.make_contiguous();
            &live[..live.len().min(self.budget.chunk_bytes)]
        };
        (!chunk.is_empty()).then_some(chunk)
    }

    pub fn wrote(&mut self, written: usize, now: Instant) {
        if self.restoring_now() {
            self.sent += written;
            if !self.restoring_now() {
                self.restoration = Vec::new();
                self.sent = 0;
                self.restoration_until = None;
            }
        } else {
            self.live.drain(..written);
        }
        self.waiting_since = self.next_chunk().is_some().then_some(now);
    }

    pub fn poll(&self, now: Instant) -> Option<Disconnect> {
        if self.restoration_until.is_some_and(|until| now >= until) {
            Some(Disconnect::RestorationDeadline)
        } else if self
            .waiting_since
            .is_some_and(|since| now >= since + self.budget.no_progress)
        {
            Some(Disconnect::NoProgress)
        } else {
            None
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.restoration_until
            .into_iter()
            .chain(
                self.waiting_since
                    .map(|since| since + self.budget.no_progress),
            )
            .min()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    Disconnected(usize, Disconnect),
    /// Unexpected loss of the final attachment starts session termination (#42).
    FinalAttachmentLost,
}

#[derive(Default)]
pub struct Attachments {
    next_id: usize,
    outbound: BTreeMap<usize, Outbound>,
}

impl Attachments {
    pub fn attach(&mut self, outbound: Outbound) -> usize {
        self.next_id += 1;
        self.outbound.insert(self.next_id, outbound);
        self.next_id
    }

    pub fn get_mut(&mut self, id: usize) -> Option<&mut Outbound> {
        self.outbound.get_mut(&id)
    }

    pub fn publish(&mut self, bytes: &[u8], now: Instant) -> Vec<SessionEvent> {
        self.disconnect_where(|outbound| outbound.publish(bytes, now).err())
    }

    pub fn poll(&mut self, now: Instant) -> Vec<SessionEvent> {
        self.disconnect_where(|outbound| outbound.poll(now))
    }

    fn disconnect_where(
        &mut self,
        mut verdict: impl FnMut(&mut Outbound) -> Option<Disconnect>,
    ) -> Vec<SessionEvent> {
        let had_attachments = !self.outbound.is_empty();
        let mut events = Vec::new();
        self.outbound
            .retain(|&id, outbound| match verdict(outbound) {
                Some(reason) => {
                    events.push(SessionEvent::Disconnected(id, reason));
                    false
                }
                None => true,
            });
        if had_attachments && self.outbound.is_empty() {
            events.push(SessionEvent::FinalAttachmentLost);
        }
        events
    }
}
