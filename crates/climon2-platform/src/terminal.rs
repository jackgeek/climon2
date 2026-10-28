//! Terminal ownership and deferred signal handling (#20).
#[cfg(not(target_os = "macos"))]
compile_error!("terminal seam requires macOS");

#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "macos")]
mod macos {
    use climon2_core::protocol::TerminalSize;
    use signal_hook::{SigId, consts::signal::*};
    use std::{
        fs::File,
        io,
        mem::MaybeUninit,
        os::fd::{AsRawFd, FromRawFd},
        sync::{
            Arc, Mutex, Once,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    pub fn require_ttys() -> io::Result<()> {
        if unsafe { libc::isatty(0) == 1 && libc::isatty(1) == 1 } {
            return Ok(());
        }
        Err(io::Error::other("climon2 exec needs a terminal"))
    }

    /// An owned duplicate; closing it never closes the process's stdin.
    pub fn input() -> io::Result<File> {
        duplicate(0)
    }

    fn duplicate(fd: i32) -> io::Result<File> {
        let fd = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    pub fn size(terminal: &File) -> io::Result<TerminalSize> {
        let mut size = MaybeUninit::<libc::winsize>::uninit();
        if unsafe { libc::ioctl(terminal.as_raw_fd(), libc::TIOCGWINSZ, size.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
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
        fn restore(&self) -> io::Result<()> {
            if unsafe { libc::tcsetattr(self.terminal.as_raw_fd(), libc::TCSANOW, &self.saved) }
                == -1
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
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
                    let _ = saved.restore();
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
        if unsafe { libc::tcsetattr(saved.terminal.as_raw_fd(), libc::TCSANOW, &raw) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(saved)
    }

    fn save(terminal: &File) -> io::Result<SavedTerminal> {
        let terminal = duplicate(terminal.as_raw_fd())?;
        let mut saved = MaybeUninit::uninit();
        if unsafe { libc::tcgetattr(terminal.as_raw_fd(), saved.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(SavedTerminal {
            terminal,
            saved: unsafe { saved.assume_init() },
        })
    }

    impl Drop for RawModeGuard {
        fn drop(&mut self) {
            let _ = self.0.restore();
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
                resize: Arc::new(AtomicBool::new(false)),
                termination: Arc::new(AtomicUsize::new(0)),
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
}
