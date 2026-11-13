//! Library layer (#93): what `submit` admits, and how an instance bounds
//! requests, records and deliveries, through the installed client library's
//! frozen C ABI. Every request lives in caller memory the host scrambles and
//! unmaps as soon as `submit` returns, so anything the library kept would
//! fault. Requests accepted before `run` and cancelled by shutdown yield
//! records with no daemon involved. Expected values come from the native
//! header and the native contract.

#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use support::{Package, passive::*};

/// Requests of public ID 7 that `submit` rejects, each with its result code
/// and stable error code.
fn rejections() -> Vec<(Value, (u64, &'static str))> {
    let invalid = (INVALID_ARGUMENT, "invalid_argument");
    let unsupported = (UNSUPPORTED_OPERATION, "unsupported_operation");
    let unrelayed = (UNSUPPORTED_OPERATION, "unsupported_capability");
    let monitor = |open: Value| json!({ "tag": "open", "request_id": 7, "open": open });
    let lists = |required: Value, optional: Value| {
        monitor(json!({ "mode": 1, "required": required, "optional": optional }))
    };
    let mut rejected = vec![
        (Value::Null, invalid),
        (json!({ "tag": 0, "request_id": 7 }), invalid),
        (json!({ "tag": 11, "request_id": 7 }), invalid),
        (json!({ "tag": u32::MAX, "request_id": 7 }), invalid),
        (
            json!({ "tag": "open", "reserved": 1, "request_id": 7, "open": { "mode": 1 } }),
            invalid,
        ),
        (
            json!({ "tag": "open", "request_id": 0, "open": { "mode": 1 } }),
            (INVALID_REQUEST_ID, "invalid_request_id"),
        ),
        (
            monitor(json!({ "mode": 2, "geometry": [80, 24] })),
            unsupported,
        ),
        (monitor(json!({ "mode": 0 })), invalid),
        (monitor(json!({ "mode": 3 })), invalid),
        (monitor(json!({ "mode": 1, "geometry": [80, 0] })), invalid),
        (monitor(json!({ "mode": 1, "geometry": [0, 24] })), invalid),
        (monitor(json!({ "mode": 1, "reserved": 1 })), invalid),
        (lists(json!({ "null": 1 }), json!([])), invalid),
        (lists(json!(REQUIRED), json!({ "dangling": 0 })), invalid),
        (lists(json!({ "dangling": u64::MAX }), json!([])), invalid),
        (lists(json!(["monitor-v1", ""]), json!([])), invalid),
        (
            lists(json!(["monitor-v1", { "hex": "ff" }]), json!([])),
            invalid,
        ),
        (
            lists(json!(["monitor-v1", { "null": 3 }]), json!([])),
            invalid,
        ),
        (
            lists(json!(["monitor-v1", { "dangling": true }]), json!([])),
            invalid,
        ),
        (
            lists(json!(["monitor-v1", "monitor-v1"]), json!([])),
            invalid,
        ),
        (
            lists(json!(REQUIRED), json!(["title-v1", "monitor-v1"])),
            invalid,
        ),
        (
            lists(json!(["monitor-v1", "status-v1"]), json!([])),
            unrelayed,
        ),
        (lists(json!(["x-later-v9"]), json!([])), unrelayed),
        (
            json!({ "tag": "leave", "request_id": 7, "generation": 0 }),
            invalid,
        ),
    ];
    for tag in [2, 3, 4, 5, 6, 7, 9, 10] {
        rejected.push((json!({ "tag": tag, "request_id": 7 }), unsupported));
    }
    rejected
}

/// `request` with every byte it does not set, inactive union storage
/// included, 0xa5.
fn filled(mut request: Value) -> Value {
    request["fill"] = json!(0xa5);
    request
}

/// The two requests a `cancelled` scenario accepted, each answered
/// `cancelled` by shutdown before any of them executed, then terminal
/// evidence of an instance that stopped with no role.
fn assert_cancelled(finish: &Value, ids: &[u64]) -> Vec<Value> {
    let (records, terminal) = finished(finish);
    assert_eq!(records.len(), ids.len(), "{records:#?}");
    for (record, id) in records.iter().zip(ids) {
        assert_rejected(record, 1, *id, ORIGIN_LIBRARY, "cancelled");
    }
    let stopped = (CLIENT_STOPPED, RETRY_NONE, LEAVE_NOT_REQUIRED);
    assert_terminal(terminal, 1, stopped, None);
    records
}

/// COM-02 Library (unknown native tags and nonzero reserved fields are
/// rejected; inactive union storage is ignored) and COM-03 Library (invalid
/// and closed rejections are synchronous, consume no request ID and retain
/// no arguments): each invalid request of public ID 7, or a null one, is
/// rejected with its stable code, and ID 7 is accepted afterwards; an Open
/// (with a one-byte optional token it does not relay) and a Leave whose
/// every byte they do not set is 0xa5 are accepted; an ID
/// that does not increase is `invalid_request_id`; only the accepted
/// requests are ever charged; after shutdown `submit` is `closed`; and each
/// accepted request that never executed is answered `cancelled`.
#[test]
fn installed_library_admits_only_valid_requests_and_rejections_cost_nothing() {
    let package = Package::new();
    let root = root(&package);
    let rejected = rejections();
    let mut steps = vec![json!({ "do": "create" })];
    steps.extend(
        rejected
            .iter()
            .map(|(request, _)| json!({ "do": "submit", "request": request })),
    );
    steps.extend([
        json!({ "do": "submit", "request": filled(open(7, &REQUIRED, &["title-v1", "x"])) }),
        json!({ "do": "submit", "request": filled(leave(9, 1)) }),
        json!({ "do": "submit", "request": open(9, &REQUIRED, &[]) }),
        json!({ "do": "submit", "request": leave(8, 1) }),
        json!({ "do": "snapshot" }),
        json!({ "do": "shutdown" }),
        json!({ "do": "submit", "request": leave(10, 1) }),
        json!({ "do": "run" }),
        json!({ "do": "finish" }),
    ]);
    let report = script(&package, &root, json!(steps));
    assert_success(&step(&report, 0)["call"]);
    for (index, (request, (code, error_code))) in rejected.iter().enumerate() {
        let call = step(&report, index + 1);
        assert_eq!(call["code"], *code, "{request:#}: {call:#}");
        assert_failure(call, *code, error_code);
    }
    let accepted = rejected.len() + 1;
    assert_success(step(&report, accepted));
    assert_success(step(&report, accepted + 1));
    for repeated in [accepted + 2, accepted + 3] {
        assert_failure(
            step(&report, repeated),
            INVALID_REQUEST_ID,
            "invalid_request_id",
        );
    }
    let queued = &step(&report, accepted + 4)["output"];
    let bytes = charge(&["daemon-wire-v1", "monitor-v1", "title-v1", "x"]) + REQUEST;
    assert_eq!(queued["request_records"], 2, "{queued:#}");
    assert_eq!(queued["peak_request_records"], 2, "{queued:#}");
    assert_eq!(queued["request_bytes"], bytes, "{queued:#}");
    assert_eq!(queued["peak_request_bytes"], bytes, "{queued:#}");
    assert_eq!(queued["operations"], 2, "{queued:#}");
    assert_success(step(&report, accepted + 5));
    assert_failure(step(&report, accepted + 6), CLOSED, "closed");
    assert_cancelled(step(&report, accepted + 8), &[7, 9]);
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}

/// COM-03 Library (full rejections are synchronous and consume no ID; each
/// accepted request reserves room for its outcome): with one limit set to
/// exactly an Open and a Leave (`request_records` and `operations` two,
/// `request_bytes` both requests' charges, and the outcome reserve of a
/// record and `metadata_record_bytes` each in `event_records` and
/// `event_bytes`), both are accepted at the limit and a third request is
/// `queue_full`. An Open too large for `request_bytes` on its own is
/// `queue_full`, and its ID is then accepted; one that fills it exactly is
/// accepted, and a Leave after it is `queue_full`. Shutdown answers each
/// accepted request `cancelled`.
#[test]
fn installed_library_bounds_requests_by_every_queue_limit() {
    let package = Package::new();
    let root = root(&package);
    let both = charge(&SELECTED) + REQUEST;
    let reserve = RECORD + 1024;
    let cases = [
        ("request_records", json!({ "request_records": 2 })),
        ("request_bytes", json!({ "request_bytes": both })),
        ("operations", json!({ "operations": 2 })),
        ("event_records", json!({ "event_records": 2 })),
        (
            "event_bytes",
            json!({ "event_bytes": 2 * reserve, "metadata_record_bytes": 1024 }),
        ),
    ];
    let unfit = "x".repeat(both as usize);
    let mut steps = Vec::new();
    let mut starts = Vec::new();
    for (name, limits) in &cases {
        let submit =
            |request: Value| json!({ "do": "submit", "instance": name, "request": request });
        steps.push(json!({ "do": "create", "instance": name, "startup": { "limits": limits } }));
        if *name == "request_bytes" {
            steps.push(submit(open(1, &REQUIRED, &["title-v1", &unfit])));
        }
        starts.push(steps.len());
        steps.extend([
            submit(open(1, &REQUIRED, &["title-v1"])),
            submit(leave(2, 1)),
            submit(leave(3, 1)),
            json!({ "do": "snapshot", "instance": name }),
            json!({ "do": "shutdown", "instance": name }),
            json!({ "do": "run", "instance": name }),
            json!({ "do": "finish", "instance": name }),
        ]);
    }
    let single = steps.len();
    let exact = json!({ "request_bytes": charge(&SELECTED) });
    steps.extend([
        json!({ "do": "create", "instance": "single", "startup": { "limits": exact } }),
        json!({ "do": "submit", "instance": "single", "request": open(1, &REQUIRED, &["title-v1"]) }),
        json!({ "do": "submit", "instance": "single", "request": leave(2, 1) }),
        json!({ "do": "shutdown", "instance": "single" }),
        json!({ "do": "run", "instance": "single" }),
        json!({ "do": "finish", "instance": "single" }),
    ]);
    let report = script(&package, &root, json!(steps));
    let oversized = step(&report, starts[1] - 1);
    assert_failure(oversized, FULL, "queue_full");
    assert_success(step(&report, single + 1));
    assert_failure(step(&report, single + 2), FULL, "queue_full");
    assert_cancelled(step(&report, single + 5), &[1]);
    for ((name, _), start) in cases.iter().zip(starts) {
        assert_success(step(&report, start));
        assert_success(step(&report, start + 1));
        let full = step(&report, start + 2);
        assert_eq!(full["code"], FULL, "{name}: {full:#}");
        assert_failure(full, FULL, "queue_full");
        let queued = &step(&report, start + 3)["output"];
        assert_eq!(queued["request_records"], 2, "{name}: {queued:#}");
        assert_eq!(queued["request_bytes"], both, "{name}: {queued:#}");
        assert_eq!(queued["operations"], 2, "{name}: {queued:#}");
        assert_cancelled(step(&report, start + 6), &[1, 2]);
    }
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}

/// Steps that queue three `cancelled` Responses on instance `name` with
/// `limits`, before anything drains them.
fn three_cancelled(name: &str, limits: Value) -> Vec<Value> {
    let submit = |request: Value| json!({ "do": "submit", "instance": name, "request": request });
    vec![
        json!({ "do": "create", "instance": name, "startup": { "limits": limits } }),
        submit(open(1, &REQUIRED, &["title-v1"])),
        submit(leave(2, 1)),
        submit(leave(3, 1)),
        json!({ "do": "shutdown", "instance": name }),
        json!({ "do": "run", "instance": name }),
        json!({ "do": "wait_run", "instance": name }),
    ]
}

/// The record counts of each delivery a drain step took.
fn batches(drain: &Value) -> Vec<u64> {
    drain["drains"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|drain| drain["drained"]["kind"] == DELIVERY)
        .map(|drain| drain["drained"]["count"].as_u64().unwrap())
        .collect()
}

/// LIB-04 Library (deliveries keep their lifetimes until released through
/// the owning library, drain never reclaims live delivery storage, and
/// every delivery pairs with its release): with one delivery allowed and two
/// records per drain, three queued Responses come as a delivery of two,
/// then `queue_full` while it is outstanding, its records unchanged when
/// observed again and still charged, then the third after its release.
/// Each `cancelled` Response is charged its record alone: its error text is
/// the library's own static text, not a copy.
#[test]
fn installed_library_holds_each_delivery_until_released() {
    let package = Package::new();
    let root = root(&package);
    let mut steps = three_cancelled("a", json!({ "deliveries": 1, "drain_records": 2 }));
    steps.extend([
        json!({ "do": "snapshot", "instance": "a" }),
        json!({ "do": "drain", "instance": "a", "keep": true }),
        json!({ "do": "drain", "instance": "a" }),
        json!({ "do": "observe", "instance": "a" }),
        json!({ "do": "snapshot", "instance": "a" }),
        json!({ "do": "release", "instance": "a" }),
        json!({ "do": "drain", "instance": "a", "keep": true }),
        json!({ "do": "finish", "instance": "a" }),
    ]);
    let report = script(&package, &root, json!(steps));
    let queued = &step(&report, 7)["output"];
    assert_eq!(queued["event_records"], 3, "{queued:#}");
    assert_eq!(queued["event_bytes"], 3 * RECORD, "{queued:#}");
    assert_eq!(queued["deliveries"], 0, "{queued:#}");
    let first = step(&report, 8);
    assert_eq!(batches(first), [2]);
    let kept: Vec<Value> = first["records"].as_array().unwrap().clone();
    let refused = &step(&report, 9)["drains"][0];
    assert_failure(&refused["call"], FULL, "queue_full");
    assert_eq!(refused["drained"]["output"], "zeroed", "{refused:#}");
    let observed = step(&report, 10).as_array().unwrap();
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0]["records"], json!(kept));
    let live = &step(&report, 11)["output"];
    assert_eq!(live["deliveries"], 1, "{live:#}");
    assert_eq!(live["event_records"], 3, "{live:#}");
    assert_eq!(live["event_bytes"], 3 * RECORD, "{live:#}");
    assert_success(&step(&report, 12)[0]);
    assert_eq!(batches(step(&report, 13)), [1]);
    let mut records = kept;
    records.extend(step(&report, 13)["records"].as_array().unwrap().clone());
    let ids: Vec<_> = records
        .iter()
        .map(|record| record["response"]["request_id"].clone())
        .collect();
    assert_eq!(ids, [json!(1), json!(2), json!(3)]);
    assert_cancelled(step(&report, 14), &[]);
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}

