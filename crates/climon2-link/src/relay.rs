//! The relay's end of a byte stream: the listener.

use crate::opening::{
    Answer, Arrival, Awaited, Opened, Opening, Purpose, REQUEST_SIZE, Reply, Request, Revisions,
    Transcript,
};
use crate::pairing::invitation::Seat;
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
    /// purpose. A hub monitor that came to pair starts a pairing run, or is
    /// refused when the invitation allows none.
    fn answer(&mut self, now: Time, bytes: &[u8; REQUEST_SIZE]) -> (Opening, Vec<u8>) {
        let Some(request) = Request::decode(bytes) else {
            return (Opening::Failed, Vec::new());
        };
        let seat = self.seat(now, &request);
        let answer = self.choose(&request, seat.is_some());
        let reply = Reply {
            answer,
            revisions: self.revisions.clone(),
        }
        .encode();
        let transcript = Transcript::new(bytes, &reply);
        self.run = self.run_for(seat, &transcript);
        let opening = outcome(request, answer, transcript);
        (opening, reply.to_vec())
    }

    /// A seat in a pairing run at `now` for a hub monitor that came to pair
    /// and shares a revision, if the invitation allows one.
    fn seat(&self, now: Time, request: &Request) -> Option<Seat> {
        self.revisions.highest_shared(&request.revisions)?;
        let pairs = request.purpose == Purpose::Pair;
        pairs.then(|| self.invitations.seat(now)).flatten()
    }

    /// What the reply to `request` says: the highest shared revision, or
    /// why there is none to use. `seated` says whether a pairing run has a
    /// seat.
    fn choose(&self, request: &Request, seated: bool) -> Answer {
        match self.revisions.highest_shared(&request.revisions) {
            None => Answer::NoSharedRevision,
            Some(_) if refuses(request.purpose, seated) => Answer::PairingUnavailable,
            Some(revision) => Answer::Chosen(revision),
        }
    }

    /// The pairing run that holds `seat`, over `transcript`. It must end 30
    /// seconds after the connection was accepted.
    fn run_for(&self, seat: Option<Seat>, transcript: &Transcript) -> Option<RelayRun> {
        let seat = seat?;
        let opening = self.request.as_ref()?.deadline;
        let deadline = opening.elapsed().saturating_add(RUN_WAIT - REQUEST_WAIT);
        let deadline = Time::since_start(deadline);
        Some(RelayRun::new(seat, transcript.clone(), deadline))
    }

    /// One step while the opening exchange runs, and the step that ends it.
    fn step_opening(
        &mut self,
        now: Time,
        input: Input<'_>,
        randomness: &mut dyn Randomness,
    ) -> Output {
        if self.opening != Opening::Pending {
            return Output::default();
        }
        let request = self
            .request
            .get_or_insert_with(|| Awaited::within(now, REQUEST_WAIT));
        let (opening, bytes) = match request.step(now, input) {
            Arrival::Waiting(deadline) => return waiting(deadline),
            Arrival::Whole(bytes) => self.answer(now, &bytes),
            Arrival::Never => (Opening::Failed, Vec::new()),
        };
        self.opening = opening;
        self.opened(now, bytes, randomness)
    }

    /// The output of the step that ended the opening exchange, which sends
    /// `bytes`. What arrived behind the opening request is the start of the
    /// hub monitor's share when a pairing run has started.
    fn opened(&mut self, now: Time, mut bytes: Vec<u8>, randomness: &mut dyn Randomness) -> Output {
        let surplus = self.request.as_ref().map(Awaited::surplus);
        if let Some((run, surplus)) = self.run.as_mut().zip(surplus) {
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

/// Whether a hub monitor that comes for `purpose` is refused a pairing run,
/// given whether one has a seat.
fn refuses(purpose: Purpose, seated: bool) -> bool {
    purpose == Purpose::Pair && !seated
}

/// The output of a step that waits for more of the opening request until
/// `deadline`.
fn waiting(deadline: Time) -> Output {
    Output {
        deadline: Some(deadline),
        ..Output::default()
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
    fn step(&mut self, now: Time, input: Input<'_>, randomness: &mut dyn Randomness) -> Output {
        match &mut self.run {
            Some(run) => run.step(now, input, randomness),
            None => self.step_opening(now, input, randomness),
        }
    }
}
