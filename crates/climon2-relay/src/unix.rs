//! The relay on macOS and Linux: its start, its screen and how it stops.
use crate::{Error, host, keys, screen, start};
use climon2_core::relay::RelayArgs;
use climon2_platform::{home, logging::LogFile};
use std::{ffi::OsString, io, path::Path};

/// Runs a relay until it is stopped. Arguments exclude argv[0]: the CLI
/// passes on what `c2 relay` was given. Returns the exit status of a clean
/// stop: 0 from the menu, 128 + signal for a termination signal, 129 for a
/// terminal that closed. A usage error fails with 2, before the climon2 home
/// is touched; any other failure with the climon2 error code, naming the
/// relay's log.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<u32, Error> {
    let arguments =
        RelayArgs::parse(arguments).map_err(|error| Error::new(error.exit_code(), error))?;
    let home = home::canonical()?;
    let log = LogFile::relay(&home)?;
    let path = log.path().to_owned();
    let filter = std::env::var("CLIMON2_LOG").ok();
    let dispatch = log
        .dispatch(filter.as_deref())
        .map_err(|error| Error::from(error).logged(&path))?;
    tracing::dispatcher::with_default(&dispatch, || relay(arguments, &home))
        .map_err(|error| Error::from(error).logged(&path))
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
fn serve(startup: &start::Startup) -> io::Result<host::Ending> {
    use host::{Ending, Event, Host};

    let (mut host, size) = Host::enter()?;
    let mut screen = screen::Screen::new(size)?;
    let view = screen::View {
        address: startup.address(),
    };
    screen.draw(&view)?;
    loop {
        match host.next()? {
            Event::End(ending) => return Ok(ending),
            Event::Resize(size) => {
                screen.resize(size)?;
                screen.draw(&view)?;
            }
            Event::Input(input) if keys::stops(input) => return Ok(Ending::Stopped),
            Event::Input(_) | Event::Idle => {}
        }
    }
}
