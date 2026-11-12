use climon2_core::{
    protocol::{
        ClientMessage, ConnectionState, MAX_FRAME_LENGTH, ServerMessage, hello_time_remaining,
        read_frame, write_frame,
    },
    wire::{self, ControlRecord, Counter, FatalCode, FrameLimit, Request},
};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};

const WRITE_WINDOW: usize = 16 * 1024 * 1024;
const NO_PROGRESS: Duration = Duration::from_secs(5);
/// Diagnostic text of every Fatal error; its code says what failed.
const FATAL_MESSAGE: &str = "connection-fatal revision-1 failure";

/// The release-constant revision-1 request and server frame limit, counting
/// type and body: the operational limits' `payload.wire_frame_bytes` default.
pub(crate) fn frame_limit() -> FrameLimit {
    FrameLimit::new(1_048_576).expect("a nonzero limit")
}

struct PendingFrame {
    bytes: Vec<u8>,
    written: usize,
}

/// The protocol a connection speaks after the frozen hello.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Protocol {
    /// Hello, then the private protocol, unless the first frame after hello
    /// has a revision-1 type and so selects revision 1.
    Private(ConnectionState),
    /// Daemon wire revision 1, with the last request ID received.
    Revision1(Counter),
}

impl Protocol {
    /// A well-formed request consumes its ID even when it is then rejected;
    /// an ID that does not increase is connection-fatal.
    fn receive_request(&mut self, id: Counter) -> Result<(), FatalCode> {
        if let Self::Revision1(last) = *self
            && id <= last
        {
            return Err(FatalCode::InvalidRequestId);
        }
        *self = Self::Revision1(id);
        Ok(())
    }
}

/// A received frame, in the protocol it was sent in.
pub(crate) enum Received {
    Private(ClientMessage),
    Request(Request),
}

/// Why a connection stopped receiving.
#[derive(Debug)]
pub(crate) enum Failure {
    /// A private-protocol failure, reported with a private `Error`.
    Private(io::Error),
    /// A revision-1 validation failure, reported with a best-effort Fatal error.
    Fatal(FatalCode),
    /// A revision-1 stream that ended between frames or whose transport
    /// failed: nothing to report.
    Closed,
}

pub(crate) struct Connection {
    stream: UnixStream,
    protocol: Protocol,
    accepted: Instant,
    incoming: Vec<u8>,
    outgoing: VecDeque<PendingFrame>,
    queued_bytes: usize,
    last_progress: Instant,
    close_when_empty: bool,
}

