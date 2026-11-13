//! Library layer and Fault (#93): passive monitor connections through the
//! installed client library against the host's fake daemon peer, an
//! adversarial fixture that sends malformed, slow, out-of-order, oversized
//! or truncated traffic and loses responses, and the native admission rules
//! of `submit`. Every case records what reached the fake daemon, so nothing
//! the library should not send (an Open before support is known, a replayed
//! request) can hide. Expected values come from the native header, the
//! native contract and the daemon wire reference.

#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use std::path::Path;
use support::{Package, passive::*};

/// A fake daemon's hello reply with `json` merged over a valid one.
fn hello(json: Value) -> Value {
    json!({ "json": json })
}

/// An instance against its own fake daemon (its `peer` script, with
/// `startup` overrides) that submits `request`, runs, then finishes.
struct Case {
    name: &'static str,
    peer: Value,
    startup: Value,
    request: Value,
}

fn case(name: &'static str, peer: Value, startup: Value, request: &Value) -> Case {
    Case {
        name,
        peer,
        startup,
        request: request.clone(),
    }
}

/// Every case in one script, four steps each.
fn run_cases<'a>(package: &Package, root: &Path, cases: impl Iterator<Item = &'a Case>) -> Value {
    let steps: Vec<Value> = cases
        .flat_map(|case| {
            let name = case.name;
            [
                json!({ "do": "create", "instance": name, "peer": case.peer, "startup": case.startup }),
                json!({ "do": "submit", "instance": name, "request": case.request }),
                json!({ "do": "run", "instance": name }),
                json!({ "do": "finish", "instance": name }),
            ]
        })
        .collect();
    script(package, root, json!(steps))
}

/// Case `index`'s submit call, records and terminal evidence.
fn outcome(report: &Value, index: usize) -> (&Value, Vec<Value>, Value) {
    assert_success(&step(report, 4 * index)["call"]);
    let (records, terminal) = finished(step(report, 4 * index + 3));
    (step(report, 4 * index + 1), records, terminal.clone())
}

/// COM-01 Fault (missing required support is rejected explicitly
/// before any role takes effect, support is never inferred from the release,
/// and incompatible peers are rejected): against fake daemons of this very
/// release, the Open is rejected (never sent: each daemon saw only hello, or
/// nothing) when hello advertises only the frozen tokens, revision 1
/// without `monitor-v1`, malformed revision-1 limits, or not a family the
/// Open requires; when hello names another session, release or protocol;
/// when the metadata names another release, another session, or a socket
/// nobody serves, or cannot be read; when the reply is not a hello, or
/// never comes within `start_ms`. Each rejection carries its error, as the
/// diagnostic and the terminal evidence do, with the retry it calls for:
/// none possible for missing support or a mismatch, passive for what can
/// recover.
#[test]
fn installed_library_rejects_an_open_the_session_cannot_honour_before_any_role() {
    let package = Package::new();
    let root = root(&package);
    let frozen = ["attach", "kill", "state-v1", "client-startup-v1"];
    let monitorless = ["attach", "daemon-wire-v1", "title-v1"];
    let titleless = ["daemon-wire-v1", "monitor-v1"];
    let request = open(1, &REQUIRED, &["title-v1"]);
    let titled = open(1, &["daemon-wire-v1", "monitor-v1", "title-v1"], &[]);
    let missing = root.join("missing.sock");
    let library = |code: &'static str| (ORIGIN_LIBRARY, code);
    let transport = |code: &'static str| (ORIGIN_TRANSPORT, code);
    let cases: Vec<(Case, (u64, &str), u64, usize)> = vec![
        (
            case(
                "frozen",
                json!({ "hello": { "json": { "capabilities": frozen }, "without": ["daemon_wire_v1"] } }),
                json!({}),
                &request,
            ),
            library("unsupported_revision"),
            RETRY_PERMANENT,
            1,
        ),
        (
            case(
                "monitorless",
                json!({ "hello": hello(json!({ "capabilities": monitorless })) }),
                json!({}),
                &request,
            ),
            library("unsupported_capability"),
            RETRY_PERMANENT,
            1,
        ),
        (
            case(
                "limitless",
                json!({ "hello": hello(json!({ "daemon_wire_v1": { "max_request_frame_bytes": 0, "max_server_frame_bytes": 1024 } })) }),
                json!({}),
                &request,
            ),
            library("unsupported_revision"),
            RETRY_PERMANENT,
            1,
        ),
        (
            case(
                "titleless",
                json!({ "hello": hello(json!({ "capabilities": titleless })) }),
                json!({}),
                &titled,
            ),
            library("unsupported_capability"),
            RETRY_PERMANENT,
            1,
        ),
        (
            case(
                "elsewhere",
                json!({ "hello": hello(json!({ "session_id": "00000000-0000-4000-8000-000000000000" })) }),
                json!({}),
                &request,
            ),
            library("invalid_configuration"),
            RETRY_PERMANENT,
            1,
        ),
        (
            case(
                "release",
                json!({ "hello": hello(json!({ "climon2_version": "9.9.9" })) }),
                json!({}),
                &request,
            ),
            library("release_mismatch"),
            RETRY_PERMANENT,
            1,
        ),
        (
            case(
                "protocol",
                json!({ "hello": hello(json!({ "protocol": 2 })) }),
                json!({}),
                &request,
            ),
            library("unsupported_revision"),
            RETRY_PERMANENT,
            1,
        ),
        (
            case(
                "recorded",
                json!({ "metadata": { "climon2_version": "9.9.9" } }),
                json!({}),
                &request,
            ),
            library("release_mismatch"),
            RETRY_PERMANENT,
            0,
        ),
        (
            case(
                "named",
                json!({}),
                json!({ "session_id": "fedcba9876543210fedcba9876543210" }),
                &request,
            ),
            library("invalid_configuration"),
            RETRY_PERMANENT,
            0,
        ),
        (
            case(
                "unserved",
                json!({ "metadata": { "socket_path": missing.to_str().unwrap() } }),
                json!({}),
                &request,
            ),
            transport("io_error"),
            RETRY_PASSIVE,
            0,
        ),
        (
            case(
                "unreadable",
                json!({}),
                json!({ "metadata_path": { "bytes": root.join("absent.pty").to_str().unwrap() } }),
                &request,
            ),
            library("io_error"),
            RETRY_PASSIVE,
            0,
        ),
        (
            case(
                "unhelloed",
                json!({ "hello": { "frame": opened(&["title-v1"]) } }),
                json!({}),
                &request,
            ),
            transport("invalid_record"),
            RETRY_PASSIVE,
            1,
        ),
        (
            case(
                "silent",
                json!({ "hello": "hold" }),
                json!({ "limits": { "start_ms": 300 } }),
                &request,
            ),
            library("timeout"),
            RETRY_PASSIVE,
            1,
        ),
    ];
    let report = run_cases(&package, &root, cases.iter().map(|(case, ..)| case));
    for (index, (case, (origin, code), retry, frames)) in cases.iter().enumerate() {
        let name = case.name;
        let (submit, records, terminal) = outcome(&report, index);
        assert_success(submit);
        assert_eq!(records.len(), 2, "{name}: {records:#?}");
        assert_rejected(&records[0], 1, 1, *origin, code);
        assert_diagnostic(&records[1], 1, *origin, code);
        let refused = (CLIENT_FAILED, *retry, LEAVE_NOT_REQUIRED);
        assert_terminal(&terminal, 1, refused, Some((*origin, code)));
        let peer = &report["peers"][name];
        let hello = if *frames == 1 { vec!["hello"] } else { vec![] };
        assert_eq!(kinds(peer), hello, "{name}: never an Open: {peer:#}");
    }
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}

