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
        write_frame(
            &mut self.outgoing,
            &ServerMessage::Exit { code }.to_frame()?,
        )?;
        let deadline = Instant::now() + climon2_core::protocol::PTY_DRAIN_CAP;
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
        match self.stream.write(&self.outgoing[self.written..]) {
            Ok(0) => Err(super::io_error("connection closed")),
            Ok(count) => {
                self.advance_written(count);
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(error),
        }
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
        let target = self.target()?;
        let mut bytes = vec![0; (target - self.incoming.len()).min(8192)];
        match self.stream.read(&mut bytes) {
            Ok(0) => return Err(super::io_error("connection closed")),
            Ok(count) => self.incoming.extend_from_slice(&bytes[..count]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error),
        }
        self.complete()
    }

    fn target(&self) -> io::Result<usize> {
        if self.incoming.len() < 4 {
            return Ok(4);
        }
        let length =
            u32::from_be_bytes(self.incoming[..4].try_into().expect("frame prefix")) as usize;
        if !(1..=MAX_FRAME_LENGTH).contains(&length) {
            return Err(super::io_error("invalid frame length"));
        }
        if self.incoming.len() == 4 {
            return Ok(5);
        }
        climon2_core::protocol::Frame::new(self.incoming[4], Vec::new())?;
        validate_data_length(self.incoming[4], length)?;
        Ok(4 + length)
    }

    fn complete(&mut self) -> io::Result<Option<ClientMessage>> {
        if self.incoming.len() < 5 || self.incoming.len() != self.target()? {
            return Ok(None);
        }
        let bytes = std::mem::take(&mut self.incoming);
        let message = parsed_message(&bytes)?;
        self.state.receive(&message)?;
        Ok(Some(message))
    }
}

fn parsed_message(bytes: &[u8]) -> io::Result<ClientMessage> {
    let mut reader = bytes;
    ClientMessage::from_frame(read_frame(&mut reader)?)
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
