//! Library layer and Fault (#95): status, viewing and acknowledgement
//! through the installed client library against the host's fake daemon
//! peer, which holds, loses, mismatches or misorders what a real daemon
//! never would. Every case records what reached the fake daemon, so a
//! replayed or wrongly sent request cannot hide. Expected values come from
//! the native header, the native contract and the daemon wire reference.

#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use support::{Package, control::*, library::CHARGE, passive::*, status::*};

/// What the fake daemon advertises: the real one's families.
const ADVERTISED: [&str; 9] = [
    "attach",
    "kill",
    "state-v1",
    "client-startup-v1",
    "daemon-wire-v1",
    "monitor-v1",
    "title-v1",
    "status-v1",
    "focus-v1",
];
/// What an interactive Open of `interactive_status` selects from it, less
/// focus, which these cases do not need.
const ATTACHED: [&str; 3] = ["daemon-wire-v1", "title-v1", "status-v1"];
/// What a monitor Open requiring status selects, titles too.
const WATCHED: [&str; 4] = ["daemon-wire-v1", "monitor-v1", "title-v1", "status-v1"];
/// What a monitor Open asks for besides what it requires.
const OPTIONAL_STATUS: [&str; 2] = ["title-v1", "status-v1"];
const PERIOD: u64 = 2;
const TIME: &str = "2026-02-28T23:59:59.999Z";
const LATER: &str = "2026-03-01T00:00:00.000Z";

/// A wire Status: `state` at `changed_at`, with `episode` when given.
fn wire(state: &str, changed_at: &str, episode: Option<&str>) -> Value {
    json!({ "state": state, "changed_at": changed_at, "episode_id": episode })
}

/// The fake daemon's Initial metadata with the title `t`, the wire
/// `status` (an `Available<Status>`) and the command `result`.
fn metadata(status: Value, result: Value) -> Value {
    send(
        0x41,
        json!({
            "title": { "available": true, "value": "t" },
            "status": status,
            "progress": { "available": false, "value": null },
            "command_result": result,
        }),
    )
}

/// An available wire status.
fn known(status: Value) -> Value {
    json!({ "available": true, "value": status })
}

fn unknown() -> Value {
    json!({ "available": false, "value": null })
}

fn status_changed(status: Value) -> Value {
    send(0x43, json!({ "status": status }))
}

fn command_final(result: Value, status: Value) -> Value {
    send(0x4c, json!({ "command_result": result, "status": status }))
}

fn exit(code: u64) -> Value {
    json!({ "kind": "exit", "code": code })
}

fn counter(value: u64) -> String {
    value.to_string()
}

/// An attach as a real daemon sends it with `status-v1` selected:
/// Response, Initial metadata reporting idle, then the grant of period 2,
/// restoration 2 at 80x24, its restoration with the Display `41`, and the
/// complete.
fn attach_records(selected: &[&str], status: Value) -> Vec<Value> {
    let ids = json!({ "period": counter(PERIOD), "restoration_id": "2" });
    let mut begun = ids.clone();
    begun["geometry"] = json!({ "cols": 80, "rows": 24 });
    let mut granted = begun.clone();
    granted["reason"] = json!("attach");
    vec![
        opened(selected),
        metadata(status, Value::Null),
        send(0x46, granted),
        send(0x48, begun),
        json!({ "send": { "type": 0x49, "hex": format!("{PERIOD:016x}{:016x}41", 2) } }),
        send(0x4a, ids),
    ]
}

/// A fake daemon that attaches selecting `selected`, answers a Leave
/// detaching, and does `then` after each request of the kinds it names.
fn attached_peer(selected: &[&str], status: Value, then: Value) -> Value {
    let mut peer = json!({
        "hello": { "json": { "capabilities": ADVERTISED } },
        "open": attach_records(selected, status),
        "leave": [leave_answer("ok", true, Value::Null)],
    });
    for (kind, actions) in then.as_object().unwrap() {
        peer[kind] = actions.clone();
    }
    peer
}

/// A fake daemon that attaches with status selected, reporting idle.
fn status_peer(then: Value) -> Value {
    attached_peer(&ATTACHED, known(wire("idle", TIME, None)), then)
}

