//! Library layer and Fault (#96): termination through the installed client
//! library against the host's fake daemon peer. Command final records on a
//! passive monitor connection, and Kill on its own auxiliary connection,
//! each with the outcomes the native contract specifies for traffic a real
//! daemon cannot be made to send: partial and known-final Kill results,
//! inconsistent repeats, lost, late and malformed answers, and closure
//! without final evidence. Every case records what reached the fake daemon,
//! so a replayed Kill would show. Expected values come from the native
//! header, the native contract and the daemon wire reference.

#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use std::path::Path;
use support::{Package, library::INSTANCE_DESCRIPTORS, passive::*};

/// What one record of a case must be.
enum Expect {
    /// A Response to the Open `id`: it succeeded.
    Opened(u64),
    /// The Initial metadata.
    Metadata,
    /// A Command final of this command result (native form).
    Final(Value),
    /// A Response to `id` with `outcome`, an error unless OK, and its
    /// result: none, a known final result, or a termination.
    Answer(u64, u64, Option<(u64, &'static str)>, Answered),
    /// A diagnostic.
    Diagnostic((u64, &'static str)),
}

/// A Response's active result.
enum Answered {
    Nothing,
    KnownFinal(Value),
    Termination(Value),
}

fn assert_records(name: &str, records: &[Value], expected: &[Expect]) {
    assert_eq!(records.len(), expected.len(), "{name}: {records:#?}");
    for (record, expected) in records.iter().zip(expected) {
        match expected {
            Expect::Opened(id) => {
                let response = assert_response(record, 1, *id, OK);
                assert_eq!(response["result_tag"], OPENED, "{name}: {record:#}");
            }
            Expect::Metadata => assert_header(record, INITIAL_METADATA, 1),
            Expect::Final(command) => assert_final(record, 1, command),
            Expect::Answer(id, outcome, error, answered) => {
                let response = assert_response(record, 1, *id, *outcome);
                match error {
                    Some((origin, code)) => assert_error(&response["error"], *origin, code),
                    None => assert_eq!(response["error"], no_error(), "{name}: {record:#}"),
                }
                let (tag, result) = match answered {
                    Answered::Nothing => (RESULT_NONE, &Value::Null),
                    Answered::KnownFinal(command) => (KNOWN_FINAL, command),
                    Answered::Termination(termination) => (TERMINATION, termination),
                };
                assert_eq!(response["result_tag"], tag, "{name}: {record:#}");
                assert_eq!(&response["result"], result, "{name}: {record:#}");
            }
            Expect::Diagnostic((origin, code)) => assert_diagnostic(record, 1, *origin, code),
        }
    }
}

/// A case: its instance's steps, then what its records, terminal evidence
/// and fake daemon must show.
struct Case {
    name: &'static str,
    steps: Vec<Value>,
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

/// Every case in one script, each on its own instance and each ended by
/// waiting for run, a snapshot and `finish`; then each one's records,
/// terminal evidence and fake daemon. Once run has returned, nothing is
/// outstanding: no request, no operation and no connection descriptor.
fn assert_cases(package: &Package, root: &Path, cases: &[Case]) -> Value {
    let mut steps = Vec::new();
    let mut starts = Vec::new();
    let ending = [
        json!({ "do": "wait_run" }),
        json!({ "do": "snapshot" }),
        json!({ "do": "finish" }),
    ];
    for case in cases {
        starts.push(steps.len());
        steps.extend(case.steps.iter().chain(&ending).map(|step| {
            let mut step = step.clone();
            step["instance"] = json!(case.name);
            step
        }));
    }
    let report = script(package, root, json!(steps));
    for (case, start) in cases.iter().zip(starts) {
        let name = case.name;
        let last = start + case.steps.len() + ending.len() - 1;
        let snapshot = &step(&report, last - 1)["output"];
        for (gauge, value) in [
            ("request_records", 0),
            ("operations", 0),
            ("descriptors", INSTANCE_DESCRIPTORS),
        ] {
            assert_eq!(snapshot[gauge], value, "{name}: {gauge}: {snapshot:#}");
        }
        let (records, terminal) = finished(step(&report, last));
        assert_records(name, &records, &case.records);
        assert_terminal(terminal, 1, case.terminal, case.error);
        assert_eq!(kinds(&report["peers"][name]), case.kinds, "{name}");
    }
    assert_all_released(&report["ledger"]);
    report
}

/// The fake daemon's Command final of `result`, with status null.
fn command_final(result: Value) -> Value {
    send(0x4c, json!({ "command_result": result, "status": null }))
}

fn exit(code: u64) -> Value {
    json!({ "kind": "exit", "code": code })
}

fn signal(number: u64, name: Value) -> Value {
    json!({ "kind": "signal", "number": number, "name": name })
}

/// A fake daemon that opens with `title-v1` and reports the title `t`,
/// then sends `then`.
fn opened_then(then: Vec<Value>) -> Value {
    let mut open = vec![opened(&SELECTED), initial(json!("t"))];
    open.extend(then);
    json!({ "open": open })
}

/// COM-06 Fault (exit differs from signal, final evidence survives
/// disconnection, inconsistent repeats fail, and closure never invents
/// completion or a successful Leave), with REC-01 Fault (a final outcome
/// stops retry; transport loss without one calls for passive retry): after
/// the Initial metadata, a Command final of exit zero, of exit 143 and of
/// signal 15 with its name each arrive as one record of their own kind and
/// value, status unavailable, and the daemon's close then stops the stream
/// cleanly: stopped, no error, retry none, Leave not required (owner
/// decision on #94). A second Command final, consistent or contradictory,
/// and one after an Initial metadata that knew the result are
/// `invalid_record`, after the first is delivered, with no retry; a
/// Command final before the Initial metadata, or with a status no Open
/// selected, is `invalid_record` and the result stays unknown, so passive
/// retry. A connection lost inside a frame after a Command final keeps
/// the final record and calls for no retry; one closed with no final
/// evidence is a failure that calls for passive retry, never a completion;
/// and a close that meets an unanswered Leave after the final leaves that
/// Leave indeterminate, never confirmed.
#[test]
fn installed_library_relays_each_command_final_once_and_closure_invents_nothing() {
    let package = Package::new();
    let root = root(&package);
    let alone = [open(1, &REQUIRED, &["title-v1"])];
    let invalid = (ORIGIN_TRANSPORT, "invalid_record");
    let clean = (CLIENT_STOPPED, RETRY_NONE, LEAVE_NOT_REQUIRED);
    let final_case = |name, result: Value, command: Value| Case {
        name,
        steps: submitted(
            opened_then(vec![command_final(result), json!("close")]),
            json!({}),
            &alone,
        ),
        records: vec![Expect::Opened(1), Expect::Metadata, Expect::Final(command)],
        terminal: clean,
        error: None,
        kinds: &["hello", "open"],
    };
    let repeated = |name, second: Value| Case {
        name,
        steps: submitted(
            opened_then(vec![command_final(exit(0)), command_final(second)]),
            json!({}),
            &alone,
        ),
        records: vec![
            Expect::Opened(1),
            Expect::Metadata,
            Expect::Final(native_command(EXIT, 0, None)),
            Expect::Diagnostic(invalid),
        ],
        terminal: (CLIENT_FAILED, RETRY_NONE, LEAVE_NOT_REQUIRED),
        error: Some(invalid),
        kinds: &["hello", "open"],
    };
    let known = send(
        0x41,
        json!({
            "title": { "available": true, "value": "t" },
            "status": { "available": false, "value": null },
            "progress": { "available": false, "value": null },
            "command_result": exit(0),
        }),
    );
    let completed = json!({
        "state": "completed",
        "changed_at": "2026-11-06T12:00:02.000Z",
        "episode_id": null,
    });
    let lost = (ORIGIN_TRANSPORT, "io_error");
    let cases = [
        final_case("exit-zero", exit(0), native_command(EXIT, 0, None)),
        final_case("exit-143", exit(143), native_command(EXIT, 143, None)),
        final_case(
            "signal-15",
            signal(15, json!("SIGTERM")),
            native_command(SIGNAL, 15, Some("SIGTERM")),
        ),
        repeated("repeated", exit(0)),
        repeated("contradicted", signal(9, Value::Null)),
        Case {
            name: "after-known",
            steps: submitted(
                json!({ "open": [opened(&SELECTED), known, command_final(exit(0))] }),
                json!({}),
                &alone,
            ),
            records: vec![
                Expect::Opened(1),
                Expect::Metadata,
                Expect::Diagnostic(invalid),
            ],
            terminal: (CLIENT_FAILED, RETRY_NONE, LEAVE_NOT_REQUIRED),
            error: Some(invalid),
            kinds: &["hello", "open"],
        },
        Case {
            name: "early",
            steps: submitted(
                json!({ "open": [opened(&SELECTED), command_final(exit(0))] }),
                json!({}),
                &alone,
            ),
            records: vec![Expect::Opened(1), Expect::Diagnostic(invalid)],
            terminal: (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
            error: Some(invalid),
            kinds: &["hello", "open"],
        },
        Case {
            name: "with-status",
            steps: submitted(
                opened_then(vec![send(
                    0x4c,
                    json!({ "command_result": exit(0), "status": completed }),
                )]),
                json!({}),
                &alone,
            ),
            records: vec![
                Expect::Opened(1),
                Expect::Metadata,
                Expect::Diagnostic(invalid),
            ],
            terminal: (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
            error: Some(invalid),
            kinds: &["hello", "open"],
        },
        Case {
            name: "survives",
            steps: submitted(
                opened_then(vec![
                    command_final(exit(2)),
                    json!({ "raw": "0000000a4c7b" }),
                    json!("close"),
                ]),
                json!({}),
                &alone,
            ),
            records: vec![
                Expect::Opened(1),
                Expect::Metadata,
                Expect::Final(native_command(EXIT, 2, None)),
                Expect::Diagnostic((ORIGIN_TRANSPORT, "truncated_frame")),
            ],
            terminal: (CLIENT_FAILED, RETRY_NONE, LEAVE_NOT_REQUIRED),
            error: Some((ORIGIN_TRANSPORT, "truncated_frame")),
            kinds: &["hello", "open"],
        },
        Case {
            name: "no-final",
            steps: submitted(opened_then(vec![json!("close")]), json!({}), &alone),
            records: vec![
                Expect::Opened(1),
                Expect::Metadata,
                Expect::Diagnostic(lost),
            ],
            terminal: (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
            error: Some(lost),
            kinds: &["hello", "open"],
        },
        Case {
            name: "leave-closed",
            steps: submitted(
                json!({
                    "open": [opened(&SELECTED), initial(json!("t")), command_final(exit(0))],
                    "leave": ["close"],
                }),
                json!({}),
                &[alone[0].clone(), leave(2, 1)],
            ),
            records: vec![
                Expect::Opened(1),
                Expect::Metadata,
                Expect::Final(native_command(EXIT, 0, None)),
                Expect::Diagnostic(lost),
            ],
            terminal: (CLIENT_FAILED, RETRY_NONE, LEAVE_INDETERMINATE),
            error: Some(lost),
            kinds: &["hello", "open", "leave"],
        },
    ];
    assert_cases(&package, &root, &cases);
    package.assert_clean();
}

/// The fake daemon's Response to the Kill.
fn kill_answer(outcome: &str, result: Value, error: Value) -> Value {
    response("$kill", outcome, result, error)
}

/// A fake daemon answering a Kill with `answer`.
fn killed(answer: Vec<Value>) -> Value {
    json!({ "kill": answer })
}

/// COM-03 Kill and DEL-02 Fault (Kill on its own hello-validated
/// connection: definitive or indeterminate, never replayed; at most one
/// pending), COM-06 Fault (partial Kill and known-final results) and
/// REC-01 Fault (retry from terminal evidence): a Kill-only instance needs
/// no Open. Its Kill, requiring nothing more, is sent once as the first
/// role of its own connection, wire ID 1, revision 1, and the run ends after
/// its result. A confirmed termination is answered OK with the command
/// result and complete cleanup: stopped, no error, retry none. A rejection
/// that knows the result carries it (retry none); a failed or
/// indeterminate Kill carries its partial termination, a known command
/// result stopping retry and an unknown one leaving passive retry; a
/// daemon lacking the revision is permanent. A lost answer, a Kill
/// outliving `kill_ms`, a mismatched or uncorrelated answer and a daemon
/// Fatal are each indeterminate, with passive retry, and the Kill is never
/// sent again. A Kill requiring a family the daemon does not advertise, or
/// whose frame exceeds the daemon's request ceiling, is rejected before it
/// is sent, for good; one whose hello is lost or never comes within
/// `kill_ms` is rejected with passive retry, and one whose hello completes
/// after shutdown is `cancelled`, never sent. A second Kill while one is
/// pending is rejected `invalid_state` at once; a Kill shutdown overtook
/// before it executed is rejected `cancelled` and nothing connects; a Kill
/// under way when shutdown comes is still answered definitively.
#[test]
fn installed_library_reports_each_kill_outcome_truthfully_and_never_replays_it() {
    let package = Package::new();
    let root = root(&package);
    let alone = [kill(1, &[])];
    let daemon = |code: &'static str| (ORIGIN_DAEMON, code);
    let library = |code: &'static str| (ORIGIN_LIBRARY, code);
    let transport = |code: &'static str| (ORIGIN_TRANSPORT, code);
    let confirmed = kill_answer(
        "ok",
        json!({ "kind": "termination", "command_result": exit(0), "cleanup": "complete" }),
        Value::Null,
    );
    let ok = Expect::Answer(
        1,
        OK,
        None,
        Answered::Termination(termination(
            Some(&native_command(EXIT, 0, None)),
            CLEANUP_COMPLETE,
        )),
    );
    let indeterminate = |name, peer: Value, startup: Value, error| Case {
        name,
        steps: submitted(peer, startup, &alone),
        records: vec![
            Expect::Answer(1, INDETERMINATE, Some(error), Answered::Nothing),
            Expect::Diagnostic(error),
        ],
        terminal: (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
        error: Some(error),
        kinds: &["hello", "kill"],
    };
    let trigger = root.join("answer-kill");
    let helloed = root.join("hello-kill");
    let refused = |name, peer: Value, startup: Value, error, retry| Case {
        name,
        steps: submitted(peer, startup, &alone),
        records: vec![
            Expect::Answer(1, REJECTED, Some(error), Answered::Nothing),
            Expect::Diagnostic(error),
        ],
        terminal: (CLIENT_FAILED, retry, LEAVE_NOT_REQUIRED),
        error: Some(error),
        kinds: &["hello"],
    };
    let cases = [
        Case {
            name: "confirmed",
            steps: submitted(killed(vec![confirmed.clone()]), json!({}), &alone),
            records: vec![ok],
            terminal: (CLIENT_STOPPED, RETRY_NONE, LEAVE_NOT_REQUIRED),
            error: None,
            kinds: &["hello", "kill"],
        },
        Case {
            name: "known-final",
            steps: submitted(
                killed(vec![kill_answer(
                    "rejected",
                    json!({ "kind": "known_final", "command_result": signal(9, Value::Null) }),
                    daemon_error("unsupported_capability"),
                )]),
                json!({}),
                &alone,
            ),
            records: vec![
                Expect::Answer(
                    1,
                    REJECTED,
                    Some(daemon("unsupported_capability")),
                    Answered::KnownFinal(native_command(SIGNAL, 9, None)),
                ),
                Expect::Diagnostic(daemon("unsupported_capability")),
            ],
            terminal: (CLIENT_FAILED, RETRY_NONE, LEAVE_NOT_REQUIRED),
            error: Some(daemon("unsupported_capability")),
            kinds: &["hello", "kill"],
        },
        Case {
            name: "partial-known",
            steps: submitted(
                killed(vec![kill_answer(
                    "failed",
                    json!({ "kind": "termination", "command_result": exit(143), "cleanup": "incomplete" }),
                    daemon_error("cleanup_failed"),
                )]),
                json!({}),
                &alone,
            ),
            records: vec![
                Expect::Answer(
                    1,
                    FAILED,
                    Some(daemon("cleanup_failed")),
                    Answered::Termination(termination(
                        Some(&native_command(EXIT, 143, None)),
                        CLEANUP_INCOMPLETE,
                    )),
                ),
                Expect::Diagnostic(daemon("cleanup_failed")),
            ],
            terminal: (CLIENT_FAILED, RETRY_NONE, LEAVE_NOT_REQUIRED),
            error: Some(daemon("cleanup_failed")),
            kinds: &["hello", "kill"],
        },
        Case {
            name: "partial-unknown",
            steps: submitted(
                killed(vec![kill_answer(
                    "indeterminate",
                    json!({ "kind": "termination", "command_result": null, "cleanup": "unknown" }),
                    daemon_error("internal_error"),
                )]),
                json!({}),
                &alone,
            ),
            records: vec![
                Expect::Answer(
                    1,
                    INDETERMINATE,
                    Some(daemon("internal_error")),
                    Answered::Termination(termination(None, CLEANUP_UNKNOWN)),
                ),
                Expect::Diagnostic(daemon("internal_error")),
            ],
            terminal: (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
            error: Some(daemon("internal_error")),
            kinds: &["hello", "kill"],
        },
        Case {
            name: "revisionless",
            steps: submitted(
                killed(vec![kill_answer(
                    "rejected",
                    Value::Null,
                    daemon_error("unsupported_revision"),
                )]),
                json!({}),
                &alone,
            ),
            records: vec![
                Expect::Answer(
                    1,
                    REJECTED,
                    Some(daemon("unsupported_revision")),
                    Answered::Nothing,
                ),
                Expect::Diagnostic(daemon("unsupported_revision")),
            ],
            terminal: (CLIENT_FAILED, RETRY_PERMANENT, LEAVE_NOT_REQUIRED),
            error: Some(daemon("unsupported_revision")),
            kinds: &["hello", "kill"],
        },
        indeterminate(
            "lost",
            killed(vec![json!("close")]),
            json!({}),
            transport("io_error"),
        ),
        indeterminate(
            "held",
            killed(vec![json!("hold")]),
            json!({ "limits": { "kill_ms": 300 } }),
            library("timeout"),
        ),
        indeterminate(
            "mismatched",
            killed(vec![kill_answer(
                "ok",
                json!({ "kind": "left", "attachment_detached": false }),
                Value::Null,
            )]),
            json!({}),
            transport("invalid_record"),
        ),
        indeterminate(
            "uncorrelated",
            killed(vec![response(
                "7",
                "ok",
                json!({ "kind": "termination", "command_result": exit(0), "cleanup": "complete" }),
                Value::Null,
            )]),
            json!({}),
            transport("invalid_request_id"),
        ),
        indeterminate(
            "fatal",
            killed(vec![send(0x4d, daemon_error("internal_error"))]),
            json!({}),
            daemon("internal_error"),
        ),
        Case {
            name: "unsupported",
            steps: submitted(
                json!({ "hello": { "json": { "capabilities": ["daemon-wire-v1", "monitor-v1"] } } }),
                json!({}),
                &[kill(1, &["title-v1"])],
            ),
            records: vec![
                Expect::Answer(
                    1,
                    REJECTED,
                    Some(library("unsupported_capability")),
                    Answered::Nothing,
                ),
                Expect::Diagnostic(library("unsupported_capability")),
            ],
            terminal: (CLIENT_FAILED, RETRY_PERMANENT, LEAVE_NOT_REQUIRED),
            error: Some(library("unsupported_capability")),
            kinds: &["hello"],
        },
        refused(
            "unfit",
            json!({ "hello": { "json": { "daemon_wire_v1": {
                "max_request_frame_bytes": 8,
                "max_server_frame_bytes": 1_048_576,
            } } } }),
            json!({}),
            library("unsupported_limit"),
            RETRY_PERMANENT,
        ),
        refused(
            "unhelloed",
            json!({ "hello": "close" }),
            json!({}),
            transport("io_error"),
            RETRY_PASSIVE,
        ),
        refused(
            "silent",
            json!({ "hello": "hold" }),
            json!({ "limits": { "kill_ms": 300 } }),
            library("timeout"),
            RETRY_PASSIVE,
        ),
        Case {
            name: "shutdown-hello",
            steps: vec![
                json!({
                    "do": "create",
                    "peer": { "hello": { "after": helloed.to_str().unwrap() } },
                }),
                json!({ "do": "submit", "request": kill(1, &[]) }),
                json!({ "do": "run" }),
                json!({ "do": "await_peer", "frames": 1 }),
                json!({ "do": "shutdown" }),
                json!({ "do": "touch", "path": helloed.to_str().unwrap() }),
            ],
            records: vec![Expect::Answer(
                1,
                REJECTED,
                Some(library("cancelled")),
                Answered::Nothing,
            )],
            terminal: (CLIENT_STOPPED, RETRY_NONE, LEAVE_NOT_REQUIRED),
            error: None,
            kinds: &["hello"],
        },
        Case {
            name: "second",
            steps: submitted(
                killed(vec![json!("hold")]),
                json!({ "limits": { "kill_ms": 1000 } }),
                &[kill(1, &[]), kill(2, &[])],
            ),
            records: vec![
                Expect::Answer(
                    2,
                    REJECTED,
                    Some(library("invalid_state")),
                    Answered::Nothing,
                ),
                Expect::Answer(
                    1,
                    INDETERMINATE,
                    Some(library("timeout")),
                    Answered::Nothing,
                ),
                Expect::Diagnostic(library("timeout")),
            ],
            terminal: (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED),
            error: Some(library("timeout")),
            kinds: &["hello", "kill"],
        },
        Case {
            name: "cancelled",
            steps: vec![
                json!({ "do": "create", "peer": killed(vec![confirmed.clone()]) }),
                json!({ "do": "submit", "request": kill(1, &[]) }),
                json!({ "do": "shutdown" }),
                json!({ "do": "run" }),
            ],
            records: vec![Expect::Answer(
                1,
                REJECTED,
                Some(library("cancelled")),
                Answered::Nothing,
            )],
            terminal: (CLIENT_STOPPED, RETRY_NONE, LEAVE_NOT_REQUIRED),
            error: None,
            kinds: &[],
        },
        Case {
            name: "shutdown-pending",
            steps: vec![
                json!({
                    "do": "create",
                    "peer": killed(vec![json!({ "wait": trigger.to_str().unwrap() }), confirmed]),
                }),
                json!({ "do": "submit", "request": kill(1, &[]) }),
                json!({ "do": "run" }),
                json!({ "do": "await_peer", "frames": 2 }),
                json!({ "do": "shutdown" }),
                json!({ "do": "touch", "path": trigger.to_str().unwrap() }),
            ],
            records: vec![Expect::Answer(
                1,
                OK,
                None,
                Answered::Termination(termination(
                    Some(&native_command(EXIT, 0, None)),
                    CLEANUP_COMPLETE,
                )),
            )],
            terminal: (CLIENT_STOPPED, RETRY_NONE, LEAVE_NOT_REQUIRED),
            error: None,
            kinds: &["hello", "kill"],
        },
    ];
    let report = assert_cases(&package, &root, &cases);
    let frames = report["peers"]["confirmed"]["frames"].as_array().unwrap();
    let body = json!({ "request_id": "1", "revision": 1, "required_capabilities": [] });
    assert_eq!(frames[1]["type"], 0x38, "{frames:#?}");
    assert_eq!(frames[1]["body"], body, "{frames:#?}");
    for name in ["held", "lost"] {
        let peer = &report["peers"][name];
        assert_eq!(peer["finished"], true, "{name}: {peer:#}");
    }
    package.assert_clean();
}
