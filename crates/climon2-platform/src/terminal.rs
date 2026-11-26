//! Terminal ownership, file-status flags, hang-up and deferred signal
//! handling (#20), and the terminal a client bridges. On Windows the terminal
//! is the console, with its own raw mode, threads and control events (#321).
pub use native::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use unix as native;

#[cfg(windows)]
#[path = "terminal/windows.rs"]
mod native;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use crate::waiting::{Interest, Watch};
    /// On Unix a lone Escape, or the start of a paste marker, waits for the next
    /// byte, as it always has (#328): a program on a Unix terminal reads an
    /// Escape followed by a byte that starts no sequence it knows as two keys.
    pub const LONE_ESCAPE: Option<std::time::Duration> = None;
    use climon2_core::{protocol::TerminalSize, session::Input};
    use signal_hook::{SigId, consts::signal::*};
    use std::{
        fs::File,
        io::{self, Read, Write},
        mem::MaybeUninit,
        os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
        sync::{
            Arc, Mutex, Once,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    pub fn require_ttys() -> io::Result<()> {
        if unsafe { libc::isatty(0) == 1 && libc::isatty(1) == 1 } {
            return Ok(());
        }
        Err(io::Error::other("c2 exec needs a terminal"))
    }

    /// An owned duplicate; closing it never closes the process's stdin.
    pub fn input() -> io::Result<File> {
        duplicate(0)
    }

    fn duplicate(fd: i32) -> io::Result<File> {
        let fd = crate::syscall::checked(unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) })?;
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    pub fn size(terminal: &File) -> io::Result<TerminalSize> {
        let mut size = MaybeUninit::<libc::winsize>::uninit();
        crate::syscall::checked(unsafe {
            libc::ioctl(terminal.as_raw_fd(), libc::TIOCGWINSZ, size.as_mut_ptr())
        })?;
        let size = unsafe { size.assume_init() };
        Ok(TerminalSize {
            cols: size.ws_col,
            rows: size.ws_row,
        }
        .initial())
    }

    struct SavedTerminal {
        terminal: File,
        saved: libc::termios,
    }

    impl SavedTerminal {
        /// Best effort: a drop or the panic hook restores, with no one to report a failure to.
        fn restore(&self) {
            unsafe { libc::tcsetattr(self.terminal.as_raw_fd(), libc::TCSANOW, &self.saved) };
        }
    }

    static ACTIVE: Mutex<Option<Arc<SavedTerminal>>> = Mutex::new(None);
    static PANIC_HOOK: Once = Once::new();

    fn install_panic_hook() {
        PANIC_HOOK.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |panic| {
                if let Ok(active) = ACTIVE.lock()
                    && let Some(saved) = active.as_ref()
                {
                    saved.restore();
                }
                previous(panic);
            }));
        });
    }

    pub struct RawModeGuard(Arc<SavedTerminal>);

    impl RawModeGuard {
        /// Enter only after Attached. Restore termios only, never screen state.
        pub fn new(terminal: &File) -> io::Result<Self> {
            install_panic_hook();
            let mut active = ACTIVE
                .lock()
                .map_err(|_| io::Error::other("terminal lock poisoned"))?;
            if active.is_some() {
                return Err(io::Error::other("raw terminal already owned"));
            }
            let saved = Arc::new(enter_raw(terminal)?);
            *active = Some(saved.clone());
            Ok(Self(saved))
        }
    }

    fn enter_raw(terminal: &File) -> io::Result<SavedTerminal> {
        let saved = save(terminal)?;
        let mut raw = saved.saved;
        unsafe { libc::cfmakeraw(&mut raw) };
        crate::syscall::checked(unsafe {
            libc::tcsetattr(saved.terminal.as_raw_fd(), libc::TCSANOW, &raw)
        })?;
        Ok(saved)
    }

    fn save(terminal: &File) -> io::Result<SavedTerminal> {
        let terminal = duplicate(terminal.as_raw_fd())?;
        let mut saved = MaybeUninit::uninit();
        crate::syscall::checked(unsafe {
            libc::tcgetattr(terminal.as_raw_fd(), saved.as_mut_ptr())
        })?;
        Ok(SavedTerminal {
            terminal,
            saved: unsafe { saved.assume_init() },
        })
    }

    impl Drop for RawModeGuard {
        fn drop(&mut self) {
            self.0.restore();
            if let Ok(mut active) = ACTIVE.lock() {
                *active = None;
            }
        }
    }

    pub struct Signals {
        resize: Arc<AtomicBool>,
        termination: Arc<AtomicUsize>,
        registrations: Vec<SigId>,
    }

    impl Signals {
        pub fn client() -> io::Result<Self> {
            let mut signals = Self::empty();
            signals.register_resize()?;
            for signal in [SIGHUP, SIGTERM, SIGINT] {
                signals.register_termination(signal)?;
            }
            Ok(signals)
        }

        /// A prompt (#139) hears every signal that would end it from the
        /// terminal or `kill`, so it can restore the terminal first.
        pub fn prompt() -> io::Result<Self> {
            let mut signals = Self::empty();
            for signal in [SIGHUP, SIGINT, SIGQUIT, SIGTERM] {
                signals.register_termination(signal)?;
            }
            Ok(signals)
        }

        /// The daemon ignores HUP/INT/PIPE and handles TERM like client loss.
        pub fn daemon() -> io::Result<Self> {
            let mut signals = Self::empty();
            for signal in [SIGHUP, SIGINT, SIGPIPE] {
                signals
                    .registrations
                    .push(unsafe { signal_hook::low_level::register(signal, || {})? });
            }
            signals.register_termination(SIGTERM)?;
            Ok(signals)
        }

        fn empty() -> Self {
            Self {
                resize: Arc::default(),
                termination: Arc::default(),
                registrations: Vec::new(),
            }
        }

        fn register_resize(&mut self) -> io::Result<()> {
            self.registrations
                .push(signal_hook::flag::register(SIGWINCH, self.resize.clone())?);
            Ok(())
        }

        fn register_termination(&mut self, signal: i32) -> io::Result<()> {
            let value = usize::try_from(signal).map_err(io::Error::other)?;
            self.registrations.push(signal_hook::flag::register_usize(
                signal,
                self.termination.clone(),
                value,
            )?);
            Ok(())
        }

        pub fn take_resize(&self) -> bool {
            self.resize.swap(false, Ordering::SeqCst)
        }

        /// Unix terminals have no Ctrl+Break (#326).
        pub fn take_break(&self) -> bool {
            false
        }

        pub fn termination_signal(&self) -> Option<u32> {
            let signal = self.termination.load(Ordering::SeqCst);
            std::num::NonZeroU32::new(u32::try_from(signal).ok()?).map(std::num::NonZeroU32::get)
        }
    }

    impl Drop for Signals {
        fn drop(&mut self) {
            for registration in self.registrations.drain(..) {
                signal_hook::low_level::unregister(registration);
            }
        }
    }

    /// The single owner of the terminal's file-status flags while bridging. Both originals
    /// are captured before either changes, because stdin and stdout may share one open-file
    /// description, and are restored through owned duplicates, so closing the bridge's
    /// descriptors can never skip or misdirect restoration.
    pub struct Nonblocking {
        saved: [(OwnedFd, i32); 2],
    }

    impl Nonblocking {
        pub fn new(input: BorrowedFd<'_>, output: BorrowedFd<'_>) -> io::Result<Self> {
            let guard = Self {
                saved: [save_flags(input)?, save_flags(output)?],
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

    fn save_flags(fd: BorrowedFd<'_>) -> io::Result<(OwnedFd, i32)> {
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

    /// A terminal read failing because the terminal was lost (EIO), which,
    /// like EOF, is a hang-up.
    pub fn hung_up(error: &io::Error) -> bool {
        error.raw_os_error() == Some(libc::EIO)
    }

    /// A terminal read, a lost terminal reading as a hang-up: like EOF, 0
    /// bytes.
    fn read_terminal(terminal: &mut File, bytes: &mut [u8]) -> io::Result<usize> {
        terminal.read(bytes).or_else(|error| {
            if hung_up(&error) {
                return Ok(0);
            }
            Err(error)
        })
    }

    /// The terminal while a client bridges it or the monitor owns it (#285):
    /// input read and output written without blocking, each with the wait
    /// slot that is ready when it can make progress. Reading 0 bytes means
    /// the terminal hung up.
    pub struct Bridged {
        input: File,
        /// An owned duplicate of stdout: the same open-file description, so
        /// the same flags.
        output: File,
        _flags: Nonblocking,
    }

    impl Bridged {
        pub fn new(input: File) -> io::Result<Self> {
            let output = File::from(io::stdout().as_fd().try_clone_to_owned()?);
            let flags = Nonblocking::new(input.as_fd(), output.as_fd())?;
            Ok(Self {
                input,
                output,
                _flags: flags,
            })
        }

        pub fn watch_input(&self) -> Watch<'_> {
            Watch::new(self.input.as_fd(), Interest::Read)
        }

        pub fn watch_output(&self) -> Watch<'_> {
            Watch::new(self.output.as_fd(), Interest::Write)
        }

        pub fn size(&self) -> io::Result<TerminalSize> {
            size(&self.input)
        }
    }

    impl Read for Bridged {
        /// A lost terminal reads as a hang-up error; like EOF it is a silent HUP.
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            read_terminal(&mut self.input, bytes)
        }
    }

    impl Write for Bridged {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.output.write(bytes)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.output.flush()
        }
    }

    /// An owned duplicate of stdout; closing it never closes the process's
    /// stdout.
    pub fn output() -> io::Result<File> {
        duplicate(1)
    }

    /// How long a prompt waits for input before it looks at its signals
    /// again.
    const PROMPT_POLL: std::time::Duration = std::time::Duration::from_millis(50);

    /// The terminal while `exec` waits for an answer to its nested-start
    /// prompt (#139): raw, with input typed before it discarded, read one
    /// byte at a time so that nothing after the answer is taken. A hang-up,
    /// interrupt, quit or termination signal ends the wait instead of the
    /// process, so dropping the prompt always restores the terminal.
    pub struct Prompt {
        _raw: RawModeGuard,
        input: File,
        output: File,
        signals: Signals,
    }

    impl Prompt {
        pub fn new(input: File, output: File) -> io::Result<Self> {
            let signals = Signals::prompt()?;
            let raw = RawModeGuard::new(&input)?;
            crate::syscall::checked(unsafe { libc::tcflush(input.as_raw_fd(), libc::TCIFLUSH) })?;
            Ok(Self {
                _raw: raw,
                input,
                output,
                signals,
            })
        }

        /// Writes `text` to the terminal; raw output needs CR LF.
        pub fn show(&mut self, text: &str) -> io::Result<()> {
            self.output.write_all(text.as_bytes())
        }

        /// The next byte typed, a hang-up, or the status `128 + signal`.
        pub fn read_input(&mut self) -> io::Result<Input> {
            loop {
                if let Some(signal) = self.signals.termination_signal() {
                    return Ok(Input::Ended(128 + signal));
                }
                let mut watch = [Watch::new(self.input.as_fd(), Interest::Read)];
                if crate::waiting::wait(&mut watch, PROMPT_POLL)? > 0 {
                    return self.read_key();
                }
            }
        }

        fn read_key(&mut self) -> io::Result<Input> {
            let mut key = [0u8];
            match read_terminal(&mut self.input, &mut key)? {
                0 => Ok(Input::Gone),
                _ => Ok(Input::Key(key[0])),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use std::io;

        /// EXT-03 (Monitor layer): a terminal read failing with EIO, as Linux
        /// fails it while hanging a terminal up, is a hang-up; any other
        /// failure is not.
        #[test]
        fn only_eio_is_a_hang_up() {
            assert!(super::hung_up(&io::Error::from_raw_os_error(libc::EIO)));
            assert!(!super::hung_up(&io::Error::from_raw_os_error(libc::EBADF)));
            assert!(!super::hung_up(&io::Error::other("no OS error")));
        }
    }
}
