//! Library-layer scenarios (#85): each drives instances through the function
//! table, from the threads the contract names, and reports what it observed.
//! Every wait is bounded; a call that does not return in time fails the host.

use super::{
    abi::{Api, Handles, call, filled, state},
    probe,
    startup::{self, Arena},
};
use climon2_core::native;
use serde_json::{Map, Value, json};
use std::{
    ptr,
    sync::mpsc::{Receiver, Sender, channel},
    thread::{self, JoinHandle},
    time::Instant,
};

type Job = Box<dyn FnOnce() + Send>;

/// A dedicated thread that creates, runs and destroys one instance, as the
/// embedding contract requires; the host hands it work and waits, bounded.
pub struct Owner {
    jobs: Option<Sender<Job>>,
    thread: JoinHandle<()>,
}

impl Owner {
    pub fn spawn(name: &str) -> Self {
        let (jobs, received) = channel::<Job>();
        let thread = thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                for job in received {
                    job();
                }
            })
            .expect("the owner thread starts");
        Self {
            jobs: Some(jobs),
            thread,
        }
    }

    /// Starts `work` on the owner thread; its result arrives on the receiver.
    pub fn start<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Receiver<T> {
        let (result, received) = channel();
        let job: Job = Box::new(move || drop(result.send(work())));
        self.jobs
            .as_ref()
            .expect("the owner accepts work")
            .send(job)
            .expect("the owner thread is alive");
        received
    }

    pub fn call<T: Send + 'static>(&self, work: impl FnOnce() -> T + Send + 'static) -> T {
        wait(&self.start(work))
    }

    /// Ends the thread's work and joins it, bounded: whether it was joined.
    pub fn join(mut self) -> bool {
        self.jobs = None;
        let deadline = probe::deadline();
        while !self.thread.is_finished() && Instant::now() < deadline {
            thread::sleep(std::time::Duration::from_millis(10));
        }
        self.thread.is_finished() && self.thread.join().is_ok()
    }
}

pub fn wait<T>(received: &Receiver<T>) -> T {
    received
        .recv_timeout(probe::deadline() - Instant::now())
        .expect("the library call returned within the host's deadline")
}

/// The configuration a scenario reads: `root` for paths and overrides of the
/// default startup.
pub struct Setup {
    pub api: Api,
    pub release: String,
    pub input: Value,
}

impl Setup {
    /// The startup `overrides` describe, in fresh caller memory.
    pub fn startup(&self, overrides: &Value) -> (Arena, *const native::Startup) {
        let root = self.input["root"].as_str().expect("the scenario's root");
        let spec = startup::merged(&startup::default(root, &self.release), overrides);
        let mut arena = Arena::new();
        let startup = startup::build(&mut arena, &spec);
        (arena, startup)
    }
}

/// A startup pointer carried to the owner thread with its memory.
struct Prepared(Arena, *const native::Startup);

// SAFETY: the startup lives in the arena that travels with it.
unsafe impl Send for Prepared {}

/// Creates on `owner`, then scrambles and unmaps the caller's startup memory,
/// so everything after `create` runs on the library's own copies.
pub fn create(setup: &Setup, owner: &Owner, overrides: &Value) -> (Value, Option<Handles>) {
    let (arena, startup) = setup.startup(overrides);
    let prepared = Prepared(arena, startup);
    let api = setup.api;
    owner.call(move || {
        let prepared = prepared;
        let created = api.create(prepared.1);
        prepared.0.scramble_and_unmap();
        let report = json!({
            "call": created.call,
            "output": created.output,
            "effective_limits": created.effective_limits,
        });
        (report, created.handles)
    })
}

/// The attempts of one call: a second, when the first failed explicitly with
/// `CM1_INTERNAL_ERROR` (a contained panic), shows what the failure left.
/// Each attempt is observed as the call's result, or as `[result, output]`.
pub fn attempts<T>(mut call: impl FnMut() -> (Value, T)) -> (Value, T) {
    let (first, value) = call();
    let code = first.get(0).unwrap_or(&first)["code"].clone();
    if code != native::INTERNAL_ERROR {
        return (json!([first]), value);
    }
    let (second, value) = call();
    (json!([first, second]), value)
}

/// Drains until terminal evidence, releasing every delivery on the way and
/// waiting on the descriptor while nothing is ready.
pub fn drain_to_terminal(api: Api, handles: Handles, descriptor: i32) -> Value {
    let mut drains = Vec::new();
    let deadline = probe::deadline();
    while Instant::now() < deadline {
        let (call, drain) = api.drain(handles);
        let ended = drain.kind == native::TERMINAL;
        drains.push(json!({ "call": call, "drained": drain.observed }));
        if let Some(delivery) = drain.delivery {
            let (released, ()) = attempts(|| (api.release_delivery(delivery), ()));
            drains.push(json!({ "release_delivery": released }));
        }
        if ended {
            break;
        }
        probe::wait_readable(descriptor, deadline);
    }
    Value::Array(drains)
}

