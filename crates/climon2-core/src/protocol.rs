//! Frozen byte framing (ADR 0001); protocol-1 messages are version-specific.
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

pub const MAX_FRAME_LENGTH: usize = 1_048_576;
/// Limits include the type byte, just like the frozen length prefix.
pub const MAX_DATA_FRAME_LENGTH: usize = 65_536;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    type_code: u8,
    body: Vec<u8>,
}

impl Frame {
    /// Validates type and size before any frame is emitted.
    pub fn new(type_code: u8, body: Vec<u8>) -> io::Result<Self> {
        validate_type(type_code)?;
        validate_body(type_code, body.len())?;
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
    match code {
        0x01 | 0x10..=0x12 | 0x20..=0x22 | 0x7f => Ok(()),
        _ => Err(invalid("unknown message type")),
    }
}

fn validate_body(code: u8, length: usize) -> io::Result<()> {
    let limit = match code {
        0x11 | 0x21 => MAX_DATA_FRAME_LENGTH,
        _ => MAX_FRAME_LENGTH,
    };
    if length >= limit {
        return Err(invalid("oversized frame"));
    }
    Ok(())
}

/// Read exactly one bounded frame. Callers must close on any error and enforce
/// the absolute hello deadline on their transport, not restart it per read.
pub fn read_frame(reader: &mut impl Read) -> io::Result<Frame> {
    let mut prefix = <[u8; 4]>::default();
    reader.read_exact(&mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if !(1..=MAX_FRAME_LENGTH).contains(&length) {
        return Err(invalid("invalid frame length"));
    }
    let mut code = [u8::default()];
    reader.read_exact(&mut code)?;
    validate_type(code[0])?;
    validate_body(code[0], length - 1)?;
    let mut body = vec![u8::default(); length - 1];
    reader.read_exact(&mut body)?;
    Frame::new(code[0], body)
}

/// Write all bytes, including when a transport performs partial writes.
pub fn write_frame(writer: &mut impl Write, frame: &Frame) -> io::Result<()> {
    let length = (frame.body.len() + 1) as u32;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&[frame.type_code])?;
    writer.write_all(&frame.body)
}

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

/// Per-connection receive state, not session ownership. A daemon must separately
/// reject an Attach while another connection has the controlling client.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConnectionState {
    #[default]
    AwaitingHello,
    Probing,
    Attached,
    Closed,
}

impl ConnectionState {
    /// Any protocol error poisons this connection; it must then be closed.
    pub fn receive(&mut self, message: &ClientMessage) -> io::Result<()> {
        let next = self.next(message);
        *self = Self::Closed;
        *self = next?;
        Ok(())
    }

    fn next(self, message: &ClientMessage) -> io::Result<Self> {
        match message {
            ClientMessage::Error { .. } => Ok(Self::Closed),
            ClientMessage::Hello {
                protocol: PROTOCOL_VERSION,
            } if self == Self::AwaitingHello => Ok(Self::Probing),
            ClientMessage::Attach(_) if self == Self::Probing => Ok(Self::Attached),
            ClientMessage::Attach(_) if self == Self::Attached => {
                Err(invalid("session already has a controlling client"))
            }
            ClientMessage::Input(_) | ClientMessage::Resize(_) if self == Self::Attached => {
                Ok(self)
            }
            _ => Err(invalid("message not allowed in connection state")),
        }
    }
}

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

    pub fn validate_client(&self, id: &SessionId, version: &VersionName) -> io::Result<()> {
        if !self.is_live_session(id) {
            return Err(invalid("session ID mismatch"));
        }
        if self.protocol != PROTOCOL_VERSION {
            return Err(invalid("protocol mismatch"));
        }
        if self.climon2_version != *version {
            return Err(invalid("climon2 version mismatch"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientMessage {
    Hello { protocol: u32 },
    Attach(TerminalSize),
    Input(Vec<u8>),
    Resize(TerminalSize),
    Error { message: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerMessage {
    Hello(HelloReply),
    Attached,
    Output(Vec<u8>),
    Exit { code: u32 },
    Error { message: String },
}

#[derive(Serialize, Deserialize)]
struct HelloRequest {
    protocol: u32,
}
#[derive(Serialize, Deserialize)]
struct Exit {
    code: u32,
}
#[derive(Serialize, Deserialize)]
struct ErrorBody {
    message: String,
}
#[derive(Serialize, Deserialize)]
struct Empty {}

fn control(code: u8, value: &impl Serialize) -> io::Result<Frame> {
    let body = serde_json::to_vec(value).map_err(io::Error::other)?;
    Frame::new(code, body)
}

fn json<T: DeserializeOwned>(frame: &Frame) -> io::Result<T> {
    crate::json::from_object(frame.body()).map_err(|_| invalid("malformed control JSON"))
}

impl ClientMessage {
    pub fn to_frame(&self) -> io::Result<Frame> {
        match self {
            Self::Hello { protocol } => control(
                0x01,
                &HelloRequest {
                    protocol: *protocol,
                },
            ),
            Self::Attach(size) => control(0x10, size),
            Self::Input(bytes) => Frame::new(0x11, bytes.clone()),
            Self::Resize(size) => control(0x12, size),
            Self::Error { message } => control(
                0x7f,
                &ErrorBody {
                    message: message.clone(),
                },
            ),
        }
    }

    pub fn from_frame(frame: Frame) -> io::Result<Self> {
        match frame.type_code {
            0x01 => json::<HelloRequest>(&frame).map(|hello| Self::Hello {
                protocol: hello.protocol,
            }),
            0x10 => json(&frame).map(Self::Attach),
            0x11 => Ok(Self::Input(frame.body)),
            0x12 => json(&frame).map(Self::Resize),
            0x7f => json::<ErrorBody>(&frame).map(|error| Self::Error {
                message: error.message,
            }),
            _ => Err(invalid("unexpected client message type")),
        }
    }
}

impl ServerMessage {
    pub fn to_frame(&self) -> io::Result<Frame> {
        match self {
            Self::Hello(reply) => control(0x01, reply),
            Self::Attached => control(0x20, &Empty {}),
            Self::Output(bytes) => Frame::new(0x21, bytes.clone()),
            Self::Exit { code } => control(0x22, &Exit { code: *code }),
            Self::Error { message } => control(
                0x7f,
                &ErrorBody {
                    message: message.clone(),
                },
            ),
        }
    }

    pub fn from_frame(frame: Frame) -> io::Result<Self> {
        match frame.type_code {
            0x01 => json(&frame).map(Self::Hello),
            0x20 => json::<Empty>(&frame).map(|_| Self::Attached),
            0x21 => Ok(Self::Output(frame.body)),
            0x22 => json::<Exit>(&frame).map(|exit| Self::Exit { code: exit.code }),
            0x7f => json::<ErrorBody>(&frame).map(|error| Self::Error {
                message: error.message,
            }),
            _ => Err(invalid("unexpected server message type")),
        }
    }
}
