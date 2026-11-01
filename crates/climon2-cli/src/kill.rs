//! `climon2 kill`: the target daemon owns shutdown; this only dispatches and waits.
use crate::{Error, attach, list};
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
    let session = attach::select(&sessions, selector)?;
    require_kill(session)?;
    let child = attach::client(home, &session.metadata, StartupEnvelope::Kill)?
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()?;
    confirm(child, deadline, &session.metadata.session_id)
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
