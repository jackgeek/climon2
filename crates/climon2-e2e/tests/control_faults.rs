//! Library layer and Fault (#94): control and display through the
//! installed client library against the host's fake daemon peer, which
//! holds, loses, mismatches or misorders what a real daemon never would.
//! Every case records what reached the fake daemon, so a replayed or
//! wrongly sent request cannot hide. Expected values come from the native
//! header, the native contract and the daemon wire reference.

#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use support::{Package, control::*, library::CHARGE, passive::*};

/// Each control request under the authority the attach granted.
fn commands(id: u64) -> [(&'static str, Value); 5] {
    [
        ("acquire", acquire(id, 1)),
        ("input", input(id, 1, CURRENT, "4142")),
        ("resize", resize(id, 1, CURRENT, 90, 25)),
        ("ready", ready(id, 1, CURRENT)),
        ("focus", focus(id, 1, CURRENT, 1)),
    ]
}

/// COM-02 and COM-03 Library control cases, local rejection: `submit`
/// refuses each malformed control request synchronously with
/// `invalid_argument`, retains nothing and never consumes its ID (an
/// interactive Open at zero geometry, Acquire of generation zero, Input with
/// a zero period, no bytes or more than `chunk_bytes`, Resize to zero
/// columns, Ready with a zero restoration, Resize whose authority names
/// generation zero, Focus of 2 or with a reserved
/// byte set); Input of `chunk_bytes` whose charge exceeds `request_bytes`
/// by one is `queue_full`, while Input charging exactly `request_bytes` is
/// accepted. The accepted request is then cancelled by shutdown itself,
/// before it returns and before `run`: the request queue is empty at once
/// and the cancellation is the only record. Nothing reaches the daemon.
#[test]
fn installed_library_rejects_malformed_control_requests_locally() {
    let package = Package::new();
    let malformed = [
        json!({ "tag": "open", "request_id": 1,
                "open": { "mode": INTERACTIVE, "geometry": [0, 24] } }),
        acquire(1, 0),
        json!({ "tag": "input", "request_id": 1, "authority": [1, 0, 2],
                "bytes": { "hex": "41" } }),
        json!({ "tag": "input", "request_id": 1, "authority": [1, 2, 2],
                "bytes": { "null": 0 } }),
        json!({ "tag": "input", "request_id": 1, "authority": [1, 2, 2],
                "bytes": { "hex": "414243444546" } }),
        json!({ "tag": "resize", "request_id": 1, "authority": [1, 2, 2],
                "geometry": [0, 25] }),
        json!({ "tag": "ready", "request_id": 1, "authority": [1, 2, 0] }),
        json!({ "tag": "resize", "request_id": 1, "authority": [0, 2, 2],
                "geometry": [90, 25] }),
        json!({ "tag": "focus", "request_id": 1, "authority": [1, 2, 2], "value": 2 }),
        json!({ "tag": "focus", "request_id": 1, "authority": [1, 2, 2], "value": 1,
                "flag_reserved": 1 }),
    ];
    let limits = json!({ "chunk_bytes": 5, "request_bytes": REQUEST + 4 });
    let mut steps = vec![json!({ "do": "create", "peer": attached_peer(json!({})),
                                 "startup": { "limits": limits } })];
    steps.extend(
        malformed
            .iter()
            .map(|request| json!({ "do": "submit", "request": request, "as": "malformed" })),
    );
    steps.extend([
        json!({ "do": "submit", "request": {
            "tag": "input", "request_id": 1, "authority": [1, 2, 2],
            "bytes": { "hex": "4142434445" } }, "as": "over" }),
        json!({ "do": "submit", "request": {
            "tag": "input", "request_id": 1, "authority": [1, 2, 2],
            "bytes": { "hex": "41424344" } }, "as": "at" }),
        json!({ "do": "snapshot", "as": "queued" }),
        json!({ "do": "shutdown" }),
        json!({ "do": "snapshot", "as": "cancelled" }),
        json!({ "do": "run" }),
        json!({ "do": "finish", "as": "finished" }),
    ]);
    let report = script(&package, &root(&package), json!(steps));
    let refused: Vec<usize> = (0..steps.len())
        .filter(|index| steps[*index]["as"] == "malformed")
        .collect();
    assert_eq!(refused.len(), malformed.len());
    for index in refused {
        assert_failure(step(&report, index), INVALID_ARGUMENT, "invalid_argument");
    }
    assert_failure(labelled(&report, &steps, "over"), FULL, "queue_full");
    assert_success(labelled(&report, &steps, "at"));
    let queued = &labelled(&report, &steps, "queued")["output"];
    assert_eq!(queued["request_records"], 1, "{queued:#}");
    assert_eq!(queued["request_bytes"], REQUEST + 4, "{queued:#}");
    let cancelled = &labelled(&report, &steps, "cancelled")["output"];
    assert_eq!(cancelled["request_records"], 0, "{cancelled:#}");
    assert_eq!(cancelled["request_bytes"], 0, "{cancelled:#}");
    assert_eq!(cancelled["operations"], 0, "{cancelled:#}");
    assert_eq!(cancelled["event_records"], 1, "{cancelled:#}");
    let (records, terminal) = finished(labelled(&report, &steps, "finished"));
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_rejected(&records[0], 1, 1, ORIGIN_LIBRARY, "cancelled");
    assert_terminal(
        terminal,
        1,
        (CLIENT_STOPPED, RETRY_NONE, LEAVE_NOT_REQUIRED),
        None,
    );
    assert_all_released(&report["ledger"]);
}

/// COM-02 and COM-03 Library control cases, `cancelled` and acceptance:
/// while an interactive Open is under way, every control request waits
/// behind it; shutdown then rejects each one `cancelled` without sending
/// it, the Open completes and is answered (an interactive Open accepted:
/// the attach and its restoration follow), and shutdown's own Leave departs
/// once, detaching the attachment.
#[test]
fn installed_library_cancels_queued_control_requests_on_shutdown() {
    let package = Package::new();
    let root = root(&package);
    let go = root.join("go");
    let mut records = vec![json!({ "wait": go.to_str().unwrap() })];
    records.extend(attach_records(&ATTACHED));
    let mut peer = attached_peer(json!({}));
    peer["open"] = json!(records);
    let mut steps = vec![
        json!({ "do": "create", "peer": peer }),
        json!({ "do": "submit", "request": interactive(1, 80, 24) }),
    ];
    let authority = [1, PERIOD, 2];
    let queued = [
        acquire(2, 1),
        json!({ "tag": "input", "request_id": 3, "authority": authority,
                "bytes": { "hex": "41" } }),
        json!({ "tag": "resize", "request_id": 4, "authority": authority,
                "geometry": [90, 25] }),
        json!({ "tag": "ready", "request_id": 5, "authority": authority }),
        json!({ "tag": "focus", "request_id": 6, "authority": authority, "value": 1 }),
    ];
    steps.extend(
        queued
            .iter()
            .map(|request| json!({ "do": "submit", "request": request })),
    );
    steps.extend([
        json!({ "do": "run" }),
        json!({ "do": "await_peer", "frames": 2 }),
        json!({ "do": "shutdown" }),
        json!({ "do": "touch", "path": go.to_str().unwrap() }),
        json!({ "do": "finish", "as": "finished" }),
    ]);
    let report = script(&package, &root, json!(steps));
    let (records, terminal) = finished(labelled(&report, &steps, "finished"));
    for (index, id) in (2..=6).enumerate() {
        assert_rejected(&records[index], 1, id, ORIGIN_LIBRARY, "cancelled");
    }
    assert_ok(&records[5], 1, 1, OPENED);
    assert_header(&records[6], INITIAL_METADATA, 1);
    assert_restored(&records[7..11], 1, ATTACH, [80, 24]);
    assert_eq!(records.len(), 11, "{records:#?}");
    assert_detached(terminal, 1);
    assert_eq!(kinds(&report["peers"]["a"]), ["hello", "open", "leave"]);
    assert_all_released(&report["ledger"]);
}

/// COM-02 and COM-03 Library control cases, stale generation and state:
/// each control request naming another generation is rejected
/// `stale_generation`, Focus without `focus-v1` selected
/// `unsupported_capability`, and a request after the instance's Leave is
/// under way `invalid_state`, all locally and never sent: the fake daemon
/// receives the Open and the Leave only. An Input whose frame exceeds the
/// daemon's request frame ceiling is rejected `unsupported_limit`, never
/// sent, and the role goes on; that daemon advertises no `monitor-v1`,
/// which an interactive Open does not need.
#[test]
fn installed_library_rejects_stale_and_unsupported_control_requests_locally() {
    let package = Package::new();
    let package_root = root(&package);
    let go = package_root.join("go-leave");
    let mut peer = attached_peer(json!({}));
    peer["open"] = json!(attach_records(&["daemon-wire-v1", "title-v1"]));
    peer["leave"] = json!([
        { "wait": go.to_str().unwrap() },
        leave_answer("ok", true, Value::Null),
    ]);
    let mut then: Vec<Value> = commands(0)
        .into_iter()
        .enumerate()
        .map(|(index, (_, mut request))| {
            request["request_id"] = json!(index as u64 + 2);
            if request["generation"].is_u64() {
                request["generation"] = json!(9);
            } else {
                request["authority"][0] = json!(9);
            }
            json!({ "do": "submit", "request": request })
        })
        .collect();
    then.extend([
        json!({ "do": "submit", "request": focus(7, 1, CURRENT, 1) }),
        json!({ "do": "submit", "request": leave(8, 1) }),
        json!({ "do": "await_peer", "frames": 3 }),
        json!({ "do": "submit", "request": input(9, 1, CURRENT, "41") }),
        json!({ "do": "drain", "until": { "response": 9 } }),
        json!({ "do": "touch", "path": go.to_str().unwrap() }),
    ]);
    let mut plan = Plan::new();
    plan.add("a", attached("a", peer, json!({}), &then));
    let mut unfit = attached_peer(json!({}));
    let without_monitor: Vec<&str> = ADVERTISED
        .into_iter()
        .filter(|family| *family != "monitor-v1")
        .collect();
    unfit["hello"] = json!({ "json": {
        "capabilities": without_monitor,
        "daemon_wire_v1": { "max_request_frame_bytes": 256, "max_server_frame_bytes": 1_048_576 },
    } });
    let large = "41".repeat(300);
    let then = [
        json!({ "do": "submit", "request": input(2, 1, CURRENT, &large) }),
        json!({ "do": "drain", "instance": "unfit", "until": { "response": 2 },
                "as": "unfit-answered" }),
        json!({ "do": "shutdown" }),
    ];
    plan.add("unfit", attached("unfit", unfit, json!({}), &then));
    let steps = plan.steps.clone();
    let (report, finishes) = plan.run(&package);
    let unfit_drained = drained(&report, &steps, "unfit-answered");
    assert_rejected(&unfit_drained[0], 1, 2, ORIGIN_LIBRARY, "unsupported_limit");
    let (_, terminal) = finished(step(&report, finishes[1].1));
    assert_detached(terminal, 1);
    assert_eq!(kinds(&report["peers"]["unfit"]), ["hello", "open", "leave"]);
    let (rest, terminal) = finished(step(&report, finishes[0].1));
    let records: Vec<Value> = report["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|step| step["records"].as_array())
        .flatten()
        .chain(&rest)
        .cloned()
        .collect();
    for id in 2..=6 {
        assert_refused(&records, id, "stale_generation");
    }
    assert_refused(&records, 7, "unsupported_capability");
    assert_refused(&records, 9, "invalid_state");
    let left = assert_ok(answer(&records, 8), 1, 8, LEFT);
    assert_eq!(left, &json!({ "attachment_detached": 1 }));
    assert_detached(terminal, 1);
    assert_eq!(kinds(&report["peers"]["a"]), ["hello", "open", "leave"]);
    assert_all_released(&report["ledger"]);
}

fn assert_refused(records: &[Value], id: u64, code: &str) {
    assert_rejected(answer(records, id), 1, id, ORIGIN_LIBRARY, code);
}

/// COM-02 and COM-03 Library control cases, `timeout` and lost responses:
/// for each control request (Acquire, Input, Resize, Ready and Focus) and
/// for an interactive Open, a daemon that holds its answer past
/// `operation_ms` (`start_ms` for the Open) gets it answered indeterminate
/// `timeout`, and the connection fails; a daemon that closes the connection
/// instead gets no fabricated Response at all, only the failure, which
/// leaves the request indeterminate in the caller's ledger. Either way the
/// request reached the daemon exactly once: nothing is replayed.
#[test]
fn installed_library_times_out_or_loses_control_requests_without_replay() {
    let package = Package::new();
    let mut plan = Plan::new();
    let mut expected = Vec::new();
    for (kind, request) in commands(2) {
        for (fate, action) in [("held", "hold"), ("lost", "close")] {
            let name = format!("{kind}-{fate}");
            let peer = attached_peer(json!({ kind: [action] }));
            let startup = json!({ "limits": { "operation_ms": 300 } });
            let then = [
                json!({ "do": "submit", "request": request }),
                json!({ "do": "wait_run" }),
            ];
            plan.add(&name, attached(&name, peer, startup, &then));
            expected.push((name, kind, fate));
        }
    }
    let held_open = attached_peer(json!({ "open": ["hold"] }));
    plan.add(
        "open-held",
        vec![
            json!({ "do": "create", "instance": "open-held", "peer": held_open,
                    "startup": { "limits": { "start_ms": 300 } } }),
            json!({ "do": "submit", "instance": "open-held", "request": interactive(1, 80, 24) }),
            json!({ "do": "run", "instance": "open-held" }),
            json!({ "do": "wait_run", "instance": "open-held" }),
        ],
    );
    let (report, finishes) = plan.run(&package);
    let timeout = (ORIGIN_LIBRARY, "timeout");
    let lost = (ORIGIN_TRANSPORT, "io_error");
    for ((name, kind, fate), (_, finish)) in expected.iter().zip(&finishes) {
        let (records, terminal) = finished(step(&report, *finish));
        let error = if *fate == "held" {
            assert_eq!(records.len(), 2, "{name}: {records:#?}");
            let response = assert_response(&records[0], 1, 2, INDETERMINATE);
            assert_eq!(response["result_tag"], RESULT_NONE, "{name}");
            assert_error(&response["error"], timeout.0, timeout.1);
            timeout
        } else {
            assert_eq!(records.len(), 1, "{name}: {records:#?}");
            lost
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
            ["hello", "open", *kind],
            "{name}"
        );
    }
    let (records, terminal) = finished(step(&report, finishes.last().unwrap().1));
    assert_eq!(records.len(), 2, "{records:#?}");
    let response = assert_response(&records[0], 1, 1, INDETERMINATE);
    assert_error(&response["error"], timeout.0, timeout.1);
    assert_diagnostic(&records[1], 1, timeout.0, timeout.1);
    assert_terminal(
        terminal,
        1,
        (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
        Some(timeout),
    );
    assert_eq!(kinds(&report["peers"]["open-held"]), ["hello", "open"]);
    assert_all_released(&report["ledger"]);
}

/// COM-02 Library control cases, the daemon's answers: each control
/// request's success maps to its own native result (Acquire and Ready the
/// authority, Input every byte, Resize nothing, Focus changed, value and
/// bytes forwarded) and a daemon rejection keeps the daemon's error; a
/// success carrying another request's result (for each command), Input
/// accepting fewer bytes than it carried, or a command's rejection naming a
/// known final result, which only a first role's may, fails the connection
/// with `invalid_record`.
#[test]
fn installed_library_maps_control_answers_and_refuses_mismatched_results() {
    let package = Package::new();
    let authority = json!({ "kind": "acquired", "changed": true, "period": "2",
                            "restoration_id": "2" });
    let results = [
        (
            "acquire",
            authority.clone(),
            ACQUIRED,
            json!({ "changed": 1, "reserved": [0, 0, 0, 0, 0, 0, 0], "period": 2, "restoration_id": 2 }),
        ),
        (
            "input",
            json!({ "kind": "input_accepted", "accepted_bytes": 2 }),
            INPUT_ACCEPTED,
            json!({ "accepted_bytes": 2 }),
        ),
        (
            "resize",
            json!({ "kind": "resize_accepted" }),
            RESIZE_ACCEPTED,
            Value::Null,
        ),
        (
            "ready",
            json!({ "kind": "ready", "changed": false, "period": "2", "restoration_id": "2" }),
            READY_RESULT,
            json!({ "changed": 0, "reserved": [0, 0, 0, 0, 0, 0, 0], "period": 2, "restoration_id": 2 }),
        ),
        (
            "focus",
            json!({ "kind": "focus", "changed": true, "focused": true, "accepted_bytes": 3 }),
            FOCUS_RESULT,
            json!({ "changed": 1, "value": 1, "reserved": [0, 0], "accepted_bytes": 3 }),
        ),
    ];
    let mut plan = Plan::new();
    for ((kind, success, _, _), (_, request)) in results.iter().zip(commands(2)) {
        let mut second = request.clone();
        second["request_id"] = json!(3);
        let peer = attached_peer(json!({
            *kind: [answered(kind, "ok", success.clone(), Value::Null)],
        }));
        let name = format!("{kind}-ok");
        let then = [
            json!({ "do": "submit", "request": request }),
            json!({ "do": "drain", "until": { "response": 2 }, "as": format!("{name}-answered") }),
            json!({ "do": "shutdown" }),
        ];
        plan.add(&name, attached(&name, peer, json!({}), &then));
        let rejected = attached_peer(json!({
            *kind: [answered(kind, "rejected", Value::Null, daemon_error("not_ready"))],
        }));
        let name = format!("{kind}-rejected");
        let then = [
            json!({ "do": "submit", "request": second }),
            json!({ "do": "drain", "until": { "response": 3 }, "as": format!("{name}-answered") }),
            json!({ "do": "shutdown" }),
        ];
        plan.add(&name, attached(&name, rejected, json!({}), &then));
    }
    let left = json!({ "kind": "left", "attachment_detached": false });
    let mut wrong: Vec<(String, &str, Value)> = commands(2)
        .into_iter()
        .map(|(kind, _)| {
            let mismatched = answered(kind, "ok", left.clone(), Value::Null);
            (format!("{kind}-mismatched"), kind, mismatched)
        })
        .collect();
    let partial = json!({ "kind": "input_accepted", "accepted_bytes": 1 });
    wrong.push((
        "partial".into(),
        "input",
        answered("input", "ok", partial, Value::Null),
    ));
    let known = json!({ "kind": "known_final", "command_result": { "kind": "exit", "code": 3 } });
    let rejected_known = answered(
        "input",
        "rejected",
        known,
        daemon_error("session_terminating"),
    );
    wrong.push(("known-final".into(), "input", rejected_known));
    let requests: Vec<(&str, Value)> = commands(2).into_iter().collect();
    for (name, kind, action) in &wrong {
        let peer = attached_peer(json!({ *kind: [action] }));
        let request = &requests.iter().find(|(named, _)| named == kind).unwrap().1;
        let then = [json!({ "do": "submit", "request": request })];
        plan.add(name, attached(name, peer, json!({}), &then));
    }
    let steps = plan.steps.clone();
    let (report, finishes) = plan.run(&package);
    let mut finishes = finishes.into_iter();
    for (kind, _, tag, result) in &results {
        let (_, finish) = finishes.next().unwrap();
        let ok = drained(&report, &steps, &format!("{kind}-ok-answered"));
        assert_eq!(assert_ok(answer(&ok, 2), 1, 2, *tag), result, "{kind}");
        let (_, terminal) = finished(step(&report, finish));
        assert_detached(terminal, 1);
        let (_, finish) = finishes.next().unwrap();
        let refused = drained(&report, &steps, &format!("{kind}-rejected-answered"));
        assert_rejected(answer(&refused, 3), 1, 3, ORIGIN_DAEMON, "not_ready");
        let (_, terminal) = finished(step(&report, finish));
        assert_detached(terminal, 1);
    }
    for (name, _, _) in &wrong {
        let (_, finish) = finishes.next().unwrap();
        let (records, terminal) = finished(step(&report, finish));
        assert_eq!(records.len(), 1, "{name}: {records:#?}");
        assert_diagnostic(&records[0], 1, ORIGIN_TRANSPORT, "invalid_record");
        assert_terminal(
            terminal,
            1,
            (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
            Some((ORIGIN_TRANSPORT, "invalid_record")),
        );
    }
    assert_all_released(&report["ledger"]);
}

/// A presentation case: its name, what the daemon sends after the Open, the
/// tags of the records delivered, and the error that ends the connection.
type RecordCase = (&'static str, Vec<Value>, Vec<u64>, (u64, &'static str));

/// COM-03 and CTL-05 Library and Fault (presentation records follow the
/// reference's rejection rules, and old authority never reaches the
/// caller): the connection fails with transport `invalid_record`, after
/// delivering what came before, on an empty Display (owner decision on
/// #81: the shared codec decodes it, the library refuses it), a Display
/// under a restoration it does not hold, a Display after its revocation, a
/// grant before the Initial metadata, a second grant without a revocation,
/// a Restoration begin or Geometry of another period, a Restoration begin
/// that does not follow the held restoration (the granted one again
/// after its beginning, or an older one: restoration IDs increase within
/// a period), and a Restoration
/// complete or Control revoked of a restoration it does not hold. A
/// Display larger than `chunk_bytes`, which the daemon's frame size allows
/// whatever `chunk_bytes` is, is delivered in order as Displays of at most
/// `chunk_bytes` under its authority: one byte over makes two records, of
/// `chunk_bytes` and of one byte, while one of exactly `chunk_bytes` stays
/// one record; both streams stay healthy until the daemon closes them.
#[test]
fn installed_library_refuses_presentation_records_out_of_authority() {
    let package = Package::new();
    let invalid = (ORIGIN_TRANSPORT, "invalid_record");
    let closed = (ORIGIN_TRANSPORT, "io_error");
    let opening = || {
        vec![
            opened(&ATTACHED),
            initial(json!("t")),
            granted(PERIOD, 2, "attach"),
            begin(PERIOD, 2),
        ]
    };
    let with = |tail: Vec<Value>| {
        let mut records = opening();
        records.extend(tail);
        records.push("hold".into());
        records
    };
    let cases: Vec<RecordCase> = vec![
        (
            "empty-display",
            with(vec![shown(PERIOD, 2, "")]),
            vec![
                RESPONSE,
                INITIAL_METADATA,
                GRANTED,
                RESTORATION_BEGIN,
                DIAGNOSTIC,
            ],
            invalid,
        ),
        (
            "unheld-display",
            with(vec![shown(PERIOD, 3, "41")]),
            vec![
                RESPONSE,
                INITIAL_METADATA,
                GRANTED,
                RESTORATION_BEGIN,
                DIAGNOSTIC,
            ],
            invalid,
        ),
        (
            "late-display",
            with(vec![revoked(PERIOD, 2, "takeover"), shown(PERIOD, 2, "41")]),
            vec![
                RESPONSE,
                INITIAL_METADATA,
                GRANTED,
                RESTORATION_BEGIN,
                REVOKED,
                DIAGNOSTIC,
            ],
            invalid,
        ),
        (
            "early-grant",
            vec![
                opened(&ATTACHED),
                granted(PERIOD, 2, "attach"),
                "hold".into(),
            ],
            vec![RESPONSE, DIAGNOSTIC],
            invalid,
        ),
        (
            "regrant",
            with(vec![granted(3, 3, "acquire")]),
            vec![
                RESPONSE,
                INITIAL_METADATA,
                GRANTED,
                RESTORATION_BEGIN,
                DIAGNOSTIC,
            ],
            invalid,
        ),
        (
            "other-geometry",
            with(vec![geometry(3, 90, 25)]),
            vec![
                RESPONSE,
                INITIAL_METADATA,
                GRANTED,
                RESTORATION_BEGIN,
                DIAGNOSTIC,
            ],
            invalid,
        ),
        (
            "display-over",
            with(vec![shown(PERIOD, 2, "4142434445"), "close".into()]),
            vec![
                RESPONSE,
                INITIAL_METADATA,
                GRANTED,
                RESTORATION_BEGIN,
                DISPLAY,
                DISPLAY,
                DIAGNOSTIC,
            ],
            closed,
        ),
        (
            "repeated-begin",
            with(vec![begin(PERIOD, 2)]),
            vec![
                RESPONSE,
                INITIAL_METADATA,
                GRANTED,
                RESTORATION_BEGIN,
                DIAGNOSTIC,
            ],
            invalid,
        ),
        (
            "older-begin",
            with(vec![begin(PERIOD, 1)]),
            vec![
                RESPONSE,
                INITIAL_METADATA,
                GRANTED,
                RESTORATION_BEGIN,
                DIAGNOSTIC,
            ],
            invalid,
        ),
        (
            "foreign-begin",
            with(vec![begin(3, 3)]),
            vec![
                RESPONSE,
                INITIAL_METADATA,
                GRANTED,
                RESTORATION_BEGIN,
                DIAGNOSTIC,
            ],
            invalid,
        ),
        (
            "unheld-complete",
            with(vec![complete(PERIOD, 3)]),
            vec![
                RESPONSE,
                INITIAL_METADATA,
                GRANTED,
                RESTORATION_BEGIN,
                DIAGNOSTIC,
            ],
            invalid,
        ),
        (
            "unheld-revoke",
            with(vec![revoked(PERIOD, 3, "takeover")]),
            vec![
                RESPONSE,
                INITIAL_METADATA,
                GRANTED,
                RESTORATION_BEGIN,
                DIAGNOSTIC,
            ],
            invalid,
        ),
        (
            "display-at",
            with(vec![shown(PERIOD, 2, "41424344"), "close".into()]),
            vec![
                RESPONSE,
                INITIAL_METADATA,
                GRANTED,
                RESTORATION_BEGIN,
                DISPLAY,
                DIAGNOSTIC,
            ],
            closed,
        ),
    ];
    let mut plan = Plan::new();
    for (name, records, _, _) in &cases {
        let mut peer = attached_peer(json!({}));
        peer["open"] = json!(records);
        plan.add(
            name,
            vec![
                json!({ "do": "create", "instance": name, "peer": peer,
                        "startup": { "limits": { "chunk_bytes": 4 } } }),
                json!({ "do": "submit", "instance": name, "request": interactive(1, 80, 24) }),
                json!({ "do": "run", "instance": name }),
            ],
        );
    }
    let (report, finishes) = plan.run(&package);
    for ((name, _, expected, error), (_, finish)) in cases.iter().zip(&finishes) {
        let (records, terminal) = finished(step(&report, *finish));
        assert_eq!(&tags(&records), expected, "{name}: {records:#?}");
        assert_diagnostic(records.last().unwrap(), 1, error.0, error.1);
        assert_terminal(
            terminal,
            1,
            (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
            Some(*error),
        );
    }
    let chunks = |name: &str| -> Vec<String> {
        let (_, finish) = finishes.iter().find(|(case, _)| case == name).unwrap();
        let records = finished(step(&report, *finish)).0;
        tagged(&records, DISPLAY)
            .into_iter()
            .map(|record| display(std::slice::from_ref(record), 1, [PERIOD, 2]))
            .collect()
    };
    assert_eq!(chunks("display-over"), ["41424344", "45"]);
    assert_eq!(chunks("display-at"), ["41424344"]);
    assert_all_released(&report["ledger"]);
}

/// CTL-04 Library and Fault (Leave racing a promotion resolves
/// atomically): a passive registration's shutdown Leave meets the daemon
/// promoting it first; the library delivers the `successor` grant and its
/// restoration (Display `00 ff 41` exact), then reports the Leave
/// confirmed with the attachment detached, never a second Leave. COM-05
/// Library and Fault (records arrive in causal order with immutable
/// identities, and a fresh attempt never sees an old one's deliveries): an
/// attached instance of generation 1 receives, as a real daemon sends them
/// when its command ends, the Command final, a Geometry and its period's
/// revocation for `session_ending` before the connection closes, a clean
/// stop with no error and no retry; a fresh instance of generation 2 against
/// another daemon receives only its own records, each of generation 2.
#[test]
fn installed_library_resolves_a_promotion_racing_leave_and_fresh_attempts_stand_alone() {
    let package = Package::new();
    let monitor = ["daemon-wire-v1", "monitor-v1", "title-v1"];
    let promoting = json!({
        "open": [opened(&monitor), initial(json!("t"))],
        "leave": [
            granted(5, 5, "successor"),
            begin(5, 5),
            shown(5, 5, "00ff41"),
            complete(5, 5),
            leave_answer("ok", true, Value::Null),
        ],
    });
    let mut plan = Plan::new();
    plan.add(
        "racing",
        vec![
            json!({ "do": "create", "instance": "racing", "peer": promoting }),
            json!({ "do": "submit", "instance": "racing",
                    "request": open(1, &REQUIRED, &["title-v1"]) }),
            json!({ "do": "run", "instance": "racing" }),
            json!({ "do": "drain", "instance": "racing", "until": { "tag": INITIAL_METADATA } }),
            json!({ "do": "shutdown", "instance": "racing" }),
        ],
    );
    let mut ending = attach_records(&ATTACHED);
    ending.extend([
        command_final(json!({ "kind": "exit", "code": 0 })),
        geometry(PERIOD, 90, 25),
        revoked(PERIOD, 2, "session_ending"),
        "close".into(),
    ]);
    let mut old = attached_peer(json!({}));
    old["open"] = json!(ending);
    plan.add(
        "old",
        attached("old", old, json!({}), &[json!({ "do": "wait_run" })]),
    );
    let mut fresh_records = attach_records(&ATTACHED);
    fresh_records[4] = shown(PERIOD, 2, "42");
    let mut fresh = attached_peer(json!({}));
    fresh["open"] = json!(fresh_records);
    plan.add(
        "fresh",
        attached(
            "fresh",
            fresh,
            json!({ "generation": 2 }),
            &[json!({ "do": "shutdown" })],
        ),
    );
    let steps = plan.steps.clone();
    let (report, finishes) = plan.run(&package);
    let (records, terminal) = finished(step(&report, finishes[0].1));
    assert_restored(&records, 1, SUCCESSOR, [80, 24]);
    assert_eq!(display(&records, 1, [5, 5]), "00ff41");
    assert_eq!(records.len(), 4, "{records:#?}");
    assert_detached(terminal, 1);
    assert_eq!(
        kinds(&report["peers"]["racing"]),
        ["hello", "open", "leave"]
    );
    let (rest, terminal) = finished(step(&report, finishes[1].1));
    let attached_old = drained(&report, &steps, "attached-old");
    let all: Vec<Value> = attached_old.iter().chain(&rest).cloned().collect();
    let completed = all
        .iter()
        .position(|record| record["tag"] == RESTORATION_COMPLETE)
        .unwrap();
    let records = &all[completed + 1..];
    assert_eq!(records.len(), 3, "{records:#?}");
    assert_final(&records[0], 1, &native_command(EXIT, 0, None));
    let actual = presented(&records[1], GEOMETRY, 1);
    assert_eq!(actual, &json!({ "period": PERIOD, "geometry": [90, 25] }));
    let ended = presented(&records[2], REVOKED, 1);
    assert_eq!(ids(ended), [PERIOD, 2]);
    assert_eq!(ended["reason"], SESSION_ENDING);
    assert_terminal(
        terminal,
        1,
        (CLIENT_STOPPED, RETRY_NONE, LEAVE_NOT_REQUIRED),
        None,
    );
    let first = drained(&report, &steps, "attached-fresh");
    assert_eq!(
        tags(&first),
        [
            RESPONSE,
            INITIAL_METADATA,
            GRANTED,
            RESTORATION_BEGIN,
            DISPLAY,
            RESTORATION_COMPLETE
        ]
    );
    assert!(
        first.iter().all(|record| record["generation"] == 2),
        "{first:#?}"
    );
    assert_eq!(display(&first, 2, [PERIOD, 2]), "42");
    let (_, terminal) = finished(step(&report, finishes[2].1));
    assert_detached(terminal, 2);
    assert_all_released(&report["ledger"]);
}

/// A final-result case: its name, what the daemon sends after the Open's
/// success, the tags delivered, and the error and retry that end it.
type FinalCase = (
    &'static str,
    Vec<Value>,
    Vec<u64>,
    Option<(u64, &'static str)>,
    u64,
);

/// COM-03 Library and Fault (the Command final follows the reference's
/// rules; minimal handling, owner decision on #94): after an Initial
/// metadata that did not know the result, a Command final reaches the
/// caller as `CM1_FINAL` with its exit or signal and status unavailable,
/// and the stream reads on until the daemon closes it: a clean stop, with
/// no error and no retry (owner decision on #94). The connection fails with transport `invalid_record` on a
/// second Command final, on one after an Initial metadata that knew the
/// result (agreeing or contradicting it), on one before the Initial
/// metadata, and on one with a status `status-v1` was not selected for;
/// only a refused first result leaves passive retry. A close inside a
/// frame after the final is still `truncated_frame`, and a close that meets
/// shutdown's Leave after the final still leaves the Leave unconfirmed:
/// indeterminate, failed with `io_error`, retry none. Exit and signal
/// detail, repeat semantics and evidence surviving disconnection belong
/// to #96.
#[test]
fn installed_library_relays_one_command_final_after_the_initial_metadata() {
    let package = Package::new();
    let monitor = ["daemon-wire-v1", "monitor-v1", "title-v1"];
    let exit = |code: u64| json!({ "kind": "exit", "code": code });
    let signal = json!({ "kind": "signal", "number": 15, "name": "SIGTERM" });
    let knowing = |result: Value| {
        send(
            0x41,
            json!({
                "title": { "available": true, "value": "t" },
                "status": { "available": false, "value": null },
                "progress": { "available": false, "value": null },
                "command_result": result,
            }),
        )
    };
    let invalid = (ORIGIN_TRANSPORT, "invalid_record");
    let cases: Vec<FinalCase> = vec![
        (
            "signalled",
            vec![
                initial(json!("t")),
                command_final(signal.clone()),
                "close".into(),
            ],
            vec![RESPONSE, INITIAL_METADATA, FINAL],
            None,
            RETRY_NONE,
        ),
        (
            "truncated",
            vec![
                initial(json!("t")),
                command_final(exit(0)),
                json!({ "raw": "0000001049" }),
                "close".into(),
            ],
            vec![RESPONSE, INITIAL_METADATA, FINAL, DIAGNOSTIC],
            Some((ORIGIN_TRANSPORT, "truncated_frame")),
            RETRY_NONE,
        ),
        (
            "second",
            vec![
                initial(json!("t")),
                command_final(exit(0)),
                command_final(exit(0)),
                "hold".into(),
            ],
            vec![RESPONSE, INITIAL_METADATA, FINAL, DIAGNOSTIC],
            Some(invalid),
            RETRY_NONE,
        ),
        (
            "agreeing",
            vec![knowing(exit(3)), command_final(exit(3)), "hold".into()],
            vec![RESPONSE, INITIAL_METADATA, DIAGNOSTIC],
            Some(invalid),
            RETRY_NONE,
        ),
        (
            "contradicting",
            vec![
                knowing(exit(3)),
                command_final(signal.clone()),
                "hold".into(),
            ],
            vec![RESPONSE, INITIAL_METADATA, DIAGNOSTIC],
            Some(invalid),
            RETRY_NONE,
        ),
        (
            "early",
            vec![command_final(exit(0)), "hold".into()],
            vec![RESPONSE, DIAGNOSTIC],
            Some(invalid),
            RETRY_PASSIVE,
        ),
        (
            "with-status",
            vec![
                initial(json!("t")),
                send(
                    0x4c,
                    json!({
                        "command_result": exit(0),
                        "status": {
                            "state": "completed",
                            "changed_at": "2026-11-06T12:00:02.000Z",
                            "episode_id": null,
                        },
                    }),
                ),
                "hold".into(),
            ],
            vec![RESPONSE, INITIAL_METADATA, DIAGNOSTIC],
            Some(invalid),
            RETRY_PASSIVE,
        ),
    ];
    let mut plan = Plan::new();
    for (name, records, _, _, _) in &cases {
        let mut sent = vec![opened(&monitor)];
        sent.extend(records.iter().cloned());
        let peer = json!({ "open": sent });
        plan.add(
            name,
            vec![
                json!({ "do": "create", "instance": name, "peer": peer }),
                json!({ "do": "submit", "instance": name,
                        "request": open(1, &REQUIRED, &["title-v1"]) }),
                json!({ "do": "run", "instance": name }),
            ],
        );
    }
    let leaving = json!({
        "open": [opened(&monitor), initial(json!("t")), command_final(exit(0))],
        "leave": ["close"],
    });
    plan.add(
        "leaving",
        vec![
            json!({ "do": "create", "instance": "leaving", "peer": leaving }),
            json!({ "do": "submit", "instance": "leaving",
                    "request": open(1, &REQUIRED, &["title-v1"]) }),
            json!({ "do": "run", "instance": "leaving" }),
            json!({ "do": "drain", "instance": "leaving", "until": { "tag": FINAL } }),
            json!({ "do": "shutdown", "instance": "leaving" }),
        ],
    );
    let (report, finishes) = plan.run(&package);
    let (rest, terminal) = finished(step(&report, finishes[cases.len()].1));
    let lost = (ORIGIN_TRANSPORT, "io_error");
    assert_eq!(tags(&rest), [DIAGNOSTIC], "{rest:#?}");
    assert_terminal(
        terminal,
        1,
        (CLIENT_FAILED, RETRY_NONE, LEAVE_INDETERMINATE),
        Some(lost),
    );
    for ((name, _, expected, error, retry), (_, finish)) in cases.iter().zip(&finishes) {
        let (records, terminal) = finished(step(&report, *finish));
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
    let (signalled, _) = finished(step(&report, finishes[0].1));
    assert_final(
        &signalled[2],
        1,
        &native_command(SIGNAL, 15, Some("SIGTERM")),
    );
    let (second, _) = finished(step(&report, finishes[2].1));
    assert_final(&second[2], 1, &native_command(EXIT, 0, None));
    assert_all_released(&report["ledger"]);
}

/// COM-03 Library and Fault (backpressure is the caller's, never a false
/// timeout): while a Display waits for room in `event_bytes`, the library
/// reads nothing, so an answer behind it cannot arrive; the Ready
/// waiting for it is not timed out at `operation_ms` (3 s). Instead,
/// once the waiting Display has had no room for `delivery_stall_ms`
/// (6 s), the connection fails with library `delivery_stalled`: the
/// Ready stays unanswered for the caller's pending ledger, never
/// fabricated, and what was queued before is still delivered, then the
/// diagnostic. Its error states how many records were still undelivered:
/// the two `chunk_bytes` records of the second Display (LIM-02, #99).
#[test]
fn installed_library_fails_a_stalled_delivery_without_timing_out_its_commands() {
    let package = Package::new();
    let chunk = 2048;
    let metadata = 1024;
    let reserve = CHARGE + metadata;
    let one_display = CHARGE + chunk;
    let big = "41".repeat(chunk as usize);
    let peer = attached_peer(json!({
        "ready": [
            shown(PERIOD, 2, &big),
            shown(PERIOD, 2, &big.repeat(2)),
            answered(
                "ready",
                "ok",
                json!({ "kind": "ready", "changed": true, "period": "2", "restoration_id": "2" }),
                Value::Null,
            ),
        ],
    }));
    let limits = json!({
        "chunk_bytes": chunk,
        "metadata_record_bytes": metadata,
        "event_bytes": reserve + one_display + CHARGE + 128,
        "progress_bytes": 1,
        "operation_ms": 3000,
        "delivery_stall_ms": 6000,
    });
    let then = [
        json!({ "do": "submit", "request": ready(2, 1, CURRENT) }),
        json!({ "do": "wait_run" }),
    ];
    let mut plan = Plan::new();
    plan.add(
        "stalled",
        attached("stalled", peer, json!({ "limits": limits }), &then),
    );
    let (report, finishes) = plan.run(&package);
    let (records, terminal) = finished(step(&report, finishes[0].1));
    assert_eq!(tags(&records), [DISPLAY, DIAGNOSTIC], "{records:#?}");
    assert_eq!(display(&records, 1, [PERIOD, 2]), big);
    assert_diagnostic(&records[1], 1, ORIGIN_LIBRARY, "delivery_stalled");
    let message = records[1]["diagnostic"]["message"].as_str().unwrap();
    assert!(message.ends_with("undelivered records: 2"), "{message}");
    assert_terminal(
        terminal,
        1,
        (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
        Some((ORIGIN_LIBRARY, "delivery_stalled")),
    );
    assert_eq!(
        kinds(&report["peers"]["stalled"]),
        ["hello", "open", "ready"]
    );
    assert_all_released(&report["ledger"]);
}

/// COM-03 Library and Fault (a wait for event room ends with it): a
/// Display split into two `chunk_bytes` records, of which only the first
/// fits `event_records` less its one-record progress reserve, waits until the caller releases what it drained;
/// once queued, nothing waits, so no stall deadline remains, and a Ready
/// the daemon never answers then reports `timeout` at `operation_ms`
/// (300 ms), well before `delivery_stall_ms` (5 s) from the wait.
#[test]
fn installed_library_times_commands_out_again_once_waiting_records_are_queued() {
    let package = Package::new();
    let mut records = attach_records(&ATTACHED);
    records.push(shown(PERIOD, 2, "4243"));
    let mut peer = attached_peer(json!({ "ready": ["hold"] }));
    peer["open"] = json!(records);
    let limits = json!({
        "chunk_bytes": 1,
        "event_records": 8,
        "progress_records": 1,
        "operation_ms": 300,
        "delivery_stall_ms": 5000,
    });
    let steps = vec![
        json!({ "do": "create", "peer": peer, "startup": { "limits": limits } }),
        json!({ "do": "submit", "request": interactive(1, 80, 24) }),
        json!({ "do": "run" }),
        json!({ "do": "await_snapshot", "gauge": "event_records", "value": 7 }),
        json!({ "do": "drain", "until": { "display": "42" } }),
        json!({ "do": "drain", "until": { "display": "43" }, "as": "queued" }),
        json!({ "do": "submit", "request": ready(2, 1, CURRENT) }),
        json!({ "do": "wait_run" }),
        json!({ "do": "finish", "as": "finished" }),
    ];
    let report = script(&package, &root(&package), json!(steps));
    assert_eq!(
        tags(&drained(&report, &steps, "queued")),
        [DISPLAY],
        "the waiting chunk"
    );
    let (rest, terminal) = finished(labelled(&report, &steps, "finished"));
    assert_eq!(tags(&rest), [RESPONSE, DIAGNOSTIC], "{rest:#?}");
    assert_response(&rest[0], 1, 2, INDETERMINATE);
    assert_diagnostic(&rest[1], 1, ORIGIN_LIBRARY, "timeout");
    assert_terminal(
        terminal,
        1,
        (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
        Some((ORIGIN_LIBRARY, "timeout")),
    );
    assert_all_released(&report["ledger"]);
}

/// REC-01 Fault (a final outcome outlives the connection): the daemon
/// sends a Command final and closes while a Display chunk ahead of it waits
/// for event room, so the library has read neither; a Focus the caller
/// then submits (its outcome still fits) cannot be written to the closed
/// connection. That write failure must not discard what the daemon sent
/// before closing: once the caller drains, the Command final is delivered
/// and the close stops the stream cleanly, never a passive retry of a
/// session that is already gone.
#[test]
fn installed_library_keeps_a_final_the_daemon_sent_before_a_write_failed() {
    let package = Package::new();
    let chunk = 8192;
    let metadata = 1024;
    let one_display = CHARGE + chunk;
    let reserve = CHARGE + metadata;
    let mut records = attach_records(&ATTACHED);
    records.push(shown(PERIOD, 2, &"41".repeat(2 * chunk as usize)));
    records.push(command_final(json!({ "kind": "exit", "code": 3 })));
    records.push(json!("close"));
    let mut peer = attached_peer(json!({}));
    peer["open"] = json!(records);
    // Room for the attach's records, one chunk and one outcome, never two
    // chunks.
    let limits = json!({
        "chunk_bytes": chunk,
        "metadata_record_bytes": metadata,
        "event_bytes": 12 * CHARGE + 2048 + one_display + reserve,
        "progress_bytes": 1,
        "delivery_stall_ms": 5000,
    });
    let focus = json!({ "tag": "focus", "request_id": 2, "authority": [1, PERIOD, 2], "value": 1 });
    let steps = vec![
        json!({ "do": "create", "peer": peer, "startup": { "limits": limits } }),
        json!({ "do": "submit", "request": interactive(1, 80, 24) }),
        json!({ "do": "run" }),
        json!({ "do": "await_snapshot", "gauge": "event_records", "value": 7 }),
        // The hello, the Open, then the peer's own close.
        json!({ "do": "await_peer", "frames": 3 }),
        json!({ "do": "submit", "request": focus }),
        json!({ "do": "drain", "until": "terminal", "as": "drained" }),
        json!({ "do": "finish", "as": "finished" }),
    ];
    let report = script(&package, &root(&package), json!(steps));
    assert_success(step(&report, 5));
    let records = drained(&report, &steps, "drained");
    let finals: Vec<_> = records.iter().filter(|r| r["tag"] == FINAL).collect();
    assert_eq!(finals.len(), 1, "{records:#?}");
    assert_eq!(
        finals[0]["final"]["command_result"],
        native_command(EXIT, 3, None),
        "{records:#?}"
    );
    let (_, terminal) = finished(labelled(&report, &steps, "finished"));
    assert_eq!(terminal["client_outcome"], CLIENT_STOPPED, "{terminal:#}");
    assert_eq!(terminal["retry"], RETRY_NONE, "{terminal:#}");
    assert_all_released(&report["ledger"]);
}

/// LIB-04 Fault (control and screen records are charged their header
/// twice and only the bytes they own): with an attach's records, a
/// Geometry and a Control revoked (`session_ending`) all still queued, the
/// event queue holds exactly eight records' headers plus the Open's
/// selected token array, the Initial metadata's one-byte title and the
/// Display's one byte; a grant, beginning, complete, Geometry or
/// revocation owns nothing. The revocation reaches the caller with its
/// reserved field zero, as the native header requires.
#[test]
fn installed_library_charges_presentation_records_only_the_display_bytes_they_own() {
    let package = Package::new();
    let mut records = attach_records(&ATTACHED);
    records.extend([
        geometry(PERIOD, 90, 25),
        revoked(PERIOD, 2, "session_ending"),
    ]);
    let mut peer = attached_peer(json!({}));
    peer["open"] = json!(records);
    let steps = vec![
        json!({ "do": "create", "peer": peer }),
        json!({ "do": "submit", "request": interactive(1, 80, 24) }),
        json!({ "do": "run" }),
        json!({ "do": "await_snapshot", "gauge": "event_records", "value": 8, "as": "queued" }),
        json!({ "do": "drain", "until": { "records": 8 }, "as": "drained" }),
        json!({ "do": "shutdown" }),
        json!({ "do": "finish" }),
    ];
    let report = script(&package, &root(&package), json!(steps));
    let queued = &labelled(&report, &steps, "queued")["output"];
    assert_eq!(queued["event_records"], 8, "{queued:#}");
    assert_eq!(
        queued["event_bytes"],
        8 * CHARGE + 3 * TOKEN + 1 + 1,
        "{queued:#}"
    );
    let drained = drained(&report, &steps, "drained");
    assert_eq!(
        tags(&drained),
        [
            RESPONSE,
            INITIAL_METADATA,
            GRANTED,
            RESTORATION_BEGIN,
            DISPLAY,
            RESTORATION_COMPLETE,
            GEOMETRY,
            REVOKED
        ],
        "{drained:#?}"
    );
    let ended = presented(&drained[7], REVOKED, 1);
    assert_eq!(ids(ended), [PERIOD, 2], "{ended:#}");
    assert_eq!(ended["reason"], SESSION_ENDING, "{ended:#}");
    assert_eq!(ended["reserved"], 0, "{ended:#}");
    assert_all_released(&report["ledger"]);
}
