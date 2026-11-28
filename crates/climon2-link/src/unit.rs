//! Units: what a link connection is made of once the handshake is done. A
//! unit is a length and one Noise transport message, and carries one
//! fragment of one record of one stream. The layout is that of the
//! protocol's shared rules.

use crate::noise::{TAG_SIZE, Transport};
use std::collections::BTreeMap;

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
}

impl Units {
    pub(crate) fn new(transport: Transport) -> Self {
        Self {
            transport,
            arrived: Vec::new(),
            partial: BTreeMap::new(),
        }
    }

    /// The units that carry `record` on `stream`, as bytes to send. A
    /// record is never empty: it starts with its tag.
    pub(crate) fn send(&mut self, stream: u32, record: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut fragments = record.chunks(FRAGMENT_MOST).peekable();
        while let Some(fragment) = fragments.next() {
            let more = fragments.peek().is_some();
            let mut plaintext = stream.to_be_bytes().to_vec();
            plaintext.push(if more { MORE } else { 0 });
            plaintext.extend(fragment);
            let ciphertext = self
                .transport
                .seal(&plaintext)
                .expect("a unit fits one Noise message");
            bytes.extend((ciphertext.len() as u16).to_be_bytes());
            bytes.extend(ciphertext);
        }
        bytes
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
        self.transport.open(ciphertext).ok_or(Fault::Undecryptable)
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
