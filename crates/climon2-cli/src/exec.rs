use crate::Error;
use climon2_core::{
    SessionId,
    cli::CommandArgs,
    config::{Assembly, ConfigSource, ExecConfig},
    layout::{ExecutableLayout, HomeLayout, VersionBinary},
    lifetime::Lifetime,
    metadata::SessionMetadata,
    protocol::TerminalSize,
    session::{NestedStart, Notice, PROMPT, SessionEnvironment},
    startup_envelope::{ENVIRONMENT_VARIABLE, StartupEnvelope},
};
use climon2_platform::{
    daemon::{self, Readiness},
    logging::{LogFile, Role},
    runtime::RuntimeDirectory,
    terminal,
};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// A nested start the user cancels creates nothing, not even a log, and
/// exits with the status [`Answer::status`](climon2_core::session::Answer::status) gives.
pub fn run(
    home: &Path,
    lifetime: Lifetime,
    nested: NestedStart,
    command: CommandArgs,
    general: &LogFile,
    filter: Option<&str>,
) -> Result<u32, Error> {
    let (config, layout) = prepare(home, lifetime)?;
    let session = NewSession::create().map_err(|error| error.logged(general.path()))?;
    // The working and runtime directories are checked before the prompt, so
    // no one is asked to start a session that is bound to fail.
    let startup = Startup::new(home, &layout, &session.id, lifetime, &config)?;
    if let Some(status) = confirm(session.environment.notice(lifetime, nested))? {
        return Ok(status);
    }
    let log = session_log(home, &session.id, general)?;
    let dispatch = log.dispatch(filter)?;
    tracing::dispatcher::with_default(&dispatch, || {
        startup
            .ready(command, session.environment, filter)
            .and_then(|daemon| startup.finish(daemon, filter, config.client))
            .map_err(|error| error.logged(log.path().parent().expect("session log directory")))
    })
}

/// Configuration and terminals are checked before anything is created.
fn prepare(home: &Path, lifetime: Lifetime) -> Result<(ExecConfig, ExecutableLayout), Error> {
    let config = exec_config(home)?;
    Ok((config, preflight(lifetime)?))
}

/// A session's ID and the environment its command gets.
struct NewSession {
    id: SessionId,
    environment: SessionEnvironment,
}

impl NewSession {
    /// Before any session log exists, so a depth overflow is recorded in
    /// the general log, which its error names.
    fn create() -> Result<Self, Error> {
        let id = new_session_id()?;
        let inherited = std::env::vars_os().collect::<Vec<_>>();
        let environment = SessionEnvironment::for_session(&id, &inherited).map_err(|error| {
            tracing::error!(%error, "cannot start the session");
            Error::new(125, error)
        })?;
        Ok(Self { id, environment })
    }
}

/// Shows `notice`: a headless session's warning on stderr; the warning and
/// prompt on the terminal itself, stdout, which an ordinary exec has checked
/// is one, then waits for the answer. The status to exit with unless it was
/// Enter.
fn confirm(notice: Notice<'_>) -> io::Result<Option<u32>> {
    match notice {
        Notice::Quiet => Ok(None),
        Notice::Warn(warning) => {
            eprintln!("{warning}");
            Ok(None)
        }
        Notice::Ask(warning) => ask(warning),
    }
}

/// The terminal is raw, and earlier input discarded, before the prompt
/// shows, so only a key typed once it shows answers it, and Ctrl+C is a key,
/// never a signal. A hang-up anywhere, opening the terminal included, is a
/// terminal gone.
fn ask(warning: &str) -> io::Result<Option<u32>> {
    let text = format!("{warning}\r\n{PROMPT}\r\n");
    Ok(terminal::ask(terminal::input(), terminal::output(), &text)?.status())
}

fn exec_config(home: &Path) -> Result<ExecConfig, Error> {
    let layout = HomeLayout::at(home).map_err(io::Error::other)?;
    ConfigSource::read(&layout)
        .and_then(|source| source.exec())
        .map_err(|error| Error::new(125, error))
}

/// TTY checks happen before anything is created or spawned; a headless
/// session needs no terminal.
fn preflight(lifetime: Lifetime) -> io::Result<ExecutableLayout> {
    if lifetime == Lifetime::Ordinary {
        terminal::require_ttys()?;
    }
    crate::executable_layout()
}

