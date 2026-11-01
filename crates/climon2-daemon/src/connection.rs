use climon2_core::protocol::{
    ClientMessage, ConnectionState, HELLO_TIMEOUT, MAX_FRAME_LENGTH, ServerMessage, read_frame,
    write_frame,
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

struct PendingFrame {
    bytes: Vec<u8>,
    written: usize,
}

pub(crate) struct Connection {
    stream: UnixStream,
    pub state: ConnectionState,
    accepted: Instant,
    incoming: Vec<u8>,
    outgoing: VecDeque<PendingFrame>,
    queued_bytes: usize,
    last_progress: Instant,
    terminal: bool,
    close_when_empty: bool,
}

impl Connection {
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            state: ConnectionState::default(),
            accepted: Instant::now(),
            incoming: Vec::new(),
            outgoing: VecDeque::new(),
            queued_bytes: 0,
            last_progress: Instant::now(),
            terminal: false,
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
        self.queued_bytes > 0 && now.duration_since(self.last_progress) >= NO_PROGRESS
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
        if self.queued_bytes.saturating_add(bytes.len()) > WRITE_WINDOW {
            return Err(super::io_error("connection output queue is full"));
        }
        self.queued_bytes += bytes.len();
        self.outgoing.push_back(PendingFrame { bytes, written: 0 });
        Ok(())
    }

    pub fn error(&mut self, message: &str) {
        self.terminal = true;
        // A terminal error may replace unsent output, but never splice a partially written frame.
        if self.outgoing.front().is_none_or(|frame| frame.written == 0) {
            self.queued_bytes = 0;
            self.outgoing.clear();
        }
        let _ = self.queue(ServerMessage::Error {
            message: message.into(),
        });
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
    /// in one iteration.
    pub fn receive_available(&mut self) -> io::Result<Vec<ClientMessage>> {
        let mut messages = Vec::new();
        if !self.receiving() {
            return Ok(messages);
        }
        while let Some(message) = self.receive()? {
            messages.extend(message);
        }
        Ok(messages)
    }

    fn receiving(&self) -> bool {
        !(self.terminal || self.close_when_empty)
    }

    /// `None` once nothing is readable; `Some(None)` after part of a frame.
    fn receive(&mut self) -> io::Result<Option<Option<ClientMessage>>> {
        self.validate_receive()?;
        self.read_message()
    }

    fn validate_receive(&self) -> io::Result<()> {
        if self.terminal {
            return Err(super::io_error("connection closed"));
        }
        if self.state == ConnectionState::AwaitingHello && self.accepted.elapsed() >= HELLO_TIMEOUT
        {
            return Err(super::io_error("hello deadline exceeded"));
        }
        Ok(())
    }

    fn read_message(&mut self) -> io::Result<Option<Option<ClientMessage>>> {
        let mut bytes = self.read_buffer()?;
        match super::unless_would_block(self.stream.read(&mut bytes))? {
            Some(count) => self.received(&bytes[..count]).map(Some),
            None => Ok(None),
        }
    }

    /// Never reads past the current frame, so each read holds at most one frame.
    fn read_buffer(&self) -> io::Result<Vec<u8>> {
        Ok(vec![0; (self.target()? - self.incoming.len()).min(8192)])
    }

    fn received(&mut self, bytes: &[u8]) -> io::Result<Option<ClientMessage>> {
        if bytes.is_empty() {
            return Err(super::io_error("connection closed"));
        }
        self.incoming.extend_from_slice(bytes);
        self.complete()
    }

    /// Bytes needed for the frame being decoded: prefix, then type, then the whole frame.
    fn target(&self) -> io::Result<usize> {
        match self.incoming.len() {
            0..4 => Ok(4),
            4 => self.frame_length().map(|_| 5),
            _ => self.frame_target(),
        }
    }

    fn frame_length(&self) -> io::Result<usize> {
        let length =
            u32::from_be_bytes(self.incoming[..4].try_into().expect("frame prefix")) as usize;
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

    fn complete(&mut self) -> io::Result<Option<ClientMessage>> {
        if !self.frame_complete()? {
            return Ok(None);
        }
        let bytes = std::mem::take(&mut self.incoming);
        self.message(&bytes).map(Some)
    }

    fn frame_complete(&self) -> io::Result<bool> {
        Ok(self.incoming.len() >= 5 && self.incoming.len() == self.target()?)
    }

    fn message(&mut self, bytes: &[u8]) -> io::Result<ClientMessage> {
        let message = parsed_message(bytes)?;
        self.state.receive(&message)?;
        Ok(message)
    }
}

fn parsed_message(bytes: &[u8]) -> io::Result<ClientMessage> {
    let mut reader = bytes;
    ClientMessage::from_frame(read_frame(&mut reader)?)
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

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}
