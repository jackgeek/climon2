//! `u32be length | u8 type | body`, where the length counts the type byte and
//! body but not the four-byte prefix.
use super::FatalCode;
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    num::NonZeroU32,
};

/// One frame: a type byte and its body. The direction that decodes a frame
/// validates its type and body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    code: u8,
    body: Vec<u8>,
}

impl Frame {
    pub fn new(code: u8, body: Vec<u8>) -> Self {
        Self { code, body }
    }

    pub fn code(&self) -> u8 {
        self.code
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    fn length(&self) -> usize {
        self.body.len() + 1
    }
}

/// A positive ceiling on frame length, counting the type byte and body.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FrameLimit(NonZeroU32);

impl FrameLimit {
    pub fn new(bytes: u32) -> Option<Self> {
        NonZeroU32::new(bytes).map(Self)
    }

    pub fn get(self) -> u32 {
        self.0.get()
    }

    /// Zero cannot hold the type byte.
    fn admit(self, length: usize) -> Result<usize, FatalCode> {
        match length {
            0 => Err(FatalCode::InvalidFrameLength),
            length if length > self.get() as usize => Err(FatalCode::FrameTooLarge),
            length => Ok(length),
        }
    }
}

/// A connection-fatal validation failure, or a transport failure that is not
/// wire validation.
#[derive(Debug)]
pub enum WireError {
    Fatal(FatalCode),
    Io(io::Error),
}

impl From<FatalCode> for WireError {
    fn from(code: FatalCode) -> Self {
        Self::Fatal(code)
    }
}

impl From<io::Error> for WireError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Read one frame that `limit` admits, deciding oversize from the prefix before
/// any body arrives. `Ok(None)` is closure exactly between frames; closure
/// inside a frame is `truncated_frame`. Close the connection on any error.
pub fn read_frame(reader: &mut impl Read, limit: FrameLimit) -> Result<Option<Frame>, WireError> {
    let Some(length) = read_prefix(reader)? else {
        return Ok(None);
    };
    let mut code = read_exactly(reader, limit.admit(length)?)?;
    let body = code.split_off(1);
    Ok(Some(Frame {
        code: code[0],
        body,
    }))
}

fn read_prefix(reader: &mut impl Read) -> Result<Option<usize>, WireError> {
    let bytes = read_up_to(reader, 4)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let prefix: [u8; 4] = bytes.try_into().map_err(|_| truncated())?;
    Ok(Some(u32::from_be_bytes(prefix) as usize))
}

fn read_exactly(reader: &mut impl Read, length: usize) -> Result<Vec<u8>, WireError> {
    let bytes = read_up_to(reader, length)?;
    (bytes.len() == length)
        .then_some(bytes)
        .ok_or_else(truncated)
}

fn read_up_to(reader: &mut impl Read, length: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(length);
    reader.take(length as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn truncated() -> WireError {
    WireError::Fatal(FatalCode::TruncatedFrame)
}

/// Write one frame if `limit` admits it; nothing is written otherwise. An I/O
/// error can leave part of the frame written, so close the connection on any
/// error. Pass the smaller of the local ceiling and the peer's advertised
/// maximum.
pub fn write_frame(
    writer: &mut impl Write,
    frame: &Frame,
    limit: FrameLimit,
) -> Result<(), WireError> {
    let prefix = (limit.admit(frame.length())? as u32).to_be_bytes();
    writer.write_all(&[&prefix[..], &[frame.code], &frame.body].concat())?;
    Ok(())
}
