use crate::{args::Options, connection::Connection, pty::Pty};
use climon2_core::protocol::{
    ClientMessage, HelloReply, PROTOCOL_VERSION, PTY_DRAIN_CAP, PTY_DRAIN_IDLE, ServerMessage,
    TerminalSize,
};
use climon2_platform::{peer, runtime::SocketListener, terminal::Signals};
use std::{
    collections::BTreeMap,
    io,
    time::{Duration, Instant},
};

const OUTPUT_CHUNK: usize = 32 * 1024;
const MAX_CONNECTIONS: usize = 64;

struct Session<'a> {
    options: &'a Options,
    connections: BTreeMap<u64, Connection>,
    next_id: u64,
    controller: Option<u64>,
    leave: bool,
    input: Vec<u8>,
    input_offset: usize,
    size: TerminalSize,
    exit: Option<u32>,
    drain: Option<Drain>,
}

struct Drain {
    started: Instant,
    idle_since: Option<Instant>,
    eof: bool,
}

pub(crate) fn serve(
    options: &Options,
    listener: &SocketListener,
    pty: &mut Pty,
    signals: &Signals,
) -> io::Result<()> {
    let mut session = Session {
        options,
        connections: BTreeMap::new(),
        next_id: 0,
        controller: None,
        leave: false,
        input: Vec::new(),
        input_offset: 0,
        size: options.size,
        exit: None,
        drain: None,
    };
    match session.run(listener, pty, signals) {
        Ok(()) => pty.finish(),
        Err(error) => {
            let _ = pty.leave();
            Err(error)
        }
    }
}

