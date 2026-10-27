use alacritty_terminal::{
    Term,
    event::{Event, EventListener},
    grid::Dimensions,
    index::Line,
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
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use std::{
    io::{self, Read, Write},
    sync::mpsc::{self, Receiver, Sender},
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

pub struct Terminal {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    reader: Option<JoinHandle<()>>,
    output: Receiver<io::Result<Vec<u8>>>,
    events: Receiver<Event>,
    term: Term<Events>,
    parser: Processor,
    recent: Vec<u8>,
    eof: bool,
    pid: Pid,
    exit: Option<i32>,
}

impl Terminal {
    pub fn spawn(command: CommandBuilder, rows: u16, cols: u16) -> io::Result<Self> {
        let size = pty_size(rows, cols)?;
        let pair = portable_pty::native_pty_system()
            .openpty(size)
            .map_err(io::Error::other)?;
        let mut input = pair.master.try_clone_reader().map_err(io::Error::other)?;
        let writer = pair.master.take_writer().map_err(io::Error::other)?;
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(io::Error::other)?;
        let pid = Pid::from_raw(
            i32::try_from(child.process_id().expect("Unix child pid")).expect("pid fits i32"),
        );
        drop(pair.slave);
        let (sender, output) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut buffer = [0; 4096];
            loop {
                let result = input.read(&mut buffer);
                let done = !matches!(result, Ok(count) if count > 0)
                    && !matches!(&result, Err(error) if error.kind() == io::ErrorKind::Interrupted);
                let message = match result {
                    Ok(count) => Ok(buffer[..count].to_vec()),
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) if error.raw_os_error() == Some(nix::libc::EIO) => Ok(Vec::new()),
                    Err(error) => Err(error),
                };
                if sender.send(message).is_err() || done {
                    break;
                }
            }
        });
        let (sender, events) = mpsc::channel();
        Ok(Self {
            master: pair.master,
            writer,
            reader: Some(reader),
            output,
            events,
            term: Term::new(
                Config::default(),
                &Size {
                    rows: rows.into(),
                    cols: cols.into(),
                },
                Events(sender),
            ),
            parser: Processor::new(),
            recent: Vec::new(),
            eof: false,
            pid,
            exit: None,
        })
    }

    pub fn send(&mut self, bytes: impl AsRef<[u8]>) -> io::Result<()> {
        self.writer
            .write_all(bytes.as_ref())
            .and_then(|()| self.writer.flush())
            .map_err(|error| self.failure(error))
    }

    pub fn screen(&self) -> String {
        let grid = self.term.grid();
        (0..grid.screen_lines())
            .map(|row| {
                (0..grid.columns())
                    .map(|col| grid[Line(row as i32)][alacritty_terminal::index::Column(col)].c)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn wait_for_text(&mut self, text: &str, timeout: Duration) -> io::Result<()> {
        self.wait(timeout, |terminal| terminal.screen().contains(text))
    }

    pub fn wait_for_bytes(&mut self, bytes: &[u8], timeout: Duration) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.wait(timeout, |terminal| {
            terminal
                .recent
                .windows(bytes.len())
                .any(|window| window == bytes)
        })
    }

    fn wait(&mut self, timeout: Duration, condition: impl Fn(&Self) -> bool) -> io::Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if condition(self) {
                return Ok(());
            }
            if Instant::now() >= deadline || self.eof {
                return Err(self.failure(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "terminal expectation not met",
                )));
            }
            self.pump(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(5)),
            )?;
        }
    }

    fn pump(&mut self, timeout: Duration) -> io::Result<()> {
        match self.output.recv_timeout(timeout) {
            Ok(result) => {
                let bytes = result.map_err(|error| self.failure(error))?;
                self.eof = bytes.is_empty();
                self.parser.advance(&mut self.term, &bytes);
                self.recent.extend(bytes);
                if self.recent.len() > 65536 {
                    self.recent.drain(..self.recent.len() - 65536);
                }
                while let Ok(event) = self.events.try_recv() {
                    if let Event::PtyWrite(reply) = event {
                        self.send(reply)?;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => self.eof = true,
            Err(mpsc::RecvTimeoutError::Timeout) => (),
        }
        Ok(())
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> io::Result<()> {
        let size = pty_size(rows, cols)?;
        self.term.resize(Size {
            rows: rows.into(),
            cols: cols.into(),
        });
        self.master
            .resize(size)
            .map_err(|error| self.failure(io::Error::other(error)))
    }

    pub fn wait_exit(&mut self, timeout: Duration) -> io::Result<i32> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.exit.is_none() {
                self.exit = match waitpid(self.pid, Some(WaitPidFlag::WNOHANG)) {
                    Ok(WaitStatus::Exited(_, code)) => Some(code),
                    Ok(WaitStatus::Signaled(_, signal, _)) => Some(128 + signal as i32),
                    Ok(_) | Err(Errno::EINTR) => None,
                    Err(error) => {
                        return Err(self.failure(io::Error::from_raw_os_error(error as i32)));
                    }
                };
            }
            if let Some(code) = self.exit.filter(|_| self.eof) {
                return Ok(code);
            }
            if Instant::now() >= deadline {
                return Err(self.failure(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "child did not exit and close its PTY",
                )));
            }
            self.pump(Duration::from_millis(5))?;
        }
    }

    fn failure(&self, error: io::Error) -> io::Error {
        io::Error::new(
            error.kind(),
            format!(
                "{error}\nSCREEN:\n{}\nRECENT BYTES:\n{:?}",
                self.screen(),
                String::from_utf8_lossy(&self.recent)
            ),
        )
    }
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
        if self.exit.is_none() {
            self.exit = match waitpid(self.pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(_, code)) => Some(code),
                Ok(WaitStatus::Signaled(_, signal, _)) => Some(128 + signal as i32),
                Ok(_) | Err(Errno::EINTR) => None,
                Err(error) => {
                    eprintln!("PTY child {} status failed: {error}", self.pid);
                    None
                }
            };
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
