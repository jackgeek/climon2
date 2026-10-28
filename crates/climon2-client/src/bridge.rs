use crate::transport::{Pending, Transport, invalid, poll};
use climon2_core::{
    SessionId,
    protocol::{ClientMessage, ServerMessage, TerminalSize},
};
use climon2_platform::terminal::{self, Signals};
use std::{
    fs::File,
    io::{self, Read},
    os::fd::AsRawFd,
};

struct Nonblocking {
    fd: i32,
    flags: i32,
}

impl Nonblocking {
    fn new(fd: i32) -> io::Result<Self> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd, flags })
    }
}

impl Drop for Nonblocking {
    fn drop(&mut self) {
        unsafe { libc::fcntl(self.fd, libc::F_SETFL, self.flags) };
    }
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
}

pub fn run(
    transport: Transport,
    terminal: File,
    signals: Signals,
    size: TerminalSize,
    id: &SessionId,
) -> io::Result<u32> {
    let _input_flags = Nonblocking::new(terminal.as_raw_fd())?;
    let _output_flags = Nonblocking::new(1)?;
    let mut bridge = Bridge {
        transport,
        terminal,
        signals,
        size,
        resize: None,
        output: Pending::default(),
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
        let mut descriptors = self.descriptors();
        poll(&mut descriptors, 20)?;
        self.flush_ready(&descriptors)?;
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

    fn descriptors(&self) -> [libc::pollfd; 3] {
        let mut socket = 0;
        if self.output.empty() {
            socket |= libc::POLLIN;
        }
        if !self.transport.outgoing.empty() {
            socket |= libc::POLLOUT;
        }
        [
            libc::pollfd {
                fd: if socket == 0 {
                    -1
                } else {
                    self.transport.stream.as_raw_fd()
                },
                events: socket,
                revents: 0,
            },
            libc::pollfd {
                fd: if self.transport.outgoing.empty() {
                    self.terminal.as_raw_fd()
                } else {
                    -1
                },
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if self.output.empty() { -1 } else { 1 },
                events: libc::POLLOUT,
                revents: 0,
            },
        ]
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
        match self.terminal.read(&mut bytes) {
            Ok(0) => Ok(false),
            Ok(count) => self
                .transport
                .send(&ClientMessage::Input(bytes[..count].to_vec()))
                .map(|()| true),
            Err(error) => terminal_read_error(error),
        }
    }
}

fn terminal_read_error(error: io::Error) -> io::Result<bool> {
    if error.raw_os_error() == Some(libc::EIO) {
        return Ok(false);
    }
    match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => Ok(true),
        _ => Err(error),
    }
}
