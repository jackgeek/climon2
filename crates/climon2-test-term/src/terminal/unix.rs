//! The Unix host: a program on a `portable-pty` PTY, reaped with `waitpid`.

use super::{Feed, Options, SharedScreen, Writer, check_size, new_screen, spawn_reader};
use nix::{
    errno::Errno,
    sys::{
        signal::{Signal, killpg},
        wait::{WaitPidFlag, WaitStatus, waitpid},
    },
    unistd::Pid,
};
use portable_pty::{CommandBuilder, MasterPty, PtySize, SlavePty};
use std::{
    env,
    ffi::OsString,
    io,
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub struct Terminal {
    master: Box<dyn MasterPty + Send>,
    /// Held until the child is reaped and its output read: macOS discards unread output
    /// once the last slave descriptor closes.
    slave: Option<Box<dyn SlavePty + Send>>,
    pub(crate) writer: Writer,
    reader: Option<JoinHandle<()>>,
    pub(crate) screen: SharedScreen,
    pid: Pid,
    exit: Option<i32>,
}

/// The `SHELL` every harness spawn carries: the test process's own, or
/// `/bin/sh` when it has none. A command without `SHELL` makes portable-pty
/// look the shell up with `getpwuid(3)`, whose static buffer parallel test
/// threads overwrite, so a spawn could see the wrong shell (#170).
pub fn harness_shell() -> OsString {
    env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into())
}

/// A command without `SHELL` makes portable-pty look the shell up with
/// `getpwuid(3)`, which parallel test threads race (#170), so every spawn
/// without one gets the harness's.
fn with_shell(mut command: CommandBuilder) -> CommandBuilder {
    if command.get_env("SHELL").is_none() {
        command.env("SHELL", harness_shell());
    }
    command
}

impl Terminal {
    pub fn spawn(command: CommandBuilder, rows: u16, cols: u16) -> io::Result<Self> {
        Self::spawn_with(command, rows, cols, Options::default())
    }

    pub fn spawn_with(
        command: CommandBuilder,
        rows: u16,
        cols: u16,
        options: Options,
    ) -> io::Result<Self> {
        let command = crate::ownership::prepare(with_shell(command));
        let pair = {
            let _allocation = climon2_platform::pty::allocation()?;
            portable_pty::native_pty_system().openpty(pty_size(rows, cols)?)
        }
        .map_err(io::Error::other)?;
        let input = pair.master.try_clone_reader().map_err(io::Error::other)?;
        let writer: Writer = Arc::new(Mutex::new(
            pair.master.take_writer().map_err(io::Error::other)?,
        ));
        let (screen, events) = new_screen(rows, cols, options);
        let marker = command
            .get_env(crate::ownership::MARKER)
            .and_then(|marker| marker.to_str())
            .map(str::to_owned);
        climon2_platform::process::before_fork();
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(io::Error::other)?;
        let pid = Pid::from_raw(
            i32::try_from(child.process_id().expect("Unix child pid")).expect("pid fits i32"),
        );
        if let Some(marker) = marker {
            crate::ownership::adopt(&marker, pid.as_raw());
        }
        let reader = spawn_reader(
            Feed {
                input,
                screen: screen.clone(),
                writer: writer.clone(),
                events,
                recent: options.recent,
                hang_up: |error| error.raw_os_error() == Some(nix::libc::EIO),
            },
            |_| (),
        );
        Ok(Self {
            master: pair.master,
            slave: Some(pair.slave),
            writer,
            reader: Some(reader),
            screen,
            pid,
            exit: None,
        })
    }

    /// The process the terminal was spawned with.
    pub fn pid(&self) -> i32 {
        self.pid.as_raw()
    }

