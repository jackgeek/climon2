//! Scripted Library-layer scenarios (#93): instances, each against a real
//! session daemon (startup overrides name its metadata and session) or a
//! fake daemon peer, driven step by step so each test states its own
//! sequence. Every wait is bounded, every call is on the timeline with its
//! thread, and every library object is in the ownership ledger.
//!
//! `climon2-library-host <library> script <json>`, where the JSON has
//! `root`, `steps` and, optionally, `"sigpipe": "default"`: the host then
//! lets `SIGPIPE` terminate it, as in a C embedder that never ignored it,
//! where Rust hosts otherwise ignore it. A step is an object with `do` and an optional
//! `instance` (default `"a"`):
//! - `create`, with `startup` overrides and an optional fake `peer` script
//!   (see `peer`), on the instance's own owner thread;
//! - `submit`, with a `request` (see `requests`) in caller memory scrambled
//!   and unmapped as soon as `submit` returns, or a null one; in it, the
//!   strings `"$period"` and `"$restoration"` are the instance's latest
//!   authority drained so far (from Control granted, Restoration begin, or
//!   an Acquire or Ready result), and `"$stale_period"` and
//!   `"$stale_restoration"` the one before it (#94);
//! - `run`, on the owner thread; `wait_run` for its report;
//! - `drain`, `until` `{"records": n}`, `{"tag": t}`, `{"response": id}`,
//!   `{"display": hex}` (the Display bytes drained in this step contain
//!   them), `{"all": [until, ...]}` or `"terminal"` (default: once), releasing each delivery unless
//!   `keep`, and stopping at the first failed drain;
//! - `observe` and `release` the deliveries kept so far;
//! - `snapshot`, `notification`, `shutdown`; `await_snapshot` until its
//!   `gauge` reaches `value`;
//! - `touch` a `path`; `probe` a `socket`'s hello `state`; `signal` a
//!   real daemon's `pid` to `stop` or `continue`;
//! - `await_peer` until it received `frames` frames;
//! - `finish`: run's report, drains to terminal evidence, handle releases,
//!   destroy and join.
//!
//! Each step reports its calls, with `elapsed_us` for the calls the contract
//! keeps bounded and free of daemon I/O; `started_us` has when each step
//! started.

use super::{
    abi::{Api, Drain, Handles},
    peer::Peer,
    probe, requests,
    scenarios::{Owner, Setup, attempts, create, wait},
    startup::{self, Arena},
};
use climon2_core::{
    native,
    protocol::{self, ClientMessage},
};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::net::UnixStream,
    path::Path,
    sync::mpsc::Receiver,
    thread,
    time::{Duration, Instant},
};

struct Instance {
    owner: Option<Owner>,
    handles: Option<Handles>,
    run: Option<Receiver<(Value, Value)>>,
    report: Option<Value>,
    kept: Vec<Drain>,
    peer: Option<Peer>,
    /// Each distinct `[period, restoration_id]` drained, in order.
    authorities: Vec<[u64; 2]>,
}

struct World<'a> {
    setup: &'a Setup,
    instances: BTreeMap<String, Instance>,
}

pub fn script(setup: &Setup) -> Value {
    if setup.input["sigpipe"] == "default" {
        // SAFETY: the host is still single-threaded here.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    let mut world = World {
        setup,
        instances: BTreeMap::new(),
    };
    let steps = setup.input["steps"].as_array().expect("steps");
    let started = Instant::now();
    let mut starts = Vec::new();
    let reports: Vec<Value> = steps
        .iter()
        .map(|step| {
            starts.push(started.elapsed().as_micros() as u64);
            world.step(step)
        })
        .collect();
    let api = setup.api;
    let peers: Map<String, Value> = world
        .instances
        .into_iter()
        .filter_map(|(name, instance)| instance.peer.map(|peer| (name, peer.report())))
        .collect();
    json!({
        "steps": reports,
        "started_us": starts,
        "peers": peers,
        "timeline": api.timeline.report(),
        "ledger": api.ledger.report(),
    })
}

