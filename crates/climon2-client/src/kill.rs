//! Kill mode (#100): no terminal and no attachment. The executable submits
//! one Kill through its own release's client library, which sends it on an
//! auxiliary hello-validated connection over daemon wire revision 1, and
//! waits for the definitive outcome. Success is a confirmed termination
//! alone, whatever the command's status.

use crate::{
    embedded::{Embedded, NativeError, session_instance},
    polling::interrupted,
};
use climon2_core::{SessionId, metadata::SessionMetadata, native};
use climon2_platform::terminal::Signals;
use std::{io, path::Path, ptr};

/// How long one wait for the library lasts before signals are checked again.
const POLL_MS: i32 = 25;

/// The Kill's answer: confirmed, or the library's error.
type Answer = Result<(), NativeError>;

/// Kills the session `metadata` (read from `path`) describes. A library that
/// is missing, of another release or faulty fails before any connection;
/// HUP, TERM or INT while waiting end it at once as `Interrupted`.
pub(crate) fn run(
    metadata: &SessionMetadata,
    path: &Path,
    home: &Path,
    signals: &Signals,
) -> io::Result<u32> {
    let instance = start(metadata, path, home)?;
    let answer = answer(&instance, signals)?;
    instance.finish();
    verdict(answer, &metadata.session_id)
}

/// An instance of this release's library for the session, with the Kill
/// submitted.
fn start(metadata: &SessionMetadata, path: &Path, home: &Path) -> io::Result<Embedded> {
    let instance = session_instance(metadata, path, home)?;
    // The first request of a fresh instance always finds room.
    instance.submit(&request())?;
    Ok(instance)
}

/// The only request: public ID 1, a Kill of the instance's session that
/// requires nothing beyond core support.
fn request() -> native::Request {
    native::Request {
        tag: native::KILL,
        reserved: 0,
        request_id: 1,
        args: native::RequestArgs {
            required: native::Tokens {
                data: ptr::null(),
                count: 0,
            },
        },
    }
}

/// Drains until the instance's stream ends, keeping the Kill's answer.
fn answer(instance: &Embedded, signals: &Signals) -> io::Result<Option<Answer>> {
    let mut answer = None;
    let mut ended = false;
    while !ended {
        interrupted(signals)?;
        let mut keep = |record: &native::Record| answer = answered(record).or(answer.take());
        ended = instance.turn(&mut keep, POLL_MS)?;
    }
    Ok(answer)
}

/// The Kill's Response, the only one a Kill-only instance delivers. The
/// library answers OK only for a termination with the command's result and
/// complete daemon cleanup.
fn answered(record: &native::Record) -> Option<Answer> {
    // SAFETY: a Response record's active member is its response, and its
    // error buffers live as long as the record.
    (record.tag == native::RESPONSE).then(|| unsafe {
        let response = &record.data.response;
        (response.outcome == native::OK)
            .then_some(())
            .ok_or_else(|| NativeError::copy(&response.error))
    })
}

/// Status 0 for a confirmed termination. Otherwise the library's
/// diagnostic, except that a Kill outliving `kill_ms`, the CLI's own
/// ten-second bound, or one never answered, is reported as the CLI reports
/// its bound.
fn verdict(answer: Option<Answer>, session: &SessionId) -> io::Result<u32> {
    match answer {
        Some(Ok(())) => Ok(0),
        Some(Err(error)) if error.code != "timeout" => Err(io::Error::other(error.message)),
        _ => Err(io::Error::other(format!(
            "termination of session {session} was not confirmed"
        ))),
    }
}
