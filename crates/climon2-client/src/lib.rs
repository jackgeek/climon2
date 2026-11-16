//! Version-matched, byte-transparent terminal client (ADR 0001).

mod bridge;
mod embedded;
mod kill;
mod library;
mod transport;

use climon2_core::{
    config::ConfigSource,
    exit::CLIMON2_ERROR,
    keys::ClientSettings,
    metadata::SessionMetadata,
    startup_envelope::{ENVIRONMENT_VARIABLE, StartupEnvelope},
};
use climon2_platform::{
    home,
    logging::{LogFile, Role},
    terminal,
};
use std::{
    ffi::OsString,
    fmt, fs,
    fs::File,
    io,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

/// How the client executable fails: the thin main prints it, after the
/// terminal is restored, and exits with its code.
pub use climon2_core::failure::Failure as Error;

fn failure(error: impl fmt::Display, logs: Option<&Path>) -> Error {
    Error {
        logs: logs.map(Path::to_owned),
        ..Error::new(CLIMON2_ERROR, error)
    }
}

fn metadata_path(arguments: impl IntoIterator<Item = OsString>) -> io::Result<PathBuf> {
    let args: Vec<_> = arguments.into_iter().collect();
    match args.as_slice() {
        [attach, flag, path]
            if attach == "attach" && flag == "--metadata" && Path::new(path).is_absolute() =>
        {
            Ok(path.into())
        }
        _ => Err(io::Error::other(
            "usage: climon2-client attach --metadata <absolute path>",
        )),
    }
}

/// Arguments exclude argv[0]; all other configuration comes from environment.
/// Errors are returned only after raw mode and file status flags are restored.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<u32, Error> {
    let operation = startup_operation().map_err(|error| failure(error, None))?;
    let path = metadata_path(arguments).map_err(|error| failure(error, None))?;
    let (metadata, log) = session_log(&path).map_err(|error| failure(error, None))?;
    let logs = log.path().parent();
    let filter = std::env::var("CLIMON2_LOG").ok();
    let dispatch = log
        .dispatch(filter.as_deref())
        .map_err(|error| failure(error, logs))?;
    tracing::dispatcher::with_default(&dispatch, || {
        let session = kill::Session {
            metadata: &path,
            log: log.path(),
            filter: filter.as_deref(),
        };
        supervised(&metadata, &session, operation)
            .map_err(|error| session_failure(error, &metadata, logs))
    })
}

/// The CLI's startup envelope wins; a directly started client attaches with config.
fn startup_operation() -> io::Result<StartupEnvelope> {
    match std::env::var_os(ENVIRONMENT_VARIABLE) {
        Some(envelope) => StartupEnvelope::parse(envelope.as_bytes()).map_err(io::Error::other),
        None => config_settings().map(StartupEnvelope::Attach),
    }
}

fn config_settings() -> io::Result<ClientSettings> {
    let source = ConfigSource::read(&home::named()?).map_err(io::Error::other)?;
    source
        .attach()
        .map(|config| config.client)
        .map_err(io::Error::other)
}

/// Before the log exists, failures carry no logs suffix.
fn session_log(path: &Path) -> io::Result<(SessionMetadata, LogFile)> {
    let metadata = read_metadata(path)?;
    // The CLI already canonicalized this home; direct clients must supply it too.
    let log = LogFile::session(
        home::named()?.root(),
        &metadata.session_id,
        Role::Client,
        None,
        None,
    )?;
    Ok((metadata, log))
}

fn read_metadata(path: &Path) -> io::Result<SessionMetadata> {
    let bytes = fs::read(path)?;
    SessionMetadata::from_json(path, &bytes).map_err(io::Error::other)
}

