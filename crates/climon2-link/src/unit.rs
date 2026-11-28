//! Units: what a link connection is made of once the handshake is done. A
//! unit is a length and one Noise transport message, and carries one
//! fragment of one record of one stream. The layout is that of the
//! protocol's shared rules.

use crate::noise::{TAG_SIZE, Transport};
use std::collections::{BTreeMap, VecDeque};
use std::ops::Bound::{Excluded, Unbounded};

/// The size of a unit's length field.
const LENGTH_SIZE: usize = 2;
/// The size of what a unit's plaintext holds before its fragment: the
/// stream and the flags.
const HEADER_SIZE: usize = 5;
/// The longest ciphertext of one unit.
const CIPHERTEXT_MOST: usize = 16_384;
/// The shortest ciphertext of one unit: a keepalive's.
const CIPHERTEXT_LEAST: usize = HEADER_SIZE + TAG_SIZE;
/// The longest fragment one unit carries.
const FRAGMENT_MOST: usize = CIPHERTEXT_MOST - CIPHERTEXT_LEAST;
/// The flag that says more of the record follows.
const MORE: u8 = 1;
/// The stream of control and advertisement.
pub(crate) const CONTROL: u32 = 0;

/// Why the units of a connection stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    /// A unit did not decrypt: it was altered, repeated, out of order or
    /// never sent by the other end.
    Undecryptable,
    /// The other end broke the protocol.
    Violation,
}

/// What the other end may send now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Allowed {
    /// The longest record, tag included.
    pub(crate) record: usize,
    /// Whether streams other than stream 0 may carry anything.
    pub(crate) streams: bool,
}

/// A whole record as it arrived: its stream and the record, tag included.
pub(crate) type Received = (u32, Vec<u8>);

/// The units of one connection, both directions.
pub(crate) struct Units {
    transport: Transport,
    /// Bytes received that have not yet formed a whole unit.
    arrived: Vec<u8>,
    /// The record each stream is part-way through receiving.
    partial: BTreeMap<u32, Vec<u8>>,
    /// The records each stream has still to send, in order. A stream with
    /// none has no entry.
    queued: BTreeMap<u32, VecDeque<Queued>>,
    /// The session stream that sent the last unit.
    turn: u32,
    /// Whether a unit has decrypted since the last call to `heard`.
    heard: bool,
}

/// A record to send, and how much of it has been sent.
struct Queued {
    record: Vec<u8>,
    sent: usize,
}

impl Units {
    pub(crate) fn new(transport: Transport) -> Self {
        Self {
            transport,
            arrived: Vec::new(),
            partial: BTreeMap::new(),
            queued: BTreeMap::new(),
            turn: CONTROL,
            heard: false,
        }
    }

    /// Queues `record` to send on `stream`, behind the records that stream
    /// has queued. A record is never empty: it starts with its tag.
    pub(crate) fn send(&mut self, stream: u32, record: Vec<u8>) {
        let queued = Queued { record, sent: 0 };
        self.queued.entry(stream).or_default().push_back(queued);
    }

    /// Up to `most` units of what is queued, as bytes to send. Stream 0
    /// goes first. The session streams then take turns, one unit each, so
    /// that no record waits behind a long record of another stream.
    pub(crate) fn flush(&mut self, most: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        for _ in 0..most {
            let Some(stream) = self.next_stream() else {
                break;
            };
            bytes.extend(self.next_unit_of(stream));
        }
        bytes
    }

    /// A keepalive, as bytes to send: a unit on stream 0 with no flags and
    /// an empty fragment.
    pub(crate) fn keepalive_unit(&mut self) -> Vec<u8> {
        seal(&mut self.transport, CONTROL, false, &[])
    }

    /// Whether a unit has decrypted since the last call.
    pub(crate) fn heard(&mut self) -> bool {
        std::mem::take(&mut self.heard)
    }

    /// Whether anything queued is still to send.
    pub(crate) fn pending(&self) -> bool {
        !self.queued.is_empty()
    }

    /// Drops what the session streams have queued: the connection is
    /// ending, and only stream 0 sends more.
    pub(crate) fn drop_streams(&mut self) {
        self.queued.retain(|stream, _| *stream == CONTROL);
    }

    /// The stream whose unit goes next: stream 0 when it has one, and
    /// otherwise the session stream after the last to send, in turn.
    fn next_stream(&self) -> Option<u32> {
        if self.queued.contains_key(&CONTROL) {
            return Some(CONTROL);
        }
        let after = self.queued.range((Excluded(self.turn), Unbounded)).next();
        let next = after.or_else(|| self.queued.first_key_value());
        next.map(|(stream, _)| *stream)
    }

    /// The next unit of the record `stream` is sending, which it has.
    fn next_unit_of(&mut self, stream: u32) -> Vec<u8> {
        let queue = self.queued.get_mut(&stream).expect("a queued stream");
        let front = queue.front_mut().expect("a queued record");
        let fragment = &front.record[front.sent..];
        let fragment = &fragment[..fragment.len().min(FRAGMENT_MOST)];
        front.sent += fragment.len();
        let more = front.sent < front.record.len();
        let unit = seal(&mut self.transport, stream, more, fragment);
        if !more {
            queue.pop_front();
        }
        if queue.is_empty() {
            self.queued.remove(&stream);
        }
        if stream != CONTROL {
            self.turn = stream;
        }
        unit
    }

