//! The relay's end of a pairing run: it answers the hub monitor's share
//! with its own, then says nothing more until the hub monitor has proved it
//! holds the PIN.

use super::invitation::Seat;
use super::keys::{Confirmed, Sender};
use super::{
    Arrived, CONFIRMATION_SIZES, Framed, PairingRun, SHARE_SIZES, closing, cpace, exchange,
    share_message,
};
use crate::opening::Transcript;
use crate::{Input, Output, Randomness, Time};

/// One pairing run at the relay, from a chosen link revision to the close.
pub(crate) struct RelayRun {
    seat: Seat,
    transcript: Transcript,
    arrived: Arrived,
    /// What the hub monitor's confirmation must confirm, once both shares
    /// have crossed.
    confirmed: Option<Confirmed>,
    pub(crate) outcome: PairingRun,
}

impl RelayRun {
    /// A run under `seat` that must end by `deadline`.
    pub(crate) fn new(seat: Seat, transcript: Transcript, deadline: Time) -> Self {
        Self {
            seat,
            transcript,
            arrived: Arrived {
                deadline,
                bytes: Vec::new(),
            },
            confirmed: None,
            outcome: PairingRun::Pending,
        }
    }

    pub(crate) fn step(
        &mut self,
        now: Time,
        input: Input<'_>,
        randomness: &mut dyn Randomness,
    ) -> Output {
        if self.outcome != PairingRun::Pending {
            return Output::default();
        }
        let going = self.arrived.take(now, input) && self.seat.live(now);
        let sent = match going {
            true => self.advance(now, randomness),
            false => None,
        };
        let output = match (&self.outcome, sent) {
            (PairingRun::Paired(_), Some(bytes)) => Some(closing(bytes)),
            (_, Some(bytes)) => self.arrived.waiting(now, bytes),
            (_, None) => None,
        };
        output.unwrap_or_else(|| self.fail(now))
    }

    /// Closes without a byte: the run failed, and counts once the relay's
    /// share has been sent.
    fn fail(&mut self, now: Time) -> Output {
        self.seat.failed(now);
        self.outcome = PairingRun::Failed {
            counted: self.seat.counted(),
        };
        closing(Vec::new())
    }

    /// Reads what has arrived and gives the bytes to send, or nothing when
    /// the run has failed.
    fn advance(&mut self, now: Time, randomness: &mut dyn Randomness) -> Option<Vec<u8>> {
        let mut sent = Vec::new();
        if self.confirmed.is_none() {
            match self.arrived.message(SHARE_SIZES) {
                Framed::Partial => return Some(sent),
                Framed::Malformed => return None,
                Framed::Whole(share) => sent = self.answer(&share, randomness)?,
            }
        }
        match self.arrived.message(CONFIRMATION_SIZES) {
            Framed::Partial => Some(sent),
            Framed::Malformed => None,
            Framed::Whole(body) => {
                sent.extend(self.confirm(now, &body)?);
                Some(sent)
            }
        }
    }

    /// Answers the hub monitor's share with the relay's own. From here the
    /// run counts as an attempt unless it succeeds.
    fn answer(&mut self, hub_share: &[u8], randomness: &mut dyn Randomness) -> Option<Vec<u8>> {
        let (relay_share, key) = exchange(&self.seat.pin, &self.transcript, |inputs| {
            cpace::respond(inputs, hub_share, randomness)
        })?;
        self.seat.count();
        self.confirmed = Some(Confirmed {
            key,
            transcript: self.transcript.clone(),
            hub_share: hub_share.try_into().ok()?,
            relay_share,
        });
        Some(share_message(&relay_share))
    }

    /// Checks the hub monitor's confirmation and opens its box before
    /// anything is sent. On success the invitation is consumed and the
    /// relay's own confirmation is the answer.
    fn confirm(&mut self, now: Time, body: &[u8]) -> Option<Vec<u8>> {
        let confirmed = self.confirmed.as_ref()?;
        let hub = confirmed.open(Sender::Hub, body)?;
        let message = confirmed.message(Sender::Relay, &self.seat.relay);
        self.seat.pair(now, &hub).then(|| {
            self.outcome = PairingRun::Paired(hub);
            message
        })
    }
}
