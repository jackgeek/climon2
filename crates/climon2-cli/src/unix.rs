//! The subcommands, each run inside the invocation's climon2 home with its
//! general log.
use crate::{Error, Handoff, Work, attach, exec, kill, monitor};
use climon2_core::layout::{ExecutableLayout, VersionBinary};
use climon2_platform::{home, logging::LogFile};
use std::{io, path::Path};

/// The install layout around this CLI: every sibling it starts, and every
/// client it execs, is found from it.
pub(crate) fn executable_layout() -> io::Result<ExecutableLayout> {
    home::executable_layout(VersionBinary::Cli)?.map_err(io::Error::other)
}

pub(crate) fn logged_run(work: Work) -> Result<u32, Error> {
    crate::logged(|home, general, filter| command(work, home, general, filter))
}

fn command(work: Work, home: &Path, general: &LogFile, filter: Option<&str>) -> Result<u32, Error> {
    match work {
        Work::Exec(lifetime, command) => exec::run(home, lifetime, command, general, filter),
        Work::List { json } => crate::list(home, json),
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
