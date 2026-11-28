//! The order rules of one link connection's records after Hello: how
//! streams are numbered, begin and end, and what the relay may advertise.
//! Each end holds one [`Rules`] and judges by it every record that crosses,
//! either way: what it is asked to send, and what arrives.

use crate::link::Refused;
use crate::stream::{Advertisement, HubRecord, HubStream, RelayRecord, RelayStream, SessionId};
use std::collections::{BTreeMap, BTreeSet};

/// The most sessions a relay may have advertised at once.
const SESSIONS_MOST: usize = 128;
/// The most credit one stream may hold, in bytes.
const CREDIT_MOST: u64 = u32::MAX as u64;

/// What the rules make of one record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// It is in turn: it changes what the rules allow next.
    Accept,
    /// A record from the hub monitor on a stream that has ended, which may
    /// have crossed the ending on the wire. Its receiver ignores it; the
    /// hub monitor does not send it.
    Crossed,
    /// A record from the relay on a stream the hub monitor has closed. It
    /// may be sent, and the hub monitor discards it.
    Discarded,
    /// It breaks a rule of the link.
    Broken,
    /// It breaks an order rule of one live session stream.
    OutOfOrder,
    /// It breaks a credit rule of one live session stream: an Event longer
    /// than the stream's credit, or a Credit of nothing or past the most a
    /// stream holds.
    Overdrawn,
}

impl Verdict {
    /// The verdict on a record that keeps a rule, or breaks one of the
    /// link.
    fn of(kept: bool) -> Self {
        match kept {
            true => Self::Accept,
            false => Self::Broken,
        }
    }

    /// What an end's caller is told when it asks to send the record.
    pub(crate) fn sent(self) -> Result<(), Refused> {
        match self {
            Self::Accept | Self::Discarded => Ok(()),
            Self::Overdrawn => Err(Refused::Credit),
            Self::Crossed | Self::Broken | Self::OutOfOrder => Err(Refused::OutOfTurn),
        }
    }
}

/// How far one session stream has got.
#[derive(Clone, Copy, Debug, Default)]
struct Lane {
    /// The relay has answered Open stream with Stream opened.
    answered: bool,
    /// The hub monitor has sent Close stream.
    closed: bool,
    /// The bytes of events the hub monitor has granted and the relay has
    /// not yet sent.
    credit: u64,
}

impl Lane {
    /// A stream that Open stream began with `credit`.
    fn new(credit: u32) -> Self {
        Self {
            credit: credit.into(),
            ..Self::default()
        }
    }

    /// Judges a record the hub monitor sends on the stream. Open stream on
    /// a live stream breaks the numbering rule, a rule of the link.
    fn hub(&mut self, record: &HubStream) -> Verdict {
        if let HubStream::Open(_) = record {
            return Verdict::Broken;
        }
        if !self.takes(record) {
            return Verdict::OutOfOrder;
        }
        self.apply(record)
    }

    /// Whether the hub monitor may send `record`, which is not Open
    /// stream, now: Close stream until it has sent one, and anything else
    /// only between the relay's answer and its own Close stream.
    fn takes(self, record: &HubStream) -> bool {
        let in_turn = matches!(record, HubStream::Close) || self.answered;
        in_turn && !self.closed
    }

    /// Does what an in-turn record of the hub monitor's does.
    fn apply(&mut self, record: &HubStream) -> Verdict {
        match record {
            HubStream::Close => self.closed = true,
            HubStream::Credit(amount) => return self.grant(*amount),
            _ => {}
        }
        Verdict::Accept
    }

    /// Adds `amount` to the stream's credit, which must be at least 1 and
    /// leave the credit within the most a stream holds.
    fn grant(&mut self, amount: u32) -> Verdict {
        let credit = self.credit + u64::from(amount);
        if amount == 0 || credit > CREDIT_MOST {
            return Verdict::Overdrawn;
        }
        self.credit = credit;
        Verdict::Accept
    }

    /// Judges a relay's record on the stream that needs and does `step`.
    /// One the hub monitor has closed the stream to is discarded, except
    /// the record that ends the stream.
    fn relay(&mut self, step: &Step) -> Verdict {
        if self.answered != step.after_answer {
            return Verdict::OutOfOrder;
        }
        let Some(credit) = self.credit.checked_sub(step.cost) else {
            return Verdict::Overdrawn;
        };
        self.credit = credit;
        self.answered = true;
        match step.ends || !self.closed {
            true => Verdict::Accept,
            false => Verdict::Discarded,
        }
    }
}

