//! The hub monitor's end of a byte stream: the dialler.

use crate::keys::{PublicKey, StaticKey};
use crate::link::{HubLink, HubRun, Link, Record, Refused};
use crate::opening::{
    Answer, Arrival, Awaited, Opened, Opening, Purpose, REPLY_SIZE, REQUEST_SIZE, Reply, Request,
    Revisions, Transcript,
};
use crate::record::{Hello, HubClose};
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
    /// The moment this end dialled: its first step.
    dialled: Option<Time>,
    /// What follows the opening exchange on a link connection, when this
    /// end holds the keys of a pairing.
    link: Option<HubRun>,
}

/// The link of an end that holds no keys: it never starts.
static NO_LINK: HubLink = Link::Pending;

impl HubEnd {
    /// An end that connects for `purpose` and supports `revisions`.
    pub fn new(purpose: Purpose, revisions: Revisions) -> Self {
        Self {
            purpose,
            revisions,
            sent: None,
            opening: Opening::Pending,
            dialled: None,
            link: None,
        }
    }

    /// An end that connects to link with a relay it has paired with. `own`
    /// is its static key for that relay, `relay` the relay's pinned key and
    /// `hello` what it states once the handshake is done.
    pub fn dialling(revisions: Revisions, own: StaticKey, relay: PublicKey, hello: Hello) -> Self {
        Self {
            link: Some(HubRun::new(own, relay, hello)),
            ..Self::new(Purpose::Link, revisions)
        }
    }

    /// How far the link has got, and how it ended.
    pub fn link(&self) -> &HubLink {
        self.link.as_ref().map_or(&NO_LINK, HubRun::link)
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
        self.link.as_mut().map(HubRun::records).unwrap_or_default()
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
        match &mut self.link {
            Some(link) => link.follow(&self.opening, dialled, randomness),
            None => Output {
                close: !matches!(self.opening, Opening::Opened(_)),
                ..Output::default()
            },
        }
    }
}
