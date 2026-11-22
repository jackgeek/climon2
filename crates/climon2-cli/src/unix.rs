//! The subcommands, each run inside the invocation's climon2 home with its
//! general log.
use crate::{Error, Handoff, Work, attach, exec, kill, list, monitor};
use climon2_core::layout::{ExecutableLayout, VersionBinary};
use climon2_platform::{
    home,
    logging::{self, LogFile},
};
use std::{io, path::Path, time::SystemTime};

/// The install layout around this CLI: every sibling it starts, and every
/// client it execs, is found from it.
pub(crate) fn executable_layout() -> io::Result<ExecutableLayout> {
    home::executable_layout(VersionBinary::Cli)?.map_err(io::Error::other)
}

pub(crate) fn logged_run(work: Work) -> Result<u32, Error> {
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
