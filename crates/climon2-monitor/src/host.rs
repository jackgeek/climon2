//! The host terminal while the monitor owns it, through the platform terminal
//! seam: termios and signals there, the monitor's own modes here.
use climon2_core::protocol::TerminalSize;
use climon2_platform::terminal::{self, RawModeGuard, Signals};
use climon2_vt::Presentation;
use std::{
    fs::File,
    io::{self, Read, Write},
    os::fd::AsRawFd,
};

/// The monitor's own alternate screen, saving the shell's cursor, then
/// bracketed paste, SGR mouse reporting of buttons and drags, and focus
/// reporting.
const ENTER: &[u8] = b"\x1b[?1049h\x1b[?2004h\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[?1004h";
/// Leaves the alternate screen, restoring the shell's cursor.
const LEAVE: &[u8] = b"\x1b[?1049l";
/// How long one wait for host input lasts before signals are checked again.
const TICK_MILLISECONDS: i32 = 20;
const READ_BYTES: usize = 4096;

/// Why the monitor leaves through its clean path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Ending {
    /// Prefix+x.
    Exit,
    /// A catchable termination signal.
    Signal(u32),
}

impl Ending {
    pub(crate) fn status(self) -> u32 {
        match self {
            Self::Exit => 0,
            Self::Signal(signal) => 128 + signal,
        }
    }
}

/// What happened at the host since the last event.
pub(crate) enum Event<'a> {
    End(Ending),
    Resize(TerminalSize),
    /// Raw host input, exactly as read.
    Input(&'a [u8]),
    Idle,
}

/// Field order is drop order: termios is restored before the signal handlers
/// go, so a late signal cannot strand the host in raw mode.
pub(crate) struct Host {
    _raw: RawModeGuard,
    signals: Signals,
    input: File,
    /// A presentation that never entered an alternate screen: its release is
    /// exactly the documented normal-shell baseline.
    baseline: Presentation,
    buffer: [u8; READ_BYTES],
}

impl Host {
    /// Signals are caught before raw mode, and raw mode precedes any mode change.
    pub(crate) fn enter() -> io::Result<(Self, TerminalSize)> {
        let input = terminal::input()?;
        let size = terminal::size(&input)?;
        let host = Self::owning(input, size)?;
        write_host(ENTER)?;
        Ok((host, size))
    }

    fn owning(input: File, size: TerminalSize) -> io::Result<Self> {
        let signals = Signals::client()?;
        let raw = RawModeGuard::new(&input)?;
        Ok(Self {
            _raw: raw,
            signals,
            input,
            baseline: Presentation::new(size.cols, size.rows),
            buffer: [0u8; READ_BYTES],
        })
    }

    /// Returns the host to the documented normal-shell baseline, then restores
    /// termios once the bytes are out. The baseline resets margins and origin
    /// mode, which home the cursor, so it is applied on the monitor's alternate
    /// screen and leaving that screen last restores the shell's cursor.
    pub(crate) fn release(mut self) -> io::Result<()> {
        let mut bytes = self.baseline.release();
        bytes.extend_from_slice(LEAVE);
        write_host(&bytes)
    }

    /// The next thing to act on: termination signals first, then resizes,
    /// then host input.
    pub(crate) fn next(&mut self) -> io::Result<Event<'_>> {
        if let Some(signal) = self.signals.termination_signal() {
            return Ok(Event::End(Ending::Signal(signal)));
        }
        if self.signals.take_resize() {
            return terminal::size(&self.input).map(Event::Resize);
        }
        self.input()
    }

    fn input(&mut self) -> io::Result<Event<'_>> {
        if !self.readable() {
            return Ok(Event::Idle);
        }
        self.read().map(Event::Input)
    }

    /// Waits one tick for input. An interrupted or failed wait reports none,
    /// and the caller checks signals before waiting again.
    fn readable(&self) -> bool {
        let mut descriptor = libc::pollfd {
            fd: self.input.as_raw_fd(),
            events: libc::POLLIN,
            revents: Default::default(),
        };
        unsafe { libc::poll(&mut descriptor, 1, TICK_MILLISECONDS) };
        descriptor.revents != 0
    }

    fn read(&mut self) -> io::Result<&[u8]> {
        let count = self.input.read(&mut self.buffer)?;
        match &self.buffer[..count] {
            [] => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the host terminal closed",
            )),
            bytes => Ok(bytes),
        }
    }
}

/// The monitor owns the host output: nothing else writes to stdout.
fn write_host(bytes: &[u8]) -> io::Result<()> {
    let mut output = io::stdout().lock();
    output.write_all(bytes)?;
    output.flush()
}
