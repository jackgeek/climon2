use climon2_core::{
    protocol::{
        self, HELLO, HelloRequest, PROTOCOL_VERSION, hello_time_remaining, read_frame, write_frame,
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

impl PendingFrame {
    /// What a queued frame holds against the output queue's bound until it
    /// is written whole or discarded: its allocation and its place in the
    /// queue, not only its bytes.
    fn charge(&self) -> usize {
        self.bytes.capacity() + std::mem::size_of::<Self>()
    }
}

/// What a connection speaks: the frozen hello, then daemon wire revision 1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Protocol {
    AwaitingHello,
    /// Daemon wire revision 1, with the last request ID received, if any.
    Revision1(Option<Counter>),
}

impl Protocol {
    /// A well-formed request consumes its ID even when it is then rejected;
    /// an ID that does not increase is connection-fatal.
    fn receive_request(&mut self, id: Counter) -> Result<(), FatalCode> {
        if let Self::Revision1(Some(last)) = *self
            && id <= last
        {
            return Err(FatalCode::InvalidRequestId);
        }
        *self = Self::Revision1(Some(id));
        Ok(())
    }
}

/// A received frame: the hello, or a revision-1 request.
pub(crate) enum Received {
    Hello,
    Request(Request),
}

/// Why a connection stopped receiving.
#[derive(Debug)]
pub(crate) enum Failure {
    /// A revision-1 validation failure, reported with a best-effort Fatal error.
    Fatal(FatalCode),
    /// Traffic that never completed a valid hello in time, or a revision-1
    /// stream that ended between frames or whose transport failed: nothing
    /// to report.
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
            protocol: Protocol::AwaitingHello,
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

    /// What the queued frames hold against the 16 MiB bound: each one's
    /// allocation and its place in the queue.
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

    /// Queues the hello reply.
    pub fn queue_frame(&mut self, frame: protocol::Frame) -> io::Result<()> {
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
        let frame = PendingFrame { bytes, written: 0 };
        let queued = self.queued_bytes.saturating_add(frame.charge());
        if WRITE_WINDOW.checked_sub(queued).is_none() {
            return Err(super::io_error("connection output queue is full"));
        }
        self.queued_bytes = queued;
        self.outgoing.push_back(frame);
        Ok(())
    }

    /// Reports why receiving failed, best effort, and closes once flushed.
    pub fn fail(&mut self, failure: Failure) {
        if let Failure::Fatal(code) = failure {
            self.fatal(code);
        }
        self.close_after_flush();
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
                self.queued_bytes -= unsent.charge();
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
            self.last_progress = Instant::now();
            if frame.written == frame.bytes.len() {
                self.queued_bytes -= frame.charge();
                self.outgoing.pop_front();
            }
        }
        Ok(())
    }

    /// Every frame already readable, so a burst sent in one write is handled
    /// in one iteration, then the failure that ended receiving, if any. The
    /// frames read before a failure are still served.
    pub fn receive_available(&mut self) -> (Vec<Received>, Option<Failure>) {
        let mut messages = Vec::new();
        if !self.receiving() {
            return (messages, None);
        }
        let failure = self.receive_into(&mut messages).err();
        (messages, failure)
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
    /// A connection still without its hello two seconds after accept is
    /// closed.
    fn receive(&mut self) -> Result<Option<Option<Received>>, Failure> {
        if self.hello_overdue() {
            return Err(Failure::Closed);
        }
        self.read_message()
    }

    fn hello_overdue(&self) -> bool {
        self.protocol == Protocol::AwaitingHello
            && hello_time_remaining(self.accepted.elapsed()).is_none()
    }

    fn read_message(&mut self) -> Result<Option<Option<Received>>, Failure> {
        let mut bytes = self.read_buffer()?;
        let read = super::unless_would_block(self.stream.read(&mut bytes));
        let Some(count) = read.map_err(|_| self.ended())? else {
            return Ok(None);
        };
        self.received(&bytes[..count]).map(Some)
    }

    /// Never reads past the current frame, so each read holds at most one
    /// frame, and the hello is read under the frozen layout.
    fn read_buffer(&self) -> Result<Vec<u8>, Failure> {
        Ok(vec![
            u8::default();
            (self.target()? - self.incoming.len()).min(8192)
        ])
    }

    fn received(&mut self, bytes: &[u8]) -> Result<Option<Received>, Failure> {
        if bytes.is_empty() {
            return Err(self.ended());
        }
        self.incoming.extend_from_slice(bytes);
        self.complete()
    }

    /// The stream ended or failed: a revision-1 stream that ended inside a
    /// frame is truncated; otherwise the connection just closed.
    fn ended(&self) -> Failure {
        if self.protocol != Protocol::AwaitingHello && !self.incoming.is_empty() {
            return Failure::Fatal(FatalCode::TruncatedFrame);
        }
        Failure::Closed
    }

    /// Bytes needed for the frame being decoded: the prefix; then the whole
    /// revision-1 frame, or first the type of the hello.
    fn target(&self) -> Result<usize, Failure> {
        if self.incoming.len() < 4 {
            return Ok(4);
        }
        match self.protocol {
            Protocol::AwaitingHello => self.hello_target().ok_or(Failure::Closed),
            Protocol::Revision1(_) => self.wire_target(),
        }
    }

    /// The hello's type once its length is valid, then the whole hello; a
    /// frame of another type is no hello.
    fn hello_target(&self) -> Option<usize> {
        let length = protocol::frame_length(self.prefix()).ok()?;
        match self.incoming.get(4) {
            None => Some(5),
            Some(&HELLO) => Some(4 + length),
            Some(_) => None,
        }
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

    fn complete(&mut self) -> Result<Option<Received>, Failure> {
        if !self.frame_complete()? {
            return Ok(None);
        }
        let bytes = std::mem::take(&mut self.incoming);
        self.decode(&bytes).map(Some)
    }

    fn frame_complete(&self) -> Result<bool, Failure> {
        Ok(self.incoming.len() >= 5 && self.incoming.len() == self.target()?)
    }

    fn decode(&mut self, bytes: &[u8]) -> Result<Received, Failure> {
        match self.protocol {
            Protocol::AwaitingHello => self.hello(bytes).ok_or(Failure::Closed),
            Protocol::Revision1(_) => self.request(bytes).map_err(Failure::Fatal),
        }
    }

    /// A valid hello of protocol 1; every later frame is revision 1.
    fn hello(&mut self, bytes: &[u8]) -> Option<Received> {
        let frame = read_frame(&mut &bytes[..]).ok()?;
        HelloRequest::from_frame(&frame)
            .ok()
            .filter(|hello| hello.protocol == PROTOCOL_VERSION)?;
        self.protocol = Protocol::Revision1(None);
        Some(Received::Hello)
    }

    /// Each request keeps the connection's request ID sequence.
    fn request(&mut self, bytes: &[u8]) -> Result<Received, FatalCode> {
        let request = decoded_request(bytes)?;
        self.protocol.receive_request(request.request_id())?;
        Ok(Received::Request(request))
    }
}

/// A whole frame the codec already admitted by its prefix.
fn decoded_request(bytes: &[u8]) -> Result<Request, FatalCode> {
    let frame = wire::read_frame(&mut &bytes[..], frame_limit())
        .ok()
        .flatten()
        .ok_or(FatalCode::InternalError)?;
    Request::from_frame(&frame)
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    const SIXTEEN_MIB: usize = 16 * 1024 * 1024;
    /// A queued frame's place in the queue: its `Vec` (pointer, capacity
    /// and length) and its written count, 8 bytes each on 64-bit targets.
    const SLOT: usize = 32;

    fn connection() -> (Connection, UnixStream) {
        let (ours, peer) = UnixStream::pair().unwrap();
        (Connection::new(ours).unwrap(), peer)
    }

    /// LIM-01 (queue bound, at the limit and one past): the output queue
    /// holds 16 MiB of what its frames hold, each frame's allocation and
    /// its 32-byte place in the queue. A frame that would take it one byte
    /// past is refused and queues nothing, so even empty frames are
    /// bounded: 524,288 fill it.
    #[test]
    fn the_output_queue_holds_sixteen_mebibytes_of_frames_and_their_overhead() {
        let (mut full, _peer) = connection();
        full.queue_bytes(vec![0; SIXTEEN_MIB - 2 * SLOT - 1])
            .unwrap();
        full.queue_bytes(vec![0; 1]).unwrap();
        assert_eq!(full.queued_bytes(), SIXTEEN_MIB);
        assert!(full.queue_bytes(Vec::new()).is_err());
        assert_eq!(full.queued_bytes(), SIXTEEN_MIB);
        let (mut empty, _peer) = connection();
        assert!(empty.queue_bytes(vec![0; SIXTEEN_MIB - SLOT + 1]).is_err());
        assert_eq!(empty.queued_bytes(), 0);
        assert!(empty.available());
        let (mut records, _peer) = connection();
        for _ in 0..524_288 {
            records.queue_bytes(Vec::new()).unwrap();
        }
        assert_eq!(records.queued_bytes(), SIXTEEN_MIB);
        assert!(records.queue_bytes(Vec::new()).is_err());
    }

    /// A frame written whole releases its charge, and a Fatal error that
    /// replaces a backlog not yet started releases the backlog's.
    #[test]
    fn written_and_replaced_frames_release_their_charge() {
        let (mut connection, mut peer) = connection();
        connection.queue_bytes(vec![7; 10]).unwrap();
        assert_eq!(connection.queued_bytes(), 10 + SLOT);
        connection.flush().unwrap();
        assert_eq!(connection.queued_bytes(), 0);
        let mut received = [0; 10];
        peer.read_exact(&mut received).unwrap();
        assert_eq!(received, [7; 10]);
        connection.queue_bytes(vec![1; 100]).unwrap();
        connection.queue_bytes(vec![2; 100]).unwrap();
        connection.fail(Failure::Fatal(FatalCode::InvalidFrameLength));
        assert_eq!(connection.queued_bytes(), 0);
        let fatal = wire::read_frame(&mut peer, frame_limit()).unwrap().unwrap();
        assert_eq!(fatal.code(), 0x4d);
    }

    /// LIM-01 (frame bound, at the limit and one past): a revision-1 frame
    /// of 1,048,576 bytes, its type byte and body, is queued whole after
    /// its four-byte prefix; one byte more is never sent.
    #[test]
    fn a_server_frame_over_the_limit_is_never_queued() {
        let (mut connection, _peer) = connection();
        let largest = wire::Frame::new(0x41, vec![b' '; 1_048_575]);
        connection.queue_wire(&largest).unwrap();
        assert_eq!(connection.queued_bytes(), 4 + 1_048_576 + SLOT);
        let oversized = wire::Frame::new(0x41, vec![b' '; 1_048_576]);
        assert!(connection.queue_wire(&oversized).is_err());
        assert_eq!(connection.queued_bytes(), 4 + 1_048_576 + SLOT);
    }
}
