use alacritty_terminal::{
    Term,
    event::{Event, EventListener},
    grid::Dimensions,
    index::{Column, Line},
    term::Config,
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

struct Screen {
    term: Term<Events>,
    recent: Vec<u8>,
    eof: bool,
    error: Option<io::Error>,
}

impl Screen {
    fn text(&self) -> String {
        let grid = self.term.grid();
        (0..grid.screen_lines())
            .map(|row| {
                (0..grid.columns())
                    .map(|col| grid[Line(row as i32)][Column(col)].c)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn failure(&self, error: io::Error) -> io::Error {
        io::Error::new(
            error.kind(),
            format!(
                "{error}\nSCREEN:\n{}\nRECENT BYTES:\n{:?}",
                self.text(),
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
                    Config::default(),
                    &Size {
                        rows: rows.into(),
                        cols: cols.into(),
                    },
                    Events(sender),
                ),
                recent: Vec::new(),
                eof: false,
                error: None,
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
                        state.recent.extend_from_slice(&buffer[..count]);
                        let excess = state.recent.len().saturating_sub(65536);
                        state.recent.drain(..excess);
                        while let Ok(event) = events.try_recv() {
                            if let Event::PtyWrite(reply) = event
                                && let Err(error) = write(&reader_writer, reply.as_bytes())
                            {
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

    pub fn screen(&self) -> String {
        self.screen.0.lock().expect("screen lock").text()
    }

    pub fn wait_for_text(&mut self, text: &str, timeout: Duration) -> io::Result<()> {
        self.wait(timeout, |screen| screen.text().contains(text))
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

fn write(writer: &Writer, bytes: &[u8]) -> io::Result<()> {
    let mut writer = writer.lock().expect("PTY writer lock");
    writer.write_all(bytes).and_then(|()| writer.flush())
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
