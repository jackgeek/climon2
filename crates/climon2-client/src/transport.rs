use climon2_core::{
    metadata::SessionMetadata,
    protocol::{
        ClientMessage, Frame, HELLO_TIMEOUT, PROTOCOL_VERSION, ServerMessage, read_frame,
        write_frame,
    },
};
use climon2_platform::terminal::Signals;
use std::{
    io::{self, Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    path::Path,
    time::Instant,
};

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn poll(descriptors: &mut [libc::pollfd], milliseconds: i32) -> io::Result<()> {
    let result = unsafe {
        libc::poll(
            descriptors.as_mut_ptr(),
            descriptors.len() as libc::nfds_t,
            milliseconds,
        )
    };
    if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub struct Transport {
    pub stream: UnixStream,
    decoder: Decoder,
    pub outgoing: Pending,
    deadline: Instant,
}

impl Transport {
    pub fn connect(metadata: &SessionMetadata) -> io::Result<Self> {
        let deadline = Instant::now() + HELLO_TIMEOUT;
        let stream = climon2_platform::connection::connect(
            Path::new(metadata.socket_path.as_str()),
            HELLO_TIMEOUT,
        )
        .map_err(|error| response_error(error, metadata))?;
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            decoder: Decoder::default(),
            outgoing: Pending::default(),
            deadline,
        })
    }

    fn wait(&self, events: i16, signals: &Signals, deadline: Option<Instant>) -> io::Result<()> {
        if signals.termination_signal().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "client interrupted",
            ));
        }
        let milliseconds = match deadline {
            Some(deadline) => {
                let left = deadline
                    .checked_duration_since(Instant::now())
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::TimedOut, "session did not respond")
                    })?;
                left.as_millis().clamp(1, 25) as i32
            }
            None => 25,
        };
        poll(
            &mut [libc::pollfd {
                fd: self.stream.as_raw_fd(),
                events,
                revents: 0,
            }],
            milliseconds,
        )
    }

    pub fn hello(&mut self, metadata: &SessionMetadata, signals: &Signals) -> io::Result<()> {
        let result = self.exchange_hello(metadata, signals);
        result.map_err(|error| response_error(error, metadata))
    }

    fn exchange_hello(&mut self, metadata: &SessionMetadata, signals: &Signals) -> io::Result<()> {
        climon2_platform::peer::authenticate(&self.stream)?;
        self.send(&ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
        })?;
        self.flush_until(signals, Some(self.deadline))?;
        let message = self.receive_until(signals, Some(self.deadline))?;
        validate_hello(message, metadata)
    }

    pub fn attached(&mut self, signals: &Signals) -> io::Result<()> {
        self.flush_until(signals, None)?;
        match self.receive_until(signals, None)? {
            ServerMessage::Attached => Ok(()),
            ServerMessage::Error { message } => Err(invalid(message)),
            _ => Err(invalid("expected Attached")),
        }
    }

    fn flush_until(&mut self, signals: &Signals, deadline: Option<Instant>) -> io::Result<()> {
        while !self.outgoing.empty() {
            self.wait(libc::POLLOUT, signals, deadline)?;
            self.outgoing.flush(&mut self.stream)?;
        }
        Ok(())
    }

    fn receive_until(
        &mut self,
        signals: &Signals,
        deadline: Option<Instant>,
    ) -> io::Result<ServerMessage> {
        loop {
            self.wait(libc::POLLIN, signals, deadline)?;
            if let Some(message) = self.receive()? {
                return Ok(message);
            }
        }
    }

    pub fn send(&mut self, message: &ClientMessage) -> io::Result<()> {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &message.to_frame()?)?;
        self.outgoing = Pending::new(bytes);
        Ok(())
    }

    pub fn receive(&mut self) -> io::Result<Option<ServerMessage>> {
        self.decoder
            .read(&mut self.stream)?
            .map(ServerMessage::from_frame)
            .transpose()
    }
}

#[derive(Default)]
struct Decoder {
    bytes: Vec<u8>,
    position: usize,
}

impl Decoder {
    fn read(&mut self, stream: &mut UnixStream) -> io::Result<Option<Frame>> {
        if self.bytes.is_empty() {
            self.bytes.resize(4, 0);
        }
        if !self.fill(stream)? {
            return Ok(None);
        }
        self.frame()
    }

    fn fill(&mut self, stream: &mut UnixStream) -> io::Result<bool> {
        match stream.read(&mut self.bytes[self.position..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "lost connection to session",
                ));
            }
            Ok(count) => self.position += count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        }
        Ok(self.position == self.bytes.len())
    }

    fn frame(&mut self) -> io::Result<Option<Frame>> {
        match read_frame(&mut self.bytes.as_slice()) {
            Ok(frame) => {
                self.bytes.clear();
                self.position = 0;
                Ok(Some(frame))
            }
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                let length =
                    u32::from_be_bytes(self.bytes[..4].try_into().map_err(io::Error::other)?)
                        as usize;
                // Validate the prefix first, then the type, before allocating a body.
                let target = if self.bytes.len() == 4 { 5 } else { length + 4 };
                self.bytes.resize(target, 0);
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
}

fn validate_hello(message: ServerMessage, metadata: &SessionMetadata) -> io::Result<()> {
    match message {
        ServerMessage::Hello(reply) => {
            reply.validate_client(&metadata.session_id, &metadata.climon2_version)
        }
        ServerMessage::Error { message } => Err(invalid(message)),
        _ => Err(invalid("expected session hello")),
    }
}

fn response_error(error: io::Error, metadata: &SessionMetadata) -> io::Error {
    if error.kind() == io::ErrorKind::TimedOut {
        return io::Error::new(
            io::ErrorKind::TimedOut,
            format!("session {} did not respond", metadata.session_id),
        );
    }
    error
}

#[derive(Default)]
pub struct Pending {
    bytes: Vec<u8>,
    position: usize,
}

impl Pending {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, position: 0 }
    }
    pub fn empty(&self) -> bool {
        self.position == self.bytes.len()
    }
    pub fn flush(&mut self, writer: &mut impl Write) -> io::Result<()> {
        match writer.write(&self.bytes[self.position..]) {
            Ok(0) if !self.empty() => Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "connection write stopped",
            )),
            Ok(count) => {
                self.position += count;
                Ok(())
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}