impl Connection {
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            protocol: Protocol::Private(ConnectionState::default()),
            accepted: Instant::now(),
            incoming: Vec::new(),
            outgoing: VecDeque::new(),
            queued_bytes: Default::default(),
            last_progress: Instant::now(),
            close_when_empty: false,
        })
    }

    pub fn available(&self) -> bool {
        self.outgoing.is_empty()
    }

    pub fn queued_bytes(&self) -> usize {
        self.queued_bytes
    }

    pub fn stalled(&self, now: Instant) -> bool {
        !self.available() && super::passed(self.last_progress + NO_PROGRESS, now)
    }

    pub fn should_close(&self) -> bool {
        self.close_when_empty && self.available()
    }

    pub fn close_after_flush(&mut self) {
        self.close_when_empty = true;
    }

    pub fn queue(&mut self, message: ServerMessage) -> io::Result<()> {
        self.queue_frame(message.to_frame()?)
    }

    pub fn queue_frame(&mut self, frame: climon2_core::protocol::Frame) -> io::Result<()> {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &frame)?;
        self.queue_bytes(bytes)
    }

    /// Queues a revision-1 frame that the server frame limit admits.
    pub fn queue_wire(&mut self, frame: &wire::Frame) -> io::Result<()> {
        let mut bytes = Vec::new();
        wire::write_frame(&mut bytes, frame, frame_limit())
            .map_err(|_| super::io_error("revision-1 frame exceeds the server frame limit"))?;
        self.queue_bytes(bytes)
    }

    fn queue_bytes(&mut self, bytes: Vec<u8>) -> io::Result<()> {
        let queued = self.queued_bytes.saturating_add(bytes.len());
        if WRITE_WINDOW.checked_sub(queued).is_none() {
            return Err(super::io_error("connection output queue is full"));
        }
        self.queued_bytes += bytes.len();
        self.outgoing.push_back(PendingFrame { bytes, written: 0 });
        Ok(())
    }

    /// Reports why receiving failed, best effort, and closes once flushed.
    pub fn fail(&mut self, failure: Failure) {
        match failure {
            Failure::Private(error) => self.error(&protocol_error(&error)),
            Failure::Fatal(code) => self.fatal(code),
            Failure::Closed => {}
        }
        self.close_after_flush();
    }

    fn error(&mut self, message: &str) {
        let error = ServerMessage::Error {
            message: message.into(),
        };
        self.last(|connection| connection.queue(error));
    }

    fn fatal(&mut self, code: FatalCode) {
        let error = wire::FatalError(wire::Error {
            code: code.into(),
            message: FATAL_MESSAGE.into(),
        });
        self.last(|connection| connection.queue_wire(&error.to_frame()));
    }

    /// Queues a connection's last frame. It replaces output not yet started,
    /// but never splices a partially written frame.
    fn last(&mut self, queue: impl FnOnce(&mut Self) -> io::Result<()>) {
        if self.outgoing.front().is_none_or(|frame| frame.written == 0) {
            for unsent in self.outgoing.drain(..) {
                self.queued_bytes -= unsent.bytes.len();
            }
        }
        let _ = queue(self);
        let _ = self.flush();
    }

    pub fn flush(&mut self) -> io::Result<()> {
        while let Some(frame) = self.outgoing.front_mut() {
            // A stream write of a non-empty buffer never returns 0; it makes progress or fails.
            let Some(count) =
                super::unless_would_block(self.stream.write(&frame.bytes[frame.written..]))?
            else {
                break;
            };
            frame.written += count;
            self.queued_bytes -= count;
            self.last_progress = Instant::now();
            if frame.written == frame.bytes.len() {
                self.outgoing.pop_front();
            }
        }
        Ok(())
    }

    /// Every frame already readable, so a burst sent in one write is handled
    /// in one iteration, then the failure that ended receiving, if any. A
    /// private failure discards the frames read with it; revision-1 frames
    /// read before a failure are still served.
    pub fn receive_available(&mut self) -> (Vec<Received>, Option<Failure>) {
        let mut messages = Vec::new();
        if !self.receiving() {
            return (messages, None);
        }
        let failure = self.receive_into(&mut messages).err();
        (served(messages, failure.as_ref()), failure)
    }

    fn receive_into(&mut self, messages: &mut Vec<Received>) -> Result<(), Failure> {
        while let Some(message) = self.receive()? {
            messages.extend(message);
        }
        Ok(())
    }

    /// False once the connection failed or is closing.
    pub fn receiving(&self) -> bool {
        !self.close_when_empty
    }

    /// `None` once nothing is readable; `Some(None)` after part of a frame.
    fn receive(&mut self) -> Result<Option<Option<Received>>, Failure> {
        self.validate_receive().map_err(Failure::Private)?;
        self.read_message()
    }

    fn validate_receive(&self) -> io::Result<()> {
        if self.protocol == Protocol::Private(ConnectionState::AwaitingHello)
            && hello_time_remaining(self.accepted.elapsed()).is_none()
        {
            return Err(super::io_error("hello deadline exceeded"));
        }
        Ok(())
    }

    fn read_message(&mut self) -> Result<Option<Option<Received>>, Failure> {
        let mut bytes = self.read_buffer()?;
        let read = super::unless_would_block(self.stream.read(&mut bytes));
        let Some(count) = read.map_err(|error| self.ended(error))? else {
            return Ok(None);
        };
        self.received(&bytes[..count]).map(Some)
    }

    /// Never reads past the current frame, so each read holds at most one
    /// frame, and the first frame after hello decides how later ones are read.
    fn read_buffer(&self) -> Result<Vec<u8>, Failure> {
        Ok(vec![
            u8::default();
            (self.target()? - self.incoming.len()).min(8192)
        ])
    }

    fn received(&mut self, bytes: &[u8]) -> Result<Option<Received>, Failure> {
        if bytes.is_empty() {
            return Err(self.ended(super::io_error("connection closed")));
        }
        self.incoming.extend_from_slice(bytes);
        self.complete()
    }

    /// The stream ended or failed. A private connection reports it; a
    /// revision-1 stream that ended inside a frame is truncated, and one that
    /// ended between frames simply closed.
    fn ended(&self, error: io::Error) -> Failure {
        match (self.revision1(), self.incoming.is_empty()) {
            (false, _) => Failure::Private(error),
            (true, true) => Failure::Closed,
            (true, false) => Failure::Fatal(FatalCode::TruncatedFrame),
        }
    }

    /// Whether the frame being read is revision 1: on a revision-1 connection,
    /// or the first frame after hello when revision 1 defines its type.
    fn revision1(&self) -> bool {
        match self.protocol {
            Protocol::Revision1(_) => true,
            Protocol::Private(state) => {
                state == ConnectionState::Probing
                    && self
                        .incoming
                        .get(4)
                        .is_some_and(|code| wire::defines(*code))
            }
        }
    }

    /// Bytes needed for the frame being decoded: the prefix; then the whole
    /// revision-1 frame, or first the type of a private one.
    fn target(&self) -> Result<usize, Failure> {
        if self.incoming.len() < 4 {
            return Ok(4);
        }
        if self.revision1() {
            return self.wire_target();
        }
        self.private_target().map_err(Failure::Private)
    }

    fn private_target(&self) -> io::Result<usize> {
        if self.incoming.len() == 4 {
            return self.frame_length().map(|_| 5);
        }
        self.frame_target()
    }

    /// A revision-1 frame's whole length, which the codec decides from its prefix.
    fn wire_target(&self) -> Result<usize, Failure> {
        let length = frame_limit()
            .frame_length(self.prefix())
            .map_err(Failure::Fatal)?;
        Ok(4 + length)
    }

    fn prefix(&self) -> [u8; 4] {
        self.incoming[..4].try_into().expect("frame prefix")
    }

    fn frame_length(&self) -> io::Result<usize> {
        let length = u32::from_be_bytes(self.prefix()) as usize;
        if !(1..=MAX_FRAME_LENGTH).contains(&length) {
            return Err(super::io_error("invalid frame length"));
        }
        Ok(length)
    }

    fn frame_target(&self) -> io::Result<usize> {
        let length = self.frame_length()?;
        validate_frame_type(self.incoming[4], length)?;
        Ok(4 + length)
    }

    fn complete(&mut self) -> Result<Option<Received>, Failure> {
        if !self.frame_complete()? {
            return Ok(None);
        }
        let revision1 = self.revision1();
        let bytes = std::mem::take(&mut self.incoming);
        self.decode(&bytes, revision1).map(Some)
    }

    fn frame_complete(&self) -> Result<bool, Failure> {
        Ok(self.incoming.len() >= 5 && self.incoming.len() == self.target()?)
    }

    fn decode(&mut self, bytes: &[u8], revision1: bool) -> Result<Received, Failure> {
        if revision1 {
            return self.request(bytes).map_err(Failure::Fatal);
        }
        self.message(bytes)
            .map(Received::Private)
            .map_err(Failure::Private)
    }

    fn message(&mut self, bytes: &[u8]) -> io::Result<ClientMessage> {
        let message = parsed_message(bytes)?;
        if let Protocol::Private(state) = &mut self.protocol {
            state.receive(&message)?;
        }
        Ok(message)
    }

    /// The first request selects revision 1; later ones keep its ID sequence.
    fn request(&mut self, bytes: &[u8]) -> Result<Received, FatalCode> {
        let request = decoded_request(bytes)?;
        self.protocol.receive_request(request_id(&request))?;
        Ok(Received::Request(request))
    }
}