/// One instance's steps: create against `peer` with `startup`, Open
/// interactively, run, and drain through the attach's Restoration
/// complete, then `then`, then `finish`, labelled `finished-<name>`.
fn attached(name: &str, peer: Value, startup: Value, then: &[Value]) -> Vec<Value> {
    let mut steps = vec![
        json!({ "do": "create", "peer": peer, "startup": startup }),
        json!({ "do": "submit", "request": interactive_status(1, 80, 24) }),
        json!({ "do": "run" }),
        json!({ "do": "drain", "until": { "tag": RESTORATION_COMPLETE } }),
    ];
    steps.extend(then.iter().cloned());
    steps.push(json!({ "do": "finish", "as": format!("finished-{name}") }));
    steps
        .into_iter()
        .map(|mut step| {
            step["instance"] = json!(name);
            step
        })
        .collect()
}

/// The fake daemon's Response to the latest request of `kind`.
fn answered(kind: &str, outcome: &str, result: Value, error: Value) -> Value {
    response(&format!("${kind}"), outcome, result, error)
}

/// The records and terminal evidence of the instance `name`.
fn ended<'a>(report: &'a Value, steps: &[Value], name: &str) -> (Vec<Value>, &'a Value) {
    finished(labelled(report, steps, &format!("finished-{name}")))
}

/// Every `submit` among `steps` labelled `accepted` was accepted.
fn assert_accepted(report: &Value, steps: &[Value]) {
    for (index, submitted) in steps.iter().enumerate() {
        if submitted["as"] == "accepted" {
            assert_success(step(report, index));
        }
    }
}

/// COM-03 Library and Fault (viewing and acknowledgement cases, local
/// outcomes): `submit` refuses a malformed Set viewing or Acknowledge
/// synchronously with `invalid_argument` (viewing of 2, a reserved byte
/// set, a zero period; Acknowledge of a zero restoration or generation),
/// consuming no ID; Set viewing and Acknowledge queued behind an Open
/// under way are rejected `cancelled` by shutdown, never sent. STA-08 Library and Fault: on a
/// connection whose Open did not select `status-v1` both are
/// `unsupported_capability`, and naming another generation is
/// `stale_generation`, all locally: the fake daemon receives only the
/// Open and the Leave.
#[test]
fn installed_library_rejects_viewing_and_acknowledgement_it_cannot_send() {
    let package = Package::new();
    let root = root(&package);
    let malformed = [
        json!({ "tag": "viewing", "request_id": 1, "authority": [1, 2, 2], "value": 2 }),
        json!({ "tag": "viewing", "request_id": 1, "authority": [1, 2, 2], "value": 1,
                "flag_reserved": 1 }),
        json!({ "tag": "viewing", "request_id": 1, "authority": [1, 0, 2], "value": 0 }),
        json!({ "tag": "acknowledge", "request_id": 1, "authority": [1, 2, 0] }),
        json!({ "tag": "acknowledge", "request_id": 1, "authority": [0, 2, 2] }),
    ];
    let go = root.join("go");
    let mut records = vec![json!({ "wait": go.to_str().unwrap() })];
    records.extend(attach_records(&ATTACHED, known(wire("idle", TIME, None))));
    let mut peer = status_peer(json!({}));
    peer["open"] = json!(records);
    let authority = [1, PERIOD, 2];
    let mut queued: Vec<Value> = vec![json!({ "do": "create", "peer": peer })];
    queued.extend(
        malformed
            .iter()
            .map(|request| json!({ "do": "submit", "request": request, "as": "malformed" })),
    );
    queued.extend([
        json!({ "do": "submit", "request": interactive_status(1, 80, 24), "as": "accepted" }),
        json!({ "do": "submit", "request": { "tag": "viewing", "request_id": 2,
                "authority": authority, "value": 1 }, "as": "accepted" }),
        json!({ "do": "submit", "request": { "tag": "acknowledge", "request_id": 3,
                "authority": authority }, "as": "accepted" }),
        json!({ "do": "run" }),
        json!({ "do": "await_peer", "frames": 2 }),
        json!({ "do": "shutdown" }),
        json!({ "do": "touch", "path": go.to_str().unwrap() }),
        json!({ "do": "finish", "as": "finished-queued" }),
    ]);
    let mut steps: Vec<Value> = queued
        .into_iter()
        .map(|mut step| {
            step["instance"] = json!("queued");
            step
        })
        .collect();
    let unselected = attached_peer(&["daemon-wire-v1", "title-v1"], unknown(), json!({}));
    let then = [
        json!({ "do": "submit", "request": view(2, 1, 1), "as": "accepted" }),
        json!({ "do": "submit", "request": acknowledge(3, 1, CURRENT), "as": "accepted" }),
        json!({ "do": "submit", "request": view(4, 9, 0), "as": "accepted" }),
        json!({ "do": "submit", "request": acknowledge(5, 9, CURRENT), "as": "accepted" }),
        json!({ "do": "drain", "until": { "all": [{ "response": 3 }, { "response": 5 }] },
                "as": "refused" }),
        json!({ "do": "shutdown" }),
    ];
    steps.extend(attached("unselected", unselected, json!({}), &then));
    let report = script(&package, &root, json!(steps));
    assert_accepted(&report, &steps);
    for (index, submitted) in steps.iter().enumerate() {
        if submitted["as"] == "malformed" {
            assert_failure(step(&report, index), INVALID_ARGUMENT, "invalid_argument");
        }
    }
    let (records, terminal) = ended(&report, &steps, "queued");
    for (index, id) in [2, 3].into_iter().enumerate() {
        assert_rejected(&records[index], 1, id, ORIGIN_LIBRARY, "cancelled");
    }
    assert_ok(&records[2], 1, 1, OPENED);
    assert_header(&records[3], INITIAL_METADATA, 1);
    assert_restored(&records[4..], 1, ATTACH, [80, 24]);
    assert_detached(terminal, 1);
    assert_eq!(
        kinds(&report["peers"]["queued"]),
        ["hello", "open", "leave"]
    );
    let refused = drained(&report, &steps, "refused");
    for (id, code) in [
        (2, "unsupported_capability"),
        (3, "unsupported_capability"),
        (4, "stale_generation"),
        (5, "stale_generation"),
    ] {
        assert_rejected(answer(&refused, id), 1, id, ORIGIN_LIBRARY, code);
    }
    let (_, terminal) = ended(&report, &steps, "unselected");
    assert_detached(terminal, 1);
    assert_eq!(
        kinds(&report["peers"]["unselected"]),
        ["hello", "open", "leave"]
    );
    assert_all_released(&report["ledger"]);
}

