use crate::{args::Options, startup::Failure};
use climon2_platform::{peer, process, runtime};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    ffi::OsString,
    io::{self, Read, Write},
    os::unix::{net::UnixStream, process::CommandExt},
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

const OBSERVE_JOBS: Duration = Duration::from_millis(100);

pub(crate) struct Pty {
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    pid: u32,
    session: process::TerminalSession,
    observed: Instant,
    reaped: bool,
}

impl Pty {
    pub fn spawn(options: &Options, startup: ExecListener) -> Result<Self, Failure> {
        let (pair, terminal) = open_pair(options.size)?;
        let pid = start_command(pair.slave.as_ref(), options, &startup.path)?;
        drop(pair.slave);
        let pty = Self::from_master(pair.master, pid, terminal)?;
        tracing::debug!(command_pid = pid, "command terminal created");
        startup.confirm().map(|()| pty)
    }

    fn from_master(
        master: Box<dyn MasterPty + Send>,
        pid: u32,
        terminal: process::Terminal,
    ) -> Result<Self, Failure> {
        let session = process::TerminalSession::new(pid, terminal)
            .map_err(|_| Failure::own("cannot identify command session"))?;
        let mut pty = Self {
            master: Some(master),
            writer: None,
            pid,
            session,
            observed: Instant::now(),
            reaped: false,
        };
        pty.writer = Some(
            pty.master()
                .take_writer()
                .map_err(|_| Failure::own("cannot open terminal input"))?,
        );
        nonblocking(pty.fd()).map_err(|_| Failure::own("cannot configure command terminal"))?;
        Ok(pty)
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }
    fn master(&self) -> &dyn MasterPty {
        self.master.as_ref().expect("live PTY").as_ref()
    }
    fn fd(&self) -> i32 {
        self.master().as_raw_fd().expect("macOS PTY descriptor")
    }
    pub fn resize(&self, value: climon2_core::protocol::TerminalSize) -> io::Result<()> {
        self.master()
            .resize(size(value))
            .map_err(|_| super::io_error("terminal resize failed"))
    }
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writer.as_mut().expect("live PTY writer").write(bytes)
    }
    /// Waits at most `timeout` for output (or hangup) without reading it.
    pub fn wait_readable(&self, timeout: std::time::Duration) {
        let mut descriptor = libc::pollfd {
            fd: self.fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let milliseconds = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
        // An interrupted or failed poll only ends this wait early; the next read reports errors.
        unsafe { libc::poll(&mut descriptor, 1, milliseconds) };
    }
    /// Only the session output pump calls this. No cloned PTY readers exist.
    pub fn read_output(&self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = unsafe { libc::read(self.fd(), bytes.as_mut_ptr().cast(), bytes.len()) };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(count as usize)
    }
    pub fn try_wait(&mut self) -> io::Result<Option<u32>> {
        let status = process::try_wait(self.pid)?;
        if let Some(status) = status {
            self.reaped = true;
            return status.code().map(Some).map_err(io::Error::other);
        }
        Ok(None)
    }
    fn close_master(&mut self) {
        close(&mut self.master, &mut self.writer);
    }
    pub fn leave(&mut self) -> io::Result<()> {
        if self.reaped {
            self.close_master();
            std::thread::sleep(climon2_core::protocol::HANGUP_GRACE);
            return self.session.kill();
        }
        let (master, writer) = (&mut self.master, &mut self.writer);
        let result = process::hangup(&mut self.session, || close(master, writer));
        self.reaped = result.is_ok();
        result.map(|_| ())
    }

    pub fn finish(&mut self) -> io::Result<()> {
        self.close_master();
        self.session.kill()
    }

    /// Records the command session's job groups while its leader may still hold the terminal;
    /// at most every `OBSERVE_JOBS`, as each look lists the terminal's processes.
    pub fn observe_jobs(&mut self) -> io::Result<()> {
        if self.observed.elapsed() < OBSERVE_JOBS {
            return Ok(());
        }
        self.observed = Instant::now();
        self.session.observe()
    }
}

