//! Kill mode (#100): no terminal and no attachment. The executable submits
//! one Kill through its own release's client library, which sends it on an
//! auxiliary hello-validated connection over daemon wire revision 1, and
//! waits for the definitive outcome. Success is a confirmed termination
//! alone, whatever the command's status; a failure reads as it did before
//! #100.

use crate::{
    embedded::{Embedded, NativeError, Session, Stream, session_instance},
    polling::interrupted,
};
use climon2_core::{SessionId, metadata::SessionMetadata, native};
use climon2_platform::terminal::Signals;
use std::{io, path::Path, ptr, time::Duration};

/// How long one wait for the library lasts before signals are checked again.
const POLL: Duration = Duration::from_millis(25);

/// The Kill's answer: confirmed, or why not.
type Answer = Result<(), Failure>;

/// Why a Kill did not succeed, as kill mode reported it before #100.
enum Failure {
    /// Sent, then unanswered for `kill_ms`, or never answered at all.
    Unconfirmed,
    /// Refused unsent: the hello went unanswered within its deadline.
    Silent,
    /// The session daemon's connection could not be made, closed or failed.
    Lost,
    /// Any other error, with its message: the daemon's own for a daemon
    /// error.
    Error(String),
}

/// Kills the session `metadata` describes. A library that is missing, of
/// another release or faulty fails before any connection; HUP, TERM or INT
/// while waiting end it at once as `Interrupted`.
pub(crate) fn run(
    metadata: &SessionMetadata,
    session: &Session,
    home: &Path,
    signals: &Signals,
) -> io::Result<u32> {
    let instance = start(metadata, session, home)?;
    let answer = answer(&instance, signals)?;
    instance.finish();
    verdict(answer, &metadata.session_id)
}

/// An instance of this release's library for the session, with the Kill
/// submitted.
fn start(metadata: &SessionMetadata, session: &Session, home: &Path) -> io::Result<Embedded> {
    let instance = session_instance(metadata, session, home)?;
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
    let mut stream = Stream::Open;
    while stream == Stream::Open {
        interrupted(signals)?;
        let mut keep = |record: &native::Record| answer = answered(record).or(answer.take());
        stream = instance.turn(&mut keep, POLL)?;
    }
    Ok(answer)
}

/// The Kill's Response, the only one a Kill-only instance delivers.
fn answered(record: &native::Record) -> Option<Answer> {
    // SAFETY: a Response record's active member is its response, and its
    // error buffers live as long as the record.
    (record.tag == native::RESPONSE).then(|| unsafe { outcome(&record.data.response) })
}

/// The library answers OK only for a termination with the command's result
/// and complete daemon cleanup.
///
/// # Safety
///
/// `response`'s error buffers are valid.
unsafe fn outcome(response: &native::Response) -> Answer {
    // SAFETY: guaranteed by the caller.
    let error = || unsafe { NativeError::copy(&response.error) };
    (response.outcome == native::OK)
        .then_some(())
        .ok_or_else(|| failure(response.outcome, error()))
}

/// A Kill sent and left unanswered for `kill_ms` is indeterminate `timeout`;
/// one refused unsent on a silent hello is a rejected `timeout`. Both are the
/// library's, and closure or I/O failure is the connection's: the daemon's
/// own errors, which may also be `io_error`, keep their message.
fn failure(outcome: u32, error: NativeError) -> Failure {
    match (outcome, error.origin, error.code.as_str()) {
        (native::INDETERMINATE, native::ORIGIN_LIBRARY, "timeout") => Failure::Unconfirmed,
        (_, native::ORIGIN_LIBRARY, "timeout") => Failure::Silent,
        (_, native::ORIGIN_TRANSPORT, "io_error" | "truncated_frame") => Failure::Lost,
        _ => Failure::Error(error.message),
    }
}

/// Status 0 for a confirmed termination, otherwise the failure's diagnostic.
fn verdict(answer: Option<Answer>, session: &SessionId) -> io::Result<u32> {
    answer
        .unwrap_or(Err(Failure::Unconfirmed))
        .map(|()| 0)
        .map_err(|failure| failure.diagnostic(session))
}

impl Failure {
    /// Kill's diagnostic from before #100. A Kill outliving `kill_ms`, the
    /// CLI's own ten-second bound, reads as the CLI reports its bound; a lost
    /// connection is `UnexpectedEof`, which the client reports as
    /// `lost connection to session <id>` in either mode.
    fn diagnostic(self, session: &SessionId) -> io::Error {
        match self {
            Self::Unconfirmed => io::Error::other(format!(
                "termination of session {session} was not confirmed"
            )),
            Self::Silent => io::Error::other(format!("session {session} did not respond")),
            Self::Lost => io::ErrorKind::UnexpectedEof.into(),
            Self::Error(message) => io::Error::other(message),
        }
    }
}
