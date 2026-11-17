//! Library layer and Fault (#99): the client library's limits and
//! accounting at their boundaries, through the installed library's frozen C
//! ABI against the host's fake daemon peer. Counters are brought near
//! exhaustion by the hooked library's `CLIMON2_LIBRARY_FAULT` (`counters:`
//! and `wire-ids:`); release archives have no hooks, so those cases run
//! only against the hooked build. Expected values are written by hand from
//! the native header, the native contract and the operational limits.

#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use std::{path::Path, thread, time::Duration};
use support::{
    Package,
    control::*,
    library::{observe_within, scenario_command},
    passive::*,
};

const MAX: u64 = u64::MAX;

/// An Input of `hex` under an authority no drain has named: `submit` admits
/// it without a role, and shutdown cancels it.
fn unrouted_input(id: u64, hex: &str) -> Value {
    json!({ "tag": "input", "request_id": id, "authority": [1, 2, 2],
            "bytes": { "hex": hex } })
}

/// A snapshot that fails because a monotonic counter is exhausted: an
/// explicit `CM1_RESOURCE_LIMIT`, its output cleared.
fn assert_exhausted(snapshot: &Value) {
    assert_failure(&snapshot["call"], RESOURCE_LIMIT, "resource_limit");
    assert_eq!(
        snapshot["output"],
        json!({ "output": "zeroed" }),
        "{snapshot:#}"
    );
}

/// The four monotonic counters a successful snapshot reports.
fn counters(snapshot: &Value) -> [u64; 4] {
    assert_success(&snapshot["call"]);
    let output = &snapshot["output"];
    [
        "input_bytes_accepted",
        "display_bytes_delivered",
        "records_drained",
        "deliveries_released",
    ]
    .map(|name| output[name].as_u64().unwrap())
}

/// Steps that queue two `cancelled` Responses with one record per
/// delivery, drain and release one delivery (`at`), then the other
/// (`past`), each followed by a snapshot.
fn drained_one_by_one() -> Vec<Value> {
    vec![
        json!({ "do": "create", "startup": { "limits": { "drain_records": 1 } } }),
        json!({ "do": "submit", "request": leave(1, 1) }),
        json!({ "do": "submit", "request": leave(2, 1) }),
        json!({ "do": "shutdown" }),
        json!({ "do": "run" }),
        json!({ "do": "wait_run" }),
        json!({ "do": "drain", "until": { "records": 1 } }),
        json!({ "do": "snapshot", "as": "at" }),
        json!({ "do": "drain", "until": { "records": 1 } }),
        json!({ "do": "snapshot", "as": "past" }),
        json!({ "do": "finish", "as": "finished" }),
    ]
}

/// LIM-05 and LIM-02 Library and Fault (`counters:` hook): the snapshot's
/// monotonic counters of accepted input bytes, delivered display bytes,
/// drained records and released deliveries each reach `u64::MAX` and are
/// reported there, at the limit; one byte, record or delivery past it never
/// wraps: every later snapshot is an explicit `CM1_RESOURCE_LIMIT` with
/// its output cleared, while submit, drain, release and shutdown go on and
/// the stream ends with its terminal evidence.
#[test]
fn installed_library_snapshot_counters_never_wrap_and_fail_explicitly() {
    let package = Package::new();
    let root = root(&package);
    let steps = json!([
        { "do": "create" },
        { "do": "snapshot" },
        { "do": "submit", "request": unrouted_input(1, "414243") },
        { "do": "snapshot" },
        { "do": "submit", "request": unrouted_input(2, "44") },
        { "do": "snapshot" },
        { "do": "shutdown" },
        { "do": "run" },
        { "do": "finish" },
    ]);
    let fault = format!("counters:{},0,0,0", MAX - 3);
    let Some(report) = script_fault(&package, &root, steps, &fault) else {
        return;
    };
    assert_eq!(counters(step(&report, 1)), [MAX - 3, 0, 0, 0]);
    assert_eq!(counters(step(&report, 3)), [MAX, 0, 0, 0]);
    assert_success(step(&report, 4));
    assert_exhausted(step(&report, 5));
    let (records, terminal) = finished(step(&report, 8));
    assert_rejected(&records[0], 1, 1, ORIGIN_LIBRARY, "cancelled");
    assert_rejected(&records[1], 1, 2, ORIGIN_LIBRARY, "cancelled");
    let stopped = (CLIENT_STOPPED, RETRY_NONE, LEAVE_NOT_REQUIRED);
    assert_terminal(terminal, 1, stopped, None);

    let peer = attached_peer(json!({ "ready": [shown(PERIOD, 2, "42")] }));
    let then = [
        json!({ "do": "snapshot", "as": "at" }),
        json!({ "do": "submit", "request": ready(2, 1, CURRENT) }),
        json!({ "do": "drain", "until": { "display": "42" } }),
        json!({ "do": "snapshot", "as": "past" }),
        json!({ "do": "shutdown" }),
    ];
    let mut plan = Plan::new();
    plan.add("display", attached("display", peer, json!({}), &then));
    let fault = format!("counters:0,{},0,0", MAX - 1);
    let steps = plan.steps.clone();
    let Some(report) = script_fault(&package, &root, json!(steps), &fault) else {
        return;
    };
    let at = counters(labelled(&report, &steps, "at"));
    assert_eq!(at[..2], [0, MAX], "the attach's one Display byte");
    assert_exhausted(labelled(&report, &steps, "past"));
    let (_, terminal) = finished(step(&report, plan.finishes[0].1));
    assert_detached(terminal, 1);

    let steps = drained_one_by_one();
    for (fault, at) in [
        (format!("counters:0,0,{},0", MAX - 1), [0, 0, MAX, 1]),
        (format!("counters:0,0,0,{}", MAX - 1), [0, 0, 1, MAX]),
    ] {
        let Some(report) = script_fault(&package, &root, json!(steps), &fault) else {
            return;
        };
        assert_eq!(counters(labelled(&report, &steps, "at")), at, "{fault}");
        assert_exhausted(labelled(&report, &steps, "past"));
        let (records, terminal) = finished(labelled(&report, &steps, "finished"));
        assert!(records.is_empty(), "{records:#?}");
        assert_terminal(terminal, 1, stopped, None);
    }
    package.assert_clean();
}