/// Drops every master handle: portable-pty's writer, then the master itself.
fn close(
    master: &mut Option<Box<dyn MasterPty + Send>>,
    writer: &mut Option<Box<dyn Write + Send>>,
) {
    writer.take();
    master.take();
}

/// The allocation lock is held only while the pair is opened. The slave's device is
/// captured now, while it exists, to scope every later session kill to this terminal.
fn open_pair(
    terminal: climon2_core::protocol::TerminalSize,
) -> Result<(portable_pty::PtyPair, process::Terminal), Failure> {
    let pair = {
        let _allocation = climon2_platform::pty::allocation()
            .map_err(|_| Failure::own("cannot lock terminal allocation"))?;
        native_pty_system().openpty(size(terminal))
    }
    .map_err(|_| Failure::own("cannot allocate command terminal"))?;
    let device = slave_terminal(pair.master.as_ref())?;
    Ok((pair, device))
}

fn slave_terminal(master: &dyn MasterPty) -> Result<process::Terminal, Failure> {
    let path = master
        .tty_name()
        .ok_or_else(|| Failure::own("cannot identify command terminal"))?;
    process::Terminal::from_path(&path)
        .map_err(|_| Failure::own("cannot identify command terminal"))
}

fn start_command(
    slave: &dyn portable_pty::SlavePty,
    options: &Options,
    startup_path: &std::path::Path,
) -> Result<u32, Failure> {
    let command = command_builder(options, startup_path)?;
    spawn_child(slave, command, options)
}

fn command_builder(
    options: &Options,
    startup_path: &std::path::Path,
) -> Result<CommandBuilder, Failure> {
    let mut command = CommandBuilder::new(
        std::env::current_exe().map_err(|_| Failure::own("cannot locate daemon executable"))?,
    );
    command.args([
        "--exec-command",
        startup_path
            .to_str()
            .ok_or_else(|| Failure::own("invalid startup path"))?,
        shell_policy(),
    ]);
    command.arg(&options.cwd);
    command.arg(&options.command);
    command.args(&options.arguments);
    command.cwd(&options.cwd);
    command.env_clear();
    for (key, value) in std::env::vars_os() {
        command.env(key, value);
    }
    Ok(command)
}

/// portable-pty always supplies SHELL, so the helper must know whether it was really set.
fn shell_policy() -> &'static str {
    if std::env::var_os("SHELL").is_none() {
        return "unset-shell";
    }
    "keep-shell"
}

fn spawn_child(
    slave: &dyn portable_pty::SlavePty,
    command: CommandBuilder,
    options: &Options,
) -> Result<u32, Failure> {
    process::validate_cwd(&options.cwd).map_err(|_| Failure {
        code: 126,
        message: "command working directory is not accessible",
    })?;
    process::before_fork();
    let child = slave
        .spawn_command(command)
        .map_err(|_| Failure::own("cannot start command terminal"))?;
    let pid = child
        .process_id()
        .ok_or_else(|| Failure::own("command process identity unavailable"))?;
    drop(child);
    Ok(pid)
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.close_master();
        if !self.reaped {
            let _ = self.session.kill();
            let _ = process::wait(self.pid);
        }
    }
}

