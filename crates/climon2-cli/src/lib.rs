//! Interactive startup orchestration; binaries communicate only through contracts.

mod attach;
mod client;
mod exec;
mod kill;
pub mod list;
#[cfg(unix)]
#[path = "unix.rs"]
mod native;
#[cfg(windows)]
#[path = "windows.rs"]
mod native;
mod sibling;

use climon2_core::{
    cli::{CliArgs, CommandArgs},
    layout::{ExecutableLayout, VersionBinary},
    lifetime::Lifetime,
    relay::RelayArgs,
    selector::Selector,
    session::NestedStart,
};
use climon2_platform::{
    home,
    logging::{self, LogFile},
};
use std::{ffi::OsString, io, path::Path, time::SystemTime};

/// How the CLI fails: the thin main prints it and exits with its code.
pub use climon2_core::failure::Failure as Error;

/// Arguments exclude argv[0]. Success is a final status, except successful
/// ordinary exec or attach, which replace this process with the metadata
/// version's client, and monitor and relay, which replace it with this
/// version's monitor and relay.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<u32, Error> {
    let arguments =
        CliArgs::parse(arguments).map_err(|error| Error::new(error.exit_code(), error))?;
    match work(arguments) {
        Ok(work) => native::logged_run(work),
        Err(text) => {
            println!("{text}");
            Ok(0)
        }
    }
}

/// The subcommands that need a home; help and version are answered here.
enum Work {
    Exec(Lifetime, NestedStart, CommandArgs),
    List { json: bool },
    Handoff(Handoff),
}

/// Subcommands that another executable of a version folder carries out: the
/// client attaches or kills, the monitor monitors and the relay relays.
enum Handoff {
    Attach(Selector),
    Kill(Selector),
    Monitor,
    Relay(RelayArgs),
}

fn work(arguments: CliArgs) -> Result<Work, String> {
    match arguments {
        CliArgs::Exec {
            lifetime,
            nested,
            command,
        } => Ok(Work::Exec(lifetime, nested, command)),
        CliArgs::List { json } => Ok(Work::List { json }),
        other => handoff(other).map(Work::Handoff),
    }
}

/// The client's subcommands; the rest go to [`sibling`].
fn handoff(arguments: CliArgs) -> Result<Handoff, String> {
    match arguments {
        CliArgs::Attach(selector) => Ok(Handoff::Attach(selector)),
        CliArgs::Kill(selector) => Ok(Handoff::Kill(selector)),
        other => sibling(other),
    }
}

/// The subcommands this version's own monitor and relay carry out; the rest
/// only present something.
fn sibling(arguments: CliArgs) -> Result<Handoff, String> {
    match arguments {
        CliArgs::Monitor => Ok(Handoff::Monitor),
        CliArgs::Relay(arguments) => Ok(Handoff::Relay(arguments)),
        other => Err(presentation(&other)),
    }
}

fn presentation(arguments: &CliArgs) -> String {
    match arguments {
        CliArgs::Version => format!("climon2 {}", env!("CARGO_PKG_VERSION")),
        _ => climon2_core::cli::USAGE.to_owned(),
    }
}

/// Runs `command` inside the invocation's climon2 home, with its general log
/// as the default subscriber and the `CLIMON2_LOG` filter; a failure is also
/// logged there.
fn logged(
    command: impl FnOnce(&Path, &LogFile, Option<&str>) -> Result<u32, Error>,
) -> Result<u32, Error> {
    let home = home::canonical()?;
    let general = LogFile::general(&home)?;
    let filter = std::env::var("CLIMON2_LOG").ok();
    let dispatch = general
        .dispatch(filter.as_deref())
        .map_err(|error| Error::from(error).logged(general.path()))?;
    tracing::dispatcher::with_default(&dispatch, || {
        logging::prune(&home, SystemTime::now());
        command(&home, &general, filter.as_deref()).map_err(|error| error.logged(general.path()))
    })
}

/// `list`, which never needs a version's client.
fn list(home: &Path, json: bool) -> Result<u32, Error> {
    list::run(home, json).map(|()| 0).map_err(Error::from)
}

/// The install layout around this CLI: every sibling it starts, and every
/// client it execs, is found from it.
fn executable_layout() -> io::Result<ExecutableLayout> {
    home::executable_layout(VersionBinary::Cli)?.map_err(io::Error::other)
}
