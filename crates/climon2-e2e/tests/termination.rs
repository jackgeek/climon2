//! Library layer (#96): termination through the installed client library's
//! frozen C ABI against real session daemons started through the packaged
//! CLI: Command final records of exits and signals, Kill on its own
//! auxiliary connection, and recovery by a fresh instance only after the
//! old one is destroyed and joined. Expected values come from the native
//! header, the native contract and the daemon wire reference, never from
//! another run of the library. Fault cases against the host's fake daemon
//! peer are in `termination_faults.rs`.

#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use support::{
    Package, TIMEOUT,
    control::Session,
    library::{archive, scenario},
    passive::*,
};

/// The Response and Initial metadata of a passive Open `id` of a session
/// whose result is not known yet.
fn assert_opened(records: &[Value], id: u64, generation: u64) {
    let response = assert_response(&records[0], generation, id, OK);
    assert_eq!(response["result_tag"], OPENED, "{records:#?}");
    assert_header(&records[1], INITIAL_METADATA, generation);
    assert_eq!(
        records[1]["metadata"]["command_result"]["present"], 0,
        "{records:#?}"
    );
}

/// COM-06 Library (exit differs from signal; the final result appears
/// once, and closure afterwards invents nothing) and REC-01 Library (a
/// final outcome calls for no retry): a passive monitor connection to each
/// of three real sessions, whose commands exit zero, exit 143 and raise
/// SIGTERM, delivers its Response and Initial metadata without a result,
/// then exactly one Command final: exit 0, exit 143 and signal 15 (no
/// name, as the daemon sends none), each a record of its own kind with
/// status unavailable; exit 143 is not signal 15. When the session ends,
/// the daemon's close stops each stream cleanly: stopped, no error, retry
/// none, Leave not required, the connection closed.
#[test]
fn installed_library_relays_each_real_command_final_apart_from_signals() {
    let cases: [(&[&str], Value); 3] = [
        (&["exit", "0"], native_command(EXIT, 0, None)),
        (&["exit", "143"], native_command(EXIT, 143, None)),
        (&["kill-self", "TERM"], native_command(SIGNAL, 15, None)),
    ];
    for (generation, (then, command)) in (1..).zip(cases) {
        let package = Package::new();
        let root = root(&package);
        let mut session = Session::ending(&package, &root, then);
        let steps = json!([
            { "do": "create", "startup": session.startup(&package, generation) },
            { "do": "submit", "request": open(1, &REQUIRED, &["title-v1"]) },
            { "do": "run" },
            { "do": "drain", "until": { "tag": INITIAL_METADATA } },
            { "do": "touch", "path": session.end_path() },
            { "do": "finish" },
        ]);
        let report = script(&package, &root, steps);
        let mut records = step(&report, 3)["records"].as_array().unwrap().clone();
        let (later, terminal) = finished(step(&report, 5));
        records.extend(later);
        assert_eq!(records.len(), 3, "{then:?}: {records:#?}");
        assert_opened(&records, 1, generation);
        assert_final(&records[2], generation, &command);
        let clean = (CLIENT_STOPPED, RETRY_NONE, LEAVE_NOT_REQUIRED);
        assert_terminal(terminal, generation, clean, None);
        assert_all_released(&report["ledger"]);
        session.terminal.wait_exit(TIMEOUT).unwrap();
        package.assert_clean();
    }
}

