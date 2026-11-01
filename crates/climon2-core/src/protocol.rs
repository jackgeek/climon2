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

pub(crate) const HELLO: u8 = 0x01;
const ATTACH: u8 = 0x10;
const INPUT: u8 = 0x11;
const RESIZE: u8 = 0x12;
const TAKE_CONTROL: u8 = 0x13;
const READY: u8 = 0x14;
const DETACH: u8 = 0x15;
const KILL: u8 = 0x16;
const ATTACHED: u8 = 0x20;
const OUTPUT: u8 = 0x21;
const EXIT: u8 = 0x22;
const GRANTED: u8 = 0x23;
const RESTORE_END: u8 = 0x24;
const REVOKED: u8 = 0x25;
const DETACHED: u8 = 0x26;
const TERMINATED: u8 = 0x27;
const ERROR: u8 = 0x7f;

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
        HELLO | ATTACH..=KILL | ATTACHED..=TERMINATED | ERROR => Ok(()),
        _ => Err(invalid("unknown message type")),
    }
}

fn validate_body(code: u8, length: usize) -> io::Result<()> {
    let limit = match code {
        INPUT | OUTPUT => MAX_DATA_FRAME_LENGTH,
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
    let body_length = read_length(reader)? - 1;
    let code = read_type(reader, body_length)?;
    let mut body = vec![u8::default(); body_length];
    reader.read_exact(&mut body)?;
    Frame::new(code, body)
}

/// The length prefix counts the type byte, so it is never zero.
fn read_length(reader: &mut impl Read) -> io::Result<usize> {
    let mut prefix = <[u8; 4]>::default();
    reader.read_exact(&mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if !(1..=MAX_FRAME_LENGTH).contains(&length) {
        return Err(invalid("invalid frame length"));
    }
    Ok(length)
}

/// Rejects unknown types and oversized bodies before reading the body.
fn read_type(reader: &mut impl Read, body_length: usize) -> io::Result<u8> {
    let mut code = [u8::default()];
    reader.read_exact(&mut code)?;
    validate_type(code[0])?;
    validate_body(code[0], body_length)?;
    Ok(code[0])
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

/// Per-connection receive state, not session ownership: the daemon decides
/// admission and control separately. After hello a connection sends Attach or
/// Kill, never both; attachment requests need an accepted Attach.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConnectionState {
    #[default]
    AwaitingHello,
    Probing,
    Attached,
    /// A kill-mode connection; it is never an attachment.
    Killing,
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
            ClientMessage::Hello { protocol } => self.after_hello(*protocol),
            _ => operation(message).map_or_else(
                || self.after_stream(),
                |chosen| self.after_operation(chosen),
            ),
        }
    }

    fn after_hello(self, protocol: u32) -> io::Result<Self> {
        match (self, protocol) {
            (Self::AwaitingHello, PROTOCOL_VERSION) => Ok(Self::Probing),
            _ => Err(not_allowed()),
        }
    }

    fn after_operation(self, chosen: Self) -> io::Result<Self> {
        match (self, chosen) {
            (Self::Probing, _) => Ok(chosen),
            (Self::Attached, Self::Attached) => {
                Err(invalid("session already has a controlling client"))
            }
            (Self::Attached | Self::Killing, _) => {
                Err(invalid("connection already sent attach or kill"))
            }
            _ => Err(not_allowed()),
        }
    }

    fn after_stream(self) -> io::Result<Self> {
        match self {
            Self::Attached => Ok(self),
            _ => Err(not_allowed()),
        }
    }
}

/// The state an operation request moves a probing connection into.
fn operation(message: &ClientMessage) -> Option<ConnectionState> {
    match message {
        ClientMessage::Attach(_) => Some(ConnectionState::Attached),
        ClientMessage::Kill => Some(ConnectionState::Killing),
        _ => None,
    }
}