impl Session<'_> {
    fn run(
        &mut self,
        listener: &SocketListener,
        pty: &mut Pty,
        signals: &Signals,
    ) -> io::Result<()> {
        loop {
            if self.tick(listener, pty, signals)? {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn tick(
        &mut self,
        listener: &SocketListener,
        pty: &mut Pty,
        signals: &Signals,
    ) -> io::Result<bool> {
        self.accept(listener)?;
        self.connections(pty)?;
        if self.leave || signals.termination_signal().is_some() {
            return pty.leave().map(|()| true);
        }
        self.advance(pty)
    }

    fn advance(&mut self, pty: &mut Pty) -> io::Result<bool> {
        self.observe_exit(pty)?;
        if self.exit.is_none() {
            self.input(pty)?;
        }
        self.pump_output(pty)?;
        self.finish()
    }

    fn accept(&mut self, listener: &SocketListener) -> io::Result<()> {
        // Bound each turn, so an accept flood cannot starve the controlling client.
        for _ in 0..8 {
            let Some(stream) = Self::accept_one(listener)? else {
                break;
            };
            self.add_connection(stream)?;
        }
        Ok(())
    }

    fn add_connection(
        &mut self,
        stream: climon2_platform::runtime::SocketStream,
    ) -> io::Result<()> {
        if self.connections.len() < MAX_CONNECTIONS {
            self.connections
                .insert(self.next_id, Connection::new(stream)?);
            self.next_id += 1;
        }
        Ok(())
    }

    fn connections(&mut self, pty: &Pty) -> io::Result<()> {
        let ids: Vec<_> = self.connections.keys().copied().collect();
        for id in ids {
            self.connection(id, pty)?;
        }
        Ok(())
    }

    fn connection(&mut self, id: u64, pty: &Pty) -> io::Result<()> {
        if self
            .connections
            .get_mut(&id)
            .expect("connection")
            .flush()
            .is_err()
        {
            self.disconnected(id);
            return Ok(());
        }
        if self.input_pending(id) {
            return self.check_disconnect(id);
        }
        self.receive(id, pty)
    }

    fn input_pending(&self, id: u64) -> bool {
        self.controller == Some(id) && !self.input.is_empty()
    }

    fn check_disconnect(&mut self, id: u64) -> io::Result<()> {
        if self
            .connections
            .get(&id)
            .expect("connection")
            .disconnected()?
        {
            self.disconnected(id);
        }
        Ok(())
    }

    fn receive(&mut self, id: u64, pty: &Pty) -> io::Result<()> {
        let incoming = self.connections.get_mut(&id).expect("connection").receive();
        match incoming {
            Ok(Some(message)) => self.message(id, message, pty),
            Ok(None) => Ok(()),
            Err(error) => {
                self.rejected(id, &error);
                Ok(())
            }
        }
    }

    fn rejected(&mut self, id: u64, error: &io::Error) {
        self.connections
            .get_mut(&id)
            .expect("connection")
            .error(&protocol_error(error));
        self.disconnected(id);
    }

    fn disconnected(&mut self, id: u64) {
        self.connections.remove(&id);
        if self.controller == Some(id) {
            self.leave = true;
        }
    }

    fn message(&mut self, id: u64, message: ClientMessage, pty: &Pty) -> io::Result<()> {
        match message {
            ClientMessage::Hello { .. } => self.hello(id),
            ClientMessage::Attach(size) => self.attach(id, size, pty),
            ClientMessage::Input(bytes) => {
                self.input = bytes;
                Ok(())
            }
            control => self.control(id, control, pty),
        }
    }

    fn hello(&mut self, id: u64) -> io::Result<()> {
        self.reply(
            id,
            ServerMessage::Hello(HelloReply {
                session_id: self.options.id.clone(),
                climon2_version: self.options.version.clone(),
                protocol: PROTOCOL_VERSION,
            }),
        )
    }

    /// Resize applies to the PTY; the only other client message, Error, disconnects.
    fn control(&mut self, id: u64, message: ClientMessage, pty: &Pty) -> io::Result<()> {
        if let ClientMessage::Resize(size) = message {
            return self.resize(size, pty);
        }
        self.disconnected(id);
        Ok(())
    }

    fn reply(&mut self, id: u64, message: ServerMessage) -> io::Result<()> {
        let connection = self.connections.get_mut(&id).expect("connection");
        connection.flush()?;
        connection.queue(message)
    }

    fn attach(&mut self, id: u64, size: TerminalSize, pty: &Pty) -> io::Result<()> {
        if self.controller.is_some() {
            self.connections
                .get_mut(&id)
                .expect("connection")
                .error("session already has a controlling client");
            self.disconnected(id);
            return Ok(());
        }
        self.resize(size.initial(), pty)?;
        self.controller = Some(id);
        self.reply(id, ServerMessage::Attached)
    }

    fn resize(&mut self, size: TerminalSize, pty: &Pty) -> io::Result<()> {
        if let Some(size) = size.resize().filter(|size| *size != self.size) {
            pty.resize(size)?;
            self.size = size;
        }
        Ok(())
    }

    fn input(&mut self, pty: &mut Pty) -> io::Result<()> {
        if self.input.is_empty() {
            return Ok(());
        }
        if let Some(count) = super::unless_would_block(pty.write(&self.input[self.input_offset..]))?
        {
            self.wrote_input(count);
        }
        Ok(())
    }

    fn wrote_input(&mut self, count: usize) {
        self.input_offset += count;
        if self.input_offset == self.input.len() {
            self.input.clear();
            self.input_offset = 0;
        }
    }

    fn observe_exit(&mut self, pty: &mut Pty) -> io::Result<()> {
        if self.exit.is_none() {
            self.exit = pty.try_wait()?;
        }
        self.start_drain();
        Ok(())
    }

    /// Drain starts once, when a reaped command has a controlling client.
    fn start_drain(&mut self) {
        if self.exit.is_some() && self.controller.is_some() && self.drain.is_none() {
            self.drain = Some(Drain {
                started: Instant::now(),
                idle_since: None,
                eof: false,
            });
        }
    }

    /// The sole PTY reader. Per-connection pending output is at most one 32 KiB frame.
    fn pump_output(&mut self, pty: &Pty) -> io::Result<()> {
        let Some(id) = self.controller else {
            return Ok(());
        };
        if !self.connections.get(&id).expect("controller").available() {
            return Ok(());
        }
        if self.drain.as_ref().is_some_and(Drain::finished) {
            return Ok(());
        }
        let mut bytes = vec![0; OUTPUT_CHUNK];
        let result = pty.read_output(&mut bytes);
        self.output_result(id, bytes, result)
    }

    fn output_result(
        &mut self,
        id: u64,
        bytes: Vec<u8>,
        result: io::Result<usize>,
    ) -> io::Result<()> {
        match super::unless_would_block(eio_as_eof(result))? {
            None => self.idle(),
            Some(0) => self.eof(),
            Some(count) => return self.output(id, bytes, count),
        }
        Ok(())
    }

    fn output(&mut self, id: u64, mut bytes: Vec<u8>, count: usize) -> io::Result<()> {
        bytes.truncate(count);
        self.reply(id, ServerMessage::Output(bytes))?;
        if let Some(drain) = &mut self.drain {
            drain.idle_since = None;
        }
        Ok(())
    }

    fn idle(&mut self) {
        if let Some(drain) = &mut self.drain {
            drain.idle_since.get_or_insert_with(Instant::now);
        }
    }

    fn eof(&mut self) {
        if let Some(drain) = &mut self.drain {
            drain.eof = true;
        }
    }

    fn accept_one(
        listener: &SocketListener,
    ) -> io::Result<Option<climon2_platform::runtime::SocketStream>> {
        match peer::accept(listener) {
            Ok(stream) => Ok(Some(stream)),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::PermissionDenied
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn finish(&mut self) -> io::Result<bool> {
        let Some(drain) = &self.drain else {
            return Ok(false);
        };
        if !drain.finished() {
            return Ok(false);
        }
        let id = self.controller.expect("attached drain");
        let connection = self.connections.get_mut(&id).expect("controller");
        connection.finish(self.exit.expect("reaped drain"))?;
        Ok(true)
    }
}

/// A hung-up macOS PTY master reports EIO, which is end of output like a 0-byte read.
fn eio_as_eof(result: io::Result<usize>) -> io::Result<usize> {
    match result {
        Err(error) if error.raw_os_error() == Some(libc::EIO) => Ok(0),
        result => result,
    }
}

fn protocol_error(error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::InvalidData {
        return error.to_string();
    }
    "invalid session protocol".into()
}

impl Drain {
    fn finished(&self) -> bool {
        self.eof
            || self
                .idle_since
                .is_some_and(|idle| idle.elapsed() >= PTY_DRAIN_IDLE)
            || self.started.elapsed() >= PTY_DRAIN_CAP
    }
}