fn size(value: climon2_core::protocol::TerminalSize) -> PtySize {
    PtySize {
        rows: value.rows,
        cols: value.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn nonblocking(fd: i32) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) struct ExecListener {
    listener: runtime::SocketListener,
    path: PathBuf,
    deadline: Instant,
}

impl ExecListener {
    pub fn bind(options: &Options) -> Result<Self, Failure> {
        let runtime = runtime::RuntimeDirectory::prepare(&options.home)
            .map_err(|_| Failure::own("cannot prepare command startup runtime"))?;
        let path = runtime
            .socket_path(&options.id)
            .map_err(|_| Failure::own("cannot locate command startup socket"))?
            .with_extension("exec");
        let listener =
            runtime::bind(&path).map_err(|_| Failure::own("cannot bind command startup socket"))?;
        let startup = Self {
            listener,
            path,
            deadline: Instant::now() + Duration::from_secs(4),
        };
        startup
            .listener
            .set_nonblocking(true)
            .map_err(|_| Failure::own("cannot configure command startup socket"))?;
        Ok(startup)
    }

    fn confirm(&self) -> Result<(), Failure> {
        let mut stream = self.accept()?;
        stream
            .set_nonblocking(true)
            .map_err(|_| Failure::own("cannot configure command startup confirmation"))?;
        confirm_exec(&mut stream, self.deadline)
    }

    fn accept(&self) -> Result<UnixStream, Failure> {
        while Instant::now() < self.deadline {
            if let Some(stream) = super::unless_would_block(peer::accept(&self.listener))
                .map_err(|_| Failure::own("command startup connection failed"))?
            {
                return Ok(stream);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Err(Failure::own("command startup deadline exceeded"))
    }
}

impl Drop for ExecListener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn exec_command(arguments: &[OsString]) -> u8 {
    exec(arguments).unwrap_or(125)
}

fn exec(arguments: &[OsString]) -> io::Result<u8> {
    if arguments.len() < 4 {
        return Err(super::io_error("invalid command invocation"));
    }
    let mut stream = UnixStream::connect(&arguments[0])?;
    peer::authenticate(&stream)?;
    stream.write_all(&[0])?;
    execute(arguments, &mut stream)
}

/// Returns only when exec failed; success replaces this process.
fn execute(arguments: &[OsString], stream: &mut UnixStream) -> io::Result<u8> {
    let error = command(arguments).exec();
    let code = launch_code(&error, std::path::Path::new(&arguments[2]));
    stream.write_all(&[code])?;
    Ok(code)
}

fn command(arguments: &[OsString]) -> Command {
    let mut command = Command::new(&arguments[3]);
    command.args(&arguments[4..]);
    command.current_dir(&arguments[2]);
    if arguments[1] == "unset-shell" {
        command.env_remove("SHELL");
    }
    command
}

/// 127 only for a missing command in a usable cwd; any other exec failure is 126.
fn launch_code(error: &io::Error, cwd: &std::path::Path) -> u8 {
    if error.kind() == io::ErrorKind::NotFound && cwd.is_dir() {
        return 127;
    }
    126
}

fn confirm_exec(stream: &mut UnixStream, deadline: Instant) -> Result<(), Failure> {
    let mut bytes = [0; 2];
    let mut length = 0;
    while Instant::now() < deadline {
        let code = confirmation_read(stream, &mut bytes, &mut length)
            .map_err(|_| Failure::own("command startup confirmation failed"))?;
        if let Some(code) = code {
            return launched(code);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Err(Failure::own("command startup deadline exceeded"))
}

fn launched(code: u8) -> Result<(), Failure> {
    if code == 0 {
        return Ok(());
    }
    Err(Failure::launch(u32::from(code)))
}

/// A 0-byte read is EOF: the helper's CLOEXEC connection closed.
fn confirmation_read(
    stream: &mut UnixStream,
    bytes: &mut [u8; 2],
    length: &mut usize,
) -> io::Result<Option<u8>> {
    match super::unless_would_block(stream.read(&mut bytes[*length..]))? {
        Some(count) => {
            *length += count;
            confirmation(&bytes[..*length], count == 0)
        }
        None => Ok(None),
    }
}

fn confirmation(bytes: &[u8], eof: bool) -> io::Result<Option<u8>> {
    match (eof, bytes) {
        (false, [] | [0]) => Ok(None),
        (true, [0]) => Ok(Some(0)),
        (_, [0, code @ (126 | 127)]) => Ok(Some(*code)),
        _ => Err(super::io_error("invalid command confirmation")),
    }
}