/// A call's report with how long it took.
fn timed(call: impl FnOnce() -> Value) -> Value {
    let started = Instant::now();
    let mut value = call();
    value["elapsed_us"] = json!(started.elapsed().as_micros() as u64);
    value
}

impl World<'_> {
    fn step(&mut self, step: &Value) -> Value {
        let name = step["instance"].as_str().unwrap_or("a").to_owned();
        let action = step["do"].as_str().expect("a step's action");
        match action {
            "create" => return self.create(&name, step),
            "touch" => return touch(step),
            "signal" => return signal(step),
            "probe" => return hello_state(Path::new(step["socket"].as_str().expect("a socket"))),
            _ => {}
        }
        let api = self.setup.api;
        let instance = self.instances.get_mut(&name).expect("a created instance");
        match action {
            "submit" => submit(api, instance, &step["request"]),
            "run" => {
                let handles = handles(instance);
                instance.run = Some(owner(instance).start(move || api.run(handles)));
                json!("started")
            }
            "wait_run" => wait_run(instance),
            "drain" => drain(api, instance, step),
            "observe" => Value::Array(instance.kept.iter().map(Drain::observe_again).collect()),
            "release" => release(api, instance),
            "snapshot" => json!(timed_pair(|| api.snapshot(handles(instance)))),
            "await_snapshot" => await_snapshot(api, instance, step),
            "notification" => json!(api.notification(handles(instance)).0),
            "shutdown" => api.shutdown(handles(instance)),
            "await_peer" => await_peer(instance, step),
            "finish" => finish(api, instance),
            _ => panic!("unknown step {action}"),
        }
    }

    fn create(&mut self, name: &str, step: &Value) -> Value {
        let mut overrides = match &step["startup"] {
            Value::Null => json!({}),
            startup => startup.clone(),
        };
        let peer = step.get("peer").map(|script| {
            let root = Path::new(self.setup.input["root"].as_str().expect("a root"));
            Peer::start(root, name, &self.setup.release, script)
        });
        if let Some(peer) = &peer {
            let path = json!({ "bytes": peer.metadata.to_str().expect("a UTF-8 path") });
            let fake = json!({ "session_id": peer.session, "metadata_path": path });
            overrides = startup::merged(&fake, &overrides);
        }
        let owner = Owner::spawn(&format!("owner-{name}"));
        let (report, handles) = create(self.setup, &owner, &overrides);
        self.instances.insert(
            name.to_owned(),
            Instance {
                owner: Some(owner),
                handles,
                run: None,
                report: None,
                kept: Vec::new(),
                peer,
                authorities: Vec::new(),
            },
        );
        report
    }
}

fn handles(instance: &Instance) -> Handles {
    instance.handles.expect("the instance was created")
}

fn owner(instance: &Instance) -> &Owner {
    instance
        .owner
        .as_ref()
        .expect("the owner thread is not joined yet")
}

fn timed_pair(call: impl FnOnce() -> (Value, Value)) -> Value {
    let started = Instant::now();
    let (call, output) = call();
    json!({ "call": call, "output": output, "elapsed_us": started.elapsed().as_micros() as u64 })
}

/// Submits `request` from caller memory, or a null request for `null`.
fn submit(api: Api, instance: &Instance, request: &Value) -> Value {
    let mut arena = Arena::new();
    let built = if request.is_null() {
        std::ptr::null()
    } else {
        requests::build(&mut arena, &substituted(instance, request))
    };
    let call = timed(|| api.submit(handles(instance), built));
    arena.scramble_and_unmap();
    call
}

