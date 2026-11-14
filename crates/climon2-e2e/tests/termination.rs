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

use climon2_test_term::Terminal;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
use support::{Package, TIMEOUT, passive::*};

/// A session daemon of the installed release: `climon2 exec` of the generic
/// fixture, which prints `READY`, waits for the file `go`, then follows
/// `then`, attached to its own private-protocol client.
struct Session {
    terminal: Terminal,
    id: String,
    pid: u32,
    metadata: PathBuf,
    socket: PathBuf,
    go: PathBuf,
}

impl Session {
    fn start(package: &Package, root: &Path, then: &[&str]) -> Self {
        let go = root.join("go");
        let mut command = package.command("climon2");
        command.args(["exec", "climon2-fixture", "print", "READY", "await-file"]);
        command.arg(&go);
        command.args(then);
        let mut terminal = Terminal::spawn(command, 30, 120).unwrap();
        terminal.wait_for_text("READY", TIMEOUT).unwrap();
        let metadata = package.metadata();
        let id = metadata.session_id.to_string();
        Self {
            terminal,
            metadata: package
                .isolation
                .climon2_home()
                .join("sessions")
                .join(format!("{id}.pty")),
            socket: PathBuf::from(metadata.socket_path.as_str()),
            pid: metadata.daemon_pid,
            id,
            go,
        }
    }

    /// Startup overrides naming this session, with `limits`.
    fn startup(&self, package: &Package, generation: u64, limits: Value) -> Value {
        let path = |path: &Path| json!({ "bytes": path.to_str().unwrap() });
        json!({
            "session_id": self.id.replace('-', ""),
            "generation": generation,
            "metadata_path": path(&self.metadata),
            "home_path": path(&package.isolation.climon2_home()),
            "runtime_path": path(self.socket.parent().unwrap()),
            "limits": limits,
        })
    }

    fn go(&self) -> &str {
        self.go.to_str().unwrap()
    }
}

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
        let mut session = Session::start(&package, &root, then);
        let steps = json!([
            { "do": "create", "startup": session.startup(&package, generation, json!({})) },
            { "do": "submit", "request": open(1, &REQUIRED, &["title-v1"]) },
            { "do": "run" },
            { "do": "drain", "until": { "tag": INITIAL_METADATA } },
            { "do": "touch", "path": session.go() },
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
/// session whose command waits forever, then submits a Kill alongside it;
/// a Kill-only instance K, which never opens, submits one too, joining the
/// same termination. After the hang-up grace, each Kill is answered OK
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
    let mut session = Session::start(&package, &root, &["await-file", &never]);
    let steps = json!([
        { "do": "create", "instance": "a", "startup": session.startup(&package, 1, json!({})) },
        { "do": "create", "instance": "k", "startup": session.startup(&package, 2, json!({})) },
        { "do": "submit", "instance": "a", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run", "instance": "a" },
        { "do": "drain", "instance": "a", "until": { "tag": INITIAL_METADATA } },
        { "do": "touch", "path": session.go() },
        { "do": "submit", "instance": "a", "request": kill(2, &[]) },
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
    assert!(!session.metadata.exists() && !session.socket.exists());

    let mut records = step(&report, 4)["records"].as_array().unwrap().clone();
    let (later, terminal) = finished(step(&report, 10));
    records.extend(later);
    assert_eq!(records.len(), 4, "{records:#?}");
    assert_opened(&records, 1, 1);
    // The Kill's answer and the Command final come over two connections,
    // in either order.
    let answer = records[2..]
        .iter()
        .find(|record| record["tag"] == RESPONSE)
        .unwrap();
    let response = assert_response(answer, 1, 2, OK);
    assert_eq!(response["result_tag"], TERMINATION, "{response:#}");
    assert_eq!(response["result"], confirmed, "{response:#}");
    let finals: Vec<_> = records[2..]
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
    let session = Session::start(&package, &root, &["exit", "0"]);
    let limits = json!({ "start_ms": 1000 });
    let steps = json!([
        { "do": "signal", "pid": session.pid, "signal": "stop" },
        { "do": "create", "instance": "a", "startup": session.startup(&package, 1, limits) },
        { "do": "submit", "instance": "a", "request": open(1, &REQUIRED, &[]) },
        { "do": "run", "instance": "a" },
        { "do": "drain", "instance": "a", "until": "terminal" },
        { "do": "destroy", "instance": "a" },
        { "do": "signal", "pid": session.pid, "signal": "continue" },
        { "do": "finish", "instance": "a" },
        { "do": "create", "instance": "b", "startup": session.startup(&package, 2, json!({})) },
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
    fs::write(&session.go, b"").unwrap();
    let mut session = session;
    session.terminal.wait_exit(TIMEOUT).unwrap();
    package.assert_clean();
}
