//! Independence scenarios (#85): instances share nothing, the process keeps
//! its own state, and the borrowed notification descriptor stays safe to poll
//! while the stream ends and the handles are in use.

use super::{
    abi::{Api, Handles},
    probe,
    scenarios::{Owner, Setup, clean_up, create, drain_to_terminal, wait},
};
use climon2_core::native;
use serde_json::{Map, Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
};

/// `independence`: instances `a` and `b`, created from different startups
/// on owner threads of their own, both running. `a` alone is used first (a
/// fault, if one is injected, reaches it), shut down and drained to terminal
/// evidence while `b` keeps running; then `b` ends. Both are cleaned up.
pub fn independence(setup: &Setup) -> Value {
    let api = setup.api;
    let (owner_a, owner_b) = (Owner::spawn("owner-a"), Owner::spawn("owner-b"));
    let (create_a, a) = create(setup, &owner_a, &setup.input["a"]);
    let (create_b, b) = create(setup, &owner_b, &setup.input["b"]);
    let (a, b) = (a.expect("a is created"), b.expect("b is created"));
    let mut report = Map::new();
    report.insert("create_a".into(), create_a);
    report.insert("create_b".into(), create_b);
    let ran_a = owner_a.start(move || api.run(a));
    let ran_b = owner_b.start(move || api.run(b));
    report.insert("snapshot_a".into(), json!(api.snapshot(a)));
    report.insert("snapshot_b".into(), json!(api.snapshot(b)));
    report.insert("shutdown_a".into(), api.shutdown(a));
    let descriptor_a = api.notification(a).1;
    report.insert("drains_a".into(), drain_to_terminal(api, a, descriptor_a));
    report.insert("run_a".into(), json!(wait(&ran_a)));
    let (call, drain) = api.drain(b);
    report.insert(
        "drain_b_while_a_ended".into(),
        json!([call, drain.observed]),
    );
    report.insert("snapshot_b_while_a_ended".into(), json!(api.snapshot(b)));
    report.insert("release_a".into(), release(api, &owner_a, a));
    report.insert("shutdown_b".into(), api.shutdown(b));
    let descriptor_b = api.notification(b).1;
    report.insert("drains_b".into(), drain_to_terminal(api, b, descriptor_b));
    report.insert("run_b".into(), json!(wait(&ran_b)));
    report.insert("release_b".into(), release(api, &owner_b, b));
    report.insert("joined".into(), json!([owner_a.join(), owner_b.join()]));
    report.insert("ledger".into(), api.ledger.report());
    Value::Object(report)
}

fn release(api: Api, owner: &Owner, handles: Handles) -> Value {
    json!({
        "release_communication": api.release_communication(handles),
        "release_control": api.release_control(handles),
        "destroy": owner.call(move || api.destroy(handles)),
    })
}

/// `ambient`: the process environment, working directory, signal
/// dispositions and mask, and standard streams, before and after a complete
/// lifecycle (with a contained panic, when one is injected).
pub fn ambient(setup: &Setup) -> Value {
    let before = probe::ambient();
    let api = setup.api;
    let owner = Owner::spawn("owner");
    let (create, handles) = create(setup, &owner, &setup.input["startup"]);
    let handles = handles.expect("create succeeds");
    let snapshot = api.snapshot(handles);
    let cleanup = clean_up(api, &owner, handles);
    let joined = owner.join();
    json!({
        "create": create,
        "snapshot": snapshot,
        "cleanup": cleanup,
        "joined": joined,
        "before": before,
        "after": probe::ambient(),
    })
}

/// The racing phases, which only the main thread advances: the stream is
/// live, then shutdown is requested, then a poll has seen it end.
const PHASES: [&str; 3] = ["running", "ending", "ended"];
const ENDING: usize = 1;
const ENDED: usize = 2;
const STOPPED: usize = 3;

/// Where the race stands, shared by the main thread, the racer and the
/// poller.
#[derive(Default)]
struct Race {
    /// An index into [`PHASES`], or [`STOPPED`].
    phase: AtomicUsize,
    /// The racer's rounds, by the phase each started in.
    rounds: [AtomicU64; 3],
    /// Polls that found the descriptor not readable.
    quiet_polls: AtomicU64,
}

/// Polls the borrowed descriptor without waiting until it is readable,
/// bounded, counting the polls that find it not readable.
fn poller(descriptor: i32, race: Arc<Race>) -> thread::JoinHandle<bool> {
    thread::Builder::new()
        .name("poller".into())
        .spawn(move || {
            probe::until(|| {
                let readable = probe::readable(descriptor);
                if !readable {
                    race.quiet_polls.fetch_add(1, Ordering::SeqCst);
                }
                readable
            })
        })
        .expect("the poller starts")
}

