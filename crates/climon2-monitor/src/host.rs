//! The host terminal while the monitor owns it, through the platform terminal
//! seam: termios and signals there, the monitor's own modes here.
use climon2_core::protocol::TerminalSize;
use climon2_platform::{
    terminal::{self, Bridged, RawModeGuard, Signals},
    waiting::{self, Watch},
};
use climon2_vt::Presentation;
use std::{
    fs::File,
    io::{self, Read, Write},
    time::Duration,
};

/// The monitor's own alternate screen, saving the shell's cursor, then
/// bracketed paste, SGR mouse reporting of buttons and drags, and focus
/// reporting.
const ENTER: &[u8] = b"\x1b[?1049h\x1b[?2004h\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[?1004h";
/// Leaves the alternate screen, restoring the shell's cursor.
const LEAVE: &[u8] = b"\x1b[?1049l";
/// How long one wait for host input, or room for host output, lasts before
/// signals are checked again.
const TICK: Duration = Duration::from_millis(20);
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
    /// Host input, and stdout, neither of which blocks (#285).
    terminal: Bridged,
    /// A presentation that never entered an alternate screen: its release is
    /// exactly the documented normal-shell baseline.
    baseline: Presentation,
    buffer: [u8; READ_BYTES],
    /// A termination signal was reported: it stays caught, and later ones
    /// change nothing, so host input is read again (#118).
    signalled: bool,
}

impl Host {
    /// Signals are caught before raw mode, and raw mode precedes any mode change.
    pub(crate) fn enter() -> io::Result<(Self, TerminalSize)> {
        let input = terminal::input()?;
        let size = terminal::size(&input)?;
        let mut host = Self::owning(input, size)?;
        host.finish(&mut Output::queued(ENTER))?;
        Ok((host, size))
    }

    fn owning(terminal: File, size: TerminalSize) -> io::Result<Self> {
        let signals = Signals::client()?;
        let raw = RawModeGuard::new(&terminal)?;
        Ok(Self {
            _raw: raw,
            signals,
            terminal: Bridged::new(terminal)?,
            baseline: Presentation::new(size.cols, size.rows),
            buffer: [0u8; READ_BYTES],
            signalled: false,
        })
    }

    /// Finishes what `output` holds, then returns the host to the
    /// documented normal-shell baseline, then restores termios once the
    /// bytes are out; the caller sends the root clear after it (#114). The
    /// baseline resets margins and origin mode, which home the cursor, so
    /// it is applied on the monitor's alternate screen and leaving that
    /// screen restores the shell's cursor.
    pub(crate) fn release(mut self, mut output: Output) -> io::Result<()> {
        #[cfg(feature = "test-hooks")]
        crate::hooks::stall("release");
        output.queue(&self.baseline.release());
        output.queue(LEAVE);
        self.finish(&mut output)
    }

    /// Writes everything `output` holds, waiting for room as long as that
    /// takes: only outside the event loop, whose end the final host phase
    /// bounds.
    fn finish(&mut self, output: &mut Output) -> io::Result<()> {
        loop {
            self.write(output)?;
            if output.idle() {
                return Ok(());
            }
            let mut watches = [output.watch(&self.terminal)];
            if let Err(error) = waiting::wait(&mut watches, TICK) {
                interrupted(error)?;
            }
        }
    }

    /// Writes as much of `output` as the host takes now, without waiting.
    pub(crate) fn write(&mut self, output: &mut Output) -> io::Result<()> {
        output.write(&mut self.terminal)
    }

    /// The next thing to act on: the first termination signal, once, then
    /// resizes, then host input. Waiting for input also ends once the host
    /// has room for `output`.
    pub(crate) fn next(&mut self, output: &Output) -> io::Result<Event<'_>> {
        let signal = self.signals.termination_signal();
        if let Some(signal) = signal.filter(|_| !self.signalled) {
            self.signalled = true;
            return Ok(Event::End(Ending::Signal(signal)));
        }
        if self.signals.take_resize() {
            return self.terminal.size().map(Event::Resize);
        }
        self.input(output)
    }

    /// Input a ready wait slot did not hold after all, as a Windows console
    /// slot may not, is idle.
    fn input(&mut self, output: &Output) -> io::Result<Event<'_>> {
        if !self.readable(output)? {
            return Ok(Event::Idle);
        }
        match self.read() {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(Event::Idle),
            read => read.map(Event::Input),
        }
    }

    /// Waits one tick for input, or until the host has room for `output`.
    /// A wait that a signal interrupts finds none, and the caller checks
    /// signals before waiting again.
    fn readable(&self, output: &Output) -> io::Result<bool> {
        let mut watches = [self.terminal.watch_input(), output.watch(&self.terminal)];
        #[cfg(feature = "test-hooks")]
        if let Err(error) = crate::hooks::wait_failure() {
            return interrupted(error);
        }
        match waiting::wait(&mut watches, TICK) {
            Err(error) => interrupted(error),
            Ok(_) => Ok(watches[0].ready()),
        }
    }

    fn read(&mut self) -> io::Result<&[u8]> {
        let count = self.terminal.read(&mut self.buffer)?;
        match &self.buffer[..count] {
            [] => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the host terminal closed",
            )),
            bytes => Ok(bytes),
        }
    }
}

