//! The CLI on Windows so far: help, version and usage errors are answered
//! before this and need no home (#315); `list` runs as on Unix (#316), and so
//! does `exec --headless` (#317). Every other subcommand fails as the CLI's
//! own error, without touching the home, until its Windows seams exist.
use crate::{Error, Work};
use climon2_core::lifetime::Lifetime;
use std::process::Command;

pub(crate) fn logged_run(work: Work) -> Result<u32, Error> {
    match work {
        Work::List { json } => crate::logged(|home, _, _| crate::list(home, json)),
        Work::Exec(Lifetime::Headless, command) => crate::logged(|home, general, filter| {
            crate::exec::run(home, Lifetime::Headless, command, general, filter)
        }),
        Work::Exec(..) | Work::Handoff(_) => Err(unsupported()),
    }
}

fn unsupported() -> Error {
    Error::new(125, "this command is not supported on Windows yet")
}

/// Only an ordinary `exec` hands off to a client, and it is refused above.
pub(crate) fn exec_client(_client: Command) -> Error {
    unsupported()
}

/// Only an ordinary `exec` keeps a daemon it may have to stop, and it is
/// refused above; a headless session outlives the CLI at once.
pub(crate) fn stop_daemon(_pid: u32) {}
