//! Fault-layer scenarios (#85), run with the `test-hooks` library only:
//! `CLIMON2_LIBRARY_FAULT` injects panics at exported boundaries and adds a
//! worker that ends with its stream, or one whose cleanup stalls.

use super::{
    abi::{Api, Handles},
    probe,
    scenarios::{Owner, Setup, attempts, create, drain_to_terminal, wait},
};
use climon2_core::native;
use serde_json::{Map, Value, json};
use std::time::Instant;

/// Creates on `owner`, once more if the first create failed explicitly.
fn created(setup: &Setup, owner: &Owner) -> (Value, Handles) {
    let overrides = &setup.input["startup"];
    let (first, handles) = create(setup, owner, overrides);
    if let Some(handles) = handles {
        return (json!([first]), handles);
    }
    let (second, handles) = create(setup, owner, overrides);
    (
        json!([first, second]),
        handles.expect("the second create succeeds"),
    )
}

fn submit(api: Api, handles: Handles) -> Value {
    let submit = api.table().submit.expect("the table has submit");
    // SAFETY: a zeroed request is never read by a library that does not
    // implement submit; the handle is live.
    let request = std::mem::MaybeUninit::<native::Request>::zeroed();
    let result = unsafe { submit(handles.communication.0, request.as_ptr()) };
    let value = Value::Object(super::abi::call(result));
    api.timeline.step("submit", &value["code"]);
    value
}

/// `fault`: every call once, in a fixed order, each retried once after an
/// explicit internal failure: create, submit, notification, snapshot, drain
/// (with a delivery: destroy and handle release while it is outstanding, and
/// its release), shutdown, then run on the owner thread, drains to terminal
/// evidence, release of the control handle, one more drain, release of the
/// communication handle and destroy. The descriptor's readiness is reported
/// where a contained panic changes it: a failure queues its diagnostic
/// before `run` publishes terminal evidence.
pub fn fault(setup: &Setup) -> Value {
    let api = setup.api;
    let owner = Owner::spawn("owner");
    let mut report = Map::new();
    let (create, handles) = created(setup, &owner);
    report.insert("create".into(), create);
    report.insert("submit".into(), attempts(|| (submit(api, handles), ())).0);
    let (notification, descriptor) = attempts(|| {
        let (call, descriptor) = api.notification(handles);
        (json!([call, descriptor]), descriptor)
    });
    report.insert("notification".into(), notification);
    let (snapshot, ()) = attempts(|| (json!(api.snapshot(handles)), ()));
    report.insert("snapshot".into(), snapshot);
    report.insert(
        "readable_before_drain".into(),
        json!(probe::readable(descriptor)),
    );
    let (drain, drained) = attempts(|| {
        let (call, drain) = api.drain(handles);
        (json!([call, drain.observed.clone()]), drain)
    });
    report.insert("drain".into(), drain);
    if let Some(delivery) = drained.delivery {
        report.insert(
            "snapshot_with_delivery".into(),
            json!(api.snapshot(handles)),
        );
        report.insert(
            "destroy_with_delivery".into(),
            owner.call(move || api.destroy(handles)),
        );
        report.insert(
            "release_communication_with_delivery".into(),
            api.release_communication(handles),
        );
        report.insert("delivery_before_release".into(), drained.observe_again());
        let (released, ()) = attempts(|| (api.release_delivery(delivery), ()));
        report.insert("release_delivery".into(), released);
    }
    report.insert(
        "readable_before_run".into(),
        json!(probe::readable(descriptor)),
    );
    report.insert(
        "shutdown".into(),
        attempts(|| (api.shutdown(handles), ())).0,
    );
    let ran = owner.start(move || {
        let run = api.run(handles);
        let again = api.run(handles);
        json!({ "run": run, "run_again": again })
    });
    report.insert("drains".into(), drain_to_terminal(api, handles, descriptor));
    report.insert("owner".into(), wait(&ran));
    report.insert("snapshot_after_run".into(), json!(api.snapshot(handles)));
    report.insert(
        "release_control".into(),
        attempts(|| (api.release_control(handles), ())).0,
    );
    let (call, drain) = api.drain(handles);
    report.insert(
        "drain_after_release_control".into(),
        json!([call, drain.observed]),
    );
    report.insert(
        "release_communication".into(),
        attempts(|| (api.release_communication(handles), ())).0,
    );
    report.insert(
        "destroy".into(),
        owner.call(move || attempts(|| (api.destroy(handles), ())).0),
    );
    report.insert("joined".into(), json!(owner.join()));
    report.insert("timeline".into(), api.timeline.report());
    report.insert("ledger".into(), api.ledger.report());
    Value::Object(report)
}

/// Snapshots until `workers` reaches `count`, bounded.
fn snapshot_with_workers(api: Api, handles: Handles, count: u64) -> Value {
    let deadline = probe::deadline();
    loop {
        let (call, metrics) = api.snapshot(handles);
        if metrics["workers"] == count || Instant::now() >= deadline {
            return json!([call, metrics]);
        }
    }
}

/// `workers`, with `CLIMON2_LIBRARY_FAULT=worker` or `stall-cleanup`: the
/// worker is running before shutdown; terminal evidence is drained while its
/// cleanup may still run; `run` then reports that cleanup separately, and
/// `destroy` succeeds only after complete cleanup. A stalled instance is
/// never destroyed or joined: the process ends with it.
pub fn workers(setup: &Setup) -> Value {
    let api = setup.api;
    let owner = Owner::spawn("owner");
    let (create, handles) = create(setup, &owner, &setup.input["startup"]);
    let handles = handles.expect("create succeeds");
    let mut report = Map::new();
    report.insert("create".into(), create);
    let ran = owner.start(move || {
        let started = Instant::now();
        let run = api.run(handles);
        let destroy = api.destroy(handles);
        json!({ "run": run, "elapsed_ms": started.elapsed().as_millis() as u64, "destroy_after_run": destroy })
    });
    report.insert(
        "snapshot_running".into(),
        snapshot_with_workers(api, handles, 1),
    );
    report.insert("shutdown".into(), api.shutdown(handles));
    let descriptor = api.notification(handles).1;
    report.insert("drains".into(), drain_to_terminal(api, handles, descriptor));
    report.insert("owner".into(), wait(&ran));
    report.insert("snapshot_after_run".into(), json!(api.snapshot(handles)));
    report.insert(
        "release_communication".into(),
        api.release_communication(handles),
    );
    report.insert("release_control".into(), api.release_control(handles));
    let destroy = owner.call(move || api.destroy(handles));
    let destroyed = destroy["code"] == native::SUCCESS;
    report.insert("destroy".into(), destroy);
    report.insert("joined".into(), json!(owner.join()));
    report.insert("instance_destroyed".into(), json!(destroyed));
    report.insert("ledger".into(), api.ledger.report());
    Value::Object(report)
}