/// Panics are logged without their payload and reported as one generic failure.
fn supervised(
    metadata: &SessionMetadata,
    session: &kill::Session,
    operation: StartupEnvelope,
) -> io::Result<u32> {
    std::panic::set_hook(Box::new(|_| tracing::error!("client panic")));
    std::panic::catch_unwind(|| match operation {
        StartupEnvelope::Attach(settings) => attach(metadata, settings),
        StartupEnvelope::Kill => kill(metadata, session),
    })
    .unwrap_or_else(|_| Err(io::Error::other("client panicked")))
}

/// Kill mode, through this release's client library: success only on
/// confirmed termination; a termination signal wins over any outcome.
fn kill(metadata: &SessionMetadata, session: &kill::Session) -> io::Result<u32> {
    let signals = terminal::Signals::client()?;
    let result = kill::run(metadata, session, home::named()?.root(), &signals);
    if let Some(signal) = signals.termination_signal() {
        return Ok(128 + signal);
    }
    result
}

fn session_failure(error: io::Error, metadata: &SessionMetadata, logs: Option<&Path>) -> Error {
    tracing::warn!(errno = error.raw_os_error(), "client connection failed");
    if error.kind() == io::ErrorKind::UnexpectedEof {
        return failure(
            format!("lost connection to session {}", metadata.session_id),
            logs,
        );
    }
    failure(error, logs)
}

fn attach(metadata: &SessionMetadata, settings: ClientSettings) -> io::Result<u32> {
    let state = Terminal::new()?;
    let result = connection(&state, metadata);
    if let Some(signal) = state.signals.termination_signal() {
        return Ok(128 + signal);
    }
    let (transport, size) = result?;
    raw_bridge(transport, state, size, metadata, settings)
}

fn connection(
    state: &Terminal,
    metadata: &SessionMetadata,
) -> io::Result<(transport::Transport, climon2_core::protocol::TerminalSize)> {
    let mut transport = transport::Transport::connect(metadata)?;
    let size = handshake(&mut transport, state, metadata)?;
    Ok((transport, size))
}

struct Terminal {
    input: File,
    signals: terminal::Signals,
}

impl Terminal {
    fn new() -> io::Result<Self> {
        terminal::require_ttys()?;
        Ok(Self {
            input: terminal::input()?,
            signals: terminal::Signals::client()?,
        })
    }
}

fn handshake(
    transport: &mut transport::Transport,
    state: &Terminal,
    metadata: &SessionMetadata,
) -> io::Result<climon2_core::protocol::TerminalSize> {
    tracing::debug!("starting hello handshake");
    transport.hello(metadata, &state.signals)?;
    let size = terminal::size(&state.input)?;
    transport.send(&climon2_core::protocol::ClientMessage::Attach(size))?;
    transport.attached(&state.signals)?;
    tracing::debug!("client attached");
    Ok(size)
}

fn raw_bridge(
    transport: transport::Transport,
    state: Terminal,
    size: climon2_core::protocol::TerminalSize,
    metadata: &SessionMetadata,
    settings: ClientSettings,
) -> io::Result<u32> {
    let raw = terminal::RawModeGuard::new(&state.input)?;
    #[cfg(feature = "test-hooks")]
    client_fault();
    let result = bridge::run(
        transport,
        state.input,
        state.signals,
        size,
        &metadata.session_id,
        settings,
    )?;
    drop(raw);
    Ok(exit_code(result, metadata))
}

#[cfg(feature = "test-hooks")]
fn client_fault() {
    if std::env::var_os("CLIMON2_CLIENT_FAULT").as_deref() == Some(std::ffi::OsStr::new("panic")) {
        panic!("CLIMON2_CLIENT_FAULT_PANIC");
    }
}

/// Raw mode is already restored, so the detach notice lands on a cooked terminal.
fn exit_code(result: bridge::BridgeExit, metadata: &SessionMetadata) -> u32 {
    match result {
        bridge::BridgeExit::Status(code) => code,
        bridge::BridgeExit::Detached => {
            eprintln!(
                "climon2: detached from session {}; reattach with: climon2 attach {}",
                metadata.session_id, metadata.session_id
            );
            0
        }
    }
}