fn new_session_id() -> io::Result<SessionId> {
    uuid::Uuid::new_v4()
        .to_string()
        .parse()
        .map_err(io::Error::other)
}

fn session_log(home: &Path, id: &SessionId, general: &LogFile) -> io::Result<LogFile> {
    let session = LogFile::session(home, id, Role::Cli, None, Some(general.path()))?;
    writeln!(
        general.writer(),
        "session {id} started; continuing in {}",
        session.path().display()
    )?;
    Ok(session)
}

struct Startup<'a> {
    home: &'a Path,
    layout: &'a ExecutableLayout,
    id: &'a SessionId,
    lifetime: Lifetime,
    max_history_rows: u32,
    assembly: Assembly,
    metadata: PathBuf,
    socket: PathBuf,
    size: TerminalSize,
    cwd: PathBuf,
}

impl Startup<'_> {
    fn new<'a>(
        home: &'a Path,
        layout: &'a ExecutableLayout,
        id: &'a SessionId,
        lifetime: Lifetime,
        config: &ExecConfig,
    ) -> Result<Startup<'a>, Error> {
        let socket = RuntimeDirectory::prepare(home)?.socket_path(id)?;
        let metadata = home.join("sessions").join(format!("{id}.pty"));
        let size = terminal_size();
        let cwd = working_directory().map_err(|error| Error::new(126, error))?;
        Ok(Startup {
            home,
            layout,
            id,
            lifetime,
            metadata,
            socket,
            size,
            cwd,
            max_history_rows: config.scrollback.max_rows.get(),
            assembly: config.assembly,
        })
    }

    fn ready(
        &self,
        command: CommandArgs,
        environment: SessionEnvironment,
        filter: Option<&str>,
    ) -> Result<Started<'_>, Error> {
        let pending = self.spawn(command, environment, filter)?;
        let pid = pending.id();
        tracing::debug!(daemon_pid = pid, "waiting for daemon");
        match pending.wait_ready(&[&self.metadata, &self.socket])? {
            Readiness::Ready => Ok(Started {
                pid,
                metadata: &self.metadata,
                socket: &self.socket,
            }),
            Readiness::Failed { code, message } => Err(Error::new(code, message)),
        }
    }

    fn spawn(
        &self,
        command: CommandArgs,
        environment: SessionEnvironment,
        filter: Option<&str>,
    ) -> io::Result<daemon::PendingDaemon> {
        let daemon = self
            .layout
            .binary(self.layout.version(), VersionBinary::Daemon)
            .canonicalize()?;
        let mut launch = Command::new(daemon);
        launch
            .env_clear()
            .envs(environment.values)
            .stderr(Stdio::null());
        launch
            .arg("--session-id")
            .arg(self.id.as_str())
            .arg("--home")
            .arg(self.home)
            .arg("--socket")
            .arg(&self.socket)
            .arg("--version")
            .arg(self.layout.version().as_str())
            .arg("--cli-pid")
            .arg(std::process::id().to_string())
            .arg("--lifetime")
            .arg(self.lifetime.name())
            .arg("--rows")
            .arg(self.size.rows.to_string())
            .arg("--cols")
            .arg(self.size.cols.to_string())
            .arg("--scrollback-max-rows")
            .arg(self.max_history_rows.to_string())
            .arg("--assembly-records")
            .arg(self.assembly.records().to_string())
            .arg("--assembly-bytes")
            .arg(self.assembly.bytes().to_string())
            .arg("--assembly-ms")
            .arg(self.assembly.lifetime_ms().to_string())
            .arg("--cwd")
            .arg(&self.cwd);
        if let Some(filter) = filter {
            launch.arg("--log").arg(filter);
        }
        daemon::spawn(launch, |launch, fd| {
            launch
                .arg("--readiness-fd")
                .arg(fd.to_string())
                .arg("--")
                .arg(command.command)
                .args(command.args);
        })
    }

    /// Ordinary exec becomes the client; headless exec only reports the session.
    /// `daemon` stops the session unless the CLI succeeds.
    fn finish(
        &self,
        daemon: Started<'_>,
        filter: Option<&str>,
        settings: climon2_core::keys::ClientSettings,
    ) -> Result<u32, Error> {
        match self.lifetime {
            Lifetime::Ordinary => self.handoff(filter, settings),
            Lifetime::Headless => Ok(self.announce(daemon)),
        }
    }

    /// The daemon confirmed creation, so the session outlives this CLI and
    /// exec succeeds even when its ID cannot be written to stdout; `list`
    /// still shows the session.
    fn announce(&self, daemon: Started<'_>) -> u32 {
        daemon.disarm();
        let mut stdout = io::stdout().lock();
        if let Err(error) = writeln!(stdout, "{}", self.id).and_then(|()| stdout.flush()) {
            tracing::warn!(%error, "cannot print the session ID");
        }
        tracing::debug!(session_id = %self.id, "headless session started");
        0
    }

    fn handoff(
        &self,
        filter: Option<&str>,
        settings: climon2_core::keys::ClientSettings,
    ) -> Result<u32, Error> {
        let mut client = self.client(&self.written_metadata()?)?;
        client.env(
            ENVIRONMENT_VARIABLE,
            StartupEnvelope::Attach(settings).to_json(),
        );
        if let Some(filter) = filter {
            client.env("CLIMON2_LOG", filter);
        }
        tracing::debug!(session_id = %self.id, "handing off to version-matched client");
        #[cfg(feature = "test-hooks")]
        hold_handoff();
        Err(crate::native::exec_client(client))
    }

    fn written_metadata(&self) -> io::Result<SessionMetadata> {
        SessionMetadata::from_json(&self.metadata, &fs::read(&self.metadata)?)
            .map_err(io::Error::other)
    }

    fn client(&self, metadata: &SessionMetadata) -> Result<Command, Error> {
        let invocation = self
            .layout
            .client_invocation(&metadata.climon2_version, &self.metadata)
            .map_err(io::Error::other)?;
        let client = invocation.program.canonicalize().map_err(|_| {
            Error::new(
                125,
                format!(
                    "session {} needs climon2 {}, which is not installed",
                    metadata.session_id, metadata.climon2_version
                ),
            )
        })?;
        let mut client = Command::new(client);
        client.args(invocation.args).env("CLIMON2_HOME", self.home);
        Ok(client)
    }
}

