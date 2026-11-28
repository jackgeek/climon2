//! The CLI on Windows: help, version and usage errors are answered before
//! this and need no home (#315); `list` (#316), `exec` (#317, #321),
//! `attach` (#321) and `kill` (#318, #321) run as on Unix, except that each
//! hand-off to a client is spawn and wait (docs/for-humans/windows/lifecycle.md,
//! "Hand-offs"), and so is the hand-off to the monitor (#327). A batch file's
//! argument that `cmd.exe` cannot represent is a usage error, answered before
//! the home is touched (#322).
use crate::{Error, Handoff, Work};
use climon2_core::cli::CommandArgs;
use climon2_platform::{
    command::{self, Refusal},
    daemon, handoff,
};
use std::process::Command;

pub(crate) fn logged_run(work: Work) -> Result<u32, Error> {
    match work {
        Work::List { json } => crate::logged(|home, _, _| crate::list(home, json)),
        Work::Exec(lifetime, nested, command) => {
            representable(&command)?;
            crate::logged(|home, general, filter| {
                crate::exec::run(home, lifetime, nested, command, general, filter)
            })
        }
        Work::Handoff(handoff) => run_handoff(handoff),
    }
}

/// Resolves `command` as the daemon will; only a refused batch argument
/// stops `exec` here. The daemon reports a command it cannot find or start.
fn representable(command: &CommandArgs) -> Result<(), Error> {
    let cwd = std::env::current_dir().unwrap_or_default();
    match command::launch(&command.command, &command.args, &cwd) {
        Err(Refusal::Unrepresentable) => Err(Error::new(
            2,
            "cmd.exe cannot pass these arguments to a batch file unchanged",
        )),
        _ => Ok(()),
    }
}

fn run_handoff(handoff: Handoff) -> Result<u32, Error> {
    match handoff {
        Handoff::Attach(selector) => crate::logged(|home, _, _| crate::attach::run(home, selector)),
        Handoff::Kill(selector) => crate::logged(|home, _, _| kill(home, &selector)),
        Handoff::Monitor => crate::logged(|_, _, _| crate::sibling::monitor()),
        Handoff::Relay(arguments) => crate::logged(|_, _, _| crate::sibling::relay(&arguments)),
    }
}

/// Kill waits for its client as exec and attach do.
fn kill(home: &std::path::Path, selector: &climon2_core::selector::Selector) -> Result<u32, Error> {
    handoff::wait_through_console_events()?;
    crate::kill::run(home, selector)
}

/// Runs the client, waits for it and exits with its status; returns only
/// when it cannot.
pub(crate) fn exec_client(client: Command) -> Error {
    Error::from(handoff::replace(client))
}

/// Ends a daemon this CLI started whose session must not outlive it.
pub(crate) fn stop_daemon(pid: u32) {
    daemon::stop(pid);
}
