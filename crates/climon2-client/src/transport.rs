use climon2_core::{
    hello::{Capabilities, HelloExtensions},
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

/// `None` when a nonblocking read or write must simply be retried after the next poll.
pub fn ready(result: io::Result<usize>) -> io::Result<Option<usize>> {
    match result {
        Ok(count) => Ok(Some(count)),
        Err(error) if retryable(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn retryable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
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

/// What the client asks of the session after hello.
#[derive(Clone, Copy)]
pub enum Request {
    Attach,
    Kill,
}

impl Request {
    fn supported(self, capabilities: &Capabilities) -> bool {
        match self {
            Self::Attach => capabilities.supports_attach(),
            Self::Kill => capabilities.supports_kill(),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Attach => "attach",
            Self::Kill => "kill",
        }
    }
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

    pub fn hello(
        &mut self,
        metadata: &SessionMetadata,
        signals: &Signals,
        request: Request,
    ) -> io::Result<()> {
        let result = self.exchange_hello(metadata, signals, request);
        result.map_err(|error| response_error(error, metadata))
    }

    fn exchange_hello(
        &mut self,
        metadata: &SessionMetadata,
        signals: &Signals,
        request: Request,
    ) -> io::Result<()> {
        climon2_platform::peer::authenticate(&self.stream)?;
        self.send(&ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
        })?;
        self.flush_until(signals, Some(self.deadline))?;
        let frame = self.receive_frame_until(signals, Some(self.deadline))?;
        validate_hello(frame, metadata, request)
    }

    pub fn attached(&mut self, signals: &Signals) -> io::Result<()> {
        self.flush_until(signals, None)?;
        match self.receive_until(signals, None)? {
            ServerMessage::Attached => Ok(()),
            message => Err(rejection(message, "expected Attached")),
        }
    }

    /// The daemon confirms only after the command ended and cleanup finished.
    pub fn terminated(&mut self, signals: &Signals) -> io::Result<()> {
        self.flush_until(signals, None)?;
        match self.receive_until(signals, None)? {
            ServerMessage::Terminated { .. } => Ok(()),
            message => Err(rejection(message, "expected Terminated")),
        }
    }

    fn flush_until(&mut self, signals: &Signals, deadline: Option<Instant>) -> io::Result<()> {
        while !self.outgoing.empty() {
            self.wait(libc::POLLOUT, signals, deadline)?;
            self.flush()?;
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

    fn receive_frame_until(
        &mut self,
        signals: &Signals,
        deadline: Option<Instant>,
    ) -> io::Result<Frame> {
        loop {
            self.wait(libc::POLLIN, signals, deadline)?;
            if let Some(frame) = self.decoder.read(&mut self.stream)? {
                return Ok(frame);
            }
        }
    }

    pub fn send(&mut self, message: &ClientMessage) -> io::Result<()> {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &message.to_frame()?)?;
        self.outgoing.append(bytes);
        Ok(())
    }

    /// A write to a lost session discards what was outgoing; reading reports the loss.
    pub fn flush(&mut self) -> io::Result<()> {
        match self.outgoing.flush(&mut self.stream) {
            Err(error) if lost(&error) => {
                self.outgoing = Pending::default();
                Ok(())
            }
            result => result,
        }
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
    fn read(&mut self, stream: &mut impl Read) -> io::Result<Option<Frame>> {
        if self.bytes.is_empty() {
            self.bytes.resize(4, 0);
        }
        if !self.fill(stream)? {
            return Ok(None);
        }
        self.frame()
    }

    fn fill(&mut self, stream: &mut impl Read) -> io::Result<bool> {
        let read = stream.read(&mut self.bytes[self.position..]);
        let Some(count) = ready(read).map_err(lost_as_eof)? else {
            return Ok(false);
        };
        if count == 0 {
            return Err(lost_connection());
        }
        self.position += count;
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
                self.grow();
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Validate the prefix first, then the type, before allocating a body.
    fn grow(&mut self) {
        let mut prefix = [0; 4];
        prefix.copy_from_slice(&self.bytes[..4]);
        let target = if self.bytes.len() == 4 {
            5
        } else {
            u32::from_be_bytes(prefix) as usize + 4
        };
        self.bytes.resize(target, 0);
    }
}

/// However the OS reports it, a session connection that is gone is unexpected loss.
fn lost(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::NotConnected
    )
}

fn lost_as_eof(error: io::Error) -> io::Error {
    if lost(&error) {
        return lost_connection();
    }
    error
}

fn lost_connection() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "lost connection to session")
}

fn validate_hello(frame: Frame, metadata: &SessionMetadata, request: Request) -> io::Result<()> {
    let extensions = HelloExtensions::from_body(frame.body());
    match ServerMessage::from_frame(frame)? {
        ServerMessage::Hello(reply) => {
            reply.validate_client(&metadata.session_id, &metadata.climon2_version)?;
            supports(&extensions, request)
        }
        message => Err(rejection(message, "expected session hello")),
    }
}

fn supports(extensions: &HelloExtensions, request: Request) -> io::Result<()> {
    extensions
        .capabilities
        .valid()
        .is_some_and(|capabilities| request.supported(capabilities))
        .then_some(())
        .ok_or_else(|| invalid(format!("session does not support {}", request.name())))
}

/// The session's own diagnosis when it sent one, otherwise what was expected.
fn rejection(message: ServerMessage, expected: &str) -> io::Error {
    match message {
        ServerMessage::Error { message } => invalid(message),
        _ => invalid(expected),
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

    pub fn append(&mut self, mut bytes: Vec<u8>) {
        if self.empty() {
            self.bytes = bytes;
            self.position = 0;
        } else {
            self.bytes.append(&mut bytes);
        }
    }

    /// Only called while bytes remain, after poll reported the writer ready; a socket or
    /// terminal write of a nonempty buffer then never returns 0.
    pub fn flush(&mut self, writer: &mut impl Write) -> io::Result<()> {
        if let Some(count) = ready(writer.write(&self.bytes[self.position..]))? {
            self.position += count;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Decoder, Pending, Transport};
    use climon2_core::protocol::ClientMessage;
    use std::{
        io::{self, ErrorKind, Read},
        os::{fd::OwnedFd, unix::net::UnixStream},
        time::Instant,
    };

    struct Failing(ErrorKind);

    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(self.0.into())
        }
    }

    #[test]
    fn a_connection_that_fails_while_reading_is_lost_and_other_errors_stay_themselves() {
        for kind in [
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
            ErrorKind::BrokenPipe,
            ErrorKind::NotConnected,
            ErrorKind::PermissionDenied,
        ] {
            let error = Decoder::default().read(&mut Failing(kind)).unwrap_err();
            let lost = kind != ErrorKind::PermissionDenied;
            let expected = if lost { ErrorKind::UnexpectedEof } else { kind };
            assert_eq!(error.kind(), expected, "{kind:?}");
        }
    }

    fn transport(stream: UnixStream) -> Transport {
        let mut transport = Transport {
            stream,
            decoder: Decoder::default(),
            outgoing: Pending::default(),
            deadline: Instant::now(),
        };
        transport.send(&ClientMessage::Detach).unwrap();
        transport
    }

    #[test]
    fn writing_to_a_lost_session_discards_outgoing_and_reading_reports_the_loss() {
        let (stream, peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let mut transport = transport(stream);
        drop(peer);
        transport.flush().unwrap();
        assert!(transport.outgoing.empty());
        let error = transport.receive().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    }

    #[test]
    fn other_write_failures_are_not_mistaken_for_loss() {
        let read_only = std::fs::File::open("/dev/null").unwrap();
        let mut transport = transport(UnixStream::from(OwnedFd::from(read_only)));
        let error = transport.flush().unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ENOTSOCK));
        assert!(!transport.outgoing.empty());
    }
}
