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
    pub(crate) fn next(&mut self, allowed: Allowed) -> Option<Result<(u32, Vec<u8>), Fault>> {
        loop {
            let ciphertext = match self.unit()? {
                Ok(ciphertext) => ciphertext,
                Err(fault) => return Some(Err(fault)),
            };
            let Some(plaintext) = self.transport.open(&ciphertext) else {
                return Some(Err(Fault::Undecryptable));
            };
            match self.fragment(&plaintext, allowed) {
                Ok(None) => {}
                Ok(Some(record)) => return Some(Ok(record)),
                Err(fault) => return Some(Err(fault)),
            }
        }
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

    /// Adds one unit's fragment to its stream's record. Gives the record
    /// when the unit ends it, and nothing for a keepalive or when more of
    /// the record follows.
    fn fragment(
        &mut self,
        plaintext: &[u8],
        allowed: Allowed,
    ) -> Result<Option<(u32, Vec<u8>)>, Fault> {
        let (header, fragment) = plaintext
            .split_first_chunk::<HEADER_SIZE>()
            .ok_or(Fault::Violation)?;
        let [a, b, c, d, flags] = *header;
        let stream = u32::from_be_bytes([a, b, c, d]);
        let streams = stream == CONTROL || allowed.streams;
        if flags & !MORE != 0 || !streams {
            return Err(Fault::Violation);
        }
        if fragment.is_empty() {
            // Only a keepalive is empty, and it never falls between the
            // fragments of a stream-0 record.
            let keepalive = stream == CONTROL && flags == 0;
            let alone = !self.partial.contains_key(&CONTROL);
            return (keepalive && alone).then_some(None).ok_or(Fault::Violation);
        }
        let record = self.partial.entry(stream).or_default();
        record.extend(fragment);
        if record.len() > allowed.record {
            return Err(Fault::Violation);
        }
        if flags & MORE != 0 {
            return Ok(None);
        }
        Ok(self.partial.remove(&stream).map(|record| (stream, record)))
    }
}
