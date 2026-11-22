//! The CLI on Windows so far: help, version and usage errors are answered
//! before this and need no home (#315); `list` runs as on Unix (#316), and so
//! do `exec --headless` (#317) and `kill` (#318). Every other subcommand
//! fails as the CLI's own error, without touching the home, until its
//! Windows seams exist.
use crate::{Error, Handoff, Work};
use climon2_core::lifetime::Lifetime;
use std::process::Command;

pub(crate) fn logged_run(work: Work) -> Result<u32, Error> {
    match work {
        Work::List { json } => crate::logged(|home, _, _| crate::list(home, json)),
        Work::Exec(Lifetime::Headless, command) => crate::logged(|home, general, filter| {
            crate::exec::run(home, Lifetime::Headless, command, general, filter)
        }),
        Work::Handoff(Handoff::Kill(selector)) => {
            crate::logged(|home, _, _| crate::kill::run(home, &selector))
        }
        Work::Exec(..) | Work::Handoff(Handoff::Attach(_) | Handoff::Monitor) => Err(unsupported()),
    }
}

fn unsupported() -> Error {
    Error::new(125, "this command is not supported on Windows yet")
}

/// Ordinary `exec` and `attach` hand off to a client, and both are refused
/// above.
pub(crate) fn exec_client(_client: Command) -> Error {
    unsupported()
}

/// Only an ordinary `exec` keeps a daemon it may have to stop, and it is
/// refused above; a headless session outlives the CLI at once.
pub(crate) fn stop_daemon(_pid: u32) {}
