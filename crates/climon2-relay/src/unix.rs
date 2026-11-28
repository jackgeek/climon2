//! The relay on macOS and Linux: its start, its screen and how it stops.
use crate::{
    Error,
    host::{Ending, Event, Host},
    keys, screen, start,
};
use climon2_core::{cli::UsageError, relay::RelayArgs};
use climon2_platform::{home, logging::LogFile};
use std::{ffi::OsString, io, path::Path};

/// Runs a relay until it is stopped. Arguments exclude argv[0]: the CLI
/// passes on what `c2 relay` was given. Returns the exit status of a clean
/// stop: 0 from the menu, 128 + signal for a termination signal, 129 for a
/// terminal that closed. A usage error fails with 2, before the climon2 home
/// is touched; any other failure with the climon2 error code, naming the
/// relay's log.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<u32, Error> {
    let arguments = RelayArgs::parse(arguments).map_err(usage)?;
    let home = home::canonical()?;
    logged(&LogFile::relay(&home)?, || relay(arguments, &home))
}

fn usage(error: UsageError) -> Error {
    Error::new(error.exit_code(), error)
}

/// Runs `work` with `log` as the default subscriber and the `CLIMON2_LOG`
/// filter. A failure names the log.
fn logged(log: &LogFile, work: impl FnOnce() -> io::Result<u32>) -> Result<u32, Error> {
    let failed = |error: io::Error| Error::from(error).logged(log.path());
    let filter = std::env::var("CLIMON2_LOG").ok();
    let dispatch = log.dispatch(filter.as_deref()).map_err(failed)?;
    tracing::dispatcher::with_default(&dispatch, work).map_err(failed)
}

/// Starts, serves until an ending, and records how the run went: the
/// terminal may be gone, so a failure is logged as well as returned for main
/// to print.
fn relay(arguments: RelayArgs, home: &Path) -> io::Result<u32> {
    let served = start::Startup::read(arguments, home).and_then(|startup| {
        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            listen = %startup.address(),
            "relay started"
        );
        serve(&startup)
    });
    let status = served
        .inspect_err(|error| tracing::error!(%error, "relay failed"))?
        .status();
    tracing::info!(status, "relay stopped");
    Ok(status)
}

/// Owns the terminal until an ending. The terminal is released when this
/// returns, before any diagnostic is printed, and the listener and the home's
/// lock go with `startup` after it.
fn serve(startup: &start::Startup) -> io::Result<Ending> {
    let (mut host, size) = Host::enter()?;
    let view = screen::View {
        address: startup.address(),
    };
    let mut screen = screen::Screen::drawn(size, &view)?;
    until_ending(|| step(&mut host, &mut screen, &view))
}

/// Takes `step` until one ends the relay.
fn until_ending(mut step: impl FnMut() -> io::Result<Option<Ending>>) -> io::Result<Ending> {
    loop {
        if let Some(ending) = step()? {
            return Ok(ending);
        }
    }
}

/// Acts on the terminal's next event.
fn step(
    host: &mut Host,
    screen: &mut screen::Screen,
    view: &screen::View,
) -> io::Result<Option<Ending>> {
    let event = host.next()?;
    act(event, screen, view)
}

/// An ending ends the relay, a resize draws `view` again, and only the stop
/// key among the input does anything.
fn act(
    event: Event<'_>,
    screen: &mut screen::Screen,
    view: &screen::View,
) -> io::Result<Option<Ending>> {
    match event {
        Event::End(ending) => Ok(Some(ending)),
        Event::Resize(size) => screen.redraw(size, view).map(|()| None),
        other => Ok(stopped(&other)),
    }
}

fn stopped(event: &Event<'_>) -> Option<Ending> {
    match event {
        Event::Input(input) if keys::stops(input) => Some(Ending::Stopped),
        _ => None,
    }
}
