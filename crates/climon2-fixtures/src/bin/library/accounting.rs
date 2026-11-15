//! The allocation scenario (#180), x86-64 GNU/Linux only: an instance's
//! snapshot beside the host's own count of the heap the library holds
//! (`heap`).

use super::{
    abi::{Api, Handles, Ptr},
    heap,
    peer::Peer,
    probe, requests,
    scenarios::{Owner, Setup, create, end, release, wait},
    startup::{self, Arena},
};
use climon2_core::native::{self, Delivery};
use serde_json::{Value, json};
use std::path::Path;

/// `allocation`: a `warm_up` instance's lifecycle first, on the same threads,
/// creates what the process and its threads keep for any instance (lazily
/// created dependency state and per-thread storage). Then, measuring from
/// nothing, the `measured` instance's lifecycle. With the hooked library's
/// `worker` fault, each run starts a worker of the library's own. With
/// `connect` (a fake daemon `peer` script and an `open` request, as `script`
/// takes them), each instance opens a passive monitor connection first.
pub fn allocation(setup: &Setup) -> Value {
    let owner = Owner::spawn("owner");
    let warm_up = lifecycle(setup, &owner, "warm_up");
    let create = setup.api.table().create.expect("the table has create");
    heap::measure(create as *const std::ffi::c_void);
    let measured = lifecycle(setup, &owner, "measured");
    json!({ "warm_up": warm_up, "measured": measured, "joined": owner.join() })
}

/// One instance, one call at a time, so its snapshot and the host's count
/// see the same order of allocations: created, ended (shut down, run on the
/// owner thread until it returns, drained to terminal evidence), released and
/// destroyed. The snapshot and the host's count are taken side by side after
/// create and once ended, when run has returned and joined any worker; a
/// connected instance still holds its deliveries then.
fn lifecycle(setup: &Setup, owner: &Owner, name: &str) -> Value {
    let api = setup.api;
    let connect = &setup.input["connect"];
    let peer = connect
        .get("peer")
        .map(|script| Peer::start(root(setup), name, &setup.release, script));
    let startup = overrides(&setup.input[name], peer.as_ref());
    let (create, handles) = create(setup, owner, &startup);
    let handles = handles.expect("the instance is created");
    let created = checkpoint(api, handles);
    let (end, kept) = match peer {
        Some(_) => connected(api, owner, handles, &connect["open"]),
        None => (end(api, owner, handles), Vec::new()),
    };
    let ended = checkpoint(api, handles);
    let released: Vec<Value> = kept
        .into_iter()
        .map(|delivery| api.release_delivery(delivery))
        .collect();
    json!({
        "create": create,
        "created": created,
        "end": end,
        "ended": ended,
        "release_deliveries": released,
        "release": release(api, owner, handles),
        "peer": peer.map(Peer::report),
    })
}

fn root(setup: &Setup) -> &Path {
    Path::new(setup.input["root"].as_str().expect("a root"))
}

/// The instance's startup, naming the fake daemon's session and metadata
/// when it has one.
fn overrides(startup: &Value, peer: Option<&Peer>) -> Value {
    let Some(peer) = peer else {
        return startup.clone();
    };
    let path = json!({ "bytes": peer.metadata.to_str().expect("a UTF-8 path") });
    let fake = json!({ "session_id": peer.session, "metadata_path": path });
    startup::merged(&fake, startup)
}

/// Ends a connected instance with as little beside its connection as the
/// contract allows: `open` is submitted, run starts on `owner`, and the host
/// only waits for the first record before it requests shutdown, then waits
/// for run to return. Then it drains to terminal evidence, keeping every
/// delivery.
fn connected(
    api: Api,
    owner: &Owner,
    handles: Handles,
    open: &Value,
) -> (Value, Vec<Ptr<Delivery>>) {
    let mut arena = Arena::new();
    let submit = api.submit(handles, requests::build(&mut arena, open));
    arena.scramble_and_unmap();
    let descriptor = api.notification(handles).1;
    let ran = owner.start(move || api.run(handles));
    let readable = probe::wait_readable(descriptor, probe::deadline());
    let shutdown = api.shutdown(handles);
    let run = wait(&ran);
    let (drains, kept) = drain_keeping(api, handles);
    let end = json!({
        "submit": submit,
        "readable": readable,
        "shutdown": shutdown,
        "run": run,
        "drains": drains,
    });
    (end, kept)
}

/// Drains to terminal evidence once run has returned, keeping every
/// delivery: everything is queued by then, so no drain is empty.
fn drain_keeping(api: Api, handles: Handles) -> (Value, Vec<Ptr<Delivery>>) {
    let (mut drains, mut kept) = (Vec::new(), Vec::new());
    loop {
        let (call, drain) = api.drain(handles);
        drains.push(json!({ "call": call, "drained": drain.observed }));
        kept.extend(drain.delivery);
        if drain.kind != native::DELIVERY {
            return (Value::Array(drains), kept);
        }
    }
}

/// A snapshot, and the host's count right after it, while nothing else calls
/// the library.
fn checkpoint(api: Api, handles: Handles) -> Value {
    let snapshot = api.snapshot(handles);
    json!({ "snapshot": snapshot, "heap": heap::observe() })
}