    /// Takes the next bytes of the byte stream.
    pub(crate) fn receive(&mut self, bytes: &[u8]) {
        self.arrived.extend(bytes);
    }

    /// The next whole record the bytes received so far hold: its stream and
    /// the record. Nothing when more bytes are needed. After a fault the
    /// connection is over and this is not called again.
    pub(crate) fn next(&mut self, allowed: Allowed) -> Option<Result<Received, Fault>> {
        loop {
            if let Some(found) = self.next_unit(allowed)?.transpose() {
                return Some(found);
            }
        }
    }

    /// Takes the next whole unit: the record it ends, if it ends one.
    /// Nothing when the unit is not whole yet.
    fn next_unit(&mut self, allowed: Allowed) -> Option<Result<Option<Received>, Fault>> {
        let ciphertext = self.unit()?;
        let plaintext = ciphertext.and_then(|ciphertext| self.open(&ciphertext));
        Some(plaintext.and_then(|plaintext| self.fragment(&plaintext, allowed)))
    }

    /// Takes the next whole unit's ciphertext off the bytes received.
    /// Nothing when the unit is not whole yet.
    fn unit(&mut self) -> Option<Result<Vec<u8>, Fault>> {
        let length = self.arrived.first_chunk::<LENGTH_SIZE>()?;
        let length = usize::from(u16::from_be_bytes(*length));
        if !(CIPHERTEXT_LEAST..=CIPHERTEXT_MOST).contains(&length) {
            return Some(Err(Fault::Violation));
        }
        let end = LENGTH_SIZE + length;
        let ciphertext = self.arrived.get(LENGTH_SIZE..end)?.to_vec();
        self.arrived.drain(..end);
        Some(Ok(ciphertext))
    }

    /// The plaintext of one unit's ciphertext.
    fn open(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, Fault> {
        let plaintext = self
            .transport
            .open(ciphertext)
            .ok_or(Fault::Undecryptable)?;
        self.heard = true;
        Ok(plaintext)
    }

    /// Adds one unit's fragment to its stream's record. Gives the record
    /// when the unit ends it, and nothing for a keepalive or when more of
    /// the record follows.
    fn fragment(&mut self, plaintext: &[u8], allowed: Allowed) -> Result<Option<Received>, Fault> {
        let (stream, flags, fragment) = header(plaintext, allowed)?;
        if fragment.is_empty() {
            return self.keepalive(stream, flags).map(|()| None);
        }
        self.append(stream, flags, fragment, allowed.record)
    }

    /// Checks a unit with an empty fragment: only a keepalive is empty, and
    /// it never falls between the fragments of a stream-0 record.
    fn keepalive(&self, stream: u32, flags: u8) -> Result<(), Fault> {
        let keepalive = stream == CONTROL && flags == 0;
        let alone = !self.partial.contains_key(&CONTROL);
        (keepalive && alone).then_some(()).ok_or(Fault::Violation)
    }

    /// Adds a fragment to the record `stream` is receiving, which may hold
    /// at most `most` bytes. Gives the record when `flags` end it.
    fn append(
        &mut self,
        stream: u32,
        flags: u8,
        fragment: &[u8],
        most: usize,
    ) -> Result<Option<Received>, Fault> {
        let record = self.partial.entry(stream).or_default();
        record.extend(fragment);
        if record.len() > most {
            return Err(Fault::Violation);
        }
        if flags & MORE != 0 {
            return Ok(None);
        }
        Ok(self.partial.remove(&stream).map(|record| (stream, record)))
    }
}

/// The unit that carries `fragment` on `stream`, as bytes to send.
fn seal(transport: &mut Transport, stream: u32, more: bool, fragment: &[u8]) -> Vec<u8> {
    let mut plaintext = stream.to_be_bytes().to_vec();
    plaintext.push(if more { MORE } else { 0 });
    plaintext.extend(fragment);
    let ciphertext = transport
        .seal(&plaintext)
        .expect("a unit fits one Noise message");
    let mut bytes = (ciphertext.len() as u16).to_be_bytes().to_vec();
    bytes.extend(ciphertext);
    bytes
}

/// A unit's plaintext split into its stream, its flags and its fragment,
/// when the flags are known and the stream may carry something now.
fn header(plaintext: &[u8], allowed: Allowed) -> Result<(u32, u8, &[u8]), Fault> {
    let (header, fragment) = plaintext
        .split_first_chunk::<HEADER_SIZE>()
        .ok_or(Fault::Violation)?;
    let [a, b, c, d, flags] = *header;
    let stream = u32::from_be_bytes([a, b, c, d]);
    let known = flags & !MORE == 0;
    let open = stream == CONTROL || allowed.streams;
    (known && open)
        .then_some((stream, flags, fragment))
        .ok_or(Fault::Violation)
}
