use crate::Error;
use climon2_core::{
    SessionId,
    cli::CommandArgs,
    layout::{ExecutableLayout, VersionBinary},
    metadata::SessionMetadata,
    protocol::TerminalSize,
    session::SessionEnvironment,
};
use climon2_platform::{
    daemon::{self, Readiness},
    logging::{LogFile, Role},
    process::{self, Signal},
    runtime::RuntimeDirectory,
    terminal,
};
use std::{
    fs,
    io::{self, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub fn run(
    home: &Path,
    command: CommandArgs,
    general: &LogFile,
    filter: Option<&str>,
) -> Result<u32, Error> {
    let layout = preflight()?;
    let id = new_session_id()?;
    let session = session_log(home, &id, general)?;
    let dispatch = session.dispatch(filter)?;
    tracing::dispatcher::with_default(&dispatch, || {
        launch(home, command, &layout, &id, filter)
            .map_err(|error| error.logged(session.path().parent().expect("session log directory")))
    })
}

/// TTY checks happen before anything is created or spawned.
fn preflight() -> io::Result<ExecutableLayout> {
    terminal::require_ttys()?;
    executable_layout()
}

fn new_session_id() -> io::Result<SessionId> {
    uuid::Uuid::new_v4()
        .to_string()
        .parse()
        .map_err(io::Error::other)
}

fn executable_layout() -> io::Result<ExecutableLayout> {
    let own_exe = std::env::current_exe()?.canonicalize()?;
    ExecutableLayout::from_cli_path(&own_exe).map_err(io::Error::other)
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
    metadata: PathBuf,
    socket: PathBuf,
    size: TerminalSize,
    cwd: PathBuf,
}

fn launch(
    home: &Path,
    command: CommandArgs,
    layout: &ExecutableLayout,
    id: &SessionId,
    filter: Option<&str>,
) -> Result<u32, Error> {
    let startup = Startup::new(home, layout, id)?;
    let inherited = std::env::vars_os().collect::<Vec<_>>();
    let environment =
        SessionEnvironment::for_session(id, &inherited).map_err(|error| Error::new(125, error))?;
    warn_if_nested(&environment);
    let _daemon = startup.ready(command, environment, filter)?;
    startup.handoff(filter)
}

fn warn_if_nested(environment: &SessionEnvironment) {
    if let Some(warning) = &environment.nested_warning {
        eprintln!("{warning}");
    }
}

impl Startup<'_> {
    fn new<'a>(
        home: &'a Path,
        layout: &'a ExecutableLayout,
        id: &'a SessionId,
    ) -> Result<Startup<'a>, Error> {
        let socket = RuntimeDirectory::prepare(home)?.socket_path(id)?;
        let metadata = home.join("sessions").join(format!("{id}.pty"));
        let size = terminal_size()?;
        let cwd = std::env::current_dir().map_err(|error| Error::new(126, error))?;
        Ok(Startup {
            home,
            layout,
            id,
            metadata,
            socket,
            size,
            cwd,
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
            .arg("--rows")
            .arg(self.size.rows.to_string())
            .arg("--cols")
            .arg(self.size.cols.to_string())
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

    fn handoff(&self, filter: Option<&str>) -> Result<u32, Error> {
        let mut client = self.client(&self.written_metadata()?)?;
        if let Some(filter) = filter {
            client.env("CLIMON2_LOG", filter);
        }
        tracing::debug!(session_id = %self.id, "handing off to version-matched client");
        Err(Error::from(client.exec()))
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

fn terminal_size() -> io::Result<TerminalSize> {
    terminal::size(&terminal::input()?)
}

struct Started<'a> {
    pid: u32,
    metadata: &'a Path,
    socket: &'a Path,
}

impl Drop for Started<'_> {
    fn drop(&mut self) {
        self.stop();
        let _ = fs::remove_file(self.metadata);
        let _ = fs::remove_file(self.socket);
    }
}

impl Started<'_> {
    fn stop(&self) {
        let _ = process::signal_group(self.pid, Signal::Terminate);
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            match process::try_wait(self.pid) {
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                _ => return,
            }
        }
        let _ = process::signal_group(self.pid, Signal::Kill);
        let _ = process::wait(self.pid);
    }
}
