//! The hub monitor's end of a byte stream: the dialler.

use crate::opening::{
    Answer, Arrival, Awaited, Opened, Opening, Purpose, REPLY_SIZE, REQUEST_SIZE, Reply, Request,
    Revisions, Transcript,
};
use crate::pairing::hub::{HubRun, Offer};
use crate::pairing::{Introduction, PairingRun, Pin, RUN_WAIT};
use crate::{End, Input, Output, Randomness, Time};
use std::time::Duration;

/// How long a hub monitor waits for the whole opening reply after it sends
/// its opening request.
const REPLY_WAIT: Duration = Duration::from_secs(10);

/// The hub monitor's end of one connection. It runs the opening exchange,
/// and then a pairing run when it was made to pair. What arrives on a
/// connection opened to link is dropped so far.
pub struct HubEnd {
    purpose: Purpose,
    revisions: Revisions,
    /// The opening request as sent and the reply awaited, once sent.
    sent: Option<([u8; REQUEST_SIZE], Awaited<REPLY_SIZE>)>,
    opening: Opening,
    /// What this end pairs with, until its pairing run starts.
    offer: Option<Offer>,
    /// The pairing run, once the opening exchange has opened for one.
    run: Option<HubRun>,
}

impl HubEnd {
    /// An end that connects for `purpose` and supports `revisions`. It
    /// runs the opening exchange and nothing after it; `pair` makes the end
    /// that pairs.
    pub fn new(purpose: Purpose, revisions: Revisions) -> Self {
        Self {
            purpose,
            revisions,
            sent: None,
            opening: Opening::Pending,
            offer: None,
            run: None,
        }
    }

    /// An end that connects to pair, with the `pin` of the relay's link
    /// invitation. Once the relay has proved it holds the same PIN, the
    /// relay receives `introduction`: the public key of the key pair this
    /// hub monitor made for this pairing alone, and its name.
    pub fn pair(revisions: Revisions, pin: Pin, introduction: Introduction) -> Self {
        Self {
            offer: Some(Offer { pin, introduction }),
            ..Self::new(Purpose::Pair, revisions)
        }
    }

    /// How far the pairing run on this connection has got, and how it
    /// ended. `Paired` holds the relay's key and name; the hub monitor
    /// reports "paired" only once its first link to that key succeeds.
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

    /// Starts the pairing run of an end made to pair, once the opening
    /// exchange has chosen a link revision: the hub monitor's share goes
    /// out, and the run must end 30 seconds after this end's first step.
    fn start_pairing(&mut self, now: Time, randomness: &mut dyn Randomness) -> Option<Output> {
        let Opening::Opened(opened) = &self.opening else {
            return None;
        };
        let (_, reply) = self.sent.as_ref()?;
        let dialled = reply.deadline.elapsed().saturating_sub(REPLY_WAIT);
        let deadline = Time::since_start(dialled.saturating_add(RUN_WAIT));
        let transcript = opened.transcript.clone();
        let (run, mut bytes) = HubRun::start(self.offer.take()?, transcript, deadline, randomness);
        let run = self.run.insert(run);
        // What arrived behind the opening reply is the start of the relay's
        // share.
        let next = run.step(now, Input::Bytes(reply.surplus()));
        bytes.extend(next.bytes);
        Some(Output { bytes, ..next })
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
    fn step(&mut self, now: Time, input: Input<'_>, randomness: &mut dyn Randomness) -> Output {
        if let Some(run) = &mut self.run {
            return run.step(now, input);
        }
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
        if let Some(output) = self.start_pairing(now, randomness) {
            return output;
        }
        Output {
            close: !matches!(self.opening, Opening::Opened(_)),
            ..Output::default()
        }
    }
}
