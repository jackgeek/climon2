//! The relay's end of a byte stream: the listener.

use crate::opening::{
    Answer, Arrival, Awaited, Opened, Opening, Purpose, REQUEST_SIZE, Reply, Request, Revisions,
    Transcript,
};
use crate::pairing::relay::RelayRun;
use crate::pairing::{Invitations, PairingRun, RUN_WAIT};
use crate::{End, Input, Output, Randomness, Time};
use std::time::Duration;

/// How long a relay waits for the whole opening request after it accepts a
/// connection.
const REQUEST_WAIT: Duration = Duration::from_secs(10);

/// The relay's end of one connection, accepted at its first step. It runs
/// the opening exchange, and then a pairing run when the hub monitor came
/// to pair. What arrives on a connection opened to link is dropped so far.
pub struct RelayEnd {
    revisions: Revisions,
    invitations: Invitations,
    /// The opening request awaited, from the first step on.
    request: Option<Awaited<REQUEST_SIZE>>,
    opening: Opening,
    /// The pairing run, once the opening exchange has opened for one.
    run: Option<RelayRun>,
}

impl RelayEnd {
    /// An end that supports `revisions`. A hub monitor that connects to
    /// pair gets a pairing run under the link invitation of `invitations`,
    /// the relay's own.
    pub fn new(revisions: Revisions, invitations: Invitations) -> Self {
        Self {
            revisions,
            invitations,
            request: None,
            opening: Opening::Pending,
            run: None,
        }
    }

    /// How far the pairing run on this connection has got, and how it
    /// ended. When it is `Paired`, the relay pins that hub monitor's key
    /// before it sends the bytes of the step that said so.
    pub fn pairing(&self) -> &PairingRun {
        match &self.run {
            Some(run) => &run.outcome,
            None => &PairingRun::Pending,
        }
    }

    /// How far the opening exchange has got, and how it ended.
    pub fn opening(&self) -> &Opening {
        &self.opening
    }

    /// Answers a whole opening request at `now`: the outcome for this end
    /// and the reply to send. A request that breaks the layout gets no
    /// reply. With no shared revision the reply says so, whatever the
    /// purpose. A hub monitor that came to pair starts a pairing run that
    /// ends 30 seconds after the connection was accepted, or is refused when
    /// the invitation allows none.
    fn answer(&mut self, now: Time, bytes: &[u8; REQUEST_SIZE]) -> (Opening, Vec<u8>) {
        let Some(request) = Request::decode(bytes) else {
            return (Opening::Failed, Vec::new());
        };
        let shared = self.revisions.highest_shared(&request.revisions);
        let seat = match (shared, request.purpose) {
            (Some(_), Purpose::Pair) => self.invitations.seat(now),
            _ => None,
        };
        let answer = match shared {
            None => Answer::NoSharedRevision,
            Some(_) if request.purpose == Purpose::Pair && seat.is_none() => {
                Answer::PairingUnavailable
            }
            Some(revision) => Answer::Chosen(revision),
        };
        let reply = Reply {
            answer,
            revisions: self.revisions.clone(),
        }
        .encode();
        let transcript = Transcript::new(bytes, &reply);
        let accepted = self.request.as_ref().map(|request| request.deadline);
        let deadline = accepted.map(|opening| {
            Time::since_start(opening.elapsed().saturating_add(RUN_WAIT - REQUEST_WAIT))
        });
        self.run = seat
            .zip(deadline)
            .map(|(seat, deadline)| RelayRun::new(seat, transcript.clone(), deadline));
        let opening = match answer {
            Answer::Chosen(revision) => Opening::Opened(Opened {
                purpose: request.purpose,
                revision,
                transcript,
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
    fn step(&mut self, now: Time, input: Input<'_>, randomness: &mut dyn Randomness) -> Output {
        if let Some(run) = &mut self.run {
            return run.step(now, input, randomness);
        }
        if self.opening != Opening::Pending {
            return Output::default();
        }
        let request = self
            .request
            .get_or_insert_with(|| Awaited::within(now, REQUEST_WAIT));
        let (opening, mut bytes) = match request.step(now, input) {
            Arrival::Waiting(deadline) => {
                return Output {
                    deadline: Some(deadline),
                    ..Output::default()
                };
            }
            Arrival::Whole(bytes) => self.answer(now, &bytes),
            Arrival::Never => (Opening::Failed, Vec::new()),
        };
        self.opening = opening;
        let surplus = self.request.as_ref().map(Awaited::surplus);
        if let Some((run, surplus)) = self.run.as_mut().zip(surplus) {
            // What arrived behind the opening request is the start of the
            // hub monitor's share.
            let next = run.step(now, Input::Bytes(surplus), randomness);
            bytes.extend(next.bytes);
            return Output { bytes, ..next };
        }
        Output {
            bytes,
            close: !matches!(self.opening, Opening::Opened(_)),
            deadline: None,
        }
    }
}
