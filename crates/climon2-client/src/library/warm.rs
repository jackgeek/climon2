//! Process- and thread-wide state the library's dependencies create lazily,
//! created first outside any instance, so no instance is charged for it
//! (#180): it is shared-release allocation, which is not tracked yet.
//!
//! At bootstrap, a diagnostics dispatcher of the library's own, writing
//! nowhere, creates tracing's process-wide dispatcher list, and one span
//! entered through it takes the process-wide locks of the span registry's
//! thread-ID allocators (`thread_local`'s and `sharded-slab`'s) for the first
//! time: on macOS a `std::sync::Mutex` allocates its pthread mutex when it is
//! first locked (on Linux a futex mutex allocates nothing), so whichever
//! instance took them first would be charged for both. Before a caller's
//! thread first works for an instance, it is warmed: the formatting buffer
//! each thread keeps for records is created. A thread an instance owns is
//! adopted instead: its thread-wide state is the instance's. (The standard
//! library's handle for a thread needs no warming: it never comes from this
//! image's allocator.)

use climon2_core::diagnostics::{CappedWriter, Filter, dispatch};
use std::{cell::Cell, io, sync::OnceLock};
use tracing::Dispatch;

/// The target of the one record each warmed thread formats.
const TARGET: &str = "climon2_client_warm";

static DIAGNOSTICS: OnceLock<Dispatch> = OnceLock::new();

/// What a thread's own state is.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Warmth {
    /// Not created yet.
    Cold,
    /// Created outside any instance.
    Warm,
    /// Its instance's, since the thread is. Only hooked builds start
    /// instance threads.
    #[cfg(any(test, feature = "test-hooks"))]
    Adopted,
}

thread_local! {
    static WARMTH: Cell<Warmth> = const { Cell::new(Warmth::Cold) };
}

/// At bootstrap: the process-wide state, and the bootstrapping thread's.
pub(super) fn process() {
    if let Ok(filter) = Filter::parse(&format!("{TARGET}=trace")) {
        let diagnostics =
            DIAGNOSTICS.get_or_init(|| dispatch(CappedWriter::new(io::sink()), filter));
        tracing::dispatcher::with_default(diagnostics, || {
            tracing::trace_span!(target: TARGET, "bootstrap").in_scope(|| {});
        });
    }
    #[cfg(feature = "test-hooks")]
    super::hooks::prepare();
    thread();
}

/// Warms this thread, once, if it works for an instance it does not belong
/// to.
pub(super) fn thread() {
    if WARMTH.get() == Warmth::Cold {
        WARMTH.set(Warmth::Warm);
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
#[cfg(any(test, feature = "test-hooks"))]
pub(super) fn adopt() {
    WARMTH.set(Warmth::Adopted);
}

#[cfg(test)]
mod tests {
    use super::super::allocation::Account;
    use super::{adopt, process, record, thread};

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
