//! Test-only faults, compiled only with `test-hooks` and selected by
//! `CLIMON2_LIBRARY_FAULT`:
//!
//! - `panic:<call>[,<call>...]` panics once in the process at the boundary of
//!   each named exported call, the first time it is made (`bootstrap` is
//!   `climon2_client_get_interface`), or at `receive`, as a running
//!   connection handles a frame from its daemon;
//! - `abort:<call>[,<call>...]` aborts the whole process at the same points,
//!   a native crash no instance can contain (#118);
//! - `worker` makes `run` start one worker that ends with its stream;
//! - `stall-cleanup` makes `run` start one worker whose cleanup never ends;
//! - `joined-worker` makes `run` start one worker that briefly holds a block
//!   of `chunk_bytes` and ends, and join it before going on, so it never runs
//!   beside its instance's other work.

use std::sync::Mutex;

/// The calls whose injected panic has happened.
static PANICKED: Mutex<Vec<&str>> = Mutex::new(Vec::new());

fn fault() -> String {
    std::env::var("CLIMON2_LIBRARY_FAULT").unwrap_or_default()
}

pub(super) fn panic_at(call: &'static str) {
    let fault = fault();
    let named = |kind: &str| {
        fault
            .strip_prefix(kind)
            .is_some_and(|calls| calls.split(',').any(|name| name == call))
    };
    if named("abort:") {
        std::process::abort();
    }
    if named("panic:") && first(call) {
        panic!("CLIMON2_LIBRARY_FAULT panic at {call}");
    }
}

/// Every exported call a panic can be injected at.
const CALLS: usize = 12;

/// Reserves the list of injected panics at bootstrap, so recording one
/// allocates nothing an instance is charged for.
pub(super) fn prepare() {
    PANICKED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .reserve(CALLS);
}

/// Whether this is the first injected panic at `call`.
fn first(call: &'static str) -> bool {
    let mut panicked = PANICKED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let first = !panicked.contains(&call);
    panicked.push(call);
    first
}

/// The worker a fault makes `run` start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Worker {
    /// Ends with its instance's stream.
    Ends,
    /// Never ends, so its cleanup stalls.
    Stalls,
    /// Holds one block of `chunk_bytes` and ends, and `run` joins it before
    /// it goes on.
    Joined,
}

/// The worker `run` starts, if any.
pub(super) fn worker() -> Option<Worker> {
    let fault = fault();
    [
        ("worker", Worker::Ends),
        ("stall-cleanup", Worker::Stalls),
        ("joined-worker", Worker::Joined),
    ]
    .into_iter()
    .find(|(name, _)| fault == *name)
    .map(|(_, worker)| worker)
}