/// `lifecycle`: create, run, shut down, drain to terminal evidence, release
/// both handles, destroy and join, with every misuse the contract rejects on
/// the way: run and destroy from the wrong thread, destroy while busy, handle
/// release before the stream ends, a repeated run.
pub fn lifecycle(setup: &Setup) -> Value {
    let api = setup.api;
    let owner = Owner::spawn("owner");
    let (create, handles) = create(setup, &owner, &setup.input["startup"]);
    let mut report = Map::new();
    report.insert("create".into(), create);
    let Some(handles) = handles else {
        return Value::Object(report);
    };
    report.insert("run_wrong_thread".into(), json!(api.run(handles)));
    report.insert("destroy_wrong_thread".into(), api.destroy(handles));
    report.insert(
        "destroy_before_run".into(),
        owner.call(move || api.destroy(handles)),
    );
    let (call, descriptor) = api.notification(handles);
    report.insert("notification".into(), call);
    report.insert("descriptor".into(), probe::descriptor(descriptor));
    report.insert(
        "readable_after_create".into(),
        json!(probe::readable(descriptor)),
    );
    report.insert("snapshot_after_create".into(), json!(api.snapshot(handles)));
    let (call, drain) = api.drain(handles);
    report.insert(
        "drain_before_shutdown".into(),
        json!([call, drain.observed]),
    );
    report.insert(
        "release_communication_before_shutdown".into(),
        api.release_communication(handles),
    );
    report.insert(
        "release_control_before_shutdown".into(),
        api.release_control(handles),
    );
    let ran = owner.start(move || {
        let run = api.run(handles);
        let again = api.run(handles);
        let destroy = api.destroy(handles);
        json!({ "run": run, "run_again": again, "destroy_before_release": destroy })
    });
    report.insert("shutdown".into(), api.shutdown(handles));
    report.insert("shutdown_again".into(), api.shutdown(handles));
    report.insert(
        "readable_after_shutdown".into(),
        json!(probe::wait_readable(descriptor, probe::deadline())),
    );
    report.insert("drains".into(), drain_to_terminal(api, handles, descriptor));
    let (call, terminal) = api.drain(handles);
    report.insert(
        "drain_after_terminal".into(),
        json!([call, terminal.observed]),
    );
    report.insert(
        "readable_after_terminal".into(),
        json!(probe::readable(descriptor)),
    );
    report.insert("owner".into(), wait(&ran));
    report.insert("snapshot_after_run".into(), json!(api.snapshot(handles)));
    report.insert(
        "release_communication".into(),
        api.release_communication(handles),
    );
    report.insert("release_control".into(), api.release_control(handles));
    report.insert("terminal_before_destroy".into(), terminal.observe_again());
    report.insert("destroy".into(), owner.call(move || api.destroy(handles)));
    report.insert(
        "descriptor_after_destroy".into(),
        probe::descriptor(descriptor),
    );
    report.insert("joined".into(), json!(owner.join()));
    report.insert("timeline".into(), api.timeline.report());
    report.insert("ledger".into(), api.ledger.report());
    Value::Object(report)
}

/// Shuts an instance down and cleans it up completely, from `owner`.
pub fn clean_up(api: Api, owner: &Owner, handles: Handles) -> Value {
    let ran = owner.start(move || api.run(handles));
    let shutdown = api.shutdown(handles);
    let descriptor = api.notification(handles).1;
    let drains = drain_to_terminal(api, handles, descriptor);
    let run = wait(&ran);
    json!({
        "shutdown": shutdown,
        "drains": drains,
        "run": run,
        "release_communication": api.release_communication(handles),
        "release_control": api.release_control(handles),
        "destroy": owner.call(move || api.destroy(handles)),
    })
}

/// `create`: one create per case, on an owner thread of its own, with the
/// open descriptors before and after; a successful one is then cleaned up.
/// A case is startup overrides, or `{"null": "startup" | "output"}`, or
/// `{"exhausted": true}` to create with no descriptor left.
pub fn creates(setup: &Setup) -> Value {
    let cases = setup.input["cases"].as_array().expect("cases");
    Value::Array(cases.iter().map(|case| create_case(setup, case)).collect())
}

fn create_case(setup: &Setup, case: &Value) -> Value {
    let api = setup.api;
    let owner = Owner::spawn("owner");
    let before = probe::open_descriptors();
    let (mut report, handles) = match case.get("null").and_then(Value::as_str) {
        Some(which) => (null_create(setup, &owner, which), None),
        None if case.get("exhausted").is_some() => {
            let (arena, startup) = setup.startup(&json!({}));
            let prepared = Prepared(arena, startup);
            owner.call(move || {
                let prepared = prepared;
                let created = probe::without_descriptors(|| api.create(prepared.1));
                prepared.0.scramble_and_unmap();
                (
                    json!({ "call": created.call, "output": created.output }),
                    created.handles,
                )
            })
        }
        None => create(setup, &owner, &case["startup"]),
    };
    if let Some(handles) = handles {
        report["cleanup"] = clean_up(api, &owner, handles);
    }
    report["descriptors_unchanged"] = json!(before == probe::open_descriptors());
    report["joined"] = json!(owner.join());
    report
}

fn null_create(setup: &Setup, owner: &Owner, which: &str) -> Value {
    let create = setup.api.table().create.expect("the table has create");
    let (arena, startup) = setup.startup(&json!({}));
    let prepared = Prepared(arena, startup);
    let null_startup = which == "startup";
    owner.call(move || {
        let prepared = prepared;
        let mut out = filled::<native::Created>();
        let (startup, output) = if null_startup {
            (ptr::null(), out.as_mut_ptr())
        } else {
            (prepared.1, ptr::null_mut())
        };
        // SAFETY: a null argument is the documented invalid-argument case;
        // the other argument is valid.
        let result = unsafe { create(startup, output) };
        prepared.0.scramble_and_unmap();
        json!({ "call": call(result), "output": state(&out) })
    })
}
