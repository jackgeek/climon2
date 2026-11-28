//! The terminal while the relay owns it, through the platform terminal seam:
//! termios and signals there, the relay's own screen modes here.
use climon2_core::{exit::TERMINAL_GONE, protocol::TerminalSize};
use climon2_platform::{
    terminal::{self, RawModeGuard, Signals},
    waiting::{self, Interest, Watch},
};
use std::{
    fs::File,
    io::{self, Read, Write},
    os::fd::AsFd,
    time::Duration,
};

/// The relay's own alternate screen.
const ENTER: &[u8] = b"\x1b[?1049h";
/// Default colours, the cursor shown again, and the shell's screen back.
const LEAVE: &[u8] = b"\x1b[0m\x1b[?25h\x1b[?1049l";
/// How long one wait for input lasts before signals are checked again.
const TICK: Duration = Duration::from_millis(20);
const READ_BYTES: usize = 256;

/// Why the relay stops. Every ending is clean.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Ending {
    /// Stop, from the relay's own menu.
    Stopped,
    /// A catchable termination signal.
    Signal(u32),
    /// The terminal's input closed.
    TerminalClosed,
}

impl Ending {
    pub(crate) fn status(self) -> u32 {
        match self {
            Self::Stopped => 0,
            Self::Signal(signal) => 128 + signal,
            Self::TerminalClosed => TERMINAL_GONE,
        }
    }
}

/// What happened at the terminal since the last event.
pub(crate) enum Event<'a> {
    End(Ending),
    Resize(TerminalSize),
    /// Raw input, exactly as read.
    Input(&'a [u8]),
    Idle,
}

/// Field order is drop order: termios is restored before the signal handlers
/// go, so a late signal cannot strand the terminal in raw mode.
pub(crate) struct Host {
    _raw: RawModeGuard,
    signals: Signals,
    input: File,
    buffer: [u8; READ_BYTES],
}

impl Host {
    /// Signals are caught before raw mode, and raw mode precedes any mode
    /// change.
    pub(crate) fn enter() -> io::Result<(Self, TerminalSize)> {
        let input = terminal::input()?;
        let size = terminal::size(&input)?;
        Ok((Self::raw(input)?, size))
    }

    /// Owns `input` in raw mode, on the relay's own screen.
    fn raw(input: File) -> io::Result<Self> {
        let signals = Signals::client()?;
        let raw = RawModeGuard::new(&input)?;
        let host = Self {
            _raw: raw,
            signals,
            input,
            buffer: [0; READ_BYTES],
        };
        write(ENTER)?;
        Ok(host)
    }

    /// The next thing to act on: a termination signal, then a resize, then
    /// input, waiting one tick for it.
    pub(crate) fn next(&mut self) -> io::Result<Event<'_>> {
        match self.signalled()? {
            Some(event) => Ok(event),
            None => self.input(),
        }
    }

    /// A termination signal, then a resize, if either came.
    fn signalled(&mut self) -> io::Result<Option<Event<'static>>> {
        if let Some(signal) = self.signals.termination_signal() {
            return Ok(Some(Event::End(Ending::Signal(signal))));
        }
        if !self.signals.take_resize() {
            return Ok(None);
        }
        terminal::size(&self.input).map(|size| Some(Event::Resize(size)))
    }

    /// Input, waiting one tick for it.
    fn input(&mut self) -> io::Result<Event<'_>> {
        if !self.readable()? {
            return Ok(Event::Idle);
        }
        Ok(read(self.input.read(&mut self.buffer), &self.buffer))
    }

    /// Waits one tick for input and says whether any is ready. A wait that a
    /// signal interrupts finds none, and the caller checks signals before
    /// waiting again.
    fn readable(&self) -> io::Result<bool> {
        let mut watches = [Watch::new(self.input.as_fd(), Interest::Read)];
        let wait = waiting::wait(&mut watches, TICK);
        waited(wait.map(|_| watches[0].ready()))
    }
}

/// What one tick's wait for input means: whether input is ready, with an
/// interrupted wait finding none.
fn waited(ready: io::Result<bool>) -> io::Result<bool> {
    match ready {
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(false),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!("cannot wait for terminal input: {error}"),
        )),
        ready => ready,
    }
}

/// The terminal is the shell's again whenever the relay lets go of it, on a
/// clean stop and on a failure alike. A terminal that has closed takes no
/// bytes, which is not an error worth reporting.
impl Drop for Host {
    fn drop(&mut self) {
        let _ = write(LEAVE);
    }
}

/// What one read of the terminal's input means. Reading nothing, or failing
/// to read at all, is the terminal closing: a read is tried only once input
/// was reported ready.
fn read(count: io::Result<usize>, buffer: &[u8]) -> Event<'_> {
    match count {
        Ok(count) if count > 0 => Event::Input(&buffer[..count]),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Event::Idle,
        _ => Event::End(Ending::TerminalClosed),
    }
}

fn write(bytes: &[u8]) -> io::Result<()> {
    let mut output = terminal::output()?;
    output.write_all(bytes)?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec #488, user story 12: 0, 143 and 129 mean menu stop, SIGTERM and
    /// a closed terminal.
    #[test]
    fn each_ending_has_its_exit_status() {
        assert_eq!(Ending::Stopped.status(), 0);
        assert_eq!(Ending::Signal(15).status(), 143);
        assert_eq!(Ending::Signal(1).status(), 129);
        assert_eq!(Ending::Signal(2).status(), 130);
        assert_eq!(Ending::TerminalClosed.status(), 129);
    }

    fn ended(event: Event<'_>) -> Option<Ending> {
        match event {
            Event::End(ending) => Some(ending),
            _ => None,
        }
    }

    #[test]
    fn bytes_read_are_input_exactly_as_read() {
        let buffer = *b"stop";
        match read(Ok(2), &buffer) {
            Event::Input(input) => assert_eq!(input, b"st"),
            _ => panic!("two bytes read are input"),
        }
        match read(Ok(1), &buffer) {
            Event::Input(input) => assert_eq!(input, b"s"),
            _ => panic!("one byte read is input"),
        }
    }

    #[test]
    fn reading_nothing_or_failing_to_read_is_the_terminal_closing() {
        let buffer = [0; 4];
        assert_eq!(ended(read(Ok(0), &buffer)), Some(Ending::TerminalClosed));
        let lost = io::Error::from_raw_os_error(5);
        assert_eq!(
            ended(read(Err(lost), &buffer)),
            Some(Ending::TerminalClosed)
        );
    }

    #[test]
    fn a_wait_says_whether_input_is_ready() {
        assert!(waited(Ok(true)).unwrap());
        assert!(!waited(Ok(false)).unwrap());
    }

    #[test]
    fn an_interrupted_wait_finds_no_input() {
        let interrupted = io::Error::from(io::ErrorKind::Interrupted);
        assert!(!waited(Err(interrupted)).unwrap());
    }

    #[test]
    fn a_failed_wait_is_reported_with_its_kind() {
        let failed = waited(Err(io::Error::from_raw_os_error(9))).unwrap_err();
        let expected = io::Error::from_raw_os_error(9).kind();
        assert_eq!(failed.kind(), expected);
        assert!(
            failed
                .to_string()
                .starts_with("cannot wait for terminal input: ")
        );
    }

    #[test]
    fn an_interrupted_read_is_idle() {
        let interrupted = io::Error::from(io::ErrorKind::Interrupted);
        assert!(matches!(read(Err(interrupted), &[0; 4]), Event::Idle));
    }
}