/// Uses the communication handle in rounds until stopped, recording each
/// phase's calls, failed calls and the descriptors returned.
fn racer(api: Api, handles: Handles, race: Arc<Race>) -> thread::JoinHandle<Value> {
    thread::Builder::new()
        .name("racer".into())
        .spawn(move || {
            let mut phases = [
                (0u64, 0u64, Vec::new()),
                (0, 0, Vec::new()),
                (0, 0, Vec::new()),
            ];
            loop {
                let phase = race.phase.load(Ordering::SeqCst);
                if phase == STOPPED {
                    break;
                }
                let (notification, descriptor) = api.notification(handles);
                let (snapshot, _) = api.snapshot(handles);
                let (calls, failures, descriptors) = &mut phases[phase];
                *calls += 2;
                *failures += u64::from(notification["code"] != native::SUCCESS)
                    + u64::from(snapshot["code"] != native::SUCCESS);
                if !descriptors.contains(&descriptor) {
                    descriptors.push(descriptor);
                }
                race.rounds[phase].fetch_add(1, Ordering::SeqCst);
            }
            let phases = PHASES
                .iter()
                .zip(phases)
                .map(|(name, (calls, failures, descriptors))| {
                    (
                        name.to_string(),
                        json!({ "calls": calls, "failures": failures, "descriptors": descriptors }),
                    )
                });
            Value::Object(phases.collect())
        })
        .expect("the racer starts")
}

/// `race`: one thread polls the borrowed descriptor and another keeps using
/// the communication handle while the stream ends. The main thread schedules
/// the overlap: shutdown only once both have made progress on the live
/// stream, and the racer stops only after a round that started once a poll
/// had seen the end. Polling must see the end, every call must succeed, and
/// the descriptor must stay the library's, open until destroy and closed by
/// it.
pub fn race(setup: &Setup) -> Value {
    let api = setup.api;
    let owner = Owner::spawn("owner");
    let (create, handles) = create(setup, &owner, &setup.input["startup"]);
    let handles = handles.expect("create succeeds");
    let mut report = Map::new();
    report.insert("create".into(), create);
    let (first, descriptor) = api.notification(handles);
    let (second, again) = api.notification(handles);
    report.insert(
        "notifications".into(),
        json!([first, second, descriptor == again]),
    );
    report.insert("descriptor".into(), probe::descriptor(descriptor));
    report.insert("descriptor_number".into(), json!(descriptor));
    report.insert("snapshot".into(), json!(api.snapshot(handles)));
    report.insert(
        "readable_after_create".into(),
        json!(probe::readable(descriptor)),
    );
    let ran = owner.start(move || api.run(handles));
    let race = Arc::new(Race::default());
    let racing = racer(api, handles, Arc::clone(&race));
    let polling = poller(descriptor, Arc::clone(&race));
    let live = probe::until(|| {
        race.rounds[0].load(Ordering::SeqCst) > 0 && race.quiet_polls.load(Ordering::SeqCst) > 0
    });
    race.phase.store(ENDING, Ordering::SeqCst);
    report.insert("shutdown".into(), api.shutdown(handles));
    report.insert(
        "poller_saw_end".into(),
        json!(polling.join().expect("the poller ends")),
    );
    race.phase.store(ENDED, Ordering::SeqCst);
    let ended = probe::until(|| race.rounds[ENDED].load(Ordering::SeqCst) > 0);
    race.phase.store(STOPPED, Ordering::SeqCst);
    report.insert("scheduled".into(), json!({ "live": live, "ended": ended }));
    report.insert(
        "quiet_polls".into(),
        json!(race.quiet_polls.load(Ordering::SeqCst)),
    );
    report.insert("racer".into(), racing.join().expect("the racer ends"));
    report.insert(
        "readable_twice".into(),
        json!([probe::readable(descriptor), probe::readable(descriptor)]),
    );
    report.insert("drains".into(), drain_to_terminal(api, handles, descriptor));
    report.insert(
        "readable_after_terminal".into(),
        json!(probe::readable(descriptor)),
    );
    report.insert("run".into(), json!(wait(&ran)));
    report.insert(
        "release_communication".into(),
        api.release_communication(handles),
    );
    report.insert("release_control".into(), api.release_control(handles));
    report.insert(
        "descriptor_before_destroy".into(),
        probe::descriptor(descriptor),
    );
    report.insert("destroy".into(), owner.call(move || api.destroy(handles)));
    report.insert(
        "descriptor_after_destroy".into(),
        probe::descriptor(descriptor),
    );
    report.insert("joined".into(), json!(owner.join()));
    report.insert("ledger".into(), api.ledger.report());
    Value::Object(report)
}