    pub fn wait_for_input_ready(&self, timeout: Duration) -> io::Result<()> {
        let deadline = Instant::now() + timeout;
        let (screen, changed) = &*self.screen;
        let mut state = screen.lock().expect("screen lock");
        loop {
            state.check()?;
            let fd = self
                .master
                .as_raw_fd()
                .ok_or_else(|| io::Error::other("terminal descriptor unavailable"))?;
            if terminal_is_raw(fd)? && state.last_output.elapsed() >= Duration::from_millis(50) {
                return Ok(());
            }
            if Instant::now() >= deadline || state.eof {
                return Err(state.failure(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "terminal input did not become ready",
                )));
            }
            (state, _) = changed
                .wait_timeout(
                    state,
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(5)),
                )
                .expect("screen wait");
        }
    }

    /// Waits until exactly `count` bytes sent to this terminal are still unread
    /// by the programs on it, as the terminal device itself reports.
    pub fn wait_for_unread_input(&self, count: usize, timeout: Duration) -> io::Result<()> {
        let device = self.device()?;
        let deadline = Instant::now() + timeout;
        loop {
            let unread = unread(&device)?;
            if unread == count {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(self
                    .screen
                    .0
                    .lock()
                    .expect("screen lock")
                    .failure(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("{unread} bytes of input unread, not {count}"),
                    )));
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Waits until a program's write to this terminal would wait, its
    /// buffer full while [`Terminal::stop_reading`] holds the host's
    /// reading, as the terminal device itself reports. A program that waits
    /// inside its write holds the terminal's write lock until the host
    /// reads, so this suits programs that write without waiting, as the
    /// monitor does (#285).
    pub fn wait_for_output_blocked(&self, timeout: Duration) -> io::Result<()> {
        let device = self.opened(std::fs::OpenOptions::new().write(true))?;
        let deadline = Instant::now() + timeout;
        while !full(&device)? {
            if Instant::now() >= deadline {
                return Err(self
                    .screen
                    .0
                    .lock()
                    .expect("screen lock")
                    .failure(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the terminal's output never filled",
                    )));
            }
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    /// The terminal device the programs read, opened without becoming anyone's
    /// controlling terminal.
    fn device(&self) -> io::Result<std::fs::File> {
        self.opened(std::fs::OpenOptions::new().read(true))
    }

    /// The terminal device the programs use, opened as `options` say,
    /// without becoming anyone's controlling terminal.
    fn opened(&self, options: &mut std::fs::OpenOptions) -> io::Result<std::fs::File> {
        use std::os::unix::fs::OpenOptionsExt;
        let path = self
            .master
            .tty_name()
            .ok_or_else(|| io::Error::other("terminal device unnamed"))?;
        options
            .custom_flags(nix::libc::O_NOCTTY | nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC)
            .open(path)
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> io::Result<()> {
        let size = pty_size(rows, cols)?;
        let mut screen = self.screen.0.lock().expect("screen lock");
        self.master
            .resize(size)
            .map_err(|error| screen.failure(io::Error::other(error)))?;
        screen.resize(rows, cols);
        Ok(())
    }

    fn poll_exit(&mut self) -> io::Result<()> {
        if self.exit.is_none() {
            self.exit = match waitpid(self.pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(_, code)) => Some(code),
                Ok(WaitStatus::Signaled(_, signal, _)) => Some(128 + signal as i32),
                Ok(_) | Err(Errno::EINTR) => None,
                Err(error) => return Err(io::Error::from_raw_os_error(error as i32)),
            };
        }
        Ok(())
    }

    fn output_pending(&self) -> bool {
        let mut master = nix::libc::pollfd {
            fd: self.master.as_raw_fd().expect("Unix PTY master"),
            events: nix::libc::POLLIN,
            revents: 0,
        };
        // poll reads one pollfd and returns at once with a zero timeout.
        let ready = unsafe { nix::libc::poll(&mut master, 1, 0) };
        ready != 0 && (ready < 0 || master.revents & nix::libc::POLLIN != 0)
    }

    pub fn wait_exit(&mut self, timeout: Duration) -> io::Result<i32> {
        let deadline = Instant::now() + timeout;
        loop {
            self.poll_exit()
                .map_err(|error| self.screen.0.lock().expect("screen lock").failure(error))?;
            let (screen, changed) = &*self.screen;
            let state = screen.lock().expect("screen lock");
            state.check()?;
            if self.exit.is_some() && !self.output_pending() {
                self.slave = None;
            }
            if let Some(code) = self.exit.filter(|_| state.eof) {
                return Ok(code);
            }
            if Instant::now() >= deadline {
                return Err(state.failure(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "child did not exit and close its PTY",
                )));
            }
            let _guard = changed
                .wait_timeout(
                    state,
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(5)),
                )
                .expect("screen wait");
        }
    }
}