/// COM-01 Fault (optional families intersect three ways, and the selection
/// must be one the request allows): a daemon advertising `status-v1` and
/// `focus-v1` but not `title-v1` is asked for no optional family at all,
/// since the library relays neither and the daemon lacks titles; its Initial
/// metadata reports the title unavailable, never a fabricated empty one.
/// A daemon that selects a family the Open did not request, or omits a
/// required one, breaks the stream (`invalid_record`).
#[test]
fn installed_library_selects_only_families_both_sides_support() {
    let package = Package::new();
    let root = root(&package);
    let advertised = ["daemon-wire-v1", "monitor-v1", "status-v1", "focus-v1"];
    let unavailable = send(
        0x41,
        json!({
            "title": { "available": false, "value": null },
            "status": { "available": false, "value": null },
            "progress": { "available": false, "value": null },
            "command_result": null,
        }),
    );
    let intersected = json!({
        "hello": hello(json!({ "capabilities": advertised })),
        "open": [opened(&REQUIRED), unavailable],
        "leave": [leave_answer("ok", false, Value::Null)],
    });
    let unrequested = json!({ "open": [opened(&["daemon-wire-v1", "monitor-v1", "status-v1"])] });
    let incomplete = json!({ "open": [opened(&["title-v1"])] });
    let optional = ["title-v1", "status-v1", "focus-v1"];
    let steps = json!([
        { "do": "create", "instance": "a", "peer": intersected },
        { "do": "submit", "instance": "a", "request": open(1, &REQUIRED, &optional) },
        { "do": "run", "instance": "a" },
        { "do": "drain", "instance": "a", "until": { "tag": INITIAL_METADATA } },
        { "do": "shutdown", "instance": "a" },
        { "do": "finish", "instance": "a" },
        { "do": "create", "instance": "b", "peer": unrequested },
        { "do": "submit", "instance": "b", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run", "instance": "b" },
        { "do": "finish", "instance": "b" },
        { "do": "create", "instance": "c", "peer": incomplete },
        { "do": "submit", "instance": "c", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run", "instance": "c" },
        { "do": "finish", "instance": "c" },
    ]);
    let report = script(&package, &root, steps);
    let open_frame = &report["peers"]["a"]["frames"][1];
    assert_eq!(open_frame["kind"], "open", "{open_frame:#}");
    assert_eq!(
        open_frame["body"],
        json!({
            "request_id": "1",
            "revision": 1,
            "required_capabilities": REQUIRED,
            "optional_capabilities": [],
            "mode": "monitor",
            "geometry": null,
        })
    );
    let records = step(&report, 3)["records"].as_array().unwrap();
    let response = assert_response(&records[0], 1, 1, OK);
    assert_eq!(response["result_tag"], OPENED);
    assert_eq!(
        response["result"],
        json!({ "connection_id": CONNECTION_BYTES, "selected": REQUIRED })
    );
    assert_eq!(records[1]["metadata"]["title"], title(UNAVAILABLE, None));
    let (_, terminal) = finished(step(&report, 5));
    assert_left(terminal, 1);
    for finish in [9, 13] {
        let (records, terminal) = finished(step(&report, finish));
        assert_eq!(records.len(), 1, "{records:#?}");
        assert_diagnostic(&records[0], 1, ORIGIN_TRANSPORT, "invalid_record");
        let broken = (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED);
        assert_terminal(
            terminal,
            1,
            broken,
            Some((ORIGIN_TRANSPORT, "invalid_record")),
        );
    }
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}

/// A fake daemon that opens with `title-v1` and then sends `after`.
fn opened_then(after: Vec<Value>) -> Value {
    let mut actions = vec![opened(&SELECTED), initial(json!("t"))];
    actions.extend(after);
    json!({ "open": actions })
}

/// COM-02 Fault (each boundary's framing, types and records,
/// with their rejection effects): a frame that is not JSON
/// (`invalid_json`), a request type (`wrong_direction`), an undefined type
/// (`unknown_message_type`), a zero length (`invalid_frame_length`), a
/// length beyond `wire_frame_bytes` (`frame_too_large`, decided from the
/// prefix), a stream ending inside a frame (`truncated_frame`), a response
/// for no request (`invalid_request_id`) or with another operation's result,
/// a title before the Open's response or before the Initial metadata, an
/// Initial metadata reporting a family nobody selected, or a title when
/// `title-v1` is not selected (`invalid_record`) each fail the connection:
/// a transport diagnostic, then failed terminal evidence with passive retry,
/// the connection closed and no Leave needed. A title one byte over
/// `title_bytes`, or a record one byte over `metadata_record_bytes`, fails
/// it too (`resource_limit`) instead of being truncated, and a daemon's
/// Fatal error is reported with its own origin and code. Nothing the daemon
/// sent after the failing frame, even in the same write, is delivered.
#[test]
fn installed_library_fails_the_connection_on_traffic_revision_one_forbids() {
    let package = Package::new();
    let root = root(&package);
    let request = open(1, &REQUIRED, &["title-v1"]);
    let untitled = open(1, &REQUIRED, &[]);
    let invalid = json!({ "send": { "type": 0x40, "hex": "7b" } });
    let leave_type = send(0x37, json!({ "request_id": "1" }));
    let unknown_type = send(0x50, json!({}));
    let left = response(
        "$open",
        "ok",
        json!({ "kind": "left", "attachment_detached": false }),
        Value::Null,
    );
    let uncorrelated = response(
        "99",
        "ok",
        json!({ "kind": "left", "attachment_detached": false }),
        Value::Null,
    );
    let status = send(
        0x41,
        json!({
            "title": { "available": true, "value": "t" },
            "status": { "available": true, "value": {
                "state": "idle", "changed_at": "2026-11-13T00:00:00.000Z", "episode_id": null } },
            "progress": { "available": false, "value": null },
            "command_result": null,
        }),
    );
    let fatal = send(
        0x4d,
        json!({ "code": "internal_error", "message": "fake daemon failed" }),
    );
    let long = "x".repeat(4097);
    let past = "y".repeat(300);
    let past_frame = 1 + serde_json::to_vec(&json!({ "title": past })).unwrap().len();
    let library = |code: &'static str| (ORIGIN_LIBRARY, code);
    let transport = |code: &'static str| (ORIGIN_TRANSPORT, code);
    let cases: Vec<(Case, (u64, &str), usize)> = vec![
        (
            case("json", json!({ "open": [invalid] }), json!({}), &request),
            transport("invalid_json"),
            0,
        ),
        (
            case(
                "direction",
                json!({ "open": [leave_type] }),
                json!({}),
                &request,
            ),
            transport("wrong_direction"),
            0,
        ),
        (
            case(
                "type",
                json!({ "open": [unknown_type] }),
                json!({}),
                &request,
            ),
            transport("unknown_message_type"),
            0,
        ),
        (
            case(
                "zero",
                json!({ "open": [{ "raw": "00000000" }] }),
                json!({}),
                &request,
            ),
            transport("invalid_frame_length"),
            0,
        ),
        (
            case(
                "oversized",
                json!({ "open": [{ "raw": "0010000140" }] }),
                json!({}),
                &request,
            ),
            transport("frame_too_large"),
            0,
        ),
        (
            case(
                "truncated",
                json!({ "open": [{ "raw": "0000000a407b22" }, "close"] }),
                json!({}),
                &request,
            ),
            transport("truncated_frame"),
            0,
        ),
        (
            case(
                "uncorrelated",
                json!({ "open": [uncorrelated] }),
                json!({}),
                &request,
            ),
            transport("invalid_request_id"),
            0,
        ),
        (
            case("mismatched", json!({ "open": [left] }), json!({}), &request),
            transport("invalid_record"),
            0,
        ),
        (
            case(
                "early",
                json!({ "open": [title_changed(json!("x"))] }),
                json!({}),
                &request,
            ),
            transport("invalid_record"),
            0,
        ),
        (
            case(
                "unsynchronized",
                json!({ "open": [opened(&SELECTED), title_changed(json!("x"))] }),
                json!({}),
                &request,
            ),
            transport("invalid_record"),
            1,
        ),
        (
            case(
                "status",
                json!({ "open": [opened(&SELECTED), status] }),
                json!({}),
                &request,
            ),
            transport("invalid_record"),
            1,
        ),
        (
            case(
                "unselected",
                json!({ "open": [opened(&REQUIRED), initial_unavailable(), title_changed(json!("x"))] }),
                json!({}),
                &untitled,
            ),
            transport("invalid_record"),
            2,
        ),
        (
            case(
                "title",
                opened_then(vec![title_changed(json!(long))]),
                json!({}),
                &request,
            ),
            library("resource_limit"),
            2,
        ),
        (
            case(
                "record",
                opened_then(vec![title_changed(json!(past))]),
                json!({ "limits": { "metadata_record_bytes": past_frame - 1 } }),
                &request,
            ),
            library("resource_limit"),
            2,
        ),
        (
            case(
                "fatal",
                opened_then(vec![
                    json!({ "together": [fatal, title_changed(json!("after"))] }),
                ]),
                json!({}),
                &request,
            ),
            (ORIGIN_DAEMON, "internal_error"),
            2,
        ),
    ];
    let report = run_cases(&package, &root, cases.iter().map(|(case, ..)| case));
    for (index, (case, (origin, code), before)) in cases.iter().enumerate() {
        let name = case.name;
        let (submit, records, terminal) = outcome(&report, index);
        assert_success(submit);
        assert_eq!(records.len(), before + 1, "{name}: {records:#?}");
        if *before > 0 {
            assert_eq!(assert_response(&records[0], 1, 1, OK)["result_tag"], OPENED);
        }
        assert_diagnostic(&records[*before], 1, *origin, code);
        let broken = (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED);
        assert_terminal(&terminal, 1, broken, Some((*origin, code)));
        assert_eq!(kinds(&report["peers"][name]), ["hello", "open"], "{name}");
    }
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}

/// The fake daemon's Initial metadata with every family unavailable.
fn initial_unavailable() -> Value {
    send(
        0x41,
        json!({
            "title": { "available": false, "value": null },
            "status": { "available": false, "value": null },
            "progress": { "available": false, "value": null },
            "command_result": null,
        }),
    )
}

/// What one record of a fault case must be.
enum Expect {
    /// A Response to the Open `id`: it succeeded.
    Opened(u64),
    /// A Response to the Leave `id`: it succeeded, detaching nothing.
    Left(u64),
    /// The Initial metadata.
    Metadata,
    /// A Title changed record.
    Title,
    /// A failed Leave `id` whose known result detached an attachment.
    FailedLeft(u64, (u64, &'static str)),
    /// A Response to `id` with `outcome`, no result and an error.
    Answer(u64, u64, (u64, &'static str)),
    /// A diagnostic.
    Diagnostic((u64, &'static str)),
}

fn assert_records(name: &str, records: &[Value], expected: &[Expect]) {
    assert_eq!(records.len(), expected.len(), "{name}: {records:#?}");
    for (record, expected) in records.iter().zip(expected) {
        match expected {
            Expect::Opened(id) => {
                let response = assert_response(record, 1, *id, OK);
                assert_eq!(response["result_tag"], OPENED, "{name}: {record:#}");
                assert_eq!(response["error"], no_error(), "{name}: {record:#}");
            }
            Expect::Left(id) => {
                let response = assert_response(record, 1, *id, OK);
                assert_eq!(response["result_tag"], LEFT, "{name}: {record:#}");
                let detached = json!({ "attachment_detached": 0 });
                assert_eq!(response["result"], detached, "{name}: {record:#}");
                assert_eq!(response["error"], no_error(), "{name}: {record:#}");
            }
            Expect::Metadata => assert_header(record, INITIAL_METADATA, 1),
            Expect::Title => assert_header(record, TITLE, 1),
            Expect::FailedLeft(id, (origin, code)) => {
                let response = assert_response(record, 1, *id, FAILED);
                assert_eq!(response["result_tag"], LEFT, "{name}: {record:#}");
                let detached = json!({ "attachment_detached": 1 });
                assert_eq!(response["result"], detached, "{name}: {record:#}");
                assert_error(&response["error"], *origin, code);
            }
            Expect::Answer(id, outcome, (origin, code)) => {
                let response = assert_response(record, 1, *id, *outcome);
                assert_eq!(response["result_tag"], RESULT_NONE, "{name}: {record:#}");
                assert_error(&response["error"], *origin, code);
            }
            Expect::Diagnostic((origin, code)) => assert_diagnostic(record, 1, *origin, code),
        }
    }
}

/// A fault case: its instance's steps, then what its records (drained by
/// `finish`, or by the step `drained` names first), terminal evidence and
/// fake daemon must show.
struct Scenario {
    name: &'static str,
    steps: Vec<Value>,
    drained: Option<usize>,
    records: Vec<Expect>,
    terminal: (u64, u64, u64),
    error: Option<(u64, &'static str)>,
    kinds: &'static [&'static str],
}

/// The steps of an instance against `peer`, with `startup` overrides, that
/// submits `requests` and runs.
fn submitted(peer: Value, startup: Value, requests: &[Value]) -> Vec<Value> {
    let mut steps = vec![json!({ "do": "create", "peer": peer, "startup": startup })];
    steps.extend(
        requests
            .iter()
            .map(|request| json!({ "do": "submit", "request": request })),
    );
    steps.push(json!({ "do": "run" }));
    steps
}

/// Every scenario in one script, each on its own instance and each ended by
/// waiting for run, a snapshot and `finish`; then each one's records,
/// terminal evidence and fake daemon, with nothing replayed. Once run has
/// returned, nothing is outstanding: no request, no operation and no
/// connection descriptor.
fn assert_scenarios(package: &Package, root: &Path, scenarios: &[Scenario]) {
    let mut steps = Vec::new();
    let mut starts = Vec::new();
    let ending = [
        json!({ "do": "wait_run" }),
        json!({ "do": "snapshot" }),
        json!({ "do": "finish" }),
    ];
    for scenario in scenarios {
        starts.push(steps.len());
        steps.extend(scenario.steps.iter().chain(&ending).map(|step| {
            let mut step = step.clone();
            step["instance"] = json!(scenario.name);
            step
        }));
    }
    let report = script(package, root, json!(steps));
    for (scenario, start) in scenarios.iter().zip(starts) {
        let name = scenario.name;
        let last = start + scenario.steps.len() + ending.len() - 1;
        let settled = &step(&report, last - 1)["output"];
        for (gauge, value) in [
            ("request_records", 0),
            ("operations", 0),
            ("descriptors", INSTANCE_DESCRIPTORS),
        ] {
            assert_eq!(settled[gauge], value, "{name}: {gauge}: {settled:#}");
        }
        let (mut records, terminal) = finished(step(&report, last));
        if let Some(drained) = scenario.drained {
            let mut earlier = step(&report, start + drained)["records"]
                .as_array()
                .unwrap()
                .clone();
            earlier.append(&mut records);
            records = earlier;
        }
        assert_records(name, &records, &scenario.records);
        assert_terminal(terminal, 1, scenario.terminal, scenario.error);
        assert_eq!(kinds(&report["peers"][name]), scenario.kinds, "{name}");
    }
    assert_all_released(&report["ledger"]);
}

/// COM-03 Fault (lost responses produce truthful outcomes, a request that
/// outlives its deadline reports `timeout`, and nothing is replayed): an
/// Open the daemon never answers within `start_ms` is answered
/// indeterminate `timeout`, and so is a Leave within `leave_ms`; a
/// connection lost with an Open or a Leave unanswered gives no fabricated
/// Response, only failed terminal evidence, which makes the caller's still
/// unanswered IDs indeterminate (a lost Leave leaves the departure
/// unconfirmed); a daemon's failed or indeterminate Leave is answered as
/// such and ends the registration, and a failed one never counts as
/// detaching, whatever result it knew; a failure's diagnostic is best
/// effort, left out when the event queue's records are full, while terminal
/// evidence keeps the error; a refused Open is answered with the
/// daemon's error and calls for no retry when the daemon knew the command's
/// final result, for none possible when it lacks support, and for passive
/// retry otherwise. Each fake daemon saw every request once.
#[test]
fn installed_library_reports_lost_and_late_answers_truthfully_without_replay() {
    let package = Package::new();
    let root = root(&package);
    let titled = open(1, &REQUIRED, &["title-v1"]);
    let alone = [titled.clone()];
    let both = [titled.clone(), leave(2, 1)];
    let library = |code: &'static str| (ORIGIN_LIBRARY, code);
    let lost = (ORIGIN_TRANSPORT, "io_error");
    let daemon = |code: &'static str| (ORIGIN_DAEMON, code);
    let refused = |outcome: &str, code: &str| json!({ "open": [response("$open", outcome, Value::Null, daemon_error(code))] });
    let answered = |outcome: &str| {
        passive_peer(json!([leave_answer(
            outcome,
            false,
            daemon_error("internal_error")
        )]))
    };
    let scenarios = [
        Scenario {
            name: "open-held",
            steps: submitted(
                json!({ "open": ["hold"] }),
                json!({ "limits": { "start_ms": 300 } }),
                &alone,
            ),
            drained: None,
            records: vec![
                Expect::Answer(1, INDETERMINATE, library("timeout")),
                Expect::Diagnostic(library("timeout")),
            ],
            terminal: (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
            error: Some(library("timeout")),
            kinds: &["hello", "open"],
        },
        Scenario {
            name: "open-lost",
            steps: submitted(json!({ "open": ["close"] }), json!({}), &alone),
            drained: None,
            records: vec![Expect::Diagnostic(lost)],
            terminal: (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
            error: Some(lost),
            kinds: &["hello", "open"],
        },
        Scenario {
            name: "leave-held",
            steps: submitted(
                passive_peer(json!(["hold"])),
                json!({ "limits": { "leave_ms": 300 } }),
                &both,
            ),
            drained: None,
            records: vec![
                Expect::Opened(1),
                Expect::Metadata,
                Expect::Answer(2, INDETERMINATE, library("timeout")),
                Expect::Diagnostic(library("timeout")),
            ],
            terminal: (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_INDETERMINATE),
            error: Some(library("timeout")),
            kinds: &["hello", "open", "leave"],
        },
        Scenario {
            name: "leave-lost",
            steps: submitted(passive_peer(json!(["close"])), json!({}), &both),
            drained: None,
            records: vec![
                Expect::Opened(1),
                Expect::Metadata,
                Expect::Diagnostic(lost),
            ],
            terminal: (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_INDETERMINATE),
            error: Some(lost),
            kinds: &["hello", "open", "leave"],
        },
        Scenario {
            name: "leave-failed",
            steps: submitted(answered("failed"), json!({}), &both),
            drained: None,
            records: vec![
                Expect::Opened(1),
                Expect::Metadata,
                Expect::Answer(2, FAILED, daemon("internal_error")),
            ],
            terminal: (CLIENT_FAILED, RETRY_NONE, LEAVE_FAILED),
            error: Some(daemon("internal_error")),
            kinds: &["hello", "open", "leave"],
        },
        Scenario {
            name: "leave-indeterminate",
            steps: submitted(answered("indeterminate"), json!({}), &both),
            drained: None,
            records: vec![
                Expect::Opened(1),
                Expect::Metadata,
                Expect::Answer(2, INDETERMINATE, daemon("internal_error")),
            ],
            terminal: (CLIENT_FAILED, RETRY_NONE, LEAVE_INDETERMINATE),
            error: Some(daemon("internal_error")),
            kinds: &["hello", "open", "leave"],
        },
        Scenario {
            name: "leave-known",
            steps: submitted(
                passive_peer(json!([response(
                    "$leave",
                    "failed",
                    json!({ "kind": "left", "attachment_detached": true }),
                    daemon_error("internal_error"),
                )])),
                json!({}),
                &both,
            ),
            drained: None,
            records: vec![
                Expect::Opened(1),
                Expect::Metadata,
                Expect::FailedLeft(2, daemon("internal_error")),
            ],
            terminal: (CLIENT_FAILED, RETRY_NONE, LEAVE_FAILED),
            error: Some(daemon("internal_error")),
            kinds: &["hello", "open", "leave"],
        },
        Scenario {
            name: "unfit-diagnostic",
            steps: submitted(
                json!({ "open": [
                    opened(&SELECTED),
                    initial(json!("t")),
                    title_changed(json!("x")),
                    send(0x4d, daemon_error("internal_error")),
                ] }),
                json!({ "limits": { "event_records": 3 } }),
                &alone,
            ),
            drained: None,
            records: vec![Expect::Opened(1), Expect::Metadata, Expect::Title],
            terminal: (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
            error: Some(daemon("internal_error")),
            kinds: &["hello", "open"],
        },
        Scenario {
            name: "unsupported",
            steps: submitted(
                refused("rejected", "unsupported_capability"),
                json!({}),
                &alone,
            ),
            drained: None,
            records: vec![
                Expect::Answer(1, REJECTED, daemon("unsupported_capability")),
                Expect::Diagnostic(daemon("unsupported_capability")),
            ],
            terminal: (CLIENT_FAILED, RETRY_PERMANENT, LEAVE_NOT_REQUIRED),
            error: Some(daemon("unsupported_capability")),
            kinds: &["hello", "open"],
        },
        Scenario {
            name: "refused",
            steps: submitted(refused("rejected", "internal_error"), json!({}), &alone),
            drained: None,
            records: vec![
                Expect::Answer(1, REJECTED, daemon("internal_error")),
                Expect::Diagnostic(daemon("internal_error")),
            ],
            terminal: (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
            error: Some(daemon("internal_error")),
            kinds: &["hello", "open"],
        },
        Scenario {
            name: "failed",
            steps: submitted(refused("failed", "internal_error"), json!({}), &alone),
            drained: None,
            records: vec![
                Expect::Answer(1, FAILED, daemon("internal_error")),
                Expect::Diagnostic(daemon("internal_error")),
            ],
            terminal: (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
            error: Some(daemon("internal_error")),
            kinds: &["hello", "open"],
        },
    ];
    assert_scenarios(&package, &root, &scenarios);
    package.assert_clean();
}

/// COM-01 and COM-03 Fault (a rejection that knows the command's final
/// result reports it, and calls for no retry): the Open is rejected with
/// the daemon's error and a known final exit code, as the Response's
/// result, before any role; terminal evidence calls for no retry. The
/// Response and the diagnostic are each charged their record and their
/// copies of the daemon's error text, and an exit code costs nothing more.
#[test]
fn installed_library_reports_a_known_final_result_and_never_retries() {
    let package = Package::new();
    let root = root(&package);
    let known = json!({ "kind": "known_final", "command_result": { "kind": "exit", "code": 3 } });
    let peer = json!({
        "open": [response("$open", "rejected", known, daemon_error("invalid_state"))],
    });
    let steps = json!([
        { "do": "create", "peer": peer },
        { "do": "submit", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run" },
        { "do": "wait_run" },
        { "do": "snapshot" },
        { "do": "finish" },
    ]);
    let report = script(&package, &root, steps);
    let queued = &step(&report, 4)["output"];
    let error = ("invalid_state".len() + "fake daemon".len()) as u64;
    assert_eq!(queued["event_records"], 2, "{queued:#}");
    assert_eq!(queued["event_bytes"], 2 * (RECORD + error), "{queued:#}");
    let (records, terminal) = finished(step(&report, 5));
    assert_eq!(records.len(), 2, "{records:#?}");
    let response = assert_response(&records[0], 1, 1, REJECTED);
    assert_eq!(response["result_tag"], KNOWN_FINAL, "{response:#}");
    let unnamed = json!({ "present": 0, "reserved": 0, "value": { "null": true, "length": 0 } });
    assert_eq!(
        response["result"],
        json!({ "kind": EXIT, "value": 3, "signal_name": unnamed })
    );
    assert_error(&response["error"], ORIGIN_DAEMON, "invalid_state");
    assert_diagnostic(&records[1], 1, ORIGIN_DAEMON, "invalid_state");
    let final_known = (CLIENT_FAILED, RETRY_NONE, LEAVE_NOT_REQUIRED);
    assert_terminal(
        terminal,
        1,
        final_known,
        Some((ORIGIN_DAEMON, "invalid_state")),
    );
    assert_eq!(kinds(&report["peers"]["a"]), ["hello", "open"]);
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}

/// EXT-01 and EXT-02 Fault (shutdown attempts the required Leave once,
/// never replays uncertain work, and reports a lost Leave as unconfirmed):
/// a shutdown Leave the daemon never answers within `leave_ms`, or whose
/// connection is lost, leaves the departure indeterminate, with no retry
/// after shutdown; a rejected one is reported rejected. A rejected explicit
/// Leave keeps the registration, and shutdown then leaves once more,
/// confirmed. Shutdown during an Open still under way waits for it, then
/// leaves once. A Leave before any Open, or while another is unanswered,
/// is `invalid_state`, and the next Open or Leave proceeds.
#[test]
fn installed_library_shutdown_leaves_once_and_reports_unconfirmed_departures() {
    let package = Package::new();
    let root = root(&package);
    let titled = open(1, &REQUIRED, &["title-v1"]);
    let library = |code: &'static str| (ORIGIN_LIBRARY, code);
    let lost = (ORIGIN_TRANSPORT, "io_error");
    let rejection = (ORIGIN_DAEMON, "invalid_state");
    let ok = leave_answer("ok", false, Value::Null);
    let rejected = leave_answer("rejected", false, daemon_error("invalid_state"));
    let confirmed = (CLIENT_STOPPED, RETRY_NONE, LEAVE_CONFIRMED);
    let shut = |peer: Value, startup: Value| {
        vec![
            json!({ "do": "create", "peer": peer, "startup": startup }),
            json!({ "do": "submit", "request": titled }),
            json!({ "do": "run" }),
            json!({ "do": "drain", "until": { "tag": INITIAL_METADATA } }),
            json!({ "do": "shutdown" }),
        ]
    };
    let opening = root.join("go-opening");
    let second = root.join("go-second");
    let scenarios = [
        Scenario {
            name: "lost",
            steps: shut(passive_peer(json!(["close"])), json!({})),
            drained: Some(3),
            records: vec![
                Expect::Opened(1),
                Expect::Metadata,
                Expect::Diagnostic(lost),
            ],
            terminal: (CLIENT_FAILED, RETRY_NONE, LEAVE_INDETERMINATE),
            error: Some(lost),
            kinds: &["hello", "open", "leave"],
        },
        Scenario {
            name: "held",
            steps: shut(
                passive_peer(json!(["hold"])),
                json!({ "limits": { "leave_ms": 300 } }),
            ),
            drained: Some(3),
            records: vec![
                Expect::Opened(1),
                Expect::Metadata,
                Expect::Diagnostic(library("timeout")),
            ],
            terminal: (CLIENT_FAILED, RETRY_NONE, LEAVE_INDETERMINATE),
            error: Some(library("timeout")),
            kinds: &["hello", "open", "leave"],
        },
        Scenario {
            name: "rejected",
            steps: shut(passive_peer(json!([rejected])), json!({})),
            drained: Some(3),
            records: vec![Expect::Opened(1), Expect::Metadata],
            terminal: (CLIENT_FAILED, RETRY_NONE, LEAVE_REJECTED),
            error: Some(rejection),
            kinds: &["hello", "open", "leave"],
        },
        Scenario {
            name: "kept",
            steps: vec![
                json!({ "do": "create", "peer": {
                    "open": [opened(&SELECTED), initial(json!("t"))],
                    "leaves": [[rejected], [ok]],
                } }),
                json!({ "do": "submit", "request": titled }),
                json!({ "do": "submit", "request": leave(2, 1) }),
                json!({ "do": "run" }),
                json!({ "do": "drain", "until": { "records": 3 } }),
                json!({ "do": "shutdown" }),
            ],
            drained: Some(4),
            records: vec![
                Expect::Opened(1),
                Expect::Metadata,
                Expect::Answer(2, REJECTED, rejection),
            ],
            terminal: confirmed,
            error: None,
            kinds: &["hello", "open", "leave", "leave"],
        },
        Scenario {
            name: "opening",
            steps: vec![
                json!({ "do": "create", "peer": {
                    "open": [
                        { "wait": opening.to_str().unwrap() },
                        opened(&SELECTED),
                        initial(json!("t")),
                    ],
                    "leave": [ok],
                } }),
                json!({ "do": "submit", "request": titled }),
                json!({ "do": "run" }),
                json!({ "do": "await_peer", "frames": 2 }),
                json!({ "do": "shutdown" }),
                json!({ "do": "touch", "path": opening.to_str().unwrap() }),
            ],
            drained: None,
            records: vec![Expect::Opened(1), Expect::Metadata],
            terminal: confirmed,
            error: None,
            kinds: &["hello", "open", "leave"],
        },
        Scenario {
            name: "early",
            steps: vec![
                json!({ "do": "create", "peer": passive_peer(json!([ok])) }),
                json!({ "do": "submit", "request": leave(1, 1) }),
                json!({ "do": "submit", "request": open(2, &REQUIRED, &["title-v1"]) }),
                json!({ "do": "run" }),
                json!({ "do": "drain", "until": { "tag": INITIAL_METADATA } }),
                json!({ "do": "shutdown" }),
            ],
            drained: Some(4),
            records: vec![
                Expect::Answer(1, REJECTED, library("invalid_state")),
                Expect::Opened(2),
                Expect::Metadata,
            ],
            terminal: confirmed,
            error: None,
            kinds: &["hello", "open", "leave"],
        },
        Scenario {
            name: "second",
            steps: vec![
                json!({ "do": "create", "peer": {
                    "open": [opened(&SELECTED), initial(json!("t"))],
                    "leave": [{ "wait": second.to_str().unwrap() }, ok],
                } }),
                json!({ "do": "submit", "request": titled }),
                json!({ "do": "submit", "request": leave(2, 1) }),
                json!({ "do": "submit", "request": leave(3, 1) }),
                json!({ "do": "run" }),
                json!({ "do": "drain", "until": { "records": 3 } }),
                json!({ "do": "touch", "path": second.to_str().unwrap() }),
            ],
            drained: Some(5),
            records: vec![
                Expect::Opened(1),
                Expect::Metadata,
                Expect::Answer(3, REJECTED, library("invalid_state")),
                Expect::Left(2),
            ],
            terminal: confirmed,
            error: None,
            kinds: &["hello", "open", "leave"],
        },
    ];
    assert_scenarios(&package, &root, &scenarios);
    package.assert_clean();
}

/// LIB-05 Fault (the library never raises `SIGPIPE` in its embedder): in a
/// host that leaves `SIGPIPE` at its default disposition, as a C embedder
/// may, the shutdown Leave is written to a fake daemon that can no longer
/// receive it. One stops reading after the Initial metadata (`EPIPE` on
/// Linux; another system may refuse the write too, or accept it unread, and
/// the Leave then outlives `leave_ms`). The other closes its connection
/// after a title the library holds back, its two-record event queue being
/// full, so the library reads nothing and its write fails. The host
/// survives each; the departure is unconfirmed, with no retry after
/// shutdown, and the full queue leaves no room for a diagnostic.
#[test]
fn installed_library_never_raises_sigpipe_in_its_embedder() {
    let package = Package::new();
    let root = root(&package);
    let end = root.join("end-deaf");
    let deaf = json!({
        "open": [
            opened(&SELECTED),
            initial(json!("t")),
            "deaf",
            { "wait": end.to_str().unwrap() },
        ],
    });
    let gone = json!({
        "open": [opened(&SELECTED), initial(json!("t")), title_changed(json!("x")), "close"],
    });
    let steps = json!([
        { "do": "create", "peer": deaf, "startup": { "limits": { "leave_ms": 2000 } } },
        { "do": "submit", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run" },
        { "do": "drain", "until": { "tag": INITIAL_METADATA } },
        { "do": "await_peer", "frames": 3 },
        { "do": "shutdown" },
        { "do": "finish" },
        { "do": "touch", "path": end.to_str().unwrap() },
        { "do": "create", "instance": "gone", "peer": gone,
          "startup": { "limits": { "event_records": 2 } } },
        { "do": "submit", "instance": "gone", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run", "instance": "gone" },
        { "do": "await_snapshot", "instance": "gone", "gauge": "event_records", "value": 2 },
        { "do": "await_peer", "instance": "gone", "frames": 3 },
        { "do": "shutdown", "instance": "gone" },
        { "do": "finish", "instance": "gone" },
    ]);
    let report = script_with(&package, &root, steps, json!({ "sigpipe": "default" }));
    let (records, terminal) = finished(step(&report, 6));
    // Linux refuses the write at once; another system may instead accept it
    // unread, and the Leave then outlives `leave_ms`.
    let refused = cfg!(target_os = "linux") || terminal["error"]["code"] == "io_error";
    let error = if refused {
        (ORIGIN_TRANSPORT, "io_error")
    } else {
        (ORIGIN_LIBRARY, "timeout")
    };
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_diagnostic(&records[0], 1, error.0, error.1);
    let unconfirmed = (CLIENT_FAILED, RETRY_NONE, LEAVE_INDETERMINATE);
    assert_terminal(terminal, 1, unconfirmed, Some(error));
    assert_eq!(report["peers"]["a"]["frames"][2]["deaf"], true);
    assert_eq!(step(&report, 12), true);
    let (records, terminal) = finished(step(&report, 14));
    assert_eq!(records.len(), 2, "{records:#?}");
    assert_eq!(assert_response(&records[0], 1, 1, OK)["result_tag"], OPENED);
    assert_header(&records[1], INITIAL_METADATA, 1);
    let lost = (ORIGIN_TRANSPORT, "io_error");
    assert_terminal(terminal, 1, unconfirmed, Some(lost));
    let peer = &report["peers"]["gone"];
    assert_eq!(peer["frames"][2]["closed"], true, "{peer:#}");
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}

/// COM-02 Fault (records exactly at their limits are accepted whole, and
/// the command result maps exactly): from the fake daemon peer, a title of
/// exactly `title_bytes`, in
/// the Initial metadata and in a change, arrives intact; so does a change
/// whose frame is exactly `metadata_record_bytes`; and a command result in
/// the Initial metadata, a signal with its name, is reported as such.
#[test]
fn installed_library_accepts_records_exactly_at_their_limits() {
    let package = Package::new();
    let root = root(&package);
    let longest = "x".repeat(4096);
    let changed = "y".repeat(4096);
    let framed = "z".repeat(2000);
    let frame = 1 + serde_json::to_vec(&json!({ "title": framed }))
        .unwrap()
        .len();
    let signalled = send(
        0x41,
        json!({
            "title": { "available": true, "value": "t" },
            "status": { "available": false, "value": null },
            "progress": { "available": false, "value": null },
            "command_result": { "kind": "signal", "number": 15, "name": "SIGTERM" },
        }),
    );
    let ok = leave_answer("ok", false, Value::Null);
    let cases = [
        (
            "title",
            json!({
                "open": [opened(&SELECTED), initial(json!(longest)), title_changed(json!(changed))],
                "leave": [ok],
            }),
            json!({}),
        ),
        (
            "record",
            json!({
                "open": [opened(&SELECTED), initial(json!("t")), title_changed(json!(framed))],
                "leave": [ok],
            }),
            json!({ "limits": { "metadata_record_bytes": frame } }),
        ),
        (
            "command",
            json!({ "open": [opened(&SELECTED), signalled], "leave": [ok] }),
            json!({}),
        ),
    ];
    let mut steps = Vec::new();
    for (name, peer, startup) in &cases {
        let records = if *name == "command" { 2 } else { 3 };
        steps.extend([
            json!({ "do": "create", "instance": name, "peer": peer, "startup": startup }),
            json!({ "do": "submit", "instance": name, "request": open(1, &REQUIRED, &["title-v1"]) }),
            json!({ "do": "run", "instance": name }),
            json!({ "do": "drain", "instance": name, "until": { "records": records } }),
            json!({ "do": "shutdown", "instance": name }),
            json!({ "do": "finish", "instance": name }),
        ]);
    }
    let report = script(&package, &root, json!(steps));
    let drained = |index: usize| step(&report, 6 * index + 3)["records"].as_array().unwrap();
    let title_case = drained(0);
    assert_eq!(
        title_case[1]["metadata"]["title"],
        title(VALUE, Some(&longest))
    );
    assert_eq!(title_case[2]["title"], title(VALUE, Some(&changed)));
    assert_eq!(drained(1)[2]["title"], title(VALUE, Some(&framed)));
    let command = &drained(2)[1]["metadata"]["command_result"];
    let name = json!({ "present": 1, "reserved": 0, "value": "SIGTERM" });
    assert_eq!(
        command,
        &json!({
            "present": 1,
            "reserved": 0,
            "value": { "kind": SIGNAL, "value": 15, "signal_name": name },
        })
    );
    for index in 0..cases.len() {
        let (rest, terminal) = finished(step(&report, 6 * index + 5));
        assert!(rest.is_empty(), "{rest:#?}");
        assert_left(terminal, 1);
    }
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}
