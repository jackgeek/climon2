use crate::transport::{Pending, Transport, invalid, poll, ready};
use climon2_core::{
    SessionId,
    protocol::{ClientMessage, ServerMessage, TerminalSize},
};
use climon2_platform::terminal::{self, Signals};
use std::{
    fs::File,
    io::{self, Read},
    os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd},
};

/// The single owner of the terminal's file-status flags while bridging. Both originals
/// are captured before either changes, because stdin and stdout may share one open-file
/// description, and are restored through owned duplicates, so closing the bridge's
/// descriptors can never skip or misdirect restoration.
struct Nonblocking {
    saved: [(OwnedFd, i32); 2],
}

impl Nonblocking {
    fn new(input: BorrowedFd<'_>, output: BorrowedFd<'_>) -> io::Result<Self> {
        let guard = Self {
            saved: [save(input)?, save(output)?],
        };
        guard.apply()?;
        Ok(guard)
    }

    fn apply(&self) -> io::Result<()> {
        for (fd, flags) in &self.saved {
            set_flags(fd, flags | libc::O_NONBLOCK)?;
        }
        Ok(())
    }
}

impl Drop for Nonblocking {
    fn drop(&mut self) {
        for (fd, flags) in self.saved.iter().rev() {
            let _ = set_flags(fd, *flags);
        }
    }
}

fn save(fd: BorrowedFd<'_>) -> io::Result<(OwnedFd, i32)> {
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((fd.try_clone_to_owned()?, flags))
}

fn set_flags(fd: &OwnedFd, flags: i32) -> io::Result<()> {
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct Output;

impl io::Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = unsafe { libc::write(1, bytes.as_ptr().cast(), bytes.len()) };
        if count < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(count as usize)
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Bridge {
    transport: Transport,
    terminal: File,
    signals: Signals,
    size: TerminalSize,
    resize: Option<TerminalSize>,
    output: Pending,
    /// The session closed its end: it accepts no more writes, but what it already
    /// sent (output, Exit, then EOF) is still read once stdout drains.
    hung_up: bool,
}

pub fn run(
    transport: Transport,
    terminal: File,
    signals: Signals,
    size: TerminalSize,
    id: &SessionId,
) -> io::Result<u32> {
    let _flags = Nonblocking::new(terminal.as_fd(), io::stdout().as_fd())?;
    let mut bridge = Bridge {
        transport,
        terminal,
        signals,
        size,
        resize: None,
        output: Pending::default(),
        hung_up: false,
    };
    bridge.run().map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            io::Error::other(format!("lost connection to session {id}"))
        } else {
            error
        }
    })
}

impl Bridge {
    fn run(&mut self) -> io::Result<u32> {
        loop {
            if let Some(signal) = self.signals.termination_signal() {
                return Ok(128 + signal);
            }
            if let Some(code) = self.tick()? {
                return Ok(code);
            }
        }
    }

    fn tick(&mut self) -> io::Result<Option<u32>> {
        self.resize()?;
        let descriptors = self.poll()?;
        self.hung_up |= descriptors[0].revents & libc::POLLHUP != 0;
        self.flush_ready(&descriptors)?;
        self.dispatch(&descriptors)
    }

    fn poll(&self) -> io::Result<[libc::pollfd; 3]> {
        let mut descriptors = self.descriptors();
        poll(&mut descriptors, 20)?;
        Ok(descriptors)
    }

    fn dispatch(&mut self, descriptors: &[libc::pollfd; 3]) -> io::Result<Option<u32>> {
        if let Some(code) = self.receive_ready(descriptors[0].revents)? {
            return Ok(Some(code));
        }
        self.input_ready(descriptors[1].revents)
    }

    fn flush_ready(&mut self, descriptors: &[libc::pollfd; 3]) -> io::Result<()> {
        if descriptors[2].revents != 0 {
            self.output.flush(&mut Output)?;
        }
        if descriptors[0].revents & libc::POLLOUT != 0 {
            self.transport.outgoing.flush(&mut self.transport.stream)?;
        }
        Ok(())
    }

    fn receive_ready(&mut self, events: i16) -> io::Result<Option<u32>> {
        if self.output.empty() && events & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            return self.receive();
        }
        Ok(None)
    }

    fn input_ready(&mut self, events: i16) -> io::Result<Option<u32>> {
        if events != 0 && !self.input()? {
            return Ok(Some(129));
        }
        Ok(None)
    }

    /// Socket, terminal input, stdout; each side waits while the other is backpressured.
    fn descriptors(&self) -> [libc::pollfd; 3] {
        let socket = self.socket_events();
        [
            descriptor(self.transport.stream.as_raw_fd(), socket, socket != 0),
            descriptor(
                self.terminal.as_raw_fd(),
                libc::POLLIN,
                self.transport.outgoing.empty(),
            ),
            descriptor(1, libc::POLLOUT, !self.output.empty()),
        ]
    }

    fn socket_events(&self) -> i16 {
        let mut events = 0;
        if self.output.empty() {
            events |= libc::POLLIN;
        }
        // macOS reports POLLHUP for any requested event, so waiting to write to a
        // hung-up session would spin while stdout is blocked.
        if !self.hung_up && !self.transport.outgoing.empty() {
            events |= libc::POLLOUT;
        }
        events
    }

    fn resize(&mut self) -> io::Result<()> {
        if self.signals.take_resize() {
            let size = terminal::size(&self.terminal)?;
            self.resize = (size != self.size).then_some(size);
        }
        if self.transport.outgoing.empty()
            && let Some(size) = self.resize.take()
        {
            self.transport.send(&ClientMessage::Resize(size))?;
            self.size = size;
        }
        Ok(())
    }

    fn receive(&mut self) -> io::Result<Option<u32>> {
        match self.transport.receive()? {
            None => Ok(None),
            Some(message) => self.message(message),
        }
    }

    fn message(&mut self, message: ServerMessage) -> io::Result<Option<u32>> {
        match message {
            ServerMessage::Output(bytes) => {
                self.output = Pending::new(bytes);
                Ok(None)
            }
            ServerMessage::Exit { code } => Ok(Some(code)),
            ServerMessage::Error { message } => Err(invalid(message)),
            _ => Err(invalid("unexpected session message")),
        }
    }

    fn input(&mut self) -> io::Result<bool> {
        let mut bytes = [0; 32768];
        match self.read_terminal(&mut bytes)? {
            Some(count) => self.forward(&bytes[..count]),
            // Another reader consumed what poll reported.
            None => Ok(true),
        }
    }

    /// An empty read is the terminal hanging up (EOF, or EIO via `hangup_as_eof`).
    fn forward(&mut self, read: &[u8]) -> io::Result<bool> {
        if read.is_empty() {
            return Ok(false);
        }
        self.transport.send(&ClientMessage::Input(read.to_vec()))?;
        Ok(true)
    }

    fn read_terminal(&mut self, bytes: &mut [u8]) -> io::Result<Option<usize>> {
        ready(self.terminal.read(bytes).or_else(hangup_as_eof))
    }
}

/// A lost terminal reads as EIO; like EOF it is a silent HUP.
fn hangup_as_eof(error: io::Error) -> io::Result<usize> {
    if error.raw_os_error() == Some(libc::EIO) {
        return Ok(0);
    }
    Err(error)
}

/// An inactive descriptor is -1, which poll ignores. `revents` is output-only.
fn descriptor(fd: i32, events: i16, active: bool) -> libc::pollfd {
    libc::pollfd {
        fd: if active { fd } else { -1 },
        events,
        revents: Default::default(),
    }
}
