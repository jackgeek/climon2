//! The hub monitor's end of a byte stream: the dialler.

use crate::keys::{PublicKey, StaticKey};
use crate::link::{HubLink, HubRun as LinkRun, Link, Record, Refused};
use crate::opening::{
    Answer, Arrival, Awaited, Opened, Opening, Purpose, REPLY_SIZE, REQUEST_SIZE, Reply, Request,
    Revisions, Transcript,
};
use crate::pairing::hub::{HubRun as PairingRunAtHub, Offer};
use crate::pairing::{Introduction, PairingRun, Pin, RUN_WAIT};
use crate::record::{Hello, HubClose};
use crate::{End, Input, Output, Randomness, Time};
use std::time::Duration;

/// How long a hub monitor waits for the whole opening reply after it sends
/// its opening request.
const REPLY_WAIT: Duration = Duration::from_secs(10);

/// The hub monitor's end of one connection. It runs the opening exchange,
/// and then a pairing run when it was made to pair, or the link when it
/// holds the keys of a pairing.
pub struct HubEnd {
    purpose: Purpose,
    revisions: Revisions,
    /// The opening request as sent and the reply awaited, once sent.
    sent: Option<([u8; REQUEST_SIZE], Awaited<REPLY_SIZE>)>,
    opening: Opening,
    /// The moment this end dialled: its first step.
    dialled: Option<Time>,
    /// What this end pairs with, until its pairing run starts.
    offer: Option<Offer>,
    /// The pairing run, once the opening exchange has opened for one.
    run: Option<PairingRunAtHub>,
    /// What follows the opening exchange on a link connection, when this
    /// end holds the keys of a pairing.
    link: Option<LinkRun>,
}

/// The link of an end that holds no keys: it never starts.
static NO_LINK: HubLink = Link::Pending;

impl HubEnd {
    /// An end that connects for `purpose` and supports `revisions`. It
    /// runs the opening exchange and nothing after it; `pair` makes the end
    /// that pairs and `dialling` the end that links.
    pub fn new(purpose: Purpose, revisions: Revisions) -> Self {
        Self {
            purpose,
            revisions,
            sent: None,
            opening: Opening::Pending,
            dialled: None,
            offer: None,
            run: None,
            link: None,
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

    /// An end that connects to link with a relay it has paired with. `own`
    /// is its static key for that relay, `relay` the relay's pinned key and
    /// `hello` what it states once the handshake is done.
    pub fn dialling(revisions: Revisions, own: StaticKey, relay: PublicKey, hello: Hello) -> Self {
        Self {
            link: Some(LinkRun::new(own, relay, hello)),
            ..Self::new(Purpose::Link, revisions)
        }
    }

    /// How far the link has got, and how it ended.
    pub fn link(&self) -> &HubLink {
        self.link.as_ref().map_or(&NO_LINK, LinkRun::link)
    }

    /// Queues one record for the relay on `stream`. It is sent at the next
    /// step.
    pub fn send(&mut self, stream: u32, tag: u8, body: &[u8]) -> Result<(), Refused> {
        let link = self.link.as_mut().ok_or(Refused::NotLinked)?;
        link.send(stream, tag, body)
    }

    /// Ends the established link with a Close that gives `reason`. The next
    /// step sends it and closes the byte stream.
    pub fn close(&mut self, reason: HubClose) -> Result<(), Refused> {
        let link = self.link.as_mut().ok_or(Refused::NotLinked)?;
        link.close(reason)
    }

    /// Takes the records the relay has sent since the last call, in the
    /// order they ended.
    pub fn records(&mut self) -> Vec<Record> {
        self.link.as_mut().map(LinkRun::records).unwrap_or_default()
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
        self.dialled = Some(now);
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
    fn step(&mut self, now: Time, input: Input<'_>, randomness: &mut dyn Randomness) -> Output {
        if let Some(run) = &mut self.run {
            return run.step(now, input);
        }
        match self.link.as_mut().filter(|link| link.begun()) {
            Some(link) => link.step(now, input),
            None => self.opening_step(now, input, randomness),
        }
    }
}

impl HubEnd {
    /// One step of the opening exchange, and the start of what follows it.
    fn opening_step(
        &mut self,
        now: Time,
        input: Input<'_>,
        randomness: &mut dyn Randomness,
    ) -> Output {
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
        self.follow(now, randomness)
    }

    /// What follows the opening exchange once it has ended.
    fn follow(&mut self, now: Time, randomness: &mut dyn Randomness) -> Output {
        let dialled = self.dialled.unwrap_or(now);
        if let Some(output) = self.start_pairing(dialled, randomness) {
            return output;
        }
        match &mut self.link {
            Some(link) => link.follow(&self.opening, dialled, randomness),
            None => Output {
                close: !matches!(self.opening, Opening::Opened(_)),
                ..Output::default()
            },
        }
    }

    /// Starts the pairing run of an end made to pair, once the opening
    /// exchange has chosen a link revision: the hub monitor's share goes
    /// out, and the run must end 30 seconds after `dialled`.
    fn start_pairing(&mut self, dialled: Time, randomness: &mut dyn Randomness) -> Option<Output> {
        let Opening::Opened(opened) = &self.opening else {
            return None;
        };
        let transcript = opened.transcript.clone();
        let deadline = Time::since_start(dialled.elapsed().saturating_add(RUN_WAIT));
        let (run, bytes) =
            PairingRunAtHub::start(self.offer.take()?, transcript, deadline, randomness);
        self.run = Some(run);
        Some(Output {
            bytes,
            close: false,
            deadline: Some(deadline),
        })
    }
}