/// LIB-04 Library (a delivery stops before `drain_payload_bytes`, and
/// always holds a record): four 10-byte titles, queued while nothing
/// drains, come two to a delivery at a 20-byte ceiling, one at a time at 19,
/// and one at a time at 1, where a record alone exceeds it.
#[test]
fn installed_library_batches_records_within_drain_payload_bytes() {
    let package = Package::new();
    let root = root(&package);
    let titles: Vec<String> = (1..=4).map(|index| format!("title-{index:04}")).collect();
    let cases = [("exact", 20), ("under", 19), ("single", 1)];
    let mut steps = Vec::new();
    for (name, ceiling) in cases {
        let go = root.join(format!("go-{name}"));
        let mut actions = vec![opened(&SELECTED), initial(json!("t"))];
        actions.push(json!({ "wait": go.to_str().unwrap() }));
        actions.extend(titles.iter().map(|title| title_changed(json!(title))));
        let peer = json!({ "open": actions, "leave": [leave_answer("ok", false, Value::Null)] });
        let limits = json!({ "drain_payload_bytes": ceiling });
        steps.extend([
            json!({ "do": "create", "instance": name, "peer": peer, "startup": { "limits": limits } }),
            json!({ "do": "submit", "instance": name, "request": open(1, &REQUIRED, &["title-v1"]) }),
            json!({ "do": "run", "instance": name }),
            json!({ "do": "drain", "instance": name, "until": { "tag": INITIAL_METADATA } }),
            json!({ "do": "touch", "path": go.to_str().unwrap() }),
            json!({ "do": "await_snapshot", "instance": name, "gauge": "event_records", "value": 4 }),
            json!({ "do": "drain", "instance": name, "until": { "records": 4 } }),
            json!({ "do": "shutdown", "instance": name }),
            json!({ "do": "finish", "instance": name }),
        ]);
    }
    let report = script(&package, &root, json!(steps));
    let expected: [&[u64]; 3] = [&[2, 2], &[1, 1, 1, 1], &[1, 1, 1, 1]];
    for (index, expected) in expected.into_iter().enumerate() {
        let start = 9 * index;
        let drained = step(&report, start + 6);
        assert_eq!(batches(drained), expected, "{drained:#}");
        let values: Vec<Value> = drained["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record["title"]["value"].clone())
            .collect();
        assert_eq!(values, json!(titles).as_array().unwrap().clone());
        let (rest, terminal) = finished(step(&report, start + 8));
        assert!(rest.is_empty(), "{rest:#?}");
        assert_left(terminal, 1);
    }
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}

/// COM-02 Library (frames split anywhere or coalesced are reassembled
/// exactly) and LIB-04 Library (the event queue is bounded, and the daemon's
/// records wait for room instead of being dropped or overrunning it): a
/// fake daemon sends its Open response and Initial metadata a byte at a
/// time, then eight Title changed records in one write, to an instance
/// whose event queue holds three records. The queue fills to three and
/// stays there until drained; then every title arrives, in order, with no
/// gap, and the queue never held more.
#[test]
fn installed_library_holds_daemon_records_while_the_event_queue_is_full() {
    let package = Package::new();
    let root = root(&package);
    let titles: Vec<Value> = (1..=8)
        .map(|index| title_changed(json!(format!("title {index}"))))
        .collect();
    let peer = json!({
        "open": [
            { "bytewise": opened(&SELECTED) },
            { "bytewise": initial(json!("t")) },
            { "together": titles },
        ],
        "leave": [leave_answer("ok", false, Value::Null)],
    });
    let steps = json!([
        { "do": "create", "peer": peer, "startup": { "limits": { "event_records": 3 } } },
        { "do": "submit", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run" },
        { "do": "await_snapshot", "gauge": "event_records", "value": 3 },
        { "do": "await_peer", "frames": 2 },
        { "do": "snapshot" },
        { "do": "drain", "until": { "records": 10 } },
        { "do": "snapshot" },
        { "do": "shutdown" },
        { "do": "finish" },
    ]);
    let report = script(&package, &root, steps);
    for index in [3, 5] {
        let full = &step(&report, index)["output"];
        assert_eq!(full["event_records"], 3, "{full:#}");
    }
    let records = step(&report, 6)["records"].as_array().unwrap();
    assert_eq!(records.len(), 10, "{records:#?}");
    assert_eq!(assert_response(&records[0], 1, 1, OK)["result_tag"], OPENED);
    assert_eq!(records[1]["metadata"]["title"], title(VALUE, Some("t")));
    for (index, record) in records[2..].iter().enumerate() {
        assert_header(record, TITLE, 1);
        let expected = format!("title {}", index + 1);
        assert_eq!(record["title"], title(VALUE, Some(&expected)), "{record:#}");
    }
    let drained = &step(&report, 7)["output"];
    assert_eq!(drained["peak_event_records"], 3, "{drained:#}");
    let (rest, terminal) = finished(step(&report, 9));
    assert!(rest.is_empty(), "{rest:#?}");
    assert_left(terminal, 1);
    assert_eq!(kinds(&report["peers"]["a"]), ["hello", "open", "leave"]);
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}

/// LIB-04 Library (each record is charged its header and the bytes it owns,
/// nothing else, until released): with every record still queued, the
/// event queue holds exactly the records' headers plus the Open's selected
/// token array, the Initial metadata's title and signal name copies, the
/// titles' copies (none for an unset one) and the copied text of the
/// daemon's error rejecting a Leave; a Response's own static text costs
/// nothing.
#[test]
fn installed_library_charges_each_record_its_header_and_the_bytes_it_owns() {
    let package = Package::new();
    let root = root(&package);
    let named = send(
        0x41,
        json!({
            "title": { "available": true, "value": "longer title" },
            "status": { "available": false, "value": null },
            "progress": { "available": false, "value": null },
            "command_result": { "kind": "signal", "number": 15, "name": "SIGTERM" },
        }),
    );
    let rejected = leave_answer("rejected", false, daemon_error("invalid_state"));
    let ok = leave_answer("ok", false, Value::Null);
    let error = ("invalid_state".len() + "fake daemon".len()) as u64;
    let cases = [
        (
            "named",
            named,
            ("longer title".len() + "SIGTERM".len()) as u64,
        ),
        ("plain", initial(json!("plain")), "plain".len() as u64),
    ];
    let mut steps = Vec::new();
    for (name, metadata, _) in &cases {
        let peer = json!({
            "open": [
                opened(&SELECTED),
                metadata,
                title_changed(Value::Null),
                title_changed(json!("abc")),
            ],
            "leaves": [[rejected], [ok]],
        });
        let submit =
            |request: Value| json!({ "do": "submit", "instance": name, "request": request });
        steps.extend([
            json!({ "do": "create", "instance": name, "peer": peer }),
            submit(open(1, &REQUIRED, &["title-v1"])),
            submit(leave(2, 1)),
            json!({ "do": "run", "instance": name }),
            json!({ "do": "await_snapshot", "instance": name, "gauge": "event_records", "value": 5 }),
            json!({ "do": "drain", "instance": name, "until": { "records": 5 } }),
            json!({ "do": "shutdown", "instance": name }),
            json!({ "do": "finish", "instance": name }),
        ]);
    }
    let report = script(&package, &root, json!(steps));
    for (index, (name, _, metadata)) in cases.iter().enumerate() {
        let start = 8 * index;
        let queued = &step(&report, start + 4)["output"];
        let owned = 3 * TOKEN + metadata + 3 + error;
        assert_eq!(queued["event_records"], 5, "{name}: {queued:#}");
        assert_eq!(
            queued["event_bytes"],
            5 * RECORD + owned,
            "{name}: {queued:#}"
        );
        let records = step(&report, start + 5)["records"].as_array().unwrap();
        let tags: Vec<_> = records.iter().map(|record| record["tag"].clone()).collect();
        assert_eq!(
            tags,
            [RESPONSE, INITIAL_METADATA, TITLE, TITLE, RESPONSE].map(Value::from),
            "{name}"
        );
        assert_eq!(records[2]["title"], title(UNSET, None), "{name}");
        assert_rejected(&records[4], 1, 2, ORIGIN_DAEMON, "invalid_state");
        let (rest, terminal) = finished(step(&report, start + 7));
        assert!(rest.is_empty(), "{name}: {rest:#?}");
        assert_left(terminal, 1);
    }
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}
