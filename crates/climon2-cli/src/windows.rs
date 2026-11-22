//! The CLI on Windows: help, version and usage errors are answered before
//! this and need no home (#315); `list` (#316), `exec` (#317, #321),
//! `attach` (#321) and `kill` (#318, #321) run as on Unix, except that each
//! hand-off to a client is spawn and wait (docs/for-humans/windows/lifecycle.md,
//! "Hand-offs"). The monitor fails as the CLI's own error, without touching
//! the home, until its Windows seams exist (#327).
use crate::{Error, Handoff, Work};
use climon2_platform::{daemon, handoff};
use std::process::Command;

pub(crate) fn logged_run(work: Work) -> Result<u32, Error> {
    match work {
        Work::List { json } => crate::logged(|home, _, _| crate::list(home, json)),
        Work::Exec(lifetime, command) => crate::logged(|home, general, filter| {
            crate::exec::run(home, lifetime, command, general, filter)
        }),
        Work::Handoff(handoff) => run_handoff(handoff),
    }
}

fn run_handoff(handoff: Handoff) -> Result<u32, Error> {
    match handoff {
        Handoff::Attach(selector) => crate::logged(|home, _, _| crate::attach::run(home, selector)),
        Handoff::Kill(selector) => crate::logged(|home, _, _| kill(home, &selector)),
        Handoff::Monitor => Err(Error::new(
            125,
            "this command is not supported on Windows yet",
        )),
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
