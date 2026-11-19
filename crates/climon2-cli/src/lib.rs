//! Interactive startup orchestration; binaries communicate only through contracts.

mod attach;
mod exec;
mod kill;
pub mod list;
mod monitor;

use climon2_core::{
    cli::{CliArgs, CommandArgs},
    layout::{ExecutableLayout, VersionBinary},
    lifetime::Lifetime,
    selector::Selector,
};
use climon2_platform::{
    home,
    logging::{self, LogFile},
};
use std::{ffi::OsString, io, path::Path, time::SystemTime};

/// How the CLI fails: the thin main prints it and exits with its code.
pub use climon2_core::failure::Failure as Error;

/// The install layout around this CLI: every sibling it starts, and every
/// client it execs, is found from it.
fn executable_layout() -> io::Result<ExecutableLayout> {
    home::executable_layout(VersionBinary::Cli)?.map_err(io::Error::other)
}

/// Arguments exclude argv[0]. Success is a final status, except successful
/// ordinary exec or attach, which replace this process with the metadata
/// version's client, and monitor, which replaces it with this version's monitor.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<u32, Error> {
    let arguments =
        CliArgs::parse(arguments).map_err(|error| Error::new(error.exit_code(), error))?;
    match work(arguments) {
        Ok(work) => logged_run(work),
        Err(text) => {
            println!("{text}");
            Ok(0)
        }
    }
}

/// The subcommands that need a home; help and version are answered here.
enum Work {
    Exec(Lifetime, CommandArgs),
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
        CliArgs::Exec { lifetime, command } => Ok(Work::Exec(lifetime, command)),
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

fn logged_run(work: Work) -> Result<u32, Error> {
    let home = home::canonical()?;
    let general = LogFile::general(&home)?;
    let filter = std::env::var("CLIMON2_LOG").ok();
    let dispatch = general
        .dispatch(filter.as_deref())
        .map_err(|error| Error::from(error).logged(general.path()))?;
    tracing::dispatcher::with_default(&dispatch, || {
        logging::prune(&home, SystemTime::now());
        command(work, &home, &general, filter.as_deref())
            .map_err(|error| error.logged(general.path()))
    })
}

fn command(work: Work, home: &Path, general: &LogFile, filter: Option<&str>) -> Result<u32, Error> {
    match work {
        Work::Exec(lifetime, command) => exec::run(home, lifetime, command, general, filter),
        Work::List { json } => list::run(home, json).map(|()| 0).map_err(Error::from),
        Work::Handoff(handoff) => handoff.run(home),
    }
}

impl Handoff {
    fn run(self, home: &Path) -> Result<u32, Error> {
        match self {
            Self::Attach(selector) => attach::run(home, selector),
            Self::Kill(selector) => kill::run(home, &selector),
            Self::Monitor => monitor::run(),
        }
    }
}
