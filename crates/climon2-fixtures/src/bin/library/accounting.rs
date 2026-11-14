//! The allocation scenario (#180), x86-64 GNU/Linux only: an instance's
//! snapshot beside the host's own count of the heap the library holds
//! (`heap`).

use super::{
    abi::{Api, Handles},
    heap,
    scenarios::{Owner, Setup, create, end, release},
};
use serde_json::{Value, json};

/// `allocation`: a `warm_up` instance's lifecycle first, on the same threads,
/// creates what the process and its threads keep for any instance (lazily
/// created dependency state and per-thread storage). Then, measuring from
/// nothing, the `measured` instance's lifecycle. With the hooked library's
/// `worker` fault, each run starts a worker of the library's own.
pub fn allocation(setup: &Setup) -> Value {
    let owner = Owner::spawn("owner");
    let warm_up = lifecycle(setup, &owner, &setup.input["warm_up"]);
    let create = setup.api.table().create.expect("the table has create");
    heap::measure(create as *const std::ffi::c_void);
    let measured = lifecycle(setup, &owner, &setup.input["measured"]);
    json!({ "warm_up": warm_up, "measured": measured, "joined": owner.join() })
}

/// One instance, one call at a time, so its snapshot and the host's count
/// see the same order of allocations: created, ended (shut down, run on the
/// owner thread until it returns, drained to terminal evidence), released and
/// destroyed. The snapshot and the host's count are taken side by side after
/// create and once ended, when run has joined any worker.
fn lifecycle(setup: &Setup, owner: &Owner, startup: &Value) -> Value {
    let api = setup.api;
    let (create, handles) = create(setup, owner, startup);
    let handles = handles.expect("the instance is created");
    let created = checkpoint(api, handles);
    let end = end(api, owner, handles);
    let ended = checkpoint(api, handles);
    json!({
        "create": create,
        "created": created,
        "end": end,
        "ended": ended,
        "release": release(api, owner, handles),
    })
}

/// A snapshot, and the host's count right after it, while nothing else calls
/// the library.
fn checkpoint(api: Api, handles: Handles) -> Value {
    let snapshot = api.snapshot(handles);
    json!({ "snapshot": snapshot, "heap": heap::observe() })
}
