//! The CLI on Windows so far (#315): help, version and usage errors are
//! answered before this and need no home. Every subcommand fails as the CLI's
//! own error until its Windows seams exist.
use crate::{Error, Work};

pub(crate) fn logged_run(_work: Work) -> Result<u32, Error> {
    Err(Error::new(
        125,
        "this command is not supported on Windows yet",
    ))
}
