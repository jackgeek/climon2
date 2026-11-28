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
        let answer = self.choose(&request);
        let reply = Reply {
            answer,
            revisions: self.revisions.clone(),
        }
        .encode();
        let opening = outcome(request, answer, Transcript::new(bytes, &reply));
        (opening, reply.to_vec())
    }

    /// What the reply to `request` says: the highest shared revision, or
    /// why there is none to use.
    fn choose(&self, request: &Request) -> Answer {
        match self.revisions.highest_shared(&request.revisions) {
            None => Answer::NoSharedRevision,
            Some(_) if self.refuses(request.purpose) => Answer::PairingUnavailable,
            Some(revision) => Answer::Chosen(revision),
        }
    }

    /// Whether a hub monitor that comes for `purpose` is refused a pairing
    /// run.
    fn refuses(&self, purpose: Purpose) -> bool {
        purpose == Purpose::Pair && self.pairing == Pairing::Unavailable
    }
}

/// How this end's opening ended once it answered `request` with `answer`;
/// `transcript` is kept only when a revision was chosen.
fn outcome(request: Request, answer: Answer, transcript: Transcript) -> Opening {
    match answer {
        Answer::Chosen(revision) => Opening::Opened(Opened {
            purpose: request.purpose,
            revision,
            transcript,
        }),
        Answer::NoSharedRevision => Opening::Incompatible {
            theirs: request.revisions,
        },
        Answer::PairingUnavailable => Opening::PairingUnavailable,
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
