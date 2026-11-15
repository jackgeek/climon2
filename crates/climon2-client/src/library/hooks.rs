//! Test-only faults, compiled only with `test-hooks` and selected by
//! `CLIMON2_LIBRARY_FAULT`:
//!
//! - `panic:<call>[,<call>...]` panics once in the process at the boundary of
//!   each named exported call, the first time it is made (`bootstrap` is
//!   `climon2_client_get_interface`), or at `receive`, as a running
//!   connection handles a frame from its daemon;
//! - `worker` makes `run` start one worker that ends with its stream;
//! - `stall-cleanup` makes `run` start one worker whose cleanup never ends.

use std::sync::Mutex;

/// The calls whose injected panic has happened.
static PANICKED: Mutex<Vec<&str>> = Mutex::new(Vec::new());

fn fault() -> String {
    std::env::var("CLIMON2_LIBRARY_FAULT").unwrap_or_default()
}

pub(super) fn panic_at(call: &'static str) {
    let fault = fault();
    let selected = fault
        .strip_prefix("panic:")
        .is_some_and(|calls| calls.split(',').any(|name| name == call));
    if selected && first(call) {
        panic!("CLIMON2_LIBRARY_FAULT panic at {call}");
    }
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

/// Whether `run` starts a worker, and whether that worker's cleanup stalls.
pub(super) fn worker() -> Option<bool> {
    let fault = fault();
    [("worker", false), ("stall-cleanup", true)]
        .into_iter()
        .find(|(name, _)| fault == *name)
        .map(|(_, stalls)| stalls)
}