/// COM-03 Kill and DEL-02 Library (Kill succeeds only after termination
/// and daemon cleanup, on its own connection, never repurposing the opened
/// role), COM-06 Library (one final result, the same in the Kill's answer
/// and the Command final) and REC-01 Library (a definitive outcome calls
/// for no retry): instance A opens a passive monitor connection to a real
/// session whose command waits forever, with two Kills queued behind its
/// Open. Once the Initial metadata arrives the first Kill is sent on its
/// own connection, and the second, while the first is pending, is
/// rejected `invalid_state` at once, never sent. A Kill-only instance K,
/// which never opens, submits one too, joining the same termination.
/// After the hang-up grace, each sent Kill is answered OK
/// with the command's result, SIGHUP (signal 1, unnamed), and complete
/// cleanup, and by then the session's metadata and socket are gone. A's
/// passive stream delivers exactly one Command final of that result and
/// the daemon's close stops it cleanly; K's run ends after its Kill's
/// result. Both: stopped, no error, retry none, Leave not required, every
/// connection closed.
#[test]
fn installed_library_kills_a_real_session_only_after_termination_and_cleanup() {
    let package = Package::new();
    let root = root(&package);
    let never = root.join("never").to_str().unwrap().to_owned();
    let mut session = Session::ending(&package, &root, &["await-file", &never]);
    let steps = json!([
        { "do": "create", "instance": "a", "startup": session.startup(&package, 1) },
        { "do": "create", "instance": "k", "startup": session.startup(&package, 2) },
        { "do": "submit", "instance": "a", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "submit", "instance": "a", "request": kill(2, &[]) },
        { "do": "submit", "instance": "a", "request": kill(3, &[]) },
        { "do": "run", "instance": "a" },
        { "do": "drain", "instance": "a", "until": { "tag": INITIAL_METADATA } },
        { "do": "submit", "instance": "k", "request": kill(1, &["daemon-wire-v1"]) },
        { "do": "run", "instance": "k" },
        { "do": "finish", "instance": "k" },
        { "do": "finish", "instance": "a" },
    ]);
    let report = script(&package, &root, steps);
    let hangup = native_command(SIGNAL, 1, None);
    let confirmed = termination(Some(&hangup), CLEANUP_COMPLETE);
    let clean = (CLIENT_STOPPED, RETRY_NONE, LEAVE_NOT_REQUIRED);

    let (killed, terminal) = finished(step(&report, 9));
    assert_eq!(killed.len(), 1, "{killed:#?}");
    let response = assert_response(&killed[0], 2, 1, OK);
    assert_eq!(response["result_tag"], TERMINATION, "{response:#}");
    assert_eq!(response["result"], confirmed, "{response:#}");
    assert_eq!(response["error"], no_error(), "{response:#}");
    assert_terminal(terminal, 2, clean, None);
    assert!(session.cleaned_up());

    let mut records = step(&report, 6)["records"].as_array().unwrap().clone();
    let (later, terminal) = finished(step(&report, 10));
    records.extend(later);
    assert_eq!(records.len(), 5, "{records:#?}");
    assert_opened(&records, 1, 1);
    assert_rejected(&records[2], 1, 3, ORIGIN_LIBRARY, "invalid_state");
    // The Kill's answer and the Command final come over two connections,
    // in either order.
    let answer = records[3..]
        .iter()
        .find(|record| record["tag"] == RESPONSE)
        .unwrap();
    let response = assert_response(answer, 1, 2, OK);
    assert_eq!(response["result_tag"], TERMINATION, "{response:#}");
    assert_eq!(response["result"], confirmed, "{response:#}");
    let finals: Vec<_> = records[3..]
        .iter()
        .filter(|record| record["tag"] == FINAL)
        .collect();
    assert_eq!(finals.len(), 1, "{records:#?}");
    assert_final(finals[0], 1, &hangup);
    assert_terminal(terminal, 1, clean, None);
    assert_all_released(&report["ledger"]);
    session.terminal.wait_exit(TIMEOUT).unwrap();
    package.assert_clean();
}