/// What a relay's record on a session stream needs, and does.
struct Step {
    /// Whether the relay must have answered Open stream before it.
    after_answer: bool,
    /// Whether it ends the stream.
    ends: bool,
    /// The credit it uses: an Event's length, and nothing for any other.
    cost: u64,
}

impl Step {
    /// The step of `record`, which is `length` bytes long, tag included.
    fn of(record: &RelayStream, length: usize) -> Self {
        let (after_answer, ends, cost) = match record {
            RelayStream::Opened(_) => (false, false, 0),
            RelayStream::Refused(_) => (false, true, 0),
            RelayStream::Ended(_) => (true, true, 0),
            RelayStream::Event(_) => (true, false, length as u64),
            RelayStream::RequestRefused(_) => (true, false, 0),
        };
        Self {
            after_answer,
            ends,
            cost,
        }
    }
}

/// The order rules of one connection.
#[derive(Default)]
pub(crate) struct Rules {
    /// The number of the last stream opened: 0 before the first.
    last: u32,
    /// The streams that have begun and not ended.
    live: BTreeMap<u32, Lane>,
    /// The sessions the relay has advertised and not withdrawn.
    held: BTreeSet<SessionId>,
    /// Whether the relay has sent Snapshot complete.
    complete: bool,
}

impl Rules {
    /// Judges a record the hub monitor sends.
    pub(crate) fn hub(&mut self, record: &HubRecord) -> Verdict {
        match self.live.get_mut(&record.stream) {
            Some(lane) => lane.hub(&record.record),
            None => self.outside(record.stream, &record.record),
        }
    }

    /// Judges a record from the hub monitor on a stream that is not live:
    /// one that begins the next stream, one that crossed the ending of a
    /// stream, or one out of turn. Stream 0 is never live, and never
    /// carries a record of the hub monitor's but Hello and Close.
    fn outside(&mut self, stream: u32, record: &HubStream) -> Verdict {
        if let HubStream::Open(open) = record {
            return self.open(stream, open.credit);
        }
        match stream <= self.last {
            true => Verdict::Crossed,
            false => Verdict::Broken,
        }
    }

    /// Begins `stream` with `credit`, when it is the next number.
    fn open(&mut self, stream: u32, credit: u32) -> Verdict {
        if Some(stream) != self.last.checked_add(1) {
            return Verdict::Broken;
        }
        self.last = stream;
        self.live.insert(stream, Lane::new(credit));
        Verdict::Accept
    }

    /// Judges a record the relay sends, `length` bytes long, tag included.
    pub(crate) fn relay(&mut self, record: &RelayRecord, length: usize) -> Verdict {
        match record {
            RelayRecord::Advertisement(advertisement) => self.advertise(advertisement),
            RelayRecord::Stream { stream, record } => {
                self.report(*stream, Step::of(record, length))
            }
        }
    }

    /// Judges a relay's record on a session stream that needs and does
    /// `step`. One that ends the stream in turn removes it.
    fn report(&mut self, stream: u32, step: Step) -> Verdict {
        let Some(lane) = self.live.get_mut(&stream) else {
            return Verdict::Broken;
        };
        let verdict = lane.relay(&step);
        if step.ends && verdict == Verdict::Accept {
            self.live.remove(&stream);
        }
        verdict
    }

    /// Judges an advertisement.
    fn advertise(&mut self, advertisement: &Advertisement) -> Verdict {
        let accepted = match advertisement {
            Advertisement::Session(session) => self.hold(session.id),
            Advertisement::Withdrawn(id) => self.held.remove(id),
            Advertisement::SnapshotComplete => !std::mem::replace(&mut self.complete, true),
        };
        Verdict::of(accepted)
    }

    /// Holds the session `id`, unless that would make more than 128.
    fn hold(&mut self, id: SessionId) -> bool {
        let room = self.held.contains(&id) || self.held.len() < SESSIONS_MOST;
        if room {
            self.held.insert(id);
        }
        room
    }
}
