//! Kill mode (#100): no terminal and no attachment. The executable submits
//! one Kill through its own release's client library, which sends it on an
//! auxiliary hello-validated connection over daemon wire revision 1, and
//! waits for the definitive outcome. Success is a confirmed termination
//! alone, whatever the command's status; a failure reads as it did before
//! #100.

use crate::embedded::{Embedded, NativeError, Startup, Stream, own_library};
use climon2_core::{SessionId, diagnostics::default_filter, metadata::SessionMetadata, native};
use climon2_platform::{logging::library_log_name, terminal::Signals};
use std::{io, path::Path, ptr, time::SystemTime};

/// How long one wait for the library lasts before signals are checked again.
const POLL_MS: i32 = 25;

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

/// What kill mode tells the library about the session besides its metadata.
pub(crate) struct Session<'a> {
    /// Where the metadata was read.
    pub(crate) metadata: &'a Path,
    /// The client's own session log, beside which the library writes its own.
    pub(crate) log: &'a Path,
    /// The client's `CLIMON2_LOG` filter, if one was given.
    pub(crate) filter: Option<&'a str>,
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
    let (library, release) = own_library()?;
    let socket = Path::new(metadata.socket_path.as_str());
    let log = library_log_name(SystemTime::now(), std::process::id())?;
    let startup = Startup {
        session: metadata.session_id.bytes(),
        release,
        metadata: session.metadata.to_owned(),
        home: home.to_owned(),
        runtime: socket.parent().unwrap_or(socket).to_owned(),
        settings: native::Settings::default(),
        log: session.log.with_file_name(log),
        filter: session.filter.unwrap_or(default_filter()).to_owned(),
    };
    let instance = Embedded::start(library, startup)?;
    instance.submit(&request()).map(|()| instance)
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
        stream = instance.turn(&mut keep, POLL_MS)?;
    }
    Ok(answer)
}

fn interrupted(signals: &Signals) -> io::Result<()> {
    signals
        .termination_signal()
        .map_or(Ok(()), |_| Err(io::ErrorKind::Interrupted.into()))
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
