//! Process- and thread-wide state the library's dependencies create lazily,
//! created first outside any instance, so no instance is charged for it
//! (#180): it is shared-release allocation, which is not tracked yet.
//!
//! At bootstrap, a diagnostics dispatcher of the library's own, writing
//! nowhere, creates tracing's process-wide dispatcher list. Before a caller's
//! thread first works for an instance, it is warmed: the standard library's
//! handle for the thread, and the formatting buffer each thread keeps for
//! records. A thread an instance owns is adopted instead: its thread-wide
//! state is the instance's.

use climon2_core::diagnostics::{CappedWriter, Filter, dispatch};
use std::{cell::Cell, io, sync::OnceLock};
use tracing::Dispatch;

/// The target of the one record each warmed thread formats.
const TARGET: &str = "climon2_client_warm";

static DIAGNOSTICS: OnceLock<Dispatch> = OnceLock::new();

thread_local! {
    /// Whether this thread's state is created, or is its instance's.
    static WARMED: Cell<bool> = const { Cell::new(false) };
}

/// At bootstrap: the process-wide state, and the bootstrapping thread's.
pub(super) fn process() {
    if let Ok(filter) = Filter::parse(&format!("{TARGET}=trace")) {
        DIAGNOSTICS.get_or_init(|| dispatch(CappedWriter::new(io::sink()), filter));
    }
    #[cfg(feature = "test-hooks")]
    super::hooks::prepare();
    thread();
}

/// Warms this thread, once, if it works for an instance it does not belong
/// to.
pub(super) fn thread() {
    if !WARMED.replace(true) {
        drop(std::thread::current());
        record();
    }
}

/// Formats one record through the library's own dispatcher, which creates
/// the thread's formatting buffer.
fn record() {
    if let Some(diagnostics) = DIAGNOSTICS.get() {
        tracing::dispatcher::with_default(diagnostics, || {
            tracing::trace!(target: TARGET, "thread warmed");
        });
    }
}

/// This thread belongs to the instance it works for: nothing is warmed.
pub(super) fn adopt() {
    WARMED.set(true);
}

#[cfg(test)]
mod tests {
    use super::{adopt, process, record, thread};
    use crate::library::allocation::Account;

    /// What formatting a record on a new thread charges an instance, after
    /// the thread is warmed or adopted as an instance's, as a call's work on
    /// an instance does first.
    fn charged(adopted: bool) -> u64 {
        std::thread::spawn(move || {
            if adopted {
                adopt();
            }
            thread();
            let account = Account::open();
            let before = account.measure().current;
            account.charged(record);
            account.measure().current - before
        })
        .join()
        .unwrap()
    }

    /// A caller's thread keeps its formatting buffer uncharged; a thread an
    /// instance owns is charged for its own.
    #[test]
    fn a_callers_thread_is_warmed_uncharged_and_an_instances_own_is_not() {
        process();
        assert_eq!(charged(false), 0);
        assert!(charged(true) > 0);
    }
}