/// `request` with the instance's drained authorities in place of
/// `"$period"`, `"$restoration"`, `"$stale_period"` and
/// `"$stale_restoration"`.
fn substituted(instance: &Instance, request: &Value) -> Value {
    let back = |by: usize| {
        let count = instance.authorities.len();
        let index = count.checked_sub(by).unwrap_or_else(|| {
            panic!(
                "{by} drained authorities needed: {:?}",
                instance.authorities
            )
        });
        instance.authorities[index]
    };
    match request {
        Value::String(text) => match text.as_str() {
            "$period" => json!(back(1)[0]),
            "$restoration" => json!(back(1)[1]),
            "$stale_period" => json!(back(2)[0]),
            "$stale_restoration" => json!(back(2)[1]),
            _ => request.clone(),
        },
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substituted(instance, item))
                .collect(),
        ),
        Value::Object(members) => Value::Object(
            members
                .iter()
                .map(|(key, member)| (key.clone(), substituted(instance, member)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The authority a drained record gives its role, if any.
fn authority(record: &Value) -> Option<[u64; 2]> {
    let tag = record["tag"].as_u64()? as u32;
    let source = match tag {
        native::GRANTED | native::RESTORATION_BEGIN => &record["presentation"],
        native::RESPONSE => &record["response"]["result"],
        _ => return None,
    };
    Some([
        source["period"].as_u64()?,
        source["restoration_id"].as_u64()?,
    ])
}

fn track(instance: &mut Instance, records: &[Value]) {
    for found in records.iter().filter_map(authority) {
        if instance.authorities.last() != Some(&found) {
            instance.authorities.push(found);
        }
    }
}

/// The Display bytes among `records`, as hex.
fn displayed(records: &[Value]) -> String {
    records
        .iter()
        .filter(|record| record["tag"] == native::DISPLAY)
        .filter_map(|record| record["presentation"]["bytes"].as_str())
        .collect()
}

fn wait_run(instance: &mut Instance) -> Value {
    if let Some(run) = instance.run.take() {
        let (call, report) = wait(&run);
        instance.report = Some(json!({ "call": call, "report": report }));
    }
    instance.report.clone().unwrap_or(Value::Null)
}

/// Whether the drains so far meet `until`: terminal evidence, `records`
/// records, a record of `tag`, or, by default, one drain.
fn met(until: &Value, drains: &[Value], records: &[Value], terminal: bool) -> bool {
    if until == "terminal" {
        return terminal;
    }
    if let Some(all) = until["all"].as_array() {
        return all
            .iter()
            .all(|until| met(until, drains, records, terminal));
    }
    if let Some(count) = until["records"].as_u64() {
        return records.len() as u64 >= count;
    }
    if let Some(tag) = until["tag"].as_u64() {
        return records.iter().any(|record| record["tag"] == tag);
    }
    if let Some(id) = until["response"].as_u64() {
        return records
            .iter()
            .any(|record| record["response"]["request_id"] == id);
    }
    if let Some(hex) = until["display"].as_str() {
        return displayed(records).contains(hex);
    }
    !drains.is_empty()
}

/// Drains until `until` is met, waiting on the descriptor while nothing is
/// ready, bounded; each delivery is released unless `keep`.
fn drain(api: Api, instance: &mut Instance, step: &Value) -> Value {
    let handles = handles(instance);
    let descriptor = api.notification(handles).1;
    let keep = step["keep"].as_bool().unwrap_or(false);
    let (mut drains, mut records, mut terminal) = (Vec::new(), Vec::new(), false);
    let deadline = probe::deadline();
    while !met(&step["until"], &drains, &records, terminal) && Instant::now() < deadline {
        let started = Instant::now();
        let (call, drained) = api.drain(handles);
        let elapsed = started.elapsed().as_micros() as u64;
        let ok = call["code"] == native::SUCCESS;
        if let Some(found) = drained.observed["records"].as_array() {
            track(instance, found);
            records.extend(found.iter().cloned());
        }
        terminal |= drained.kind == native::TERMINAL;
        let mut entry =
            json!({ "call": call, "drained": drained.observed.clone(), "elapsed_us": elapsed });
        match drained.delivery {
            Some(delivery) if !keep => {
                let started = Instant::now();
                let (released, ()) = attempts(|| (api.release_delivery(delivery), ()));
                entry["release"] = json!({
                    "calls": released,
                    "elapsed_us": started.elapsed().as_micros() as u64,
                });
            }
            Some(_) => instance.kept.push(drained),
            None => {}
        }
        drains.push(entry);
        if !ok {
            break;
        }
        if drained_nothing(&drains) && !met(&step["until"], &drains, &records, terminal) {
            probe::wait_readable(descriptor, deadline);
        }
    }
    json!({ "drains": drains, "records": records, "terminal": terminal })
}

fn drained_nothing(drains: &[Value]) -> bool {
    drains
        .last()
        .is_some_and(|last| last["drained"]["kind"] == native::EMPTY)
}

fn release(api: Api, instance: &mut Instance) -> Value {
    let released: Vec<Value> = instance
        .kept
        .drain(..)
        .filter_map(|drain| drain.delivery)
        .map(|delivery| timed(|| api.release_delivery(delivery)))
        .collect();
    Value::Array(released)
}

/// Snapshots until `gauge` reaches `value`, bounded: the last snapshot.
fn await_snapshot(api: Api, instance: &Instance, step: &Value) -> Value {
    let gauge = step["gauge"].as_str().expect("a gauge");
    let deadline = probe::deadline();
    loop {
        let (call, output) = api.snapshot(handles(instance));
        if output[gauge] == step["value"] || Instant::now() >= deadline {
            return json!({ "call": call, "output": output });
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn touch(step: &Value) -> Value {
    let path = step["path"].as_str().expect("a path");
    fs::write(path, b"").expect("the trigger file is written");
    json!("touched")
}

/// Stops or resumes a process, such as a real session daemon.
fn signal(step: &Value) -> Value {
    let pid = step["pid"].as_i64().expect("a pid") as libc::pid_t;
    let signal = match step["signal"].as_str() {
        Some("stop") => libc::SIGSTOP,
        Some("continue") => libc::SIGCONT,
        other => panic!("unknown signal {other:?}"),
    };
    // SAFETY: kill has no memory-safety preconditions.
    let sent = unsafe { libc::kill(pid, signal) } == 0;
    assert!(sent, "signal {pid}: {}", std::io::Error::last_os_error());
    json!("signalled")
}

fn await_peer(instance: &Instance, step: &Value) -> Value {
    let count = step["frames"].as_u64().expect("a frame count") as usize;
    let peer = instance.peer.as_ref().expect("a fake peer");
    json!(peer.await_frames(count))
}

/// The session's hello `state`, as `climon2 list` reads it.
fn hello_state(socket: &Path) -> Value {
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return json!({ "error": "connect" });
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a read timeout");
    let hello = ClientMessage::Hello {
        protocol: protocol::PROTOCOL_VERSION,
    }
    .to_frame()
    .expect("a hello frame");
    protocol::write_frame(&mut stream, &hello).expect("hello is sent");
    let reply = protocol::read_frame(&mut stream).expect("a hello reply");
    let body: Value = serde_json::from_slice(reply.body()).expect("a JSON reply");
    json!({ "state": body["state"], "capabilities": body["capabilities"] })
}

/// Waits for run, drains to terminal evidence, releases both handles,
/// destroys the instance and joins its owner thread.
fn finish(api: Api, instance: &mut Instance) -> Value {
    let handles = handles(instance);
    let drained = drain(api, instance, &json!({ "until": "terminal" }));
    let run = wait_run(instance);
    let released = release(api, instance);
    let release_communication = api.release_communication(handles);
    let release_control = api.release_control(handles);
    let destroy = owner(instance).call(move || api.destroy(handles));
    let joined = instance.owner.take().is_some_and(Owner::join);
    json!({
        "drained": drained,
        "run": run,
        "released": released,
        "release_communication": release_communication,
        "release_control": release_control,
        "destroy": destroy,
        "joined": joined,
    })
}
