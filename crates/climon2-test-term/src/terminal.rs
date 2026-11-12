use crate::{
    cell::{Cell, Cursor},
    input::{Host, Mouse},
    signals::{HostSignal, Signals},
};
use alacritty_terminal::{
    Term,
    event::{Event, EventListener},
    grid::Dimensions,
    index::{Column, Line},
    term::{Config, Osc52, TermMode},
    vte::ansi::Processor,
};
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
    io::{self, Read, Write},
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

struct Size {
    rows: usize,
    cols: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

struct Events(Sender<Event>);
impl EventListener for Events {
    fn send_event(&self, event: Event) {
        self.0.send(event).expect("terminal event receiver");
    }
}

/// How the emulated host answers and how much native scrollback it keeps.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Whether the host answers queries (`Event::PtyWrite`) like a real terminal.
    pub replies: bool,
    /// Native scrollback rows kept by the host.
    pub history: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            replies: true,
            history: 10_000,
        }
    }
}

/// The host terminal's mode state, for release and restoration assertions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modes {
    pub alternate_screen: bool,
    pub cursor_visible: bool,
    /// Any mouse tracking or SGR mouse encoding mode.
    pub mouse: bool,
    /// Button-event (press, release and drag) mouse tracking, DEC 1002.
    pub mouse_drag: bool,
    /// SGR extended mouse encoding, DEC 1006.
    pub sgr_mouse: bool,
    pub focus: bool,
    pub bracketed_paste: bool,
    pub application_cursor: bool,
    pub application_keypad: bool,
    pub autowrap: bool,
    pub origin: bool,
    pub insert: bool,
    pub line_feed_new_line: bool,
}

impl Modes {
    /// Plan 1.7.1's documented normal-shell baseline.
    pub const BASELINE: Self = Self {
        alternate_screen: false,
        cursor_visible: true,
        mouse: false,
        mouse_drag: false,
        sgr_mouse: false,
        focus: false,
        bracketed_paste: false,
        application_cursor: false,
        application_keypad: false,
        autowrap: true,
        origin: false,
        insert: false,
        line_feed_new_line: false,
    };

    pub(crate) fn of(mode: TermMode) -> Self {
        Self {
            alternate_screen: mode.contains(TermMode::ALT_SCREEN),
            cursor_visible: mode.contains(TermMode::SHOW_CURSOR),
            mouse: mode.intersects(TermMode::MOUSE_MODE | TermMode::SGR_MOUSE),
            mouse_drag: mode.contains(TermMode::MOUSE_DRAG),
            sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
            focus: mode.contains(TermMode::FOCUS_IN_OUT),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            application_cursor: mode.contains(TermMode::APP_CURSOR),
            application_keypad: mode.contains(TermMode::APP_KEYPAD),
            autowrap: mode.contains(TermMode::LINE_WRAP),
            origin: mode.contains(TermMode::ORIGIN),
            insert: mode.contains(TermMode::INSERT),
            line_feed_new_line: mode.contains(TermMode::LINE_FEED_NEW_LINE),
        }
    }
}

struct Screen {
    term: Term<Events>,
    replies: bool,
    title: Option<String>,
    /// Queries and clipboard operations the host was asked to perform.
    requests: Vec<String>,
    signals: Signals,
    recent: Vec<u8>,
    eof: bool,
    error: Option<io::Error>,
    last_output: Instant,
}

