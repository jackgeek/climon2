//! Interactive startup orchestration; binaries communicate only through contracts.

mod attach;
mod client;
mod exec;
mod kill;
pub mod list;
mod monitor;
#[cfg(unix)]
#[path = "unix.rs"]
mod native;
#[cfg(windows)]
#[path = "windows.rs"]
mod native;
mod ownership;

use climon2_core::{
    cli::{CliArgs, CommandArgs},
    layout::{ExecutableLayout, VersionBinary},
    lifetime::Lifetime,
    owner::Refusal,
    selector::Selector,
    session::NestedStart,
};
use climon2_platform::{
    home,
    logging::{self, LogFile},
    owner,
};
use std::{ffi::OsString, io, path::Path, time::SystemTime};

/// How the CLI fails: the thin main prints it and exits with its code.
pub use climon2_core::failure::Failure as Error;

/// Arguments exclude argv[0]. Success is a final status, except successful
/// ordinary exec or attach, which replace this process with the metadata
/// version's client, and monitor, which replaces it with this version's monitor.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<u32, Error> {
    let arguments =
        CliArgs::parse(arguments).map_err(|error| Error::new(error.exit_code(), error))?;
    let (arguments, force) = arguments.unforced();
    if let Some(action) = home_command(&arguments) {
        return unowned(|home, _, _| ownership::run(home, action));
    }
    match work(arguments) {
        Ok(work) => native::logged_run(work, force),
        Err(text) => {
            println!("{text}");
            Ok(0)
        }
    }
}

/// `home` and `home release`, which use the home without checking its owner.
fn home_command(arguments: &CliArgs) -> Option<ownership::Action> {
    match arguments {
        CliArgs::Home => Some(ownership::Action::Show),
        CliArgs::HomeRelease { force } => Some(ownership::Action::Release { force: *force }),
        _ => None,
    }
}

/// The subcommands that need a home this host owns; help and version are
/// answered here.
enum Work {
    Exec(Lifetime, NestedStart, CommandArgs),
    List { json: bool },
    Handoff(Handoff),
}

/// Subcommands that another executable of a version folder carries out: the
/// client attaches or kills, and the monitor monitors.
enum Handoff {
    Attach(Selector),
    Kill(Selector),
    Monitor,
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

fn handoff(arguments: CliArgs) -> Result<Handoff, String> {
    match arguments {
        CliArgs::Attach(selector) => Ok(Handoff::Attach(selector)),
        CliArgs::Kill(selector) => Ok(Handoff::Kill(selector)),
        CliArgs::Monitor => Ok(Handoff::Monitor),
        other => Err(presentation(&other)),
    }
}

fn presentation(arguments: &CliArgs) -> String {
    match arguments {
        CliArgs::Version => format!("climon2 {}", env!("CARGO_PKG_VERSION")),
        _ => climon2_core::cli::USAGE.to_owned(),
    }
}

/// Runs `command` inside the invocation's climon2 home, once the home is
/// known to be this host's (ADR 0011), or, with `force`, also when another host
/// owns it or its record cannot be read, with a warning.
fn logged(
    force: bool,
    command: impl FnOnce(&Path, &LogFile, Option<&str>) -> Result<u32, Error>,
) -> Result<u32, Error> {
    in_home(|home| forced(home, force), command)
}

/// The owner check, forgiving what `--force` skips.
fn forced(home: &Path, force: bool) -> Result<(), Refusal> {
    if let Some(refusal) = climon2_core::owner::forgive(owner::check(home), force)? {
        tracing::warn!(%refusal, "--force uses this climon2 home without claiming it");
        eprintln!("{}", climon2_core::owner::forced_warning(&refusal));
    }
    Ok(())
}

/// Runs `command` inside the invocation's climon2 home whoever owns it: only
/// `c2 home` and `c2 home release`, which diagnose and clear a refusal.
fn unowned(
    command: impl FnOnce(&Path, &LogFile, Option<&str>) -> Result<u32, Error>,
) -> Result<u32, Error> {
    in_home(|_| Ok(()), command)
}

/// Runs `command` inside the invocation's climon2 home, with its general log
/// as the default subscriber and the `CLIMON2_LOG` filter, after `check`; a
/// failure is also logged there.
fn in_home(
    check: impl FnOnce(&Path) -> Result<(), Refusal>,
    command: impl FnOnce(&Path, &LogFile, Option<&str>) -> Result<u32, Error>,
) -> Result<u32, Error> {
    let home = home::canonical()?;
    let general = LogFile::general(&home)?;
    let filter = std::env::var("CLIMON2_LOG").ok();
    let dispatch = general
        .dispatch(filter.as_deref())
        .map_err(|error| Error::from(error).logged(general.path()))?;
    tracing::dispatcher::with_default(&dispatch, || {
        // Before anything else touches the home.
        check(&home).map_err(|refusal| Error::new(125, refusal).logged(general.path()))?;
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
