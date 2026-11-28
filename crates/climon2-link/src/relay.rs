//! The relay's end of a byte stream: the listener.

use crate::keys::{PublicKey, StaticKey};
use crate::link::{Link, Refused, RelayLink, RelayRun as LinkRun};
use crate::opening::{
    Answer, Arrival, Awaited, Opened, Opening, Purpose, REQUEST_SIZE, Reply, Request, Revisions,
    Transcript,
};
use crate::pairing::invitation::Seat;
use crate::pairing::relay::RelayRun as PairingRunAtRelay;
use crate::pairing::{Invitations, PairingRun, RUN_WAIT};
use crate::record::{Hello, RelayClose};
use crate::stream::{HubRecord, RelayRecord};
use crate::{End, Input, Output, Randomness, Time};
use std::time::Duration;

/// How long a relay waits for the whole opening request after it accepts a
/// connection.
const REQUEST_WAIT: Duration = Duration::from_secs(10);

/// The relay's end of one connection, accepted at its first step. It runs
/// the opening exchange, and then a pairing run when the hub monitor came
/// to pair, or the link when it came to link and this end can link.
pub struct RelayEnd {
    revisions: Revisions,
    invitations: Invitations,
    /// The opening request awaited, from the first step on.
    request: Option<Awaited<REQUEST_SIZE>>,
    opening: Opening,
    /// The moment this end accepted the connection: its first step.
    accepted: Option<Time>,
    /// The pairing run, once the opening exchange has opened for one.
    run: Option<PairingRunAtRelay>,
    /// What follows the opening exchange on a link connection, when this
    /// end holds an identity key.
    link: Option<LinkRun>,
}

/// The link of an end that holds no identity key: it never starts.
static NO_LINK: RelayLink = Link::Pending;

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
            accepted: None,
            run: None,
            link: None,
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

    /// This end, able to link: `identity` is the relay's identity key,
    /// `remembered` the static public keys of the hub monitors it has
    /// paired with, and `hello` what it states to one of them.
    pub fn linking(self, identity: StaticKey, remembered: Vec<PublicKey>, hello: Hello) -> Self {
        Self {
            link: Some(LinkRun::new(identity, remembered, hello)),
            ..self
        }
    }

    /// This end, refusing the remembered hub monitor `hub`: once the
    /// handshake is done it answers that hub monitor's Hello with a Close
    /// that gives `reason`, in place of its own Hello. A later refusal of
    /// the same key replaces an earlier one. The protocol's refusals are
    /// "not paired", "on hold", "busy" and "replaced". A hub monitor the
    /// relay does not remember gets "not paired" whatever is refused, and
    /// an end that cannot link refuses nothing.
    pub fn refusing(self, hub: PublicKey, reason: RelayClose) -> Self {
        Self {
            link: self.link.map(|link| link.refusing(hub, reason)),
            ..self
        }
    }

    /// How far the link has got, and how it ended. Nothing is linked until
    /// this is `Established`: a handshake alone, which anyone who recorded
    /// one can replay, leaves it `Pending`.
    pub fn link(&self) -> &RelayLink {
        self.link.as_ref().map_or(&NO_LINK, LinkRun::link)
    }

    /// Queues one record for the hub monitor. It is sent from the next step
    /// on.
    pub fn send(&mut self, record: &RelayRecord) -> Result<(), Refused> {
        let link = self.link.as_mut().ok_or(Refused::NotLinked)?;
        link.send(record)
    }

    /// Ends the established link with a Close that gives `reason`. The next
    /// step sends it and closes the byte stream.
    pub fn close(&mut self, reason: RelayClose) -> Result<(), Refused> {
        let link = self.link.as_mut().ok_or(Refused::NotLinked)?;
        link.close(reason)
    }

    /// Takes the records the hub monitor has sent since the last call, in
    /// the order they ended.
    pub fn records(&mut self) -> Vec<HubRecord> {
        self.link.as_mut().map(LinkRun::records).unwrap_or_default()
    }

    /// Takes the session streams the hub monitor has broken a rule of since
    /// the last call, in the order it broke them. Each has ended as a
    /// stream violation and the link carries on: the relay ignores what the
    /// hub monitor sends on it, and the caller shuts the stream's client
    /// down, answers the stream if it has not, and sends Stream ended with
    /// the reason "stream violation".
    pub fn violations(&mut self) -> Vec<u32> {
        self.link
            .as_mut()
            .map(LinkRun::violations)
            .unwrap_or_default()
    }

    /// How far the opening exchange has got, and how it ended.
    pub fn opening(&self) -> &Opening {
        &self.opening
    }

    /// Answers a whole opening request at `now`, on a connection accepted
    /// at `accepted`: the outcome for this end and the reply to send. A
    /// request that breaks the layout gets no reply. With no shared
    /// revision the reply says so, whatever the purpose. A hub monitor that
    /// came to pair starts a pairing run, or is refused when the invitation
    /// allows none.
    fn answer(
        &mut self,
        now: Time,
        accepted: Time,
        bytes: &[u8; REQUEST_SIZE],
    ) -> (Opening, Vec<u8>) {
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
        self.run = seat.map(|seat| run_for(seat, &transcript, accepted));
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
}

/// The pairing run that holds `seat`, over `transcript`. It must end 30
/// seconds after the connection was `accepted`.
fn run_for(seat: Seat, transcript: &Transcript, accepted: Time) -> PairingRunAtRelay {
    let deadline = Time::since_start(accepted.elapsed().saturating_add(RUN_WAIT));
    PairingRunAtRelay::new(seat, transcript.clone(), deadline)
}

/// Whether a hub monitor that comes for `purpose` is refused a pairing run,
/// given whether one has a seat.
fn refuses(purpose: Purpose, seated: bool) -> bool {
    purpose == Purpose::Pair && !seated
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
        if let Some(run) = &mut self.run {
            return run.step(now, input, randomness);
        }
        match self.link.as_mut().filter(|link| link.begun()) {
            Some(link) => link.step(now, input, randomness),
            None => self.opening_step(now, input),
        }
    }
}

impl RelayEnd {
    /// One step of the opening exchange, and the start of what follows it.
    fn opening_step(&mut self, now: Time, input: Input<'_>) -> Output {
        if self.opening != Opening::Pending {
            return Output::default();
        }
        let accepted = *self.accepted.get_or_insert(now);
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
            Arrival::Whole(bytes) => self.answer(now, accepted, &bytes),
            Arrival::Never => (Opening::Failed, Vec::new()),
        };
        self.opening = opening;
        let deadline = self.follow(accepted);
        Output {
            bytes,
            close: !matches!(self.opening, Opening::Opened(_)),
            deadline,
        }
    }

    /// Starts what follows the opening exchange of a connection accepted
    /// at `accepted`: the wait for the hub monitor's share when a pairing
    /// run has started, or the link's own follow. Gives the deadline the
    /// wait names.
    fn follow(&mut self, accepted: Time) -> Option<Time> {
        if let Some(run) = &self.run {
            return Some(run.deadline());
        }
        let opening = &self.opening;
        self.link
            .as_mut()
            .and_then(|link| link.follow(opening, accepted))
    }
}
