//! The relay's end of a byte stream: the listener.

use crate::keys::{PublicKey, StaticKey};
use crate::link::{Link, Record, Refused, RelayLink, RelayRun};
use crate::opening::{
    Answer, Arrival, Awaited, Opened, Opening, Pairing, Purpose, REQUEST_SIZE, Reply, Request,
    Revisions, Transcript,
};
use crate::record::{Hello, RelayClose};
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
    /// The moment this end accepted the connection: its first step.
    accepted: Option<Time>,
    /// What follows the opening exchange on a link connection, when this
    /// end holds an identity key.
    link: Option<RelayRun>,
}

/// The link of an end that holds no identity key: it never starts.
static NO_LINK: RelayLink = Link::Pending;

impl RelayEnd {
    /// An end that supports `revisions`. `pairing` says whether a hub
    /// monitor that connects to pair can start a pairing run.
    pub fn new(revisions: Revisions, pairing: Pairing) -> Self {
        Self {
            revisions,
            pairing,
            request: None,
            opening: Opening::Pending,
            accepted: None,
            link: None,
        }
    }

    /// This end, able to link: `identity` is the relay's identity key,
    /// `remembered` the static public keys of the hub monitors it has
    /// paired with, and `hello` what it states to one of them.
    pub fn linking(self, identity: StaticKey, remembered: Vec<PublicKey>, hello: Hello) -> Self {
        Self {
            link: Some(RelayRun::new(identity, remembered, hello)),
            ..self
        }
    }

    /// How far the link has got, and how it ended. Nothing is linked until
    /// this is `Established`: a handshake alone, which anyone who recorded
    /// one can replay, leaves it `Pending`.
    pub fn link(&self) -> &RelayLink {
        self.link.as_ref().map_or(&NO_LINK, RelayRun::link)
    }

    /// Queues one record for the hub monitor on `stream`. It is sent at the
    /// next step.
    pub fn send(&mut self, stream: u32, tag: u8, body: &[u8]) -> Result<(), Refused> {
        let link = self.link.as_mut().ok_or(Refused::NotLinked)?;
        link.send(stream, tag, body)
    }

    /// Ends the established link with a Close that gives `reason`. The next
    /// step sends it and closes the byte stream.
    pub fn close(&mut self, reason: RelayClose) -> Result<(), Refused> {
        let link = self.link.as_mut().ok_or(Refused::NotLinked)?;
        link.close(reason)
    }

    /// Takes the records the hub monitor has sent since the last call, in
    /// the order they ended.
    pub fn records(&mut self) -> Vec<Record> {
        self.link
            .as_mut()
            .map(RelayRun::records)
            .unwrap_or_default()
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
    fn step(&mut self, now: Time, input: Input<'_>, randomness: &mut dyn Randomness) -> Output {
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
            Arrival::Whole(bytes) => self.answer(&bytes),
            Arrival::Never => (Opening::Failed, Vec::new()),
        };
        self.opening = opening;
        let opening = &self.opening;
        let deadline = self
            .link
            .as_mut()
            .and_then(|link| link.follow(opening, accepted));
        Output {
            bytes,
            close: !matches!(self.opening, Opening::Opened(_)),
            deadline,
        }
    }
}