/// LIM-02 Library (snapshot measurements): `input_bytes_accepted` counts
/// the bytes of every Input `submit` accepts, from its acceptance, and
/// nothing for one it rejects; `display_bytes_delivered` counts the bytes
/// of every Display drained to the caller, split at `chunk_bytes`;
/// `records_drained` and `deliveries_released` count exactly the records
/// and deliveries the host drained and released. The connection's wire
/// request IDs, which the library assigns, start at one and increase.
#[test]
fn installed_library_counts_accepted_input_and_delivered_display() {
    let package = Package::new();
    let peer = attached_peer(json!({
        "input": [
            answered("input", "ok", json!({ "kind": "input_accepted", "accepted_bytes": 3 }),
                     Value::Null),
            shown(PERIOD, 2, "6263646566"),
        ],
    }));
    let then = [
        json!({ "do": "snapshot", "as": "attached" }),
        json!({ "do": "submit", "request": input(2, 1, CURRENT, "4142434445"),
                "as": "oversized" }),
        json!({ "do": "submit", "request": input(3, 1, CURRENT, "616263"), "as": "accepted" }),
        json!({ "do": "snapshot", "as": "submitted" }),
        json!({ "do": "drain", "until": { "display": "6263646566" }, "as": "shown" }),
        json!({ "do": "snapshot", "as": "drained" }),
        json!({ "do": "shutdown" }),
    ];
    let limits = json!({ "limits": { "chunk_bytes": 4, "drain_payload_bytes": 65536 } });
    let mut plan = Plan::new();
    plan.add("a", attached("a", peer, limits, &then));
    let steps = plan.steps.clone();
    let (report, _) = plan.run(&package);
    let deliveries = |label: &str| {
        let drains = labelled(&report, &steps, label)["drains"]
            .as_array()
            .unwrap();
        let delivered = drains
            .iter()
            .filter(|drain| drain["drained"]["kind"] == DELIVERY);
        delivered.count() as u64
    };
    let attach = deliveries("attached-a");
    assert_eq!(
        counters(labelled(&report, &steps, "attached")),
        [0, 1, 6, attach],
        "Response, Initial metadata, grant, beginning, Display 41, complete"
    );
    assert_failure(
        labelled(&report, &steps, "oversized"),
        INVALID_ARGUMENT,
        "invalid_argument",
    );
    assert_success(labelled(&report, &steps, "accepted"));
    assert_eq!(
        counters(labelled(&report, &steps, "submitted")),
        [3, 1, 6, attach]
    );
    let shown = drained(&report, &steps, "shown");
    assert_eq!(tags(&shown), [RESPONSE, DISPLAY, DISPLAY], "{shown:#?}");
    assert_eq!(
        counters(labelled(&report, &steps, "drained")),
        [3, 1 + 5, 6 + 3, attach + deliveries("shown")]
    );
    let peer = &report["peers"]["a"];
    assert_eq!(kinds(peer), ["hello", "open", "input", "leave"]);
    let frames = &peer["frames"];
    let wire = [1, 3].map(|frame| &frames[frame]["body"]["request_id"]);
    assert_eq!(wire, ["1", "3"], "the Open, then the Leave after Input 2");
    assert_all_released(&report["ledger"]);
}

