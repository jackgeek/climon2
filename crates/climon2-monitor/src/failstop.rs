//! The monitor's bounded fail-stop (spec #73, "Lifecycle and exit"; #118).
//! When cleanup cannot be confirmed, the monitor restores the host, sends
//! the root clear and prints its diagnostic within
//! `deadlines.final_host_ms`, then ends the whole process with the climon2
//! error code. It never kills a thread or unloads a library: `_exit` ends
//! the process without running exit handlers or library destructors, so
//! no client code still running is torn down under it, and nothing claims a
//! join or detach that did not happen.
use climon2_core::exit::CLIMON2_ERROR;
use std::{
    io::{self, Write},
    path::Path,
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};

/// Bounds the final host phase: unless disarmed within its allowance, it
/// ends the process with the climon2 error code, whatever the host is
/// doing.
pub(crate) struct Watchdog {
    _disarm: Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Watchdog {
    /// Starts the allowance `within` now. A watchdog that cannot start
    /// leaves the phase unbounded, which the general log records.
    pub(crate) fn arm(within: Duration, dispatch: tracing::Dispatch) -> Self {
        let (disarm, disarmed) = mpsc::channel::<()>();
        let watched = move || {
            if disarmed.recv_timeout(within) == Err(RecvTimeoutError::Timeout) {
                tracing::dispatcher::with_default(&dispatch, || {
                    tracing::error!(
                        within_ms = within.as_millis(),
                        "final host restoration unfinished"
                    );
                });
                exit();
            }
        };
        let thread = thread::Builder::new()
            .name("final-host".into())
            .spawn(watched);
        let thread = thread
            .inspect_err(|error| tracing::warn!(%error, "final host phase unbounded"))
            .ok();
        Self {
            _disarm: disarm,
            thread,
        }
    }

    /// The final host phase finished in time.
    pub(crate) fn disarm(self) {
        let Self { _disarm, thread } = self;
        drop(_disarm);
        thread.map(JoinHandle::join);
    }
}

/// Prints `error` as the thin main would, with the general log `log`, then
/// ends the whole process with the climon2 error code. The host is already
/// released.
pub(crate) fn stop(error: &io::Error, log: &Path) -> ! {
    tracing::error!(%error, "monitor failed");
    let failure = crate::Error::from(io::Error::other(error.to_string())).logged(log);
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "climon2: error: {failure}");
    let _ = stderr.flush();
    exit()
}

fn exit() -> ! {
    // SAFETY: `_exit` only ends the process; the monitor's own output is
    // flushed and its general log unbuffered.
    unsafe { libc::_exit(CLIMON2_ERROR as i32) }
}
