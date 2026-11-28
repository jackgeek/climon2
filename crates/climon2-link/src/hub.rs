//! The hub monitor's end of a byte stream: the dialler.

use crate::opening::{
    Answer, Arrival, Awaited, Opened, Opening, Purpose, REPLY_SIZE, REQUEST_SIZE, Reply, Request,
    Revisions, Transcript,
};
use crate::{End, Input, Output, Randomness, Time};
use std::time::Duration;

/// How long a hub monitor waits for the whole opening reply after it sends
/// its opening request.
const REPLY_WAIT: Duration = Duration::from_secs(10);

/// The hub monitor's end of one connection. So far it runs the opening
/// exchange and nothing after it: what arrives once the exchange has ended
/// is dropped.
pub struct HubEnd {
    purpose: Purpose,
    revisions: Revisions,
    /// The opening request as sent and the reply awaited, once sent.
    sent: Option<([u8; REQUEST_SIZE], Awaited<REPLY_SIZE>)>,
    opening: Opening,
}

impl HubEnd {
    /// An end that connects for `purpose` and supports `revisions`.
    pub fn new(purpose: Purpose, revisions: Revisions) -> Self {
        Self {
            purpose,
            revisions,
            sent: None,
            opening: Opening::Pending,
        }
    }

    /// How far the opening exchange has got, and how it ended.
    pub fn opening(&self) -> &Opening {
        &self.opening
    }

    /// Sends the opening request and starts the wait for the reply.
    fn open(&mut self, now: Time) -> Output {
        let request = Request {
            purpose: self.purpose,
            revisions: self.revisions.clone(),
        }
        .encode();
        let reply = Awaited::within(now, REPLY_WAIT);
        let deadline = Some(reply.deadline);
        self.sent = Some((request, reply));
        Output {
            bytes: request.to_vec(),
            close: false,
            deadline,
        }
    }

    /// What a whole opening reply means to this end. A reply is a failed
    /// dial unless it is exactly what a relay holding the list it states
    /// would answer to this end's request.
    fn judge(&self, request: &[u8; REQUEST_SIZE], bytes: &[u8; REPLY_SIZE]) -> Opening {
        let Some(reply) = Reply::decode(bytes) else {
            return Opening::Failed;
        };
        let shared = self.revisions.highest_shared(&reply.revisions);
        match (reply.answer, shared) {
            (Answer::Chosen(revision), Some(highest)) if revision == highest => {
                Opening::Opened(Opened {
                    purpose: self.purpose,
                    revision,
                    transcript: Transcript::new(request, bytes),
                })
            }
            (Answer::NoSharedRevision, None) => Opening::Incompatible {
                theirs: reply.revisions,
            },
            (Answer::PairingUnavailable, _) if self.purpose == Purpose::Pair => {
                Opening::PairingUnavailable
            }
            _ => Opening::Failed,
        }
    }
}

impl End for HubEnd {
    fn step(&mut self, now: Time, input: Input<'_>, _: &mut dyn Randomness) -> Output {
        if self.opening != Opening::Pending {
            return Output::default();
        }
        let Some((request, reply)) = &mut self.sent else {
            return self.open(now);
        };
        let request = *request;
        self.opening = match reply.step(now, input) {
            Arrival::Waiting(deadline) => {
                return Output {
                    deadline: Some(deadline),
                    ..Output::default()
                };
            }
            Arrival::Whole(bytes) => self.judge(&request, &bytes),
            Arrival::Never => Opening::Failed,
        };
        Output {
            close: !matches!(self.opening, Opening::Opened(_)),
            ..Output::default()
        }
    }
}
