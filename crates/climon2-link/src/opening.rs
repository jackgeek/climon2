//! The opening exchange (ADR 0010): the first message each way on every
//! connection, frozen for ever. The hub monitor's opening request states its
//! purpose and the link revisions it supports; the relay's opening reply
//! chooses the highest revision both support, or refuses. The layouts are
//! those of the protocol's opening request and opening reply pages.

use crate::{Input, Time};
use std::time::Duration;

/// The size of an opening request on the wire.
pub(crate) const REQUEST_SIZE: usize = 26;
/// The size of an opening reply on the wire.
pub(crate) const REPLY_SIZE: usize = 20;
/// What every opening request starts with: `c2link`, a carriage return and a
/// line feed.
const MARKER: [u8; 8] = *b"c2link\r\n";
/// How many link revisions one opening message has room for.
const SLOTS: usize = 8;
/// The size of a revision list on the wire: its count, then every slot.
const LIST_SIZE: usize = 1 + 2 * SLOTS;

/// Why a hub monitor connects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// To pair, using a link invitation.
    Pair,
    /// To link, using the keys a pairing pinned.
    Link,
}

/// Whether a relay can start a pairing run on a new connection. Its caller
/// knows: a run needs a live link invitation and no other run in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pairing {
    /// A pairing run can start.
    Available,
    /// No pairing run can start now.
    Unavailable,
}

/// The link revisions one end supports: one to eight of them, ascending,
/// with no zero and no repeat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revisions(Vec<u16>);

impl Revisions {
    /// The list `revisions`, or nothing when it is not one to eight
    /// revisions, ascending, with no zero and no repeat.
    pub fn new(revisions: &[u16]) -> Option<Self> {
        let ascending = revisions.windows(2).all(|pair| pair[0] < pair[1]);
        let valid = revisions.len() <= SLOTS && revisions.first()? != &0 && ascending;
        valid.then(|| Self(revisions.to_vec()))
    }

    /// The revisions, ascending.
    pub fn as_slice(&self) -> &[u16] {
        &self.0
    }

    /// The highest revision in both lists, if they share one.
    pub(crate) fn highest_shared(&self, other: &Self) -> Option<u16> {
        let shared = |revision: &u16| other.0.contains(revision);
        self.0.iter().rev().copied().find(shared)
    }

    /// The list as an opening message carries it: the count, the revisions
    /// in the first slots and zero in every unused slot.
    fn encode(&self) -> [u8; LIST_SIZE] {
        let mut bytes = [0; LIST_SIZE];
        bytes[0] = self.0.len() as u8;
        for (slot, revision) in bytes[1..].chunks_exact_mut(2).zip(&self.0) {
            slot.copy_from_slice(&revision.to_be_bytes());
        }
        bytes
    }

    /// The list an opening message carries, or nothing when it breaks the
    /// layout.
    fn decode(bytes: &[u8]) -> Option<Self> {
        let (count, slots) = bytes.split_first()?;
        let slots: Vec<u16> = slots
            .chunks_exact(2)
            .map(|slot| u16::from_be_bytes([slot[0], slot[1]]))
            .collect();
        let (used, unused) = slots.split_at_checked(usize::from(*count))?;
        let list = Self::new(used)?;
        unused.iter().all(|slot| *slot == 0).then_some(list)
    }
}

/// An opening request: what a hub monitor sends first.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Request {
    pub(crate) purpose: Purpose,
    pub(crate) revisions: Revisions,
}

impl Request {
    pub(crate) fn encode(&self) -> [u8; REQUEST_SIZE] {
        let purpose = match self.purpose {
            Purpose::Pair => 1,
            Purpose::Link => 2,
        };
        let mut bytes = [0; REQUEST_SIZE];
        bytes[..8].copy_from_slice(&MARKER);
        bytes[8] = purpose;
        bytes[9..].copy_from_slice(&self.revisions.encode());
        bytes
    }

    /// The request in `bytes`, or nothing when they break the layout in any
    /// way.
    pub(crate) fn decode(bytes: &[u8; REQUEST_SIZE]) -> Option<Self> {
        let purpose = match bytes[8] {
            1 => Purpose::Pair,
            2 => Purpose::Link,
            _ => return None,
        };
        let revisions = Revisions::decode(&bytes[9..])?;
        (bytes[..8] == MARKER).then_some(Self { purpose, revisions })
    }
}

/// What an opening reply says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// The connection continues in this link revision.
    Chosen(u16),
    /// The two ends share no link revision.
    NoSharedRevision,
    /// The relay can start no pairing run now.
    PairingUnavailable,
}

/// An opening reply: the relay's answer and the link revisions it supports.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Reply {
    pub(crate) answer: Answer,
    pub(crate) revisions: Revisions,
}