/// REC-01 Library (startup failure calls for passive retry, and a fresh
/// attempt waits for the previous instance's run, destroy and join; an
/// instance never reconnects implicitly): with the real session daemon
/// stopped, instance A's Open gets no hello within `start_ms` and is
/// rejected `timeout`: failed, retry passive. While A's handles are still
/// held, its destroy is busy; A then releases them, is destroyed and its
/// owner thread joined, having opened nothing else. Only then is B, a
/// fresh instance with the next generation, created, and its passive Open
/// to the resumed daemon succeeds, with its own Leave confirmed on
/// shutdown.
#[test]
fn installed_library_recovers_only_through_a_fresh_instance_after_join() {
    let package = Package::new();
    let root = root(&package);
    let session = Session::ending(&package, &root, &["exit", "0"]);
    let mut stalled = session.startup(&package, 1);
    stalled["limits"] = json!({ "start_ms": 1000 });
    let steps = json!([
        { "do": "signal", "pid": session.pid, "signal": "stop" },
        { "do": "create", "instance": "a", "startup": stalled },
        { "do": "submit", "instance": "a", "request": open(1, &REQUIRED, &[]) },
        { "do": "run", "instance": "a" },
        { "do": "drain", "instance": "a", "until": "terminal" },
        { "do": "destroy", "instance": "a" },
        { "do": "signal", "pid": session.pid, "signal": "continue" },
        { "do": "finish", "instance": "a" },
        { "do": "create", "instance": "b", "startup": session.startup(&package, 2) },
        { "do": "submit", "instance": "b", "request": open(1, &REQUIRED, &[]) },
        { "do": "run", "instance": "b" },
        { "do": "drain", "instance": "b", "until": { "tag": INITIAL_METADATA } },
        { "do": "shutdown", "instance": "b" },
        { "do": "finish", "instance": "b" },
    ]);
    let report = script(&package, &root, steps);
    let records = step(&report, 4)["records"].as_array().unwrap();
    assert_eq!(records.len(), 2, "{records:#?}");
    assert_rejected(&records[0], 1, 1, ORIGIN_LIBRARY, "timeout");
    assert_diagnostic(&records[1], 1, ORIGIN_LIBRARY, "timeout");
    assert_failure(step(&report, 5), 7, "busy");
    let (later, terminal) = finished(step(&report, 7));
    assert!(later.is_empty(), "{later:#?}");
    let failed = (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED);
    assert_terminal(terminal, 1, failed, Some((ORIGIN_LIBRARY, "timeout")));
    let opened = step(&report, 11)["records"].as_array().unwrap();
    assert_eq!(opened.len(), 2, "{opened:#?}");
    assert_opened(opened, 1, 2);
    let (later, terminal) = finished(step(&report, 13));
    assert!(later.is_empty(), "{later:#?}");
    assert_left(terminal, 2);
    assert_all_released(&report["ledger"]);
    session.end();
    let mut session = session;
    session.terminal.wait_exit(TIMEOUT).unwrap();
    package.assert_clean();
}