/// A second terminal that a command opens by path as its input, and that the
/// test hangs up while the command's own terminal stays open.
pub struct InputTerminal {
    master: Box<dyn MasterPty + Send>,
    _slave: Box<dyn SlavePty + Send>,
    path: std::path::PathBuf,
}

impl InputTerminal {
    pub fn open(rows: u16, cols: u16) -> io::Result<Self> {
        let pair = {
            let _allocation = climon2_platform::pty::allocation()?;
            portable_pty::native_pty_system().openpty(pty_size(rows, cols)?)
        }
        .map_err(io::Error::other)?;
        let path = pair
            .master
            .tty_name()
            .ok_or_else(|| io::Error::other("terminal device unnamed"))?;
        Ok(Self {
            master: pair.master,
            _slave: pair.slave,
            path,
        })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Closes the master side: every reader of the terminal sees it hang up.
    pub fn hang_up(self) {
        drop(self.master);
    }
}

/// Input queued on a terminal device that no reader has taken yet.
fn unread(device: &std::fs::File) -> io::Result<usize> {
    use std::os::fd::AsRawFd;
    let mut count: nix::libc::c_int = 0;
    // FIONREAD writes one int, the count of bytes queued for reading.
    if unsafe { nix::libc::ioctl(device.as_raw_fd(), nix::libc::FIONREAD, &mut count) } == -1 {
        return Err(io::Error::last_os_error());
    }
    usize::try_from(count).map_err(io::Error::other)
}

/// Whether the terminal's buffer is full: no write to it is under way,
/// yet a write would take nothing. Linux reports a terminal unwritable
/// while any write to it is under way, holding its write lock, however much
/// room is left, and a loaded host can deschedule that write midway for
/// milliseconds. A write of nothing through the nonblocking `device` is
/// refused while one is, and writes nothing otherwise.
fn full(device: &std::fs::File) -> io::Result<bool> {
    match nix::unistd::write(device, &[]) {
        Ok(_) => Ok(!writable(device)?),
        Err(Errno::EAGAIN) => Ok(false),
        Err(error) => Err(io::Error::from(error)),
    }
}

/// Whether a write to `device` would take bytes now.
fn writable(device: &std::fs::File) -> io::Result<bool> {
    use std::os::fd::AsRawFd;
    let mut entry = nix::libc::pollfd {
        fd: device.as_raw_fd(),
        events: nix::libc::POLLOUT,
        revents: 0,
    };
    // poll reads and updates one pollfd and returns at once with a zero
    // timeout.
    if unsafe { nix::libc::poll(&mut entry, 1, 0) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(entry.revents & nix::libc::POLLOUT != 0)
}

fn terminal_is_raw(fd: i32) -> io::Result<bool> {
    let mut termios = std::mem::MaybeUninit::<nix::libc::termios>::uninit();
    if unsafe { nix::libc::tcgetattr(fd, termios.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let termios = unsafe { termios.assume_init() };
    Ok(termios.c_lflag & (nix::libc::ICANON | nix::libc::ECHO) == 0)
}

fn pty_size(rows: u16, cols: u16) -> io::Result<PtySize> {
    check_size(rows, cols)?;
    Ok(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Err(error) = self.poll_exit() {
            eprintln!("PTY child {} status failed: {error}", self.pid);
        }
        match killpg(self.pid, Signal::SIGKILL) {
            Ok(()) | Err(Errno::ESRCH) => (),
            Err(error) => eprintln!("PTY group {} cleanup failed: {error}", self.pid),
        }
        if self.exit.is_none() {
            match waitpid(self.pid, None) {
                Ok(_) | Err(Errno::ECHILD) => (),
                Err(error) => eprintln!("PTY child {} reap failed: {error}", self.pid),
            }
        }
        self.slave = None;
        if let Some(reader) = self.reader.take() {
            let deadline = Instant::now() + Duration::from_secs(1);
            while !reader.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            if reader.is_finished() {
                reader.join().expect("PTY reader panicked");
            } else {
                eprintln!("PTY reader did not close after process-group cleanup");
            }
        }
    }
}