impl Reply {
    pub(crate) fn encode(&self) -> [u8; REPLY_SIZE] {
        let (result, chosen) = match self.answer {
            Answer::Chosen(revision) => (1, revision),
            Answer::NoSharedRevision => (2, 0),
            Answer::PairingUnavailable => (3, 0),
        };
        let mut bytes = [0; REPLY_SIZE];
        bytes[0] = result;
        bytes[1..3].copy_from_slice(&chosen.to_be_bytes());
        bytes[3..].copy_from_slice(&self.revisions.encode());
        bytes
    }

    /// The reply in `bytes`, or nothing when they break the layout: an
    /// unknown result, a refusal that names a revision, or a bad list.
    pub(crate) fn decode(bytes: &[u8; REPLY_SIZE]) -> Option<Self> {
        let answer = Answer::decode(bytes[0], u16::from_be_bytes([bytes[1], bytes[2]]))?;
        let revisions = Revisions::decode(&bytes[3..])?;
        Some(Self { answer, revisions })
    }
}

impl Answer {
    /// The answer an opening reply's result byte and chosen revision give,
    /// or nothing for an unknown result or a refusal that names a revision.
    fn decode(result: u8, chosen: u16) -> Option<Self> {
        match (result, chosen) {
            (1, revision) => Some(Self::Chosen(revision)),
            (2, 0) => Some(Self::NoSharedRevision),
            (3, 0) => Some(Self::PairingUnavailable),
            _ => None,
        }
    }
}

/// The transcript of an opening exchange: the 26 bytes of the opening
/// request followed by the 20 bytes of the opening reply, exactly as they
/// crossed the byte stream. The key exchange that follows takes it as
/// input, so editing either message makes that exchange fail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript([u8; REQUEST_SIZE + REPLY_SIZE]);

impl Transcript {
    pub(crate) fn new(request: &[u8; REQUEST_SIZE], reply: &[u8; REPLY_SIZE]) -> Self {
        let mut bytes = [0; REQUEST_SIZE + REPLY_SIZE];
        bytes[..REQUEST_SIZE].copy_from_slice(request);
        bytes[REQUEST_SIZE..].copy_from_slice(reply);
        Self(bytes)
    }

    /// The request's bytes, then the reply's.
    pub fn as_bytes(&self) -> &[u8; REQUEST_SIZE + REPLY_SIZE] {
        &self.0
    }
}

/// An opening exchange that chose a link revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    /// Why the hub monitor connected.
    pub purpose: Purpose,
    /// The link revision the connection continues in.
    pub revision: u16,
    /// Both opening messages, for the key exchange that follows.
    pub transcript: Transcript,
}

/// How far one end's opening exchange has got, and how it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opening {
    /// The exchange has not finished.
    Pending,
    /// A link revision was chosen and the connection continues in it.
    Opened(Opened),
    /// The two ends share no link revision. `theirs` is the list the other
    /// end supports; the connection is closed.
    Incompatible {
        /// The link revisions the other end supports.
        theirs: Revisions,
    },
    /// The hub monitor came to pair and the relay can start no pairing run
    /// now. The connection is closed.
    PairingUnavailable,
    /// The other end's opening message was malformed, partial or late. The
    /// connection is closed and nothing was sent in answer to it. For a hub
    /// monitor this is a failed dial.
    Failed,
}

/// A message of exactly `N` bytes that must have arrived by a deadline.
pub(crate) struct Awaited<const N: usize> {
    /// The last moment at which the message's bytes are in time.
    pub(crate) deadline: Time,
    arrived: Vec<u8>,
}

/// What became of an awaited message after one step.
pub(crate) enum Arrival<const N: usize> {
    /// Part of it is still to come, by this deadline.
    Waiting(Time),
    /// All of it is here.
    Whole([u8; N]),
    /// It will never be whole: the deadline passed or the other end closed.
    Never,
}

impl<const N: usize> Awaited<N> {
    /// A message due within `wait` of `now`.
    pub(crate) fn within(now: Time, wait: Duration) -> Self {
        Self {
            deadline: Time::since_start(now.elapsed().saturating_add(wait)),
            arrived: Vec::new(),
        }
    }

    /// Takes one step's input. Bytes that arrive after the deadline are
    /// late, and a message still partial at the deadline never completes.
    /// Bytes past the message's end are not part of it and are dropped.
    pub(crate) fn step(&mut self, now: Time, input: Input<'_>) -> Arrival<N> {
        if now > self.deadline || input == Input::Closed {
            return Arrival::Never;
        }
        self.take(input);
        self.arrival(now)
    }

    /// Keeps the bytes `input` brings, if any.
    fn take(&mut self, input: Input<'_>) {
        if let Input::Bytes(bytes) = input {
            self.arrived.extend(bytes);
        }
    }

    /// What the bytes kept so far make of the message at `now`, which is
    /// not past the deadline.
    fn arrival(&self, now: Time) -> Arrival<N> {
        match self.arrived.first_chunk() {
            Some(whole) => Arrival::Whole(*whole),
            None if now == self.deadline => Arrival::Never,
            None => Arrival::Waiting(self.deadline),
        }
    }
}
