//! The relay's end of a byte stream: the listener.

use crate::opening::{
    Answer, Arrival, Awaited, Opened, Opening, Pairing, Purpose, REQUEST_SIZE, Reply, Request,
    Revisions, Transcript,
};
use crate::{End, Input, Output, Randomness, Time};
use std::time::Duration;

/// How long a relay waits for the whole opening request after it accepts a
/// connection.
const REQUEST_WAIT: Duration = Duration::from_secs(10);

/// The relay's end of one connection, accepted at its first step. So far it
/// runs the opening exchange and nothing after it: what arrives once the
/// exchange has ended is dropped.
pub struct RelayEnd {
    revisions: Revisions,
    pairing: Pairing,
    /// The opening request awaited, from the first step on.
    request: Option<Awaited<REQUEST_SIZE>>,
    opening: Opening,
}

impl RelayEnd {
    /// An end that supports `revisions`. `pairing` says whether a hub
    /// monitor that connects to pair can start a pairing run.
    pub fn new(revisions: Revisions, pairing: Pairing) -> Self {
        Self {
            revisions,
            pairing,
            request: None,
            opening: Opening::Pending,
        }
    }

    /// How far the opening exchange has got, and how it ended.
    pub fn opening(&self) -> &Opening {
        &self.opening
    }

    /// Answers a whole opening request: the outcome for this end and the
    /// reply to send. A request that breaks the layout gets no reply. With
    /// no shared revision the reply says so, whatever the purpose.
    fn answer(&self, bytes: &[u8; REQUEST_SIZE]) -> (Opening, Vec<u8>) {
        let Some(request) = Request::decode(bytes) else {
            return (Opening::Failed, Vec::new());
        };
        let pairing_refused =
            request.purpose == Purpose::Pair && self.pairing == Pairing::Unavailable;
        let answer = match self.revisions.highest_shared(&request.revisions) {
            None => Answer::NoSharedRevision,
            Some(_) if pairing_refused => Answer::PairingUnavailable,
            Some(revision) => Answer::Chosen(revision),
        };
        let reply = Reply {
            answer,
            revisions: self.revisions.clone(),
        }
        .encode();
        let opening = match answer {
            Answer::Chosen(revision) => Opening::Opened(Opened {
                purpose: request.purpose,
                revision,
                transcript: Transcript::new(bytes, &reply),
            }),
            Answer::NoSharedRevision => Opening::Incompatible {
                theirs: request.revisions,
            },
            Answer::PairingUnavailable => Opening::PairingUnavailable,
        };
        (opening, reply.to_vec())
    }
}

impl End for RelayEnd {
    fn step(&mut self, now: Time, input: Input<'_>, _: &mut dyn Randomness) -> Output {
        if self.opening != Opening::Pending {
            return Output::default();
        }
        let request = self
            .request
            .get_or_insert_with(|| Awaited::within(now, REQUEST_WAIT));
        let (opening, bytes) = match request.step(now, input) {
            Arrival::Waiting(deadline) => {
                return Output {
                    deadline: Some(deadline),
                    ..Output::default()
                };
            }
            Arrival::Whole(bytes) => self.answer(&bytes),
            Arrival::Never => (Opening::Failed, Vec::new()),
        };
        self.opening = opening;
        Output {
            bytes,
            close: !matches!(self.opening, Opening::Opened(_)),
            deadline: None,
        }
    }
}