/// LIM-05 Library (public request IDs): an ID `submit` rejects is not
/// consumed, so `u64::MAX`, refused `queue_full` once, is then accepted at
/// the limit; nothing is past it, so every later ID, `u64::MAX` again or
/// one, is `invalid_request_id` and the high-water mark never wraps.
#[test]
fn installed_library_public_request_ids_end_at_the_limit_without_wrapping() {
    let package = Package::new();
    let root = root(&package);
    let oversized = open(MAX, &REQUIRED, &["title-v1"]);
    let steps = json!([
        { "do": "create", "startup": { "limits": { "request_bytes": REQUEST } } },
        { "do": "submit", "request": oversized },
        { "do": "submit", "request": leave(MAX, 1) },
        { "do": "submit", "request": leave(MAX, 1) },
        { "do": "submit", "request": leave(1, 1) },
        { "do": "shutdown" },
        { "do": "run" },
        { "do": "finish" },
    ]);
    let report = script(&package, &root, steps);
    assert_failure(step(&report, 1), FULL, "queue_full");
    assert_success(step(&report, 2));
    for past in [3, 4] {
        let call = step(&report, past);
        assert_failure(call, INVALID_REQUEST_ID, "invalid_request_id");
    }
    let (records, _) = finished(step(&report, 7));
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_rejected(&records[0], 1, MAX, ORIGIN_LIBRARY, "cancelled");
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}

/// LIM-05 Library and Fault (`wire-ids:` hook): the library assigns each
/// connection's wire request IDs and never wraps them. The Open takes the
/// last one, `u64::MAX`, at the limit; one past it, a Ready and an explicit
/// Leave are each rejected with library `resource_limit`, never sent, and
/// the role goes on; shutdown's own Leave cannot be sent either, so the
/// stream ends with that Leave failed, the client failed with
/// `resource_limit` and no retry. The fake daemon received the Open alone.
#[test]
fn installed_library_refuses_requests_past_the_last_wire_id_unsent() {
    let package = Package::new();
    let then = [
        json!({ "do": "submit", "request": ready(2, 1, CURRENT) }),
        json!({ "do": "submit", "request": leave(3, 1) }),
        json!({ "do": "drain", "until": { "response": 3 }, "as": "refused" }),
        json!({ "do": "shutdown" }),
    ];
    let mut plan = Plan::new();
    plan.add(
        "a",
        attached("a", attached_peer(json!({})), json!({}), &then),
    );
    let steps = plan.steps.clone();
    let root = root(&package);
    let fault = format!("wire-ids:{}", MAX - 1);
    let Some(report) = script_fault(&package, &root, json!(steps), &fault) else {
        return;
    };
    let refused = drained(&report, &steps, "refused");
    assert_eq!(refused.len(), 2, "{refused:#?}");
    assert_rejected(&refused[0], 1, 2, ORIGIN_LIBRARY, "resource_limit");
    assert_rejected(&refused[1], 1, 3, ORIGIN_LIBRARY, "resource_limit");
    let (records, terminal) = finished(step(&report, plan.finishes[0].1));
    assert!(records.is_empty(), "{records:#?}");
    assert_terminal(
        terminal,
        1,
        (CLIENT_FAILED, RETRY_NONE, LEAVE_FAILED),
        Some((ORIGIN_LIBRARY, "resource_limit")),
    );
    let peer = &report["peers"]["a"];
    assert_eq!(kinds(peer), ["hello", "open"]);
    assert_eq!(peer["frames"][1]["body"]["request_id"], MAX.to_string());
    assert_all_released(&report["ledger"]);
}

