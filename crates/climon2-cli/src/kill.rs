//! `c2 kill`: the target daemon owns shutdown; this only dispatches and waits.
use crate::{Error, client, list};
use climon2_core::{SessionId, selector::Selector, startup_envelope::StartupEnvelope};
use std::{
    process::{Child, ExitStatus, Stdio},
    time::{Duration, Instant},
};

/// One monotonic deadline from selection to confirmation (plan 1.7).
const DEADLINE: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(10);

pub fn run(home: &std::path::Path, selector: &Selector) -> Result<u32, Error> {
    let deadline = Instant::now() + DEADLINE;
    let sessions = list::discover_live(home)?;
    let session = client::select(&sessions, selector)?;
    let child = start(home, session)?;
    confirm(child, deadline, &session.metadata.session_id)
}

fn start(home: &std::path::Path, session: &list::LiveSession) -> Result<Child, Error> {
    require_kill(session)?;
    let child = client::client(home, &session.metadata, StartupEnvelope::Kill)?
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()?;
    Ok(child)
}

fn require_kill(session: &list::LiveSession) -> Result<(), Error> {
    let capabilities = session.extensions.capabilities.valid();
    capabilities
        .is_some_and(|capabilities| capabilities.supports_kill())
        .then_some(())
        .ok_or_else(|| {
            Error::new(
                125,
                format!(
                    "session {} does not support kill",
                    session.metadata.session_id
                ),
            )
        })
}

/// Only the helper this CLI spawned is ever stopped; an accepted shutdown continues.
fn confirm(mut child: Child, deadline: Instant, id: &SessionId) -> Result<u32, Error> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(code(status));
        }
        if deadline.checked_duration_since(Instant::now()).is_none() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::new(
                125,
                format!("termination of session {id} was not confirmed"),
            ));
        }
        std::thread::sleep(POLL);
    }
}

/// The helper already reported its own failure; a signalled helper confirmed nothing.
fn code(status: ExitStatus) -> u32 {
    status.code().map_or(125, |code| code as u32)
}

// `confirm` reached through `c2 kill` races the helper's own deadline: the helper
// usually reports first, so its deadline branch ran only in some runs and its
// coverage, and so its CRAP score, changed from run to run. These drive it directly.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;

    fn id() -> SessionId {
        SessionId::from_bytes([0x11; 16])
    }

    fn shell(script: &str) -> Child {
        Command::new("/bin/sh")
            .args(["-c", script])
            .spawn()
            .unwrap()
    }

    #[test]
    fn a_helper_that_exits_gives_its_status() {
        let deadline = Instant::now() + DEADLINE;
        assert_eq!(confirm(shell("exit 3"), deadline, &id()).unwrap(), 3);
    }

    #[test]
    fn a_signalled_helper_confirmed_nothing() {
        let deadline = Instant::now() + DEADLINE;
        assert_eq!(confirm(shell("kill -9 $$"), deadline, &id()).unwrap(), 125);
    }

    #[test]
    fn a_helper_still_running_at_the_deadline_is_stopped_and_reaped() {
        let child = shell("exec sleep 30");
        let pid = child.id() as libc::pid_t;
        let started = Instant::now();
        let error = confirm(child, started + Duration::from_millis(50), &id()).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(error.code, 125);
        assert_eq!(
            error.message,
            format!("termination of session {} was not confirmed", id())
        );
        // Reaped: no process, not even a zombie, has its pid any more.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }
}
