use climon2_core::protocol::{
    ClientMessage, ConnectionState, HELLO_TIMEOUT, MAX_FRAME_LENGTH, ServerMessage, read_frame,
    write_frame,
};
use std::{
    io::{self, Read, Write},
    net::Shutdown,
    os::{fd::AsRawFd, unix::net::UnixStream},
    time::Instant,
};

pub(crate) struct Connection {
    stream: UnixStream,
    pub state: ConnectionState,
    accepted: Instant,
    incoming: Vec<u8>,
    outgoing: Vec<u8>,
    written: usize,
    terminal: bool,
}

impl Connection {
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            state: ConnectionState::default(),
            accepted: Instant::now(),
            incoming: Vec::new(),
            outgoing: Vec::new(),
            written: 0,
            terminal: false,
        })
    }

    pub fn available(&self) -> bool {
        self.outgoing.is_empty()
    }

    pub fn disconnected(&self) -> io::Result<bool> {
        let mut descriptor = libc::pollfd {
            fd: self.stream.as_raw_fd(),
            events: 0,
            revents: 0,
        };
        if unsafe { libc::poll(&mut descriptor, 1, 0) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(descriptor.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0)
    }

    pub fn queue(&mut self, message: ServerMessage) -> io::Result<()> {
        if !self.available() {
            return Err(super::io_error("connection output queue is full"));
        }
        write_frame(&mut self.outgoing, &message.to_frame()?)
    }

    pub fn finish(&mut self, code: u32) -> io::Result<()> {
        let frame = ServerMessage::Exit { code }.to_frame()?;
        write_frame(&mut self.outgoing, &frame)?;
        self.flush_until(Instant::now() + climon2_core::protocol::PTY_DRAIN_CAP)
    }

    fn flush_until(&mut self, deadline: Instant) -> io::Result<()> {
        while !self.available() && Instant::now() < deadline {
            self.flush()?;
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Ok(())
    }

    pub fn error(&mut self, message: &str) {
        self.terminal = true;
        // A terminal error may replace unsent output, but never splice a partially written frame.
        if self.written == 0 {
            self.outgoing.clear();
        }
        if self.available() {
            let _ = self.queue(ServerMessage::Error {
                message: message.into(),
            });
        }
        let _ = self.flush();
    }

    pub fn flush(&mut self) -> io::Result<()> {
        if self.available() {
            return Ok(());
        }
        // A stream write of a non-empty buffer never returns 0; it makes progress or fails.
        if let Some(count) =
            super::unless_would_block(self.stream.write(&self.outgoing[self.written..]))?
        {
            self.advance_written(count);
        }
        Ok(())
    }

    fn advance_written(&mut self, count: usize) {
        self.written += count;
        if self.written == self.outgoing.len() {
            self.outgoing.clear();
            self.written = 0;
        }
    }

    pub fn receive(&mut self) -> io::Result<Option<ClientMessage>> {
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

    fn read_message(&mut self) -> io::Result<Option<ClientMessage>> {
        let mut bytes = self.read_buffer()?;
        match super::unless_would_block(self.stream.read(&mut bytes))? {
            Some(count) => self.received(&bytes[..count]),
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
