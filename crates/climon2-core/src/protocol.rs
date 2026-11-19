//! The frozen hello handshake (ADR 0001): its byte framing, its request and
//! reply, and its deadline. It never changes and is the only exchange that
//! crosses releases; daemon wire revision 1 (`crate::wire`) follows it.
use crate::{SessionId, VersionName};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};
use std::time::Duration;

pub const PROTOCOL_VERSION: u32 = 1;
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(2);
pub const PTY_DRAIN_IDLE: Duration = Duration::from_millis(100);
pub const PTY_DRAIN_CAP: Duration = Duration::from_secs(1);
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
pub const HANGUP_GRACE: Duration = Duration::from_secs(5);

/// The hello type, in both directions; the frozen layout has no other.
pub const HELLO: u8 = 0x01;

pub const MAX_FRAME_LENGTH: usize = 1_048_576;

/// A frame of the frozen layout: a hello request or reply.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    type_code: u8,
    body: Vec<u8>,
}

impl Frame {
    /// Validates type and size before any frame is emitted.
    pub fn new(type_code: u8, body: Vec<u8>) -> io::Result<Self> {
        validate_type(type_code)?;
        validate_body(body.len())?;
        Ok(Self { type_code, body })
    }

    pub fn type_code(&self) -> u8 {
        self.type_code
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn validate_type(code: u8) -> io::Result<()> {
    if code != HELLO {
        return Err(invalid("unknown message type"));
    }
    Ok(())
}

/// The length prefix counts the type byte too, so a body is one byte
/// shorter than the frame limit at most.
fn validate_body(length: usize) -> io::Result<()> {
    if length >= MAX_FRAME_LENGTH {
        return Err(invalid("oversized frame"));
    }
    Ok(())
}

/// Read exactly one bounded frame. Callers must close on any error and enforce
/// the absolute hello deadline on their transport, not restart it per read.
pub fn read_frame(reader: &mut impl Read) -> io::Result<Frame> {
    let body_length = read_length(reader)? - 1;
    let code = read_type(reader)?;
    let mut body = vec![u8::default(); body_length];
    reader.read_exact(&mut body)?;
    Frame::new(code, body)
}

fn read_length(reader: &mut impl Read) -> io::Result<usize> {
    let mut prefix = <[u8; 4]>::default();
    reader.read_exact(&mut prefix)?;
    frame_length(prefix)
}

/// The frame length a prefix declares, type and body: 1 to
/// [`MAX_FRAME_LENGTH`], since it counts the type byte.
pub fn frame_length(prefix: [u8; 4]) -> io::Result<usize> {
    let length = u32::from_be_bytes(prefix) as usize;
    if !(1..=MAX_FRAME_LENGTH).contains(&length) {
        return Err(invalid("invalid frame length"));
    }
    Ok(length)
}

/// Rejects any type but hello before reading the body.
fn read_type(reader: &mut impl Read) -> io::Result<u8> {
    let mut code = [u8::default()];
    reader.read_exact(&mut code)?;
    validate_type(code[0])?;
    Ok(code[0])
}

/// Write all bytes, including when a transport performs partial writes.
pub fn write_frame(writer: &mut impl Write, frame: &Frame) -> io::Result<()> {
    let length = (frame.body.len() + 1) as u32;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&[frame.type_code])?;
    writer.write_all(&frame.body)
}

/// A host terminal's size in cells, as the CLI hands it to the daemon it
/// starts; a zero dimension means the size is unknown.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    pub fn initial(self) -> Self {
        self.resize().unwrap_or(Self { cols: 80, rows: 24 })
    }

    pub fn resize(self) -> Option<Self> {
        if self.cols == 0 || self.rows == 0 {
            return None;
        }
        Some(self)
    }
}

/// Elapsed time is measured from accept (daemon) or before connect (prober).
pub fn hello_time_remaining(elapsed: Duration) -> Option<Duration> {
    HELLO_TIMEOUT
        .checked_sub(elapsed)
        .filter(|left| !left.is_zero())
}

/// The client's hello, `{"protocol":<n>}`; unknown fields are ignored.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HelloRequest {
    pub protocol: u32,
}

impl HelloRequest {
    pub fn to_frame(&self) -> io::Result<Frame> {
        hello_frame(self)
    }

    pub fn from_frame(frame: &Frame) -> io::Result<Self> {
        json(frame)
    }
}

/// The daemon's hello reply: its frozen required fields. Unknown fields,
/// such as the optional advertisement (`crate::hello`), are ignored.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HelloReply {
    pub session_id: SessionId,
    pub climon2_version: VersionName,
    pub protocol: u32,
}

impl HelloReply {
    /// Probes deliberately ignore the daemon's version and protocol.
    pub fn is_live_session(&self, expected: &SessionId) -> bool {
        self.session_id == *expected
    }

    pub fn to_frame(&self) -> io::Result<Frame> {
        hello_frame(self)
    }

    pub fn from_frame(frame: &Frame) -> io::Result<Self> {
        json(frame)
    }
}

/// A hello frame whose body is `value` as JSON.
pub(crate) fn hello_frame(value: &impl Serialize) -> io::Result<Frame> {
    let body = serde_json::to_vec(value).map_err(io::Error::other)?;
    Frame::new(HELLO, body)
}

fn json<T: DeserializeOwned>(frame: &Frame) -> io::Result<T> {
    crate::json::from_object(frame.body()).map_err(|_| invalid("malformed hello JSON"))
}
