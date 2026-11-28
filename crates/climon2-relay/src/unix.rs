//! The relay on macOS and Linux: its start, its screen and how it stops.
use crate::{
    Error,
    desk::Desk,
    host::{Ending, Event, Host},
    keys::{self, Key},
    links, screen, start,
};
use climon2_core::{cli::UsageError, relay::RelayArgs};
use climon2_platform::{home, logging::LogFile};
use std::{ffi::OsString, io, net::SocketAddr, path::Path};

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
        serve(startup)
    });
    let status = served
        .inspect_err(|error| tracing::error!(%error, "relay failed"))?
        .status();
    tracing::info!(status, "relay stopped");
    Ok(status)
}

/// Owns the terminal until an ending, accepting connections meanwhile. The
/// terminal is released when this returns, before any diagnostic is
/// printed, and the listener and the home's lock go with the relay after
/// it.
fn serve(startup: start::Startup) -> io::Result<Ending> {
    let serving = startup.serve();
    let (mut host, size) = Host::enter()?;
    let mut relay = Relay {
        address: serving.address,
        desk: Desk::new(serving.links.invitations().clone()),
    };
    let mut shown = relay.view();
    let mut screen = screen::Screen::drawn(size, &shown)?;
    until_ending(|| step(&mut host, &mut screen, &mut relay, &mut shown))
}

/// What the screen is drawn from.
struct Relay {
    address: SocketAddr,
    desk: Desk,
}

impl Relay {
    fn view(&self) -> screen::View {
        screen::View {
            address: self.address,
            invitation: self.desk.shown(links::now()),
        }
    }
}

/// Takes `step` until one ends the relay.
fn until_ending(mut step: impl FnMut() -> io::Result<Option<Ending>>) -> io::Result<Ending> {
    loop {
        if let Some(ending) = step()? {
            return Ok(ending);
        }
    }
}

/// Acts on the terminal's next event, then draws the screen again if what
/// it shows has changed.
fn step(
    host: &mut Host,
    screen: &mut screen::Screen,
    relay: &mut Relay,
    shown: &mut screen::View,
) -> io::Result<Option<Ending>> {
    let event = host.next()?;
    let ending = act(event, screen, relay, shown)?;
    let view = relay.view();
    if view != *shown {
        screen.draw(&view)?;
        *shown = view;
    }
    Ok(ending)
}

/// An ending ends the relay, a resize draws the screen again, the pair key
/// mints an invitation and the stop key stops.
fn act(
    event: Event<'_>,
    screen: &mut screen::Screen,
    relay: &mut Relay,
    shown: &screen::View,
) -> io::Result<Option<Ending>> {
    match event {
        Event::End(ending) => Ok(Some(ending)),
        Event::Resize(size) => screen.redraw(size, shown).map(|()| None),
        Event::Input(input) => Ok(pressed(keys::action(input), relay)),
        Event::Idle => Ok(None),
    }
}

/// What an action key does.
fn pressed(key: Option<Key>, relay: &mut Relay) -> Option<Ending> {
    match key {
        Some(Key::Stop) => Some(Ending::Stopped),
        Some(Key::Pair) => {
            relay.desk.mint(links::now());
            None
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::Shown;
    use climon2_link::{Introduction, Invitations};
    use std::net::Ipv4Addr;

    fn relay() -> Relay {
        let introduction = Introduction::new([7; 32], "buildbox").unwrap();
        Relay {
            address: SocketAddr::from((Ipv4Addr::LOCALHOST, 7222)),
            desk: Desk::new(Invitations::new(introduction)),
        }
    }

    #[test]
    fn the_view_shows_the_address_and_the_invitation() {
        let relay = relay();
        let view = relay.view();
        assert_eq!(view.address, relay.address);
        assert_eq!(view.invitation, Shown::Nothing);
    }

    #[test]
    fn the_pair_key_mints_and_the_stop_key_stops() {
        let mut relay = relay();
        assert_eq!(pressed(None, &mut relay), None);
        assert_eq!(relay.view().invitation, Shown::Nothing);
        assert_eq!(pressed(Some(Key::Pair), &mut relay), None);
        assert!(matches!(relay.view().invitation, Shown::Live { .. }));
        assert_eq!(pressed(Some(Key::Stop), &mut relay), Some(Ending::Stopped));
    }
}