/// COM-03 Kill and DEL-02 Library (a Kill that cannot reach the daemon is
/// rejected, never sent, and leaves the opened role alone) and REC-01
/// Library: instance A opens a passive role on a real session, then the
/// session daemon is stopped. A's Kill, its hello unanswered within
/// `kill_ms`, is rejected `timeout`, before it is sent, and nothing else is
/// delivered: no diagnostic, the role still open. Once the daemon resumes,
/// A's shutdown sends the role's Leave, which the daemon confirms: stopped,
/// no error, retry none. The session was never terminated: its command
/// still runs until the test ends it, and it exits 0.
#[test]
fn installed_library_keeps_a_real_role_when_its_kill_cannot_reach_the_daemon() {
    let package = Package::new();
    let root = root(&package);
    let mut session = Session::idle(&package, &root);
    let mut startup = session.startup(&package, 1);
    startup["limits"] = json!({ "kill_ms": 1000 });
    let steps = json!([
        { "do": "create", "startup": startup },
        { "do": "submit", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run" },
        { "do": "drain", "until": { "tag": INITIAL_METADATA } },
        { "do": "signal", "pid": session.pid, "signal": "stop" },
        { "do": "submit", "request": kill(2, &[]) },
        { "do": "drain", "until": { "response": 2 } },
        { "do": "signal", "pid": session.pid, "signal": "continue" },
        { "do": "shutdown" },
        { "do": "finish" },
    ]);
    let report = script(&package, &root, steps);
    let opened = step(&report, 3)["records"].as_array().unwrap();
    assert_eq!(opened.len(), 2, "{opened:#?}");
    assert_opened(opened, 1, 1);
    let killed = step(&report, 6)["records"].as_array().unwrap();
    assert_eq!(killed.len(), 1, "{killed:#?}");
    assert_rejected(&killed[0], 1, 2, ORIGIN_LIBRARY, "timeout");
    let (later, terminal) = finished(step(&report, 9));
    assert!(later.is_empty(), "{later:#?}");
    assert_left(terminal, 1);
    assert_all_released(&report["ledger"]);
    session.end();
    assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
    package.assert_clean();
}

/// REC-01 Library (recoverable failures call for passive retry; a
/// configuration the session cannot honour is permanent): instance A's
/// passive role on a real session loses its transport when the session
/// daemon is killed: the stream fails with transport `io_error`, retry
/// passive, its Leave not required, and no result or Leave is invented.
/// Instance B, whose startup names another session beside that session's
/// real metadata, is refused before any role: its Open is rejected
/// `invalid_configuration`, for good. With the hooked library's
/// `panic:receive`, instance C's connection panics at the first frame it
/// receives, the Open's Response: the panic is contained and fails the
/// stream with library `internal_error`, retry passive, the Open
/// unanswered. Skipped against an exact release archive, whose library
/// has no hooks.
#[test]
fn installed_library_classifies_real_failures_for_retry() {
    let package = Package::new();
    let root = root(&package);
    let mut session = Session::idle(&package, &root);
    let mut another = session.startup(&package, 2);
    another["session_id"] = json!("0123456789abcdef0123456789abcdef");
    let steps = json!([
        { "do": "create", "instance": "b", "startup": another },
        { "do": "submit", "instance": "b", "request": open(1, &REQUIRED, &[]) },
        { "do": "run", "instance": "b" },
        { "do": "finish", "instance": "b" },
        { "do": "create", "instance": "a", "startup": session.startup(&package, 1) },
        { "do": "submit", "instance": "a", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run", "instance": "a" },
        { "do": "drain", "instance": "a", "until": { "tag": INITIAL_METADATA } },
        { "do": "signal", "pid": session.pid, "signal": "kill" },
        { "do": "finish", "instance": "a" },
    ]);
    let report = script(&package, &root, steps);
    let (refused, terminal) = finished(step(&report, 3));
    assert_eq!(refused.len(), 2, "{refused:#?}");
    assert_rejected(&refused[0], 2, 1, ORIGIN_LIBRARY, "invalid_configuration");
    assert_diagnostic(&refused[1], 2, ORIGIN_LIBRARY, "invalid_configuration");
    let permanent = (CLIENT_FAILED, RETRY_PERMANENT, LEAVE_NOT_REQUIRED);
    let error = Some((ORIGIN_LIBRARY, "invalid_configuration"));
    assert_terminal(terminal, 2, permanent, error);
    let opened = step(&report, 7)["records"].as_array().unwrap();
    assert_eq!(opened.len(), 2, "{opened:#?}");
    assert_opened(opened, 1, 1);
    let (lost, terminal) = finished(step(&report, 9));
    assert_eq!(lost.len(), 1, "{lost:#?}");
    assert_diagnostic(&lost[0], 1, ORIGIN_TRANSPORT, "io_error");
    let passive = (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED);
    assert_terminal(terminal, 1, passive, Some((ORIGIN_TRANSPORT, "io_error")));
    assert_all_released(&report["ledger"]);
    assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 125);
    assert!(package.list().is_empty());

    if !archive() {
        let session = Session::idle(&package, &root);
        let steps = json!([
            { "do": "create", "instance": "c", "startup": session.startup(&package, 3) },
            { "do": "submit", "instance": "c", "request": open(1, &REQUIRED, &[]) },
            { "do": "run", "instance": "c" },
            { "do": "finish", "instance": "c" },
        ]);
        let input = json!({ "root": root.to_str().unwrap(), "steps": steps });
        let report = scenario(&package, "script", &input, Some("panic:receive"));
        let (panicked, terminal) = finished(step(&report, 3));
        assert_eq!(panicked.len(), 1, "{panicked:#?}");
        assert_diagnostic(&panicked[0], 3, ORIGIN_LIBRARY, "internal_error");
        let error = Some((ORIGIN_LIBRARY, "internal_error"));
        assert_terminal(terminal, 3, passive, error);
        assert_all_released(&report["ledger"]);
        session.end();
        let mut session = session;
        assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
    }
    package.assert_clean();
}
