//! The order rules of one link connection's records after Hello: how
//! streams are numbered, begin and end, and what the relay may advertise.
//! Each end holds one [`Rules`] and judges by it every record that crosses,
//! either way: what it is asked to send, and what arrives.

use crate::stream::{Advertisement, HubRecord, HubStream, RelayRecord, RelayStream, SessionId};
use std::collections::{BTreeMap, BTreeSet};

/// The most sessions a relay may have advertised at once.
const SESSIONS_MOST: usize = 128;

/// What the rules make of one record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// It is in turn: it changes what the rules allow next.
    Accept,
    /// A record from the hub monitor on a stream that has ended, which may
    /// have crossed the ending on the wire. Its receiver ignores it; the
    /// hub monitor does not send it.
    Crossed,
    /// A record from the relay on a stream the hub monitor has closed. It
    /// may be sent, and the hub monitor discards it.
    Discarded,
    /// It breaks a rule.
    Broken,
}

impl Verdict {
    /// The verdict on a record that keeps a rule, or breaks it.
    fn of(kept: bool) -> Self {
        match kept {
            true => Self::Accept,
            false => Self::Broken,
        }
    }

    /// Whether the record is sent when its sender is asked to send it.
    pub(crate) fn sendable(self) -> bool {
        matches!(self, Self::Accept | Self::Discarded)
    }
}

/// How far one session stream has got.
#[derive(Clone, Copy, Debug, Default)]
struct Lane {
    /// The relay has answered Open stream with Stream opened.
    answered: bool,
    /// The hub monitor has sent Close stream.
    closed: bool,
}

impl Lane {
    /// Whether the hub monitor may send `record` on the stream now.
    fn allows(self, record: &HubStream) -> bool {
        let in_turn = match record {
            HubStream::Open(_) => false,
            HubStream::Close => true,
            HubStream::Request(_) => self.answered,
        };
        in_turn && !self.closed
    }
}

/// What a relay's record on a session stream needs, and does.
struct Step {
    /// Whether the relay must have answered Open stream before it.
    after_answer: bool,
    /// Whether it ends the stream.
    ends: bool,
}

impl Step {
    fn of(record: &RelayStream) -> Self {
        let (after_answer, ends) = match record {
            RelayStream::Opened(_) => (false, false),
            RelayStream::Refused(_) => (false, true),
            RelayStream::Ended(_) => (true, true),
            _ => (true, false),
        };
        Self { after_answer, ends }
    }
}

/// The order rules of one connection.
#[derive(Default)]
pub(crate) struct Rules {
    /// The number of the last stream opened: 0 before the first.
    last: u32,
    /// The streams that have begun and not ended.
    live: BTreeMap<u32, Lane>,
    /// The sessions the relay has advertised and not withdrawn.
    held: BTreeSet<SessionId>,
    /// Whether the relay has sent Snapshot complete.
    complete: bool,
}

impl Rules {
    /// Judges a record the hub monitor sends.
    pub(crate) fn hub(&mut self, record: &HubRecord) -> Verdict {
        let Some(lane) = self.live.get_mut(&record.stream) else {
            return self.outside(record.stream, &record.record);
        };
        let allowed = lane.allows(&record.record);
        lane.closed |= allowed && matches!(record.record, HubStream::Close);
        Verdict::of(allowed)
    }

    /// Judges a record from the hub monitor on a stream that is not live:
    /// one that begins the next stream, one that crossed the ending of a
    /// stream, or one out of turn.
    fn outside(&mut self, stream: u32, record: &HubStream) -> Verdict {
        if let HubStream::Open(_) = record {
            return self.open(stream);
        }
        match (1..=self.last).contains(&stream) {
            true => Verdict::Crossed,
            false => Verdict::Broken,
        }
    }

    /// Begins `stream`, when it is the next number.
    fn open(&mut self, stream: u32) -> Verdict {
        if Some(stream) != self.last.checked_add(1) {
            return Verdict::Broken;
        }
        self.last = stream;
        self.live.insert(stream, Lane::default());
        Verdict::Accept
    }

    /// Judges a record the relay sends.
    pub(crate) fn relay(&mut self, record: &RelayRecord) -> Verdict {
        match record {
            RelayRecord::Advertisement(advertisement) => self.advertise(advertisement),
            RelayRecord::Stream { stream, record } => self.report(*stream, Step::of(record)),
        }
    }

    /// Judges a relay's record on a session stream that needs and does
    /// `step`.
    fn report(&mut self, stream: u32, step: Step) -> Verdict {
        let Some(lane) = self.live.get_mut(&stream) else {
            return Verdict::Broken;
        };
        if lane.answered != step.after_answer {
            return Verdict::Broken;
        }
        lane.answered = true;
        let closed = lane.closed;
        if step.ends {
            self.live.remove(&stream);
            return Verdict::Accept;
        }
        match closed {
            true => Verdict::Discarded,
            false => Verdict::Accept,
        }
    }

    /// Judges an advertisement.
    fn advertise(&mut self, advertisement: &Advertisement) -> Verdict {
        let accepted = match advertisement {
            Advertisement::Session(session) => self.hold(session.id),
            Advertisement::Withdrawn(id) => self.held.remove(id),
            Advertisement::SnapshotComplete => !std::mem::replace(&mut self.complete, true),
        };
        Verdict::of(accepted)
    }

    /// Holds the session `id`, unless that would make more than 128.
    fn hold(&mut self, id: SessionId) -> bool {
        let room = self.held.contains(&id) || self.held.len() < SESSIONS_MOST;
        if room {
            self.held.insert(id);
        }
        room
    }
}
