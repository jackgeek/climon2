//! A session daemon owns one PTY and one output pump; clients never own terminal handles.
#[cfg(not(target_os = "macos"))]
compile_error!("climon2-daemon currently supports macOS only");

mod args;
mod connection;
mod pty;
mod session;
mod startup;

use climon2_platform::{
    daemon::{Readiness, ReadinessWriter},
    logging::{LogFile, Role},
};
use std::{ffi::OsString, io};

/// Run the private daemon invocation documented in this crate's README.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> u8 {
    let arguments: Vec<_> = arguments.into_iter().collect();
    if arguments.first().is_some_and(|arg| arg == "--exec-command") {
        return pty::exec_command(&arguments[1..]);
    }
    run_session(arguments)
}

fn run_session(arguments: Vec<OsString>) -> u8 {
    let (options, mut readiness) = match invocation(arguments) {
        Ok(invocation) => invocation,
        Err(_) => {
            eprintln!("climon2: error: invalid daemon invocation");
            return 125;
        }
    };
    let result = logged(&options, &mut readiness);
    report(result, &mut readiness)
}

fn report(result: Result<(), startup::Failure>, readiness: &mut ReadinessWriter) -> u8 {
    match result {
        Ok(()) => 0,
        Err(failure) => report_failure(failure, readiness),
    }
}

fn report_failure(failure: startup::Failure, readiness: &mut ReadinessWriter) -> u8 {
    let _ = readiness.send(&Readiness::Failed {
        code: failure.code,
        message: failure.message.into(),
    });
    failure.code as u8
}

fn invocation(arguments: Vec<OsString>) -> io::Result<(args::Options, ReadinessWriter)> {
    let options = args::Options::parse(arguments)?;
    // The CLI transfers this descriptor exclusively to this daemon.
    let readiness = unsafe { ReadinessWriter::from_inherited(options.readiness_fd) }?;
    Ok((options, readiness))
}

fn logged(
    options: &args::Options,
    readiness: &mut ReadinessWriter,
) -> Result<(), startup::Failure> {
    let log = LogFile::session(
        &options.home,
        &options.id,
        Role::Daemon,
        Some(options.cli_pid),
        None,
    )
    .map_err(|_| startup::Failure::own("cannot create daemon log"))?;
    let dispatch = log
        .dispatch(options.log.as_deref())
        .map_err(|_| startup::Failure::own("cannot configure daemon logging"))?;
    std::panic::set_hook(Box::new(|_| eprintln!("climon2: daemon panic")));
    tracing::dispatcher::with_default(&dispatch, || prepared(options, readiness, &log))
}

fn prepared(
    options: &args::Options,
    readiness: &mut ReadinessWriter,
    log: &LogFile,
) -> Result<(), startup::Failure> {
    startup::fault(log)?;
    let resources = startup::Resources::prepare(options)?;
    let exec_listener = pty::ExecListener::bind(options)?;
    let _stderr = startup::capture_diagnostics(log)?;
    start(options, readiness, resources, exec_listener)
}

fn start(
    options: &args::Options,
    readiness: &mut ReadinessWriter,
    mut resources: startup::Resources,
    exec_listener: pty::ExecListener,
) -> Result<(), startup::Failure> {
    climon2_core::test_hook_marker!();
    let signals = climon2_platform::terminal::Signals::daemon()
        .map_err(|_| startup::Failure::own("cannot register daemon signals"))?;
    let mut pty = pty::Pty::spawn(options, exec_listener)?;
    announce(options, readiness, &mut resources, &pty)?;
    serve(options, &mut resources, &mut pty, &signals)
}

fn announce(
    options: &args::Options,
    readiness: &mut ReadinessWriter,
    resources: &mut startup::Resources,
    pty: &pty::Pty,
) -> Result<(), startup::Failure> {
    resources.publish(options)?;
    ready(readiness)?;
    tracing::info!(session_id = %options.id, command_pid = pty.pid(), "session ready");
    Ok(())
}

/// Borrows `resources` so a failed session still drops the PTY before removing session files.
fn serve(
    options: &args::Options,
    resources: &mut startup::Resources,
    pty: &mut pty::Pty,
    signals: &climon2_platform::terminal::Signals,
) -> Result<(), startup::Failure> {
    session::serve(options, &resources.listener, pty, signals)
        .map_err(|_| startup::Failure::own("session I/O failed"))?;
    tracing::info!("session ended");
    resources
        .cleanup()
        .map_err(|_| startup::Failure::own("cannot remove session files"))
}

fn ready(readiness: &mut ReadinessWriter) -> Result<(), startup::Failure> {
    readiness
        .send(&Readiness::Ready)
        .map_err(|_| startup::Failure::own("cannot report daemon readiness"))
}

fn io_error(message: &'static str) -> io::Error {
    io::Error::other(message)
}

/// Nonblocking I/O: `WouldBlock` is no progress yet (`None`), never a failure.
fn unless_would_block<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
        Err(error) => Err(error),
    }
}