fn not_allowed() -> io::Error {
    invalid("message not allowed in connection state")
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
    Hello {
        protocol: u32,
    },
    Attach(TerminalSize),
    Input(Vec<u8>),
    Resize(TerminalSize),
    TakeControl,
    /// Restoration for this control period is written; enable input.
    Ready {
        period: u64,
    },
    Detach,
    /// Kill-mode request, sent instead of Attach.
    Kill,
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerMessage {
    Hello(HelloReply),
    Attached,
    Output(Vec<u8>),
    Exit {
        code: u32,
    },
    /// This client controls; its restoration follows.
    Granted {
        period: u64,
    },
    RestoreEnd {
        period: u64,
    },
    Revoked,
    Detached,
    /// Kill confirmed after cleanup.
    Terminated {
        code: u32,
    },
    Error {
        message: String,
    },
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
struct Period {
    period: u64,
}
#[derive(Serialize, Deserialize)]
struct ErrorBody {
    message: String,
}
#[derive(Serialize, Deserialize)]
struct Empty {}

pub(crate) fn control(code: u8, value: &impl Serialize) -> io::Result<Frame> {
    let body = serde_json::to_vec(value).map_err(io::Error::other)?;
    Frame::new(code, body)
}

fn json<T: DeserializeOwned>(frame: &Frame) -> io::Result<T> {
    crate::json::from_object(frame.body()).map_err(|_| invalid("malformed control JSON"))
}

fn error_frame(message: &str) -> io::Result<Frame> {
    control(
        ERROR,
        &ErrorBody {
            message: message.to_owned(),
        },
    )
}

fn error_message(frame: &Frame) -> io::Result<String> {
    json::<ErrorBody>(frame).map(|error| error.message)
}

impl ClientMessage {
    pub fn to_frame(&self) -> io::Result<Frame> {
        match self {
            Self::Hello { protocol } => control(
                HELLO,
                &HelloRequest {
                    protocol: *protocol,
                },
            ),
            Self::Input(bytes) => Frame::new(INPUT, bytes.clone()),
            Self::Error { message } => error_frame(message),
            _ => self.request_frame(),
        }
    }

    /// Requests carry a size, a control period or an empty object.
    fn request_frame(&self) -> io::Result<Frame> {
        match self {
            Self::Attach(size) | Self::Resize(size) => control(self.size_type_code(), size),
            Self::Ready { period } => control(READY, &Period { period: *period }),
            _ => control(self.empty_type_code(), &Empty {}),
        }
    }

    /// Attach and Resize share a body and differ only in type code.
    fn size_type_code(&self) -> u8 {
        match self {
            Self::Attach(_) => ATTACH,
            _ => RESIZE,
        }
    }

    fn empty_type_code(&self) -> u8 {
        match self {
            Self::TakeControl => TAKE_CONTROL,
            Self::Detach => DETACH,
            _ => KILL,
        }
    }

    pub fn from_frame(frame: Frame) -> io::Result<Self> {
        match frame.type_code {
            HELLO => json::<HelloRequest>(&frame).map(|hello| Self::Hello {
                protocol: hello.protocol,
            }),
            INPUT => Ok(Self::Input(frame.body)),
            ERROR => error_message(&frame).map(|message| Self::Error { message }),
            _ => Self::from_request_frame(&frame),
        }
    }

    fn from_request_frame(frame: &Frame) -> io::Result<Self> {
        match frame.type_code {
            ATTACH => json(frame).map(Self::Attach),
            RESIZE => json(frame).map(Self::Resize),
            READY => json::<Period>(frame).map(|ready| Self::Ready {
                period: ready.period,
            }),
            _ => Self::from_empty_frame(frame),
        }
    }

    fn from_empty_frame(frame: &Frame) -> io::Result<Self> {
        let message = match frame.type_code {
            TAKE_CONTROL => Self::TakeControl,
            DETACH => Self::Detach,
            KILL => Self::Kill,
            _ => return Err(invalid("unexpected client message type")),
        };
        json::<Empty>(frame).map(|_| message)
    }
}

impl ServerMessage {
    pub fn to_frame(&self) -> io::Result<Frame> {
        match self {
            Self::Hello(reply) => control(HELLO, reply),
            Self::Output(bytes) => Frame::new(OUTPUT, bytes.clone()),
            Self::Error { message } => error_frame(message),
            _ => self.lifecycle_frame(),
        }
    }

    /// Lifecycle messages carry an exit code, a control period or nothing.
    fn lifecycle_frame(&self) -> io::Result<Frame> {
        match self {
            Self::Exit { code } | Self::Terminated { code } => {
                control(self.code_type_code(), &Exit { code: *code })
            }
            Self::Granted { period } | Self::RestoreEnd { period } => {
                control(self.period_type_code(), &Period { period: *period })
            }
            _ => control(self.empty_type_code(), &Empty {}),
        }
    }

    fn code_type_code(&self) -> u8 {
        match self {
            Self::Exit { .. } => EXIT,
            _ => TERMINATED,
        }
    }

    fn period_type_code(&self) -> u8 {
        match self {
            Self::Granted { .. } => GRANTED,
            _ => RESTORE_END,
        }
    }

    fn empty_type_code(&self) -> u8 {
        match self {
            Self::Attached => ATTACHED,
            Self::Revoked => REVOKED,
            _ => DETACHED,
        }
    }

    pub fn from_frame(frame: Frame) -> io::Result<Self> {
        match frame.type_code {
            HELLO => json(&frame).map(Self::Hello),
            OUTPUT => Ok(Self::Output(frame.body)),
            ERROR => error_message(&frame).map(|message| Self::Error { message }),
            _ => Self::from_lifecycle_frame(&frame),
        }
    }

    fn from_lifecycle_frame(frame: &Frame) -> io::Result<Self> {
        match frame.type_code {
            EXIT => json::<Exit>(frame).map(|exit| Self::Exit { code: exit.code }),
            TERMINATED => json::<Exit>(frame).map(|exit| Self::Terminated { code: exit.code }),
            _ => Self::from_period_frame(frame),
        }
    }

    fn from_period_frame(frame: &Frame) -> io::Result<Self> {
        match frame.type_code {
            GRANTED => json::<Period>(frame).map(|body| Self::Granted {
                period: body.period,
            }),
            RESTORE_END => json::<Period>(frame).map(|body| Self::RestoreEnd {
                period: body.period,
            }),
            _ => Self::from_empty_frame(frame),
        }
    }

    fn from_empty_frame(frame: &Frame) -> io::Result<Self> {
        let message = match frame.type_code {
            ATTACHED => Self::Attached,
            REVOKED => Self::Revoked,
            DETACHED => Self::Detached,
            _ => return Err(invalid("unexpected server message type")),
        };
        json::<Empty>(frame).map(|_| message)
    }
}
