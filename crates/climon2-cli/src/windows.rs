//! The CLI on Windows so far: help, version and usage errors are answered
//! before this and need no home (#315); `list` runs as on Unix (#316). Every
//! other subcommand fails as the CLI's own error, without touching the home,
//! until its Windows seams exist.
use crate::{Error, Work};

pub(crate) fn logged_run(work: Work) -> Result<u32, Error> {
    match work {
        Work::List { json } => crate::logged(|home, _, _| crate::list(home, json)),
        Work::Exec(..) | Work::Handoff(_) => Err(Error::new(
            125,
            "this command is not supported on Windows yet",
        )),
    }
}
