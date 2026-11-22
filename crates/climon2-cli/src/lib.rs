//! Interactive startup orchestration; binaries communicate only through contracts.

#[cfg(unix)]
mod attach;
#[cfg(unix)]
mod exec;
#[cfg(unix)]
mod kill;
#[cfg(unix)]
pub mod list;
#[cfg(unix)]
mod monitor;
#[cfg(unix)]
#[path = "unix.rs"]
mod native;
#[cfg(windows)]
#[path = "windows.rs"]
mod native;

use climon2_core::{
    cli::{CliArgs, CommandArgs},
    lifetime::Lifetime,
    selector::Selector,
};
use std::ffi::OsString;

/// How the CLI fails: the thin main prints it and exits with its code.
pub use climon2_core::failure::Failure as Error;

/// Arguments exclude argv[0]. Success is a final status, except successful
/// ordinary exec or attach, which replace this process with the metadata
/// version's client, and monitor, which replaces it with this version's monitor.
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
/// Only Unix runs them so far, so elsewhere nothing reads their arguments.
#[cfg_attr(
    not(unix),
    expect(dead_code, reason = "no subcommand runs on Windows yet")
)]
enum Work {
    Exec(Lifetime, CommandArgs),
    List { json: bool },
    Handoff(Handoff),
}

/// Subcommands that another executable of a version folder carries out: the
/// client attaches or kills, and the monitor monitors.
#[cfg_attr(
    not(unix),
    expect(dead_code, reason = "no subcommand runs on Windows yet")
)]
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
