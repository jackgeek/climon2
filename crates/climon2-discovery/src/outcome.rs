//! What one probe proves about a session with valid metadata, and the opt-in
//! cleanup of stale metadata.
use crate::{StaleMetadata, hello};
use climon2_core::{
    hello::{HelloExtensions, SessionState},
    metadata::SessionMetadata,
    protocol::HELLO_TIMEOUT,
};
use climon2_platform::{connection, process};
use std::{fs, io, path::Path, time::Instant};

/// A session whose daemon answered the frozen hello with this session's ID.
#[derive(Clone, Debug)]
pub struct LiveSession {
    pub metadata: SessionMetadata,
    pub extensions: HelloExtensions,
}

impl LiveSession {
    /// The advertised state, only when `state-v1` is advertised with a valid snapshot.
    pub fn state(&self) -> Option<SessionState> {
        let capabilities = self.extensions.capabilities.valid()?;
        let state = self.extensions.state.valid().copied();
        state.filter(|_| capabilities.supports_state())
    }
}

/// What one probe proved about a session with valid metadata.
#[derive(Debug)]
pub enum Outcome {
    /// Answered the frozen hello as this session.
    Live(LiveSession),
    /// Uncertain or unavailable: neither proven live nor proven stale, so never cleaned up.
    Uncertain(SessionMetadata, Uncertainty),
    /// Definitively stale: the socket is missing or refuses, and the recorded daemon
    /// process is provably gone.
    Stale(SessionMetadata, Cleanup),
}

/// Why a probe proved neither liveness nor staleness.
#[derive(Debug)]
pub enum Uncertainty {
    /// Something answered, but not with this session's hello.
    Mismatched,
    /// The hello exchange failed or missed its deadline.
    Unanswered(io::Error),
    /// Connecting failed in a way that proves nothing about the daemon.
    Unreachable(io::Error),
    /// The socket is missing or refuses, but the recorded daemon process still runs.
    DaemonAlive,
    /// The socket is missing or refuses, and the daemon's identity cannot be checked.
    IdentityUnknown(io::Error),
}

/// What became of a stale session's metadata file and socket.
#[derive(Debug)]
pub enum Cleanup {
    /// Kept, because the caller did not opt in to removal.
    Retained,
    /// Both are gone.
    Removed,
    /// Removal failed; whatever remains is kept.
    Failed(io::Error),
}

impl Outcome {
    pub fn metadata(&self) -> &SessionMetadata {
        match self {
            Self::Live(session) => &session.metadata,
            Self::Uncertain(metadata, _) | Self::Stale(metadata, _) => metadata,
        }
    }

    /// The live session, if the probe proved one.
    pub fn live(self) -> Option<LiveSession> {
        match self {
            Self::Live(session) => Some(session),
            _ => None,
        }
    }
}

/// One probe thread per session, so a slow daemon delays only its own outcome.
pub(crate) fn probe_all(
    home: &Path,
    sessions: Vec<SessionMetadata>,
    stale: StaleMetadata,
) -> io::Result<Vec<Outcome>> {
    std::thread::scope(|scope| {
        let workers: Vec<_> = sessions
            .into_iter()
            .map(|metadata| scope.spawn(move || probe(home, metadata, stale)))
            .collect();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .map_err(|_| io::Error::other("session probe panicked"))
            })
            .collect()
    })
}

fn probe(home: &Path, metadata: SessionMetadata, stale: StaleMetadata) -> Outcome {
    let deadline = Instant::now() + HELLO_TIMEOUT;
    match connection::connect(Path::new(metadata.socket_path.as_str()), HELLO_TIMEOUT) {
        Ok(stream) => answered(hello::hello(stream, deadline, &metadata), metadata),
        Err(error) => connection_failed(home, metadata, error, stale),
    }
}

fn answered(result: io::Result<Option<HelloExtensions>>, metadata: SessionMetadata) -> Outcome {
    match result {
        Ok(Some(extensions)) => Outcome::Live(LiveSession {
            metadata,
            extensions,
        }),
        Ok(None) => Outcome::Uncertain(metadata, Uncertainty::Mismatched),
        Err(error) => Outcome::Uncertain(metadata, Uncertainty::Unanswered(error)),
    }
}

/// Only a missing or refusing socket can begin to prove a session stale.
fn connection_failed(
    home: &Path,
    metadata: SessionMetadata,
    error: io::Error,
    stale: StaleMetadata,
) -> Outcome {
    if matches!(
        error.raw_os_error(),
        Some(libc::ENOENT | libc::ECONNREFUSED)
    ) {
        identity(home, metadata, stale)
    } else {
        Outcome::Uncertain(metadata, Uncertainty::Unreachable(error))
    }
}

/// Only a proven dead daemon identity makes such a session stale.
fn identity(home: &Path, metadata: SessionMetadata, stale: StaleMetadata) -> Outcome {
    match process::is_same_process(metadata.daemon_pid, metadata.daemon_start_time) {
        Ok(false) => {
            let cleanup = clean_up(home, &metadata, stale);
            Outcome::Stale(metadata, cleanup)
        }
        Ok(true) => Outcome::Uncertain(metadata, Uncertainty::DaemonAlive),
        Err(error) => Outcome::Uncertain(metadata, Uncertainty::IdentityUnknown(error)),
    }
}

fn clean_up(home: &Path, metadata: &SessionMetadata, stale: StaleMetadata) -> Cleanup {
    match stale {
        StaleMetadata::Retain => Cleanup::Retained,
        StaleMetadata::Remove => remove(home, metadata),
    }
}

fn remove(home: &Path, metadata: &SessionMetadata) -> Cleanup {
    let path = home
        .join("sessions")
        .join(format!("{}.pty", metadata.session_id));
    let result = remove_if_present(&path)
        .and_then(|()| remove_if_present(Path::new(metadata.socket_path.as_str())));
    match result {
        Ok(()) => Cleanup::Removed,
        Err(error) => Cleanup::Failed(error),
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