/// The frames read before receiving ended that are still served.
fn served(messages: Vec<Received>, failure: Option<&Failure>) -> Vec<Received> {
    match failure {
        Some(Failure::Private(_)) => Vec::new(),
        _ => messages,
    }
}

fn parsed_message(bytes: &[u8]) -> io::Result<ClientMessage> {
    let mut reader = bytes;
    ClientMessage::from_frame(read_frame(&mut reader)?)
}

/// A whole frame the codec already admitted by its prefix.
fn decoded_request(bytes: &[u8]) -> Result<Request, FatalCode> {
    let frame = wire::read_frame(&mut &bytes[..], frame_limit())
        .ok()
        .flatten()
        .ok_or(FatalCode::InternalError)?;
    Request::from_frame(&frame)
}

fn request_id(request: &Request) -> Counter {
    match request {
        Request::Open(open) => open.request_id,
        Request::Leave(leave) => leave.request_id,
        Request::Kill(kill) => kill.request_id,
    }
}

fn validate_frame_type(code: u8, length: usize) -> io::Result<()> {
    climon2_core::protocol::Frame::new(code, Vec::new())?;
    validate_data_length(code, length)
}

fn validate_data_length(code: u8, length: usize) -> io::Result<()> {
    if matches!(code, 0x11 | 0x21) && length > climon2_core::protocol::MAX_DATA_FRAME_LENGTH {
        return Err(super::io_error("oversized data frame"));
    }
    Ok(())
}

/// A private decoding failure names itself; any other framing failure is generic.
fn protocol_error(error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::InvalidData {
        return error.to_string();
    }
    "invalid session protocol".into()
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}