/// A viewing or acknowledgement case: its name, the request kind and
/// request, and what the fake daemon does after it.
type AnswerCase = (&'static str, &'static str, Value, Vec<Value>);

/// STA-08 Library and Fault (Set viewing and Acknowledge, with their
/// authority-checked results): each reaches the daemon once, as the wire
/// reference writes it under the authority it names; viewing true and
/// false map to `CM1_VIEWING_RESULT` with changed, the viewing now and no
/// bytes, and Acknowledge to `CM1_ACKNOWLEDGEMENT` with changed, the
/// no-change success explicit; an acknowledgement's Status reaches the
/// caller before its success. COM-03 Library and Fault (viewing and
/// acknowledgement cases): the daemon's rejections (`stale_period`,
/// `stale_restoration`, `not_ready`, `not_viewing`) keep its error; a
/// success reporting another viewing than asked, or another request's
/// result, fails the connection with `invalid_record`; a daemon that holds
/// its answer past `operation_ms` gets it answered indeterminate
/// `timeout`, and one that closes the connection gets no fabricated
/// Response at all; either way nothing is replayed.
#[test]
fn installed_library_maps_viewing_and_acknowledgement_answers_without_replay() {
    let package = Package::new();
    let root = root(&package);
    let on = view(2, 1, 1);
    let off = view(2, 1, 0);
    let ack = acknowledge(2, 1, CURRENT);
    let viewing_ok = |changed: bool, viewing: bool| {
        let result = json!({ "kind": "viewing", "changed": changed, "viewing": viewing });
        vec![answered("viewing", "ok", result, Value::Null)]
    };
    let acknowledged_ok = |changed: bool| {
        let result = json!({ "kind": "acknowledgement", "changed": changed });
        vec![answered("acknowledge", "ok", result, Value::Null)]
    };
    let refused =
        |kind: &str, code: &str| vec![answered(kind, "rejected", Value::Null, daemon_error(code))];
    let mut acknowledging = vec![status_changed(wire("acknowledged", LATER, Some("3")))];
    acknowledging.extend(acknowledged_ok(true));
    let answered_cases: Vec<AnswerCase> = vec![
        ("viewing-on", "viewing", on.clone(), viewing_ok(true, true)),
        (
            "viewing-off",
            "viewing",
            off.clone(),
            viewing_ok(false, false),
        ),
        ("acknowledged", "acknowledge", ack.clone(), acknowledging),
        (
            "unchanged",
            "acknowledge",
            ack.clone(),
            acknowledged_ok(false),
        ),
        (
            "stale-period",
            "viewing",
            on.clone(),
            refused("viewing", "stale_period"),
        ),
        (
            "not-ready",
            "viewing",
            off.clone(),
            refused("viewing", "not_ready"),
        ),
        (
            "stale-restoration",
            "acknowledge",
            ack.clone(),
            refused("acknowledge", "stale_restoration"),
        ),
        (
            "not-viewing",
            "acknowledge",
            ack.clone(),
            refused("acknowledge", "not_viewing"),
        ),
    ];
    let mut steps = Vec::new();
    for (name, kind, request, actions) in &answered_cases {
        let then = [
            json!({ "do": "submit", "request": request, "as": "accepted" }),
            json!({ "do": "drain", "until": { "response": 2 }, "as": format!("{name}-answered") }),
            json!({ "do": "shutdown" }),
        ];
        steps.extend(attached(
            name,
            status_peer(json!({ *kind: actions })),
            json!({}),
            &then,
        ));
    }
    let acknowledgement = json!({ "kind": "acknowledgement", "changed": true });
    let viewed_true = json!({ "kind": "viewing", "changed": false, "viewing": true });
    let mismatched_cases: Vec<AnswerCase> = vec![
        (
            "viewing-on-contradicted",
            "viewing",
            on.clone(),
            viewing_ok(true, false),
        ),
        (
            "viewing-off-contradicted",
            "viewing",
            off.clone(),
            viewing_ok(true, true),
        ),
        (
            "viewing-acknowledged",
            "viewing",
            on.clone(),
            vec![answered("viewing", "ok", acknowledgement, Value::Null)],
        ),
        (
            "acknowledge-viewed",
            "acknowledge",
            ack.clone(),
            vec![answered("acknowledge", "ok", viewed_true, Value::Null)],
        ),
    ];
    let lost_cases: Vec<AnswerCase> = vec![
        ("viewing-held", "viewing", on.clone(), vec!["hold".into()]),
        ("viewing-lost", "viewing", on.clone(), vec!["close".into()]),
        (
            "acknowledge-held",
            "acknowledge",
            ack.clone(),
            vec!["hold".into()],
        ),
        (
            "acknowledge-lost",
            "acknowledge",
            ack.clone(),
            vec!["close".into()],
        ),
    ];
    for (name, kind, request, actions) in mismatched_cases.iter().chain(&lost_cases) {
        let then = [
            json!({ "do": "submit", "request": request, "as": "accepted" }),
            json!({ "do": "wait_run" }),
        ];
        let startup = json!({ "limits": { "operation_ms": 300 } });
        steps.extend(attached(
            name,
            status_peer(json!({ *kind: actions })),
            startup,
            &then,
        ));
    }
    let report = script(&package, &root, json!(steps));
    assert_accepted(&report, &steps);
    let expected = [
        (VIEWING_RESULT, viewed(1, 1)),
        (VIEWING_RESULT, viewed(0, 0)),
        (ACKNOWLEDGEMENT, acknowledged(1)),
        (ACKNOWLEDGEMENT, acknowledged(0)),
    ];
    for ((name, _, _, _), (tag, result)) in answered_cases.iter().zip(&expected) {
        let records = drained(&report, &steps, &format!("{name}-answered"));
        assert_eq!(assert_ok(answer(&records, 2), 1, 2, *tag), result, "{name}");
    }
    let acknowledging = drained(&report, &steps, "acknowledged-answered");
    assert_eq!(
        tags(&acknowledging),
        [STATUS, RESPONSE],
        "{acknowledging:#?}"
    );
    assert_eq!(
        acknowledging[0]["status"],
        status(ACKNOWLEDGED, Some(3), LATER)
    );
    for (name, code) in [
        ("stale-period", "stale_period"),
        ("not-ready", "not_ready"),
        ("stale-restoration", "stale_restoration"),
        ("not-viewing", "not_viewing"),
    ] {
        let records = drained(&report, &steps, &format!("{name}-answered"));
        assert_rejected(answer(&records, 2), 1, 2, ORIGIN_DAEMON, code);
    }
    for (name, kind, request, _) in &answered_cases {
        let (_, terminal) = ended(&report, &steps, name);
        assert_detached(terminal, 1);
        let peer = &report["peers"][name];
        assert_eq!(kinds(peer), ["hello", "open", kind, "leave"], "{name}");
        let mut body = json!({ "request_id": "2", "period": "2", "restoration_id": "2" });
        if *kind == "viewing" {
            body["viewing"] = json!(request["value"] == 1);
        }
        assert_eq!(peer["frames"][2]["body"], body, "{name}");
    }
    let invalid = (ORIGIN_TRANSPORT, "invalid_record");
    for (name, kind, _, _) in &mismatched_cases {
        let (records, terminal) = ended(&report, &steps, name);
        assert_eq!(records.len(), 1, "{name}: {records:#?}");
        assert_diagnostic(&records[0], 1, invalid.0, invalid.1);
        assert_terminal(
            terminal,
            1,
            (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
            Some(invalid),
        );
        assert_eq!(
            kinds(&report["peers"][name]),
            ["hello", "open", kind],
            "{name}"
        );
    }
    for (name, kind, _, actions) in &lost_cases {
        let (records, terminal) = ended(&report, &steps, name);
        let error = if actions[0] == "hold" {
            assert_eq!(records.len(), 2, "{name}: {records:#?}");
            let response = assert_response(&records[0], 1, 2, INDETERMINATE);
            assert_eq!(response["result_tag"], RESULT_NONE, "{name}");
            assert_error(&response["error"], ORIGIN_LIBRARY, "timeout");
            (ORIGIN_LIBRARY, "timeout")
        } else {
            assert_eq!(records.len(), 1, "{name}: {records:#?}");
            (ORIGIN_TRANSPORT, "io_error")
        };
        assert_diagnostic(records.last().unwrap(), 1, error.0, error.1);
        assert_terminal(
            terminal,
            1,
            (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
            Some(error),
        );
        assert_eq!(
            kinds(&report["peers"][name]),
            ["hello", "open", kind],
            "{name}"
        );
    }
    assert_all_released(&report["ledger"]);
}

/// A status case: its name, what the daemon sends after the Open's
/// success, the tags delivered, and the error and retry that end it.
type StatusCase = (
    &'static str,
    Vec<Value>,
    Vec<u64>,
    Option<(u64, &'static str)>,
    u64,
);

/// STA-07 Library and Fault (status records keep the daemon's identity
/// exactly, and nothing is fabricated): with `status-v1` selected, the
/// Initial metadata's status and every Status changed reach the caller
/// with their state, episode and stored change time as sent, each of the
/// six states mapping to its native value in order and the episode present
/// exactly for attention; the final status arrives only with the Command
/// final, completed for exit 0 and failed otherwise, and an Initial
/// metadata that knew the result carries its final status; the daemon's
/// close then stops the stream cleanly. A status record owns no buffer: it
/// costs the event allowance its record's charge alone. The connection
/// fails with transport `invalid_record` on a Status changed before the
/// Initial metadata, without `status-v1` selected, after the result was
/// known (from the Initial metadata or a Command final), or carrying a
/// final state or an episode outside attention; on an Initial metadata
/// whose status availability differs from the selection, or whose status is
/// final without the result or nonfinal with it; and on a Command final
/// without the status `status-v1` was selected for. CTL-03 Library and
/// Fault (status case): a passive registration receives all of it without
/// control.
#[test]
fn installed_library_relays_status_records_by_the_reference_rules() {
    let package = Package::new();
    let root = root(&package);
    let idle = known(wire("idle", TIME, None));
    let invalid = (ORIGIN_TRANSPORT, "invalid_record");
    let selected_records = |records: Vec<Value>| {
        let mut sent = vec![opened(&WATCHED)];
        sent.extend(records);
        sent
    };
    let failed = command_final(exit(3), wire("failed", LATER, None));
    let completed = command_final(exit(0), wire("completed", LATER, None));
    let cases: Vec<StatusCase> = vec![
        (
            "stream",
            selected_records(vec![
                metadata(known(wire("needs_attention", TIME, Some("7"))), Value::Null),
                status_changed(wire("acknowledged", TIME, Some("7"))),
                status_changed(wire("idle", LATER, None)),
                status_changed(wire("running", LATER, None)),
                status_changed(wire("needs_attention", LATER, Some("18446744073709551615"))),
                failed.clone(),
                "close".into(),
            ]),
            vec![
                RESPONSE,
                INITIAL_METADATA,
                STATUS,
                STATUS,
                STATUS,
                STATUS,
                FINAL,
            ],
            None,
            RETRY_NONE,
        ),
        (
            "completed",
            selected_records(vec![
                metadata(idle.clone(), Value::Null),
                completed.clone(),
                "close".into(),
            ]),
            vec![RESPONSE, INITIAL_METADATA, FINAL],
            None,
            RETRY_NONE,
        ),
        (
            "known",
            selected_records(vec![
                metadata(known(wire("completed", TIME, None)), exit(0)),
                "close".into(),
            ]),
            vec![RESPONSE, INITIAL_METADATA],
            None,
            RETRY_NONE,
        ),
        (
            "early",
            selected_records(vec![
                status_changed(wire("running", TIME, None)),
                "hold".into(),
            ]),
            vec![RESPONSE, DIAGNOSTIC],
            Some(invalid),
            RETRY_PASSIVE,
        ),
        (
            "unselected",
            vec![
                opened(&["daemon-wire-v1", "monitor-v1", "title-v1"]),
                metadata(unknown(), Value::Null),
                status_changed(wire("running", TIME, None)),
                "hold".into(),
            ],
            vec![RESPONSE, INITIAL_METADATA, DIAGNOSTIC],
            Some(invalid),
            RETRY_PASSIVE,
        ),
        (
            "available-unselected",
            vec![
                opened(&["daemon-wire-v1", "monitor-v1", "title-v1"]),
                metadata(idle.clone(), Value::Null),
                "hold".into(),
            ],
            vec![RESPONSE, DIAGNOSTIC],
            Some(invalid),
            RETRY_PASSIVE,
        ),
        (
            "unavailable-selected",
            selected_records(vec![metadata(unknown(), Value::Null), "hold".into()]),
            vec![RESPONSE, DIAGNOSTIC],
            Some(invalid),
            RETRY_PASSIVE,
        ),
        (
            "final-without-result",
            selected_records(vec![
                metadata(known(wire("completed", TIME, None)), Value::Null),
                "hold".into(),
            ]),
            vec![RESPONSE, DIAGNOSTIC],
            Some(invalid),
            RETRY_PASSIVE,
        ),
        (
            "result-without-final",
            selected_records(vec![
                metadata(known(wire("running", TIME, None)), exit(0)),
                "hold".into(),
            ]),
            vec![RESPONSE, DIAGNOSTIC],
            Some(invalid),
            RETRY_PASSIVE,
        ),
        (
            "after-known",
            selected_records(vec![
                metadata(known(wire("failed", TIME, None)), exit(3)),
                status_changed(wire("running", LATER, None)),
                "hold".into(),
            ]),
            vec![RESPONSE, INITIAL_METADATA, DIAGNOSTIC],
            Some(invalid),
            RETRY_NONE,
        ),
        (
            "after-final",
            selected_records(vec![
                metadata(idle.clone(), Value::Null),
                completed.clone(),
                status_changed(wire("running", LATER, None)),
                "hold".into(),
            ]),
            vec![RESPONSE, INITIAL_METADATA, FINAL, DIAGNOSTIC],
            Some(invalid),
            RETRY_NONE,
        ),
        (
            "final-without-status",
            selected_records(vec![
                metadata(idle.clone(), Value::Null),
                command_final(exit(0), Value::Null),
                "hold".into(),
            ]),
            vec![RESPONSE, INITIAL_METADATA, DIAGNOSTIC],
            Some(invalid),
            RETRY_PASSIVE,
        ),
        (
            "final-state",
            selected_records(vec![
                metadata(idle.clone(), Value::Null),
                status_changed(wire("completed", LATER, None)),
                "hold".into(),
            ]),
            vec![RESPONSE, INITIAL_METADATA, DIAGNOSTIC],
            Some(invalid),
            RETRY_PASSIVE,
        ),
        (
            "running-episode",
            selected_records(vec![
                metadata(idle.clone(), Value::Null),
                status_changed(wire("running", LATER, Some("1"))),
                "hold".into(),
            ]),
            vec![RESPONSE, INITIAL_METADATA, DIAGNOSTIC],
            Some(invalid),
            RETRY_PASSIVE,
        ),
    ];
    let hello = json!({ "json": { "capabilities": ADVERTISED } });
    let mut steps = Vec::new();
    for (name, sent, _, _, _) in &cases {
        let peer = json!({ "hello": hello, "open": sent });
        steps.extend(
            [
                json!({ "do": "create", "peer": peer }),
                json!({ "do": "submit", "request": open(1, &REQUIRED, &OPTIONAL_STATUS) }),
                json!({ "do": "run" }),
                json!({ "do": "finish", "as": format!("finished-{name}") }),
            ]
            .into_iter()
            .map(|mut step| {
                step["instance"] = json!(name);
                step
            }),
        );
    }
    let charged = json!({
        "hello": hello,
        "open": [opened(&WATCHED), metadata(idle.clone(), Value::Null),
                 status_changed(wire("running", LATER, None))],
        "leave": [leave_answer("ok", false, Value::Null)],
    });
    steps.extend(
        [
            json!({ "do": "create", "peer": charged }),
            json!({ "do": "submit", "request": open(1, &REQUIRED, &OPTIONAL_STATUS) }),
            json!({ "do": "run" }),
            json!({ "do": "await_snapshot", "gauge": "event_records", "value": 3,
                    "as": "charged" }),
            json!({ "do": "shutdown" }),
            json!({ "do": "finish", "as": "finished-charged" }),
        ]
        .into_iter()
        .map(|mut step| {
            step["instance"] = json!("charged");
            step
        }),
    );
    let report = script(&package, &root, json!(steps));
    for (name, _, expected, error, retry) in &cases {
        let (records, terminal) = ended(&report, &steps, name);
        assert_eq!(&tags(&records), expected, "{name}: {records:#?}");
        let outcome = match error {
            Some((origin, code)) => {
                assert_diagnostic(records.last().unwrap(), 1, *origin, code);
                CLIENT_FAILED
            }
            None => CLIENT_STOPPED,
        };
        assert_terminal(terminal, 1, (outcome, *retry, LEAVE_NOT_REQUIRED), *error);
    }
    let (stream, _) = ended(&report, &steps, "stream");
    let pending = status(NEEDS_ATTENTION, Some(7), TIME);
    assert_eq!(stream[1]["metadata"]["status"], available(pending));
    assert_eq!(stream[1]["metadata"]["command_result"]["present"], 0);
    let changes = [
        status(ACKNOWLEDGED, Some(7), TIME),
        status(IDLE, None, LATER),
        status(RUNNING, None, LATER),
        status(NEEDS_ATTENTION, Some(u64::MAX), LATER),
    ];
    for (record, expected) in stream[2..6].iter().zip(&changes) {
        assert_header(record, STATUS, 1);
        assert_eq!(&record["status"], expected, "{record:#}");
    }
    assert_header(&stream[6], FINAL, 1);
    assert_eq!(
        stream[6]["final"]["command_result"],
        native_command(EXIT, 3, None)
    );
    let finished_failed = available(status(STATUS_FAILED, None, LATER));
    assert_eq!(stream[6]["final"]["status"], finished_failed);
    let (completed, _) = ended(&report, &steps, "completed");
    assert_eq!(
        completed[1]["metadata"]["status"],
        available(status(IDLE, None, TIME))
    );
    assert_eq!(
        completed[2]["final"]["command_result"],
        native_command(EXIT, 0, None)
    );
    let finished_completed = available(status(COMPLETED, None, LATER));
    assert_eq!(completed[2]["final"]["status"], finished_completed);
    let (knowing, _) = ended(&report, &steps, "known");
    let metadata = &knowing[1]["metadata"];
    assert_eq!(metadata["status"], available(status(COMPLETED, None, TIME)));
    assert_eq!(
        metadata["command_result"]["value"],
        native_command(EXIT, 0, None)
    );
    let (unselected, _) = ended(&report, &steps, "unselected");
    assert_eq!(unselected[1]["metadata"]["status"], unavailable());
    let charges = &labelled(&report, &steps, "charged")["output"];
    let selected = TOKEN * WATCHED.len() as u64;
    assert_eq!(
        charges["event_bytes"],
        3 * CHARGE + selected + "t".len() as u64,
        "{charges:#}"
    );
    let (records, terminal) = ended(&report, &steps, "charged");
    assert_eq!(
        tags(&records),
        [RESPONSE, INITIAL_METADATA, STATUS],
        "{records:#?}"
    );
    assert_left(terminal, 1);
    assert_all_released(&report["ledger"]);
}