/// An interrupted wait found no input; any other failed wait ends the monitor.
fn interrupted(error: io::Error) -> io::Result<bool> {
    if error.kind() == io::ErrorKind::Interrupted {
        return Ok(false);
    }
    Err(io::Error::new(
        error.kind(),
        format!("cannot wait for host input: {error}"),
    ))
}

/// Bytes on their way to the host, in order: once queued, they are written
/// whole, however long the host takes (#285).
#[derive(Default)]
pub(crate) struct Output {
    bytes: Vec<u8>,
    written: usize,
}

impl Output {
    fn queued(bytes: &[u8]) -> Self {
        let mut output = Self::default();
        output.queue(bytes);
        output
    }

    /// `bytes` follow everything queued before them.
    pub(crate) fn queue(&mut self, bytes: &[u8]) {
        self.bytes.drain(..self.written);
        self.written = 0;
        self.bytes.extend_from_slice(bytes);
    }

    /// Whether everything queued has been written.
    pub(crate) fn idle(&self) -> bool {
        self.written == self.bytes.len()
    }

    /// The wait slot that is ready once `terminal` has room for what waits:
    /// idle while nothing does.
    fn watch<'a>(&self, terminal: &'a Bridged) -> Watch<'a> {
        if self.idle() {
            return Watch::idle();
        }
        terminal.watch_output()
    }

    /// Writes until everything is written or `to` takes no more now. A
    /// write to a terminal of a nonempty buffer never returns 0.
    fn write(&mut self, to: &mut impl Write) -> io::Result<()> {
        while !self.idle() {
            match to.write(&self.bytes[self.written..]) {
                Ok(count) => self.written += count,
                Err(error) if later(&error) => return Ok(()),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

/// A write that the host refused for now, to be tried again once it has
/// room.
fn later(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}

/// Only the root clear, once the host is released, is written to stdout
/// directly: nothing else writes to it.
pub(crate) fn write_host(bytes: &[u8]) -> io::Result<()> {
    let mut output = io::stdout().lock();
    output.write_all(bytes)?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host with room for `room` more bytes, taking at most `chunk` a
    /// write, then refusing with `refusal`.
    struct Slow {
        taken: Vec<u8>,
        room: usize,
        chunk: usize,
        refusal: io::ErrorKind,
    }

    impl Slow {
        fn new(room: usize, refusal: io::ErrorKind) -> Self {
            Self {
                taken: Vec::new(),
                room,
                chunk: 3,
                refusal,
            }
        }
    }

    impl Write for Slow {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.room == 0 {
                return Err(self.refusal.into());
            }
            let count = bytes.len().min(self.chunk).min(self.room);
            self.taken.extend_from_slice(&bytes[..count]);
            self.room -= count;
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// #285 (pure supplement): queued bytes reach the host whole and in
    /// order, as much as it has room for at each write and never waiting
    /// for more: what it refuses for now waits, idle only once everything
    /// was written, and bytes queued meanwhile follow it.
    #[test]
    fn queued_bytes_are_written_in_order_as_the_host_takes_them() {
        for refusal in [io::ErrorKind::WouldBlock, io::ErrorKind::Interrupted] {
            let mut host = Slow::new(4, refusal);
            let mut output = Output::queued(b"frame-1");
            assert!(!output.idle());
            output.write(&mut host).unwrap();
            assert_eq!(host.taken, b"fram");
            assert!(!output.idle());
            output.queue(b"BEL");
            host.room = 5;
            output.write(&mut host).unwrap();
            assert_eq!(host.taken, b"frame-1BE");
            host.room = 100;
            output.write(&mut host).unwrap();
            assert_eq!(host.taken, b"frame-1BEL");
            assert!(output.idle());
            output.queue(b"frame-2");
            output.write(&mut host).unwrap();
            assert_eq!(host.taken, b"frame-1BELframe-2");
            assert!(output.idle());
        }
    }

    /// #285 (pure supplement): any other failure to write is the host's
    /// failure, and nothing more is written.
    #[test]
    fn a_failed_host_write_is_returned() {
        let mut host = Slow::new(2, io::ErrorKind::BrokenPipe);
        let mut output = Output::queued(b"frame");
        let error = output.write(&mut host).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(host.taken, b"fr");
        assert!(!output.idle());
        assert!(Output::default().idle());
    }
}