/// LIM-04 Library and Fault (descriptor exhaustion): with no descriptor
/// left in the process when the Open executes, the library cannot read the
/// session metadata; the Open is rejected with library `resource_limit`,
/// never sent, and the stream ends failed with that error and a passive
/// retry: explicit, never silent, and never reported as the session's
/// absence. Nothing reached the fake daemon.
#[test]
fn installed_library_reports_descriptor_exhaustion_explicitly() {
    let package = Package::new();
    let root = root(&package);
    let steps = json!([
        { "do": "create", "peer": passive_peer(json!([])) },
        { "do": "submit", "request": open(1, &REQUIRED, &[]) },
        { "do": "limit_descriptors", "free": 0 },
        { "do": "run" },
        { "do": "wait_run" },
        { "do": "restore_descriptors" },
        { "do": "finish" },
    ]);
    let report = script(&package, &root, steps);
    let (records, terminal) = finished(step(&report, 6));
    assert_eq!(tags(&records), [RESPONSE, DIAGNOSTIC], "{records:#?}");
    assert_rejected(&records[0], 1, 1, ORIGIN_LIBRARY, "resource_limit");
    assert_diagnostic(&records[1], 1, ORIGIN_LIBRARY, "resource_limit");
    assert_terminal(
        terminal,
        1,
        (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
        Some((ORIGIN_LIBRARY, "resource_limit")),
    );
    assert!(kinds(&report["peers"]["a"]).is_empty());
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}

/// LIM-02 Library (the progress reserve is not room for ordinary work):
/// each accepted request reserves its outcome in the event allowance less
/// the progress reserve, never in the reserve. With room for exactly two
/// outcomes outside a reserve of one record, or of one outcome's bytes,
/// two Leaves are accepted at the limit and a third is `queue_full`,
/// though the whole allowance would hold it.
#[test]
fn installed_library_keeps_the_progress_reserve_from_submitted_work() {
    let package = Package::new();
    let root = root(&package);
    let reserve = support::library::CHARGE + 1024;
    let cases = [
        json!({ "event_records": 3, "progress_records": 1 }),
        json!({ "event_bytes": 3 * reserve, "progress_bytes": reserve,
                "metadata_record_bytes": 1024 }),
    ];
    let mut steps = Vec::new();
    for (index, limits) in cases.iter().enumerate() {
        let name = index.to_string();
        let submit =
            |request: Value| json!({ "do": "submit", "instance": name, "request": request });
        steps.extend([
            json!({ "do": "create", "instance": name, "startup": { "limits": limits } }),
            submit(leave(1, 1)),
            submit(leave(2, 1)),
            submit(leave(3, 1)),
            json!({ "do": "shutdown", "instance": name }),
            json!({ "do": "run", "instance": name }),
            json!({ "do": "finish", "instance": name }),
        ]);
    }
    let report = script(&package, &root, json!(steps));
    for (index, limits) in cases.iter().enumerate() {
        let start = index * 7;
        assert_success(step(&report, start + 1));
        assert_success(step(&report, start + 2));
        let full = step(&report, start + 3);
        assert_eq!(full["code"], FULL, "{limits}: {full:#}");
        assert_failure(full, FULL, "queue_full");
        let (records, _) = finished(step(&report, start + 6));
        assert_eq!(records.len(), 2, "{limits}: {records:#?}");
    }
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}

/// LIM-02 Library and Fault (protected progress stays serviceable): with
/// an event allowance of four records, two of them the progress reserve,
/// and a Ready's outcome reserved, a Display fills the ordinary room at its
/// limit; the Command final after it, a protected record, is queued into
/// the reserve although nothing was drained; the next Display, one past
/// the ordinary room, waits, never dropped, until the caller releases the
/// first delivery, and then arrives in order.
#[test]
fn installed_library_serves_protected_records_from_the_progress_reserve() {
    let package = Package::new();
    let peer = attached_peer(json!({
        "ready": [
            shown(PERIOD, 2, "42"),
            command_final(json!({ "kind": "exit", "code": 0 })),
            shown(PERIOD, 2, "43"),
        ],
    }));
    let then = [
        json!({ "do": "submit", "request": ready(2, 1, CURRENT) }),
        json!({ "do": "await_snapshot", "gauge": "event_records", "value": 2 }),
        json!({ "do": "drain", "keep": true, "as": "reserved" }),
        json!({ "do": "release" }),
        json!({ "do": "drain", "until": { "display": "43" }, "as": "waited" }),
        json!({ "do": "shutdown" }),
    ];
    // The next Display waits until a release, never long enough to stall.
    let limits = json!({ "limits": {
        "event_records": 4, "progress_records": 2, "delivery_stall_ms": 30_000 } });
    let mut plan = Plan::new();
    plan.add("a", attached("a", peer, limits, &then));
    let steps = plan.steps.clone();
    let (report, finishes) = plan.run(&package);
    let reserved = drained(&report, &steps, "reserved");
    assert_eq!(tags(&reserved), [DISPLAY, FINAL], "{reserved:#?}");
    assert_eq!(display(&reserved, 1, [PERIOD, 2]), "42");
    assert_final(&reserved[1], 1, &native_command(EXIT, 0, None));
    let waited = drained(&report, &steps, "waited");
    assert_eq!(tags(&waited), [DISPLAY], "{waited:#?}");
    assert_eq!(display(&waited, 1, [PERIOD, 2]), "43");
    let (_, terminal) = finished(step(&report, finishes[0].1));
    assert_detached(terminal, 1);
    assert_all_released(&report["ledger"]);
}

/// The fake daemon's attach whose restoration sends `steps` after its
/// beginning, in place of one Display and the complete.
fn restoring_peer(steps: Vec<Value>) -> Value {
    let mut records = attach_records(&ATTACHED);
    records.truncate(4);
    records.extend(steps);
    let mut peer = attached_peer(json!({}));
    peer["open"] = json!(records);
    peer
}

/// The restoration stall deadline of every case: many times the longest
/// pause between the fake daemon's Displays (250 ms), so load cannot make
/// a paced restoration stall.
const STALL_MS: u64 = 5_000;

/// How long a restoration case's host may take: the longest case lasts
/// about 15 s unloaded.
const HOST_BOUND: Duration = Duration::from_secs(60);

/// `count` Displays of the restoration, each followed by 250 ms.
fn paced(count: usize) -> Vec<Value> {
    (0..count)
        .flat_map(|_| [shown(PERIOD, 2, "41"), json!({ "pause_ms": 250 })])
        .collect()
}

/// Two Title changed records, 4 s and 8 s after what went before: the
/// second comes 3 s past `STALL_MS` after the restoration's last Display.
fn titled_later() -> [Value; 4] {
    let pause = json!({ "pause_ms": 4_000 });
    [
        pause.clone(),
        title_changed(json!("a")),
        pause,
        title_changed(json!("b")),
    ]
}

/// One restoration deadline case: the fake daemon's records after the
/// beginning, `restoration_total_ms`, what the host drains in turn (each
/// within the host's 10 s bound) and the tags of every record it drains,
/// or none for Displays, however many, then the diagnostic.
struct Restoration {
    name: &'static str,
    records: Vec<Value>,
    total_ms: u64,
    drains: Vec<Value>,
    expected: Option<Vec<u64>>,
}

/// The tags of an attach's records before its restoration's first
/// Display, then `displays` Displays, then `after`.
fn restored_tags(displays: usize, after: &[u64]) -> Vec<u64> {
    let mut tags = vec![RESPONSE, INITIAL_METADATA, GRANTED, RESTORATION_BEGIN];
    tags.extend([DISPLAY].repeat(displays));
    tags.extend(after);
    tags
}

fn until_tag(tag: u64) -> Value {
    json!({ "tag": tag })
}

/// LIM-01 Library and Fault (`restoration_stall_ms` and
/// `restoration_total_ms`): a restoration is timed from its beginning
/// until its complete or a revocation. With a 5 s stall deadline, 28
/// Displays 250 ms apart keep a restoration alive for 7 s, each queued
/// Display restarting the deadline, and it completes; one that stops after
/// a Display fails the connection with library `timeout` at the stall
/// deadline. With a 6 s total deadline, a restoration of two such
/// Displays completes, and one of forty, going on past it, fails the same
/// way at the total deadline though no gap stalls. Either failure calls
/// for a passive retry, and the records before it stay delivered. Once a
/// restoration completes, or is revoked, its deadlines no longer apply:
/// the connection outlives the stall deadline by 3 s with no Display, and
/// the attachment detaches cleanly.
#[test]
fn installed_library_bounds_a_restoration_by_its_stall_and_total_deadlines() {
    let package = Package::new();
    let completed = |displays: usize, after: &[Value]| {
        let mut records = paced(displays);
        records.push(complete(PERIOD, 2));
        records.extend(after.iter().cloned());
        records
    };
    let mut stopped = paced(1);
    stopped.push(json!("hold"));
    let mut revoked_early = paced(1);
    revoked_early.push(revoked(PERIOD, 2, "takeover"));
    revoked_early.extend(titled_later());
    let complete_tag = until_tag(RESTORATION_COMPLETE);
    let titled = [until_tag(TITLE), until_tag(TITLE)];
    let cases = [
        Restoration {
            name: "stall-within",
            records: completed(28, &titled_later()),
            total_ms: 60_000,
            drains: [
                vec![json!({ "tag": DISPLAY, "count": 14 }), complete_tag.clone()],
                titled.to_vec(),
            ]
            .concat(),
            expected: Some(restored_tags(28, &[RESTORATION_COMPLETE, TITLE, TITLE])),
        },
        Restoration {
            name: "stall-past",
            records: stopped,
            total_ms: 60_000,
            drains: vec![complete_tag.clone()],
            expected: Some(restored_tags(1, &[DIAGNOSTIC])),
        },
        Restoration {
            name: "total-within",
            records: completed(2, &[]),
            total_ms: 6_000,
            drains: vec![complete_tag.clone()],
            expected: Some(restored_tags(2, &[RESTORATION_COMPLETE])),
        },
        Restoration {
            name: "total-past",
            records: paced(40),
            total_ms: 6_000,
            drains: vec![complete_tag],
            expected: None,
        },
        Restoration {
            name: "revoked",
            records: revoked_early,
            total_ms: 60_000,
            drains: [vec![until_tag(REVOKED)], titled.to_vec()].concat(),
            expected: Some(restored_tags(1, &[REVOKED, TITLE, TITLE])),
        },
    ];
    let root = root(&package);
    thread::scope(|scope| {
        let hosts: Vec<_> = (cases.iter())
            .map(|case| (case, scope.spawn(|| run_restoration(&package, &root, case))))
            .collect();
        for (case, host) in hosts {
            let (records, terminal) = host.join().unwrap();
            assert_restoration(case, &records, &terminal);
        }
    });
}

/// Runs one restoration case in a host of its own, beside the others: its
/// instance attaches, drains in turn, then shuts down within its deadlines
/// or waits for the run past them. The records it drained, its terminal
/// evidence.
fn run_restoration(package: &Package, root: &Path, case: &Restoration) -> (Vec<Value>, Value) {
    let limits = json!({ "limits": {
        "restoration_stall_ms": STALL_MS, "restoration_total_ms": case.total_ms } });
    let peer = restoring_peer(case.records.clone());
    let mut steps = vec![
        json!({ "do": "create", "peer": peer, "startup": limits }),
        json!({ "do": "submit", "request": interactive(1, 80, 24) }),
        json!({ "do": "run" }),
    ];
    let labels: Vec<String> = (0..case.drains.len())
        .map(|index| format!("{}-{index}", case.name))
        .collect();
    for (until, label) in case.drains.iter().zip(&labels) {
        steps.push(json!({ "do": "drain", "until": until, "as": label }));
    }
    let past = case.name.ends_with("past");
    steps.push(json!({ "do": if past { "wait_run" } else { "shutdown" } }));
    let mut plan = Plan::new();
    plan.add(
        case.name,
        steps
            .into_iter()
            .map(|mut step| {
                step["instance"] = json!(case.name);
                step
            })
            .collect(),
    );
    let input = json!({ "root": root.to_str().unwrap(), "steps": plan.steps });
    let report = observe_within(scenario_command(package, "script", &input), HOST_BOUND);
    assert_all_released(&report["ledger"]);
    let records = (labels.iter())
        .flat_map(|label| drained(&report, &plan.steps, label))
        .collect();
    let terminal = finished(step(&report, plan.finishes[0].1)).1.clone();
    (records, terminal)
}

/// One restoration case's outcome: exactly its records; then, within its
/// deadlines or ended by a revocation, a clean detach, or, past a
/// deadline, `timeout` naming the deadline its name starts with, and a
/// passive retry.
fn assert_restoration(case: &Restoration, records: &[Value], terminal: &Value) {
    let name = case.name;
    let displays = records.len().saturating_sub(5);
    let expected =
        (case.expected.clone()).unwrap_or_else(|| restored_tags(displays, &[DIAGNOSTIC]));
    assert_eq!(tags(records), expected, "{name}: {records:#?}");
    if !name.ends_with("past") {
        return assert_detached(terminal, 1);
    }
    let last = records.last().unwrap();
    assert_diagnostic(last, 1, ORIGIN_LIBRARY, "timeout");
    let message = last["diagnostic"]["message"].as_str().unwrap();
    let deadline = match name {
        "stall-past" => "restoration_stall_ms",
        _ => "restoration_total_ms",
    };
    assert!(message.contains(deadline), "{name}: {message}");
    assert_terminal(
        terminal,
        1,
        (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
        Some((ORIGIN_LIBRARY, "timeout")),
    );
}
