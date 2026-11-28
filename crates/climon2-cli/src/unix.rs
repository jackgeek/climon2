//! The subcommands, each run inside the invocation's climon2 home with its
//! general log.
use crate::{Error, Handoff, Work, attach, exec, kill, sibling};
use climon2_platform::{
    logging::LogFile,
    process::{self, Signal},
};
use std::{
    os::unix::process::CommandExt,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

pub(crate) fn logged_run(work: Work) -> Result<u32, Error> {
    crate::logged(|home, general, filter| command(work, home, general, filter))
}

fn command(work: Work, home: &Path, general: &LogFile, filter: Option<&str>) -> Result<u32, Error> {
    match work {
        Work::Exec(lifetime, nested, command) => {
            exec::run(home, lifetime, nested, command, general, filter)
        }
        Work::List { json } => crate::list(home, json),
        Work::Handoff(handoff) => handoff.run(home),
    }
}

impl Handoff {
    fn run(self, home: &Path) -> Result<u32, Error> {
        match self {
            Self::Attach(selector) => attach::run(home, selector),
            Self::Kill(selector) => kill::run(home, &selector),
            Self::Monitor => sibling::monitor(),
            Self::Relay(arguments) => sibling::relay(&arguments),
        }
    }
}

/// Replaces this process with `client`; returns only when that fails.
pub(crate) fn exec_client(mut client: Command) -> Error {
    Error::from(client.exec())
}

/// Ends a daemon this CLI started whose session must not outlive it.
pub(crate) fn stop_daemon(pid: u32) {
    let _ = process::signal_group(pid, Signal::Terminate);
    let deadline = Instant::now() + Duration::from_secs(6);
    while deadline.checked_duration_since(Instant::now()).is_some() {
        match process::try_wait(pid) {
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            _ => return,
        }
    }
    let _ = process::signal_group(pid, Signal::Kill);
    let _ = process::wait(pid);
}