/// TIOCGWINSZ's value for an unknown dimension.
const UNKNOWN: u16 = 0;

/// The stdin terminal's size; without one (headless exec), the fallback for
/// an unknown size, 80x24.
fn terminal_size() -> TerminalSize {
    let unknown = TerminalSize {
        cols: UNKNOWN,
        rows: UNKNOWN,
    };
    terminal::input()
        .and_then(|input| terminal::size(&input))
        .unwrap_or(unknown)
        .initial()
}

struct Started<'a> {
    pid: u32,
    metadata: &'a Path,
    socket: &'a Path,
}

/// The working directory the session's command starts in: this CLI's. Test
/// hook: `CLIMON2_CLI_CWD` stands in for one no process can be started in,
/// such as one over 260 characters on Windows.
fn working_directory() -> io::Result<PathBuf> {
    #[cfg(feature = "test-hooks")]
    if let Some(cwd) = std::env::var_os("CLIMON2_CLI_CWD") {
        return Ok(PathBuf::from(cwd));
    }
    std::env::current_dir()
}

/// Test hook: announces `<CLIMON2_CLI_HOLD>.held`, then waits until the
/// `CLIMON2_CLI_HOLD` file exists before the client starts.
#[cfg(feature = "test-hooks")]
fn hold_handoff() {
    use std::{thread, time::Duration};
    let Some(release) = std::env::var_os("CLIMON2_CLI_HOLD").map(PathBuf::from) else {
        return;
    };
    let _ = fs::write(release.with_extension("held"), b"");
    while !release.exists() {
        thread::sleep(Duration::from_millis(10));
    }
}

impl Drop for Started<'_> {
    fn drop(&mut self) {
        self.stop();
        let _ = fs::remove_file(self.metadata);
        let _ = fs::remove_file(self.socket);
    }
}

impl Started<'_> {
    /// The session outlives this CLI.
    fn disarm(self) {
        std::mem::forget(self);
    }

    fn stop(&self) {
        crate::native::stop_daemon(self.pid);
    }
}