impl Screen {
    fn text(&self) -> String {
        let grid = self.term.grid();
        (0..grid.screen_lines())
            .map(|row| self.row(row as i32))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn row(&self, line: i32) -> String {
        let grid = self.term.grid();
        (0..grid.columns())
            .map(|col| grid[Line(line)][Column(col)].c)
            .collect()
    }

    fn cells(&self, line: i32) -> Vec<Cell> {
        let grid = self.term.grid();
        (0..grid.columns())
            .map(|col| Cell::of(&grid[Line(line)][Column(col)]))
            .collect()
    }

    /// Native scrollback rows, oldest first, without trailing blanks.
    fn history(&self) -> Vec<String> {
        let rows = self.term.grid().history_size() as i32;
        (1..=rows)
            .rev()
            .map(|row| self.row(-row).trim_end().to_owned())
            .collect()
    }

    fn handle(&mut self, event: Event, writer: &Writer) -> io::Result<()> {
        match event {
            Event::Title(title) => self.title = Some(title),
            Event::ResetTitle => self.title = None,
            Event::PtyWrite(reply) => {
                self.requests.push(format!("reply {reply:?}"));
                if self.replies {
                    write(writer, reply.as_bytes())?;
                }
            }
            Event::ClipboardStore(..)
            | Event::ClipboardLoad(..)
            | Event::ColorRequest(..)
            | Event::TextAreaSizeRequest(..) => self.requests.push(format!("{event:?}")),
            _ => (),
        }
        Ok(())
    }

    fn failure(&self, error: io::Error) -> io::Error {
        io::Error::new(
            error.kind(),
            format!(
                "{error}\nSCREEN:\n{}\nHOST SIGNALS:\n{:?}\nRECENT BYTES:\n{:?}",
                self.text(),
                self.signals.recorded(),
                String::from_utf8_lossy(&self.recent)
            ),
        )
    }

    fn check(&self) -> io::Result<()> {
        match &self.error {
            Some(error) => Err(self.failure(io::Error::new(error.kind(), error.to_string()))),
            None => Ok(()),
        }
    }
}

type Writer = Arc<Mutex<Box<dyn Write + Send>>>;
type SharedScreen = Arc<(Mutex<Screen>, Condvar)>;

pub struct Terminal {
    master: Box<dyn MasterPty + Send>,
    /// Held until the child is reaped and its output read: macOS discards unread output
    /// once the last slave descriptor closes.
    slave: Option<Box<dyn SlavePty + Send>>,
    writer: Writer,
    reader: Option<JoinHandle<()>>,
    screen: SharedScreen,
    pid: Pid,
    exit: Option<i32>,
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
        #[cfg(target_os = "linux")]
        let command = crate::ownership::prepare(command);
        let pair = {
            let _allocation = climon2_platform::pty::allocation()?;
            portable_pty::native_pty_system().openpty(pty_size(rows, cols)?)
        }
        .map_err(io::Error::other)?;
        let mut input = pair.master.try_clone_reader().map_err(io::Error::other)?;
        let writer = Arc::new(Mutex::new(
            pair.master.take_writer().map_err(io::Error::other)?,
        ));
        let (sender, events) = mpsc::channel();
        let screen = Arc::new((
            Mutex::new(Screen {
                term: Term::new(
                    Config {
                        scrolling_history: options.history,
                        osc52: Osc52::CopyPaste,
                        ..Config::default()
                    },
                    &Size {
                        rows: rows.into(),
                        cols: cols.into(),
                    },
                    Events(sender),
                ),
                replies: options.replies,
                title: None,
                requests: Vec::new(),
                signals: Signals::default(),
                recent: Vec::new(),
                eof: false,
                error: None,
                last_output: Instant::now(),
            }),
            Condvar::new(),
        ));
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
        let reader_screen = screen.clone();
        let reader_writer = writer.clone();
        let reader = thread::spawn(move || {
            let (screen, changed) = &*reader_screen;
            let mut parser: Processor = Processor::new();
            let mut buffer = [0; 4096];
            loop {
                let result = input.read(&mut buffer);
                let mut state = screen.lock().expect("screen lock");
                match result {
                    Ok(0) => state.eof = true,
                    Ok(count) => {
                        parser.advance(&mut state.term, &buffer[..count]);
                        state.signals.push(&buffer[..count]);
                        state.recent.extend_from_slice(&buffer[..count]);
                        state.last_output = Instant::now();
                        let excess = state.recent.len().saturating_sub(65536);
                        state.recent.drain(..excess);
                        while let Ok(event) = events.try_recv() {
                            if let Err(error) = state.handle(event, &reader_writer) {
                                state.error = Some(error);
                                break;
                            }
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) if error.raw_os_error() == Some(nix::libc::EIO) => state.eof = true,
                    Err(error) => state.error = Some(error),
                }
                changed.notify_all();
                if state.eof || state.error.is_some() {
                    break;
                }
            }
        });
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

    pub fn send(&mut self, bytes: impl AsRef<[u8]>) -> io::Result<()> {
        write(&self.writer, bytes.as_ref())
            .map_err(|error| self.screen.0.lock().expect("screen lock").failure(error))
    }

    /// Reports host focus gained (`CSI I`) or lost (`CSI O`), as a real
    /// terminal does only while the application has focus reporting
    /// (`CSI ? 1004 h`) on.
    pub fn focus(&mut self, focused: bool) -> io::Result<()> {
        self.inject(|host| host.focus(focused))
    }

    /// Reports a mouse `action` at the visible-screen cell (`row`, `column`),
    /// 0-based, in SGR encoding (`CSI < Cb ; Cx ; Cy M` or `m`, 1-based), as a
    /// real terminal does only with SGR encoding (`CSI ? 1006 h`) and a
    /// tracking mode that reports the action on.
    pub fn mouse(&mut self, action: Mouse, row: usize, column: usize) -> io::Result<()> {
        self.inject(|host| host.mouse(action, row, column))
    }

    /// Pastes `text` unchanged between `CSI 200 ~` and `CSI 201 ~`, as a real
    /// terminal does while the application has bracketed paste
    /// (`CSI ? 2004 h`) on; send unbracketed paste with [`Terminal::send`].
    pub fn paste(&mut self, text: impl AsRef<[u8]>) -> io::Result<()> {
        self.inject(|host| host.paste(text.as_ref()))
    }

    /// Sends the input the emulated host makes of an event, or nothing, failing
    /// with [`io::ErrorKind::InvalidInput`], when the host would not send it.
    fn inject(&mut self, encode: impl FnOnce(&Host) -> Result<Vec<u8>, String>) -> io::Result<()> {
        let bytes = {
            let screen = self.screen.0.lock().expect("screen lock");
            let host = Host {
                mode: *screen.term.mode(),
                rows: screen.term.screen_lines(),
                columns: screen.term.columns(),
            };
            encode(&host).map_err(|reason| {
                screen.failure(io::Error::new(io::ErrorKind::InvalidInput, reason))
            })?
        };
        self.send(bytes)
    }

    pub fn screen(&self) -> String {
        self.screen.0.lock().expect("screen lock").text()
    }

    /// The process the terminal was spawned with.
    pub fn pid(&self) -> i32 {
        self.pid.as_raw()
    }

    /// The visible screen row `row` (0-based), with every column.
    pub fn row(&self, row: usize) -> String {
        self.screen.0.lock().expect("screen lock").row(row as i32)
    }

    /// The visible screen row `row` (0-based), cell by cell with every column.
    pub fn cells(&self, row: usize) -> Vec<Cell> {
        self.screen.0.lock().expect("screen lock").cells(row as i32)
    }

    pub fn history(&self) -> Vec<String> {
        self.screen.0.lock().expect("screen lock").history()
    }

    /// The host's window title; `None` before any title or after a reset.
    pub fn title(&self) -> Option<String> {
        self.screen.0.lock().expect("screen lock").title.clone()
    }

    pub fn modes(&self) -> Modes {
        Modes::of(*self.screen.0.lock().expect("screen lock").term.mode())
    }

    /// The cursor's position, visibility, shape and blink, read together.
    pub fn cursor(&self) -> Cursor {
        Cursor::of(&self.screen.0.lock().expect("screen lock").term)
    }

    /// Every query and clipboard operation the host was asked to perform.
    pub fn host_requests(&self) -> Vec<String> {
        self.screen.0.lock().expect("screen lock").requests.clone()
    }

    /// Every OSC 7501 report and bell the host received, in the order its parser
    /// acted on them.
    pub fn host_signals(&self) -> Vec<HostSignal> {
        self.screen
            .0
            .lock()
            .expect("screen lock")
            .signals
            .recorded()
            .to_vec()
    }

    /// The most recent output bytes (at most 64 KiB).
    pub fn received(&self) -> Vec<u8> {
        self.screen.0.lock().expect("screen lock").recent.clone()
    }

    pub fn wait_for_text(&mut self, text: &str, timeout: Duration) -> io::Result<()> {
        self.wait(timeout, |screen| screen.text().contains(text))
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

    /// Waits until `condition` holds for the visible screen text.
    pub fn wait_for_screen(
        &mut self,
        timeout: Duration,
        condition: impl Fn(&str) -> bool,
    ) -> io::Result<()> {
        self.wait(timeout, |screen| condition(&screen.text()))
    }

    pub fn wait_for_bytes(&mut self, bytes: &[u8], timeout: Duration) -> io::Result<()> {
        self.wait(timeout, |screen| {
            bytes.is_empty()
                || screen
                    .recent
                    .windows(bytes.len())
                    .any(|window| window == bytes)
        })
    }

    /// Waits until `condition` holds for every host signal received so far.
    pub fn wait_for_host_signals(
        &mut self,
        timeout: Duration,
        condition: impl Fn(&[HostSignal]) -> bool,
    ) -> io::Result<()> {
        self.wait(timeout, |screen| condition(screen.signals.recorded()))
    }

    fn wait(&self, timeout: Duration, condition: impl Fn(&Screen) -> bool) -> io::Result<()> {
        let deadline = Instant::now() + timeout;
        let (screen, changed) = &*self.screen;
        let mut state = screen.lock().expect("screen lock");
        loop {
            state.check()?;
            if condition(&state) {
                return Ok(());
            }
            if Instant::now() >= deadline || state.eof {
                return Err(state.failure(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "terminal expectation not met",
                )));
            }
            (state, _) = changed
                .wait_timeout(state, deadline.saturating_duration_since(Instant::now()))
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

    /// The terminal device the programs read, opened without becoming anyone's
    /// controlling terminal.
    fn device(&self) -> io::Result<std::fs::File> {
        use std::os::unix::fs::OpenOptionsExt;
        let path = self
            .master
            .tty_name()
            .ok_or_else(|| io::Error::other("terminal device unnamed"))?;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOCTTY | nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC)
            .open(path)
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> io::Result<()> {
        let size = pty_size(rows, cols)?;
        let mut screen = self.screen.0.lock().expect("screen lock");
        self.master
            .resize(size)
            .map_err(|error| screen.failure(io::Error::other(error)))?;
        screen.term.resize(Size {
            rows: rows.into(),
            cols: cols.into(),
        });
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

fn write(writer: &Writer, bytes: &[u8]) -> io::Result<()> {
    let mut writer = writer.lock().expect("PTY writer lock");
    writer.write_all(bytes).and_then(|()| writer.flush())
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

fn terminal_is_raw(fd: i32) -> io::Result<bool> {
    let mut termios = std::mem::MaybeUninit::<nix::libc::termios>::uninit();
    if unsafe { nix::libc::tcgetattr(fd, termios.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let termios = unsafe { termios.assume_init() };
    Ok(termios.c_lflag & (nix::libc::ICANON | nix::libc::ECHO) == 0)
}

fn pty_size(rows: u16, cols: u16) -> io::Result<PtySize> {
    if rows < 2 || cols < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PTY needs at least 2 rows and columns",
        ));
    }
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
