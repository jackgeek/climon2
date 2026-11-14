//! The allocation scenario (#180), Linux only: an instance's snapshot beside
//! the host's own count of the heap its calls hold (`heap`).

use super::{
    abi::{Api, Handles},
    heap,
    scenarios::{Owner, Setup, create, drain_to_terminal},
};
use serde_json::{Value, json};

/// `allocation`: a `warm_up` instance's lifecycle first, on the same threads,
/// creates what the process and its threads keep for any instance (lazily
/// created dependency state and per-thread storage). Then, measuring from
/// nothing, the `measured` instance's lifecycle.
pub fn allocation(setup: &Setup) -> Value {
    let owner = Owner::spawn("owner");
    let warm_up = lifecycle(setup, &owner, &setup.input["warm_up"]);
    heap::measure();
    let measured = lifecycle(setup, &owner, &setup.input["measured"]);
    json!({ "warm_up": warm_up, "measured": measured, "joined": owner.join() })
}

/// One instance, one call at a time, so its snapshot and the host's count
/// see the same order of allocations: created, then shut down and run on the
/// owner thread until it returns, drained to terminal evidence, released and
/// destroyed. The snapshot and the host's count are taken side by side after
/// create and after the drains.
fn lifecycle(setup: &Setup, owner: &Owner, startup: &Value) -> Value {
    let api = setup.api;
    let (create, handles) = create(setup, owner, startup);
    let handles = handles.expect("the instance is created");
    let created = checkpoint(api, handles);
    let shutdown = api.shutdown(handles);
    let run = owner.call(move || api.run(handles));
    let descriptor = api.notification(handles).1;
    let drains = drain_to_terminal(api, handles, descriptor);
    let ended = checkpoint(api, handles);
    json!({
        "create": create,
        "created": created,
        "shutdown": shutdown,
        "run": run,
        "drains": drains,
        "ended": ended,
        "release_communication": api.release_communication(handles),
        "release_control": api.release_control(handles),
        "destroy": owner.call(move || api.destroy(handles)),
    })
}

/// A snapshot, and the host's count right after it, while nothing else calls
/// the library.
fn checkpoint(api: Api, handles: Handles) -> Value {
    let snapshot = api.snapshot(handles);
    json!({ "snapshot": snapshot, "heap": heap::observe() })
}
