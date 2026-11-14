//! Library layer (#93): passive monitor connections through the installed
//! client library's frozen C ABI, against real session daemons started
//! through the packaged CLI. The `climon2-library-host` fixture's `script`
//! scenario drives each instance step by step from the threads the contract
//! names; expected values come from the native header, the native contract
//! and the daemon wire reference, never from another run of the library.
//! Fault cases against the host's fake daemon peer are in
//! `passive_faults.rs`.

#[allow(dead_code)]
mod support;

use climon2_test_term::Terminal;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
use support::{Package, TIMEOUT, library::INSTANCE_DESCRIPTORS, passive::*};

/// A session daemon of the installed release: `climon2 exec` of the generic
/// fixture, which sets the title `first`, prints `TITLE-SET`, waits for
/// `go`, then in one write empties the title, sets `café`, resets the
/// terminal (unset) and sets `last` through OSC 0, prints `BURST-DONE`, and
/// waits for `end` to exit 0.
struct Session {
    terminal: Terminal,
    id: String,
    metadata: PathBuf,
    socket: PathBuf,
    go: PathBuf,
    end: PathBuf,
}

impl Session {
    fn start(package: &Package, root: &Path) -> Self {
        let (go, end) = (root.join("go"), root.join("end"));
        let mut command = package.command("climon2");
        command.args(["exec", "climon2-fixture", "print", "\x1b]2;first\x07"]);
        command.args(["print", "TITLE-SET", "await-file"]);
        command.arg(&go);
        command.args(["print", "\x1b]2;\x07\x1b]2;café\x07\x1bc\x1b]0;last\x07"]);
        command.args(["print", "BURST-DONE", "await-file"]);
        command.arg(&end);
        let mut terminal = Terminal::spawn(command, 30, 120).unwrap();
        // The title precedes this text through the daemon's terminal.
        terminal.wait_for_text("TITLE-SET", TIMEOUT).unwrap();
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
            id,
            go,
            end,
        }
    }

    /// The same session, detached by its client's escape prefix: its
    /// command keeps running with no attachment.
    fn detached(package: &Package, root: &Path) -> Self {
        let mut session = Self::start(package, root);
        session.terminal.send(b"\x1dd").unwrap();
        assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
        let notice = format!("climon2: detached from session {}", session.id);
        session
            .terminal
            .wait_for_bytes(notice.as_bytes(), TIMEOUT)
            .unwrap();
        session
    }

    /// Startup overrides naming this session.
    fn startup(&self, package: &Package, generation: u64) -> Value {
        let path = |path: &Path| json!({ "bytes": path.to_str().unwrap() });
        json!({
            "session_id": self.id.replace('-', ""),
            "generation": generation,
            "metadata_path": path(&self.metadata),
            "home_path": path(&package.isolation.climon2_home()),
            "runtime_path": path(self.socket.parent().unwrap()),
        })
    }

    fn socket(&self) -> &str {
        self.socket.to_str().unwrap()
    }

    /// Lets the fixture finish and exit 0; an attached client then ends with
    /// it.
    fn end(&self) {
        fs::write(&self.go, b"").unwrap();
        fs::write(&self.end, b"").unwrap();
    }
}

/// The Response and Initial metadata a passive Open of a session titled
/// `first` delivers: OK `opened` with a fresh connection GUID and the
/// families the library relays and the daemon selected, then the atomic
/// report: the title by value, status and progress explicitly unavailable,
/// no command result.
fn assert_opened(records: &[Value], id: u64, generation: u64, selected: &[&str]) {
    assert_eq!(records.len(), 2, "{records:#?}");
    let response = assert_response(&records[0], generation, id, OK);
    assert_eq!(response["result_tag"], OPENED);
    assert_eq!(response["error"], no_error());
    let opened = &response["result"];
    let connection = opened["connection_id"].as_str().unwrap();
    assert_eq!(connection.len(), 32);
    assert_ne!(connection, "0".repeat(32), "a fresh connection GUID");
    let mut tokens: Vec<_> = opened["selected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|token| token.as_str().unwrap())
        .collect();
    tokens.sort_unstable();
    assert_eq!(tokens, selected);
    assert_header(&records[1], INITIAL_METADATA, generation);
    let metadata = &records[1]["metadata"];
    assert_eq!(metadata["title"], title(VALUE, Some("first")));
    assert_eq!(metadata["status"], json!({ "available": 0, "reserved": 0 }));
    assert_eq!(
        metadata["progress"],
        json!({ "available": 0, "reserved": 0 })
    );
    assert_eq!(metadata["command_result"]["present"], 0);
    assert_eq!(metadata["command_result"]["reserved"], 0);
}

/// The burst's Title changed records: empty, `café`, unset by the reset,
/// then `last`, in order and nothing else.
fn assert_burst(records: &[Value], generation: u64) {
    let expected = [
        title(VALUE, Some("")),
        title(VALUE, Some("café")),
        title(UNSET, None),
        title(VALUE, Some("last")),
    ];
    assert_eq!(records.len(), expected.len(), "{records:#?}");
    for (record, expected) in records.iter().zip(expected) {
        assert_header(record, TITLE, generation);
        assert_eq!(record["title"], expected, "{record:#}");
    }
}

/// The steps of a passive registration of `session`: hello `state` before,
/// Open from caller memory, its Response and Initial metadata kept as live
/// deliveries, `state` while registered, the title burst, the kept
/// deliveries observed again, `state` again, then shutdown with them still
/// live, and the whole cleanup.
fn passive_steps(package: &Package, session: &Session, generation: u64) -> Value {
    json!([
        { "do": "create", "startup": session.startup(package, generation) },
        { "do": "probe", "socket": session.socket() },
        { "do": "submit", "request": open(5, &REQUIRED, &OPTIONAL) },
        { "do": "snapshot" },
        { "do": "run" },
        { "do": "drain", "until": { "tag": INITIAL_METADATA }, "keep": true },
        { "do": "probe", "socket": session.socket() },
        { "do": "snapshot" },
        { "do": "touch", "path": session.go.to_str().unwrap() },
        { "do": "drain", "until": { "records": 4 } },
        { "do": "observe" },
        { "do": "probe", "socket": session.socket() },
        { "do": "shutdown" },
        { "do": "finish" },
    ])
}

/// What every passive registration of `passive_steps` shows, whatever
/// controls the session: `state` unchanged throughout, Response and atomic
/// Initial metadata, the gap-free burst, live deliveries unchanged until
/// released, one confirmed Leave on shutdown.
fn assert_passive(report: &Value, generation: u64, state: &Value) {
    assert_success(&step(report, 0)["call"]);
    for index in [1, 6, 11] {
        assert_eq!(&step(report, index)["state"], state, "step {index}");
    }
    assert_success(step(report, 2));
    let queued = &step(report, 3)["output"];
    assert_eq!(queued["request_records"], 1, "{queued:#}");
    assert_eq!(queued["operations"], 1, "{queued:#}");
    let all: Vec<&str> = REQUIRED.iter().chain(&OPTIONAL).copied().collect();
    assert_eq!(queued["request_bytes"], charge(&all), "{queued:#}");
    let selected = ["daemon-wire-v1", "focus-v1", "monitor-v1", "title-v1"];
    let first = step(report, 5)["records"].as_array().unwrap();
    assert_opened(first, 5, generation, &selected);
    // Whether the two records share a delivery depends on when the drain
    // ran; every kept delivery stays charged and readable until released.
    let observed = step(report, 10).as_array().unwrap();
    let opened = &step(report, 7)["output"];
    assert_eq!(opened["request_records"], 0, "{opened:#}");
    assert_eq!(opened["request_bytes"], 0, "{opened:#}");
    assert_eq!(opened["operations"], 0, "{opened:#}");
    assert_eq!(opened["deliveries"], observed.len(), "{opened:#}");
    assert_eq!(opened["event_records"], 2, "{opened:#}");
    assert_eq!(
        opened["descriptors"],
        INSTANCE_DESCRIPTORS + 1,
        "{opened:#}"
    );
    assert_burst(step(report, 9)["records"].as_array().unwrap(), generation);
    let kept: Vec<Value> = observed
        .iter()
        .flat_map(|delivery| delivery["records"].as_array().unwrap().clone())
        .collect();
    assert_opened(&kept, 5, generation, &selected);
    assert_success(step(report, 12));
    let finish = step(report, 13);
    let (records, terminal) = finished(finish);
    assert!(records.is_empty(), "{records:#?}");
    assert_left(terminal, generation);
    for released in finish["released"].as_array().unwrap() {
        assert_success(released);
    }
    assert_all_released(&report["ledger"]);
}

/// CTL-01 Library (a passive registration of a session controlled
/// elsewhere never acquires control and is not an attachment: hello `state`
/// is the same before, during and after it, and the controlling client
/// keeps receiving the session's live output), CTL-03 Library (title case:
/// Response, atomic Initial metadata with the title by value, then
/// gap-free Title changed records by value, empty and unset distinct),
/// COM-01 Library (the optional families intersect: of six requested, only
/// `title-v1` and `focus-v1`, which the library relays and the daemon
/// advertises, are selected; an unknown family is not), COM-03 Library (the Open's
/// arguments were copied before `submit` returned and the caller's memory
/// unmapped; public ID 5 is answered; the request is charged until it
/// executes), LIB-04 Library (the kept deliveries' records are unchanged
/// after later drains, until released through the library) and EXT-01
/// Library (shutdown with live deliveries attempts one Leave: confirmed,
/// closed, stopped; worker cleanup and handle release reported apart).
#[test]
fn installed_library_registers_passively_while_controlled_elsewhere_and_relays_titles() {
    let package = Package::new();
    let root = root(&package);
    let mut session = Session::start(&package, &root);
    let report = script(&package, &root, passive_steps(&package, &session, 3));
    let attached = json!({ "lifecycle": "attached", "attachments": 1, "controlled": true });
    assert_passive(&report, 3, &attached);
    session
        .terminal
        .wait_for_text("BURST-DONE", TIMEOUT)
        .unwrap();
    session.end();
    assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
    package.assert_clean();
}

/// CTL-01 Library (detached case: registering never acquires a detached
/// session; hello `state` stays detached, with no attachment and no
/// controller, before, during and after the registration) and CTL-03
/// Library (core metadata flows to a passive connection with nothing
/// attached), with the same Open, burst and confirmed Leave on shutdown.
#[test]
fn installed_library_registration_never_acquires_a_detached_session() {
    let package = Package::new();
    let root = root(&package);
    let session = Session::detached(&package, &root);
    let report = script(&package, &root, passive_steps(&package, &session, 4));
    let detached = json!({ "lifecycle": "detached", "attachments": 0, "controlled": false });
    assert_passive(&report, 4, &detached);
    session.end();
    package.assert_clean();
}

/// COM-03 Library (explicit Leave: the public request gets its own
/// `left` Response, passive deregistration so nothing detached; a second
/// Open on the instance is rejected without effect, `invalid_state`, and a
/// Leave naming another generation `stale_generation`; IDs only increase
/// and each maps to its own wire ID) and EXT-01 Library (after the
/// confirmed explicit Leave, terminal evidence is stopped with Leave
/// confirmed, and no other Leave is attempted).
#[test]
fn installed_library_explicit_leave_departs_once_and_is_answered() {
    let package = Package::new();
    let root = root(&package);
    let session = Session::start(&package, &root);
    let steps = json!([
        { "do": "create", "startup": session.startup(&package, 2) },
        { "do": "submit", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run" },
        { "do": "drain", "until": { "tag": INITIAL_METADATA } },
        { "do": "submit", "request": open(2, &REQUIRED, &[]) },
        { "do": "submit", "request": leave(3, 9) },
        { "do": "submit", "request": leave(4, 2) },
        { "do": "drain", "until": "terminal" },
        { "do": "submit", "request": leave(5, 2) },
        { "do": "finish" },
    ]);
    let report = script(&package, &root, steps);
    for index in [1, 4, 5, 6] {
        assert_success(step(&report, index));
    }
    let records = step(&report, 7)["records"].as_array().unwrap();
    assert_eq!(records.len(), 3, "{records:#?}");
    assert_rejected(&records[0], 2, 2, ORIGIN_LIBRARY, "invalid_state");
    assert_rejected(&records[1], 2, 3, ORIGIN_LIBRARY, "stale_generation");
    let left = assert_response(&records[2], 2, 4, OK);
    assert_eq!(left["result_tag"], LEFT, "{left:#}");
    assert_eq!(left["result"], json!({ "attachment_detached": 0 }));
    assert_eq!(left["error"], no_error());
    assert_failure(step(&report, 8), CLOSED, "closed");
    let (records, terminal) = finished(step(&report, 9));
    assert!(records.is_empty(), "{records:#?}");
    assert_left(terminal, 2);
    assert_all_released(&report["ledger"]);
    session.end();
    package.assert_clean();
}

/// EXT-02 Library (an unconfirmed required Leave is a failure, never a false
/// departure): a passive registration of a real session whose daemon then
/// stops (`SIGSTOP`) before shutdown. Shutdown's own Leave is written but
/// never answered, so after `leave_ms` the departure is unconfirmed: a
/// library `timeout` diagnostic, then failed terminal evidence with Leave
/// indeterminate, nothing detached, the connection closed and no retry once
/// shutdown was requested. The daemon then resumes and the session ends
/// normally.
#[test]
fn installed_library_reports_a_lost_leave_to_a_stopped_daemon_as_unconfirmed() {
    let package = Package::new();
    let root = root(&package);
    let session = Session::start(&package, &root);
    let daemon = package.metadata().daemon_pid;
    let mut startup = session.startup(&package, 8);
    startup["limits"] = json!({ "leave_ms": 300 });
    let steps = json!([
        { "do": "create", "startup": startup },
        { "do": "submit", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run" },
        { "do": "drain", "until": { "tag": INITIAL_METADATA } },
        { "do": "signal", "pid": daemon, "signal": "stop" },
        { "do": "shutdown" },
        { "do": "wait_run" },
        { "do": "finish" },
        { "do": "signal", "pid": daemon, "signal": "continue" },
    ]);
    let report = script(&package, &root, steps);
    assert_opened(
        step(&report, 3)["records"].as_array().unwrap(),
        1,
        8,
        &["daemon-wire-v1", "monitor-v1", "title-v1"],
    );
    assert_eq!(step(&report, 4), "signalled");
    assert_success(step(&report, 5));
    let (records, terminal) = finished(step(&report, 7));
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_diagnostic(&records[0], 8, ORIGIN_LIBRARY, "timeout");
    assert_terminal(
        terminal,
        8,
        (CLIENT_FAILED, RETRY_NONE, LEAVE_INDETERMINATE),
        Some((ORIGIN_LIBRARY, "timeout")),
    );
    assert_eq!(step(&report, 8), "signalled");
    assert_all_released(&report["ledger"]);
    session.end();
    package.assert_clean();
}

/// COM-01 Library (incompatible peers are rejected before any role takes
/// effect): the real daemon advertises 1,048,576-byte server frames, which
/// fit an instance's `wire_frame_bytes` exactly at that limit (the other
/// tests) but not one byte under it: the Open is rejected with
/// `unsupported_limit`, for good, and the session is untouched.
#[test]
fn installed_library_rejects_a_daemon_whose_frames_exceed_wire_frame_bytes() {
    let package = Package::new();
    let root = root(&package);
    let session = Session::start(&package, &root);
    let mut startup = session.startup(&package, 6);
    startup["limits"] = json!({ "wire_frame_bytes": 1_048_575 });
    let steps = json!([
        { "do": "create", "startup": startup },
        { "do": "submit", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run" },
        { "do": "probe", "socket": session.socket() },
        { "do": "finish" },
    ]);
    let report = script(&package, &root, steps);
    let (records, terminal) = finished(step(&report, 4));
    assert_eq!(records.len(), 2, "{records:#?}");
    assert_rejected(&records[0], 6, 1, ORIGIN_LIBRARY, "unsupported_limit");
    assert_diagnostic(&records[1], 6, ORIGIN_LIBRARY, "unsupported_limit");
    let refused = (CLIENT_FAILED, RETRY_PERMANENT, LEAVE_NOT_REQUIRED);
    assert_terminal(
        terminal,
        6,
        refused,
        Some((ORIGIN_LIBRARY, "unsupported_limit")),
    );
    let attached = json!({ "lifecycle": "attached", "attachments": 1, "controlled": true });
    assert_eq!(step(&report, 3)["state"], attached);
    session.end();
    package.assert_clean();
}

/// How long a call the contract keeps free of daemon I/O and waiting may
/// take on a loaded host: far less than the time the stalled daemon is
/// stalled.
const PROMPT_US: u64 = 1_000_000;

fn assert_prompt(call: &Value) {
    let elapsed = call["elapsed_us"].as_u64().unwrap();
    assert!(elapsed < PROMPT_US, "{elapsed} µs: {call:#}");
}

/// LIB-05 Library and Fault (a stalled instance does not stop another, and
/// `submit`, `drain` and `release_delivery` never perform daemon I/O or
/// wait): instance `stalled` sends its Open to a fake daemon that does not
/// answer; meanwhile instance `live` registers with a real session daemon,
/// drains its Response and Initial metadata, and shuts down with its Leave
/// confirmed. The stalled instance's `submit` of a Leave, `drain` and
/// `snapshot`, and every `live` drain and release, return at once. Once the
/// fake daemon answers, the stalled instance's Open succeeds and its queued
/// Leave departs, sent only after the Initial metadata.
#[test]
fn installed_library_instance_stalled_by_its_daemon_never_stops_another() {
    let package = Package::new();
    let root = root(&package);
    let session = Session::start(&package, &root);
    let go = root.join("go-stalled");
    let stalled = json!({
        "open": [{ "wait": go.to_str().unwrap() }, opened(&SELECTED), initial(json!("t"))],
        "leave": [leave_answer("ok", false, Value::Null)],
    });
    let steps = json!([
        { "do": "create", "instance": "stalled", "peer": stalled,
          "startup": { "limits": { "start_ms": 60_000 } } },
        { "do": "create", "instance": "live", "startup": session.startup(&package, 2) },
        { "do": "submit", "instance": "stalled", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run", "instance": "stalled" },
        { "do": "await_peer", "instance": "stalled", "frames": 2 },
        { "do": "submit", "instance": "live", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run", "instance": "live" },
        { "do": "drain", "instance": "live", "until": { "tag": INITIAL_METADATA } },
        { "do": "submit", "instance": "stalled", "request": leave(2, 1) },
        { "do": "drain", "instance": "stalled" },
        { "do": "snapshot", "instance": "stalled" },
        { "do": "shutdown", "instance": "live" },
        { "do": "finish", "instance": "live" },
        { "do": "touch", "path": go.to_str().unwrap() },
        { "do": "finish", "instance": "stalled" },
    ]);
    let report = script(&package, &root, steps);
    assert_eq!(step(&report, 4), true);
    let live = step(&report, 7);
    let records = live["records"].as_array().unwrap();
    assert_opened(records, 1, 2, &SELECTED);
    for drain in live["drains"].as_array().unwrap() {
        assert_prompt(drain);
        if drain.get("release").is_some() {
            for call in drain["release"]["calls"].as_array().unwrap() {
                assert_success(call);
            }
            assert_prompt(&drain["release"]);
        }
    }
    let submit = step(&report, 8);
    assert_success(submit);
    assert_prompt(submit);
    let drain = &step(&report, 9)["drains"][0];
    assert_eq!(drain["drained"]["kind"], EMPTY, "{drain:#}");
    assert_prompt(drain);
    let snapshot = step(&report, 10);
    assert_prompt(snapshot);
    assert_eq!(snapshot["output"]["request_records"], 1, "{snapshot:#}");
    assert_eq!(snapshot["output"]["operations"], 2, "{snapshot:#}");
    let (rest, terminal) = finished(step(&report, 12));
    assert!(rest.is_empty(), "{rest:#?}");
    assert_left(terminal, 2);
    let (records, terminal) = finished(step(&report, 14));
    assert_eq!(records.len(), 3, "{records:#?}");
    assert_eq!(assert_response(&records[0], 1, 1, OK)["result_tag"], OPENED);
    assert_header(&records[1], INITIAL_METADATA, 1);
    let left = assert_response(&records[2], 1, 2, OK);
    assert_eq!(left["result_tag"], LEFT, "{left:#}");
    assert_left(terminal, 1);
    assert_eq!(
        kinds(&report["peers"]["stalled"]),
        ["hello", "open", "leave"]
    );
    assert_all_released(&report["ledger"]);
    session.end();
    package.assert_clean();
}

/// COM-01 Library (core and monitor support are implicit: an Open naming no
/// family, as the native contract's minimal example does, still registers
/// passively, and the daemon selects none) and COM-02 Library (an empty
/// selection is null and zero, as every empty native list is): the
/// Response's selection is a null list of zero tokens, the Initial metadata
/// reports the title unavailable, and shutdown leaves, confirmed.
#[test]
fn installed_library_opens_naming_no_family_and_selects_none() {
    let package = Package::new();
    let root = root(&package);
    let session = Session::start(&package, &root);
    let minimal = json!({ "tag": "open", "request_id": 1, "open": { "mode": 1 } });
    let steps = json!([
        { "do": "create", "startup": session.startup(&package, 3) },
        { "do": "submit", "request": minimal },
        { "do": "run" },
        { "do": "drain", "until": { "tag": INITIAL_METADATA } },
        { "do": "shutdown" },
        { "do": "finish" },
    ]);
    let report = script(&package, &root, steps);
    assert_success(step(&report, 1));
    let records = step(&report, 3)["records"].as_array().unwrap();
    assert_eq!(records.len(), 2, "{records:#?}");
    let response = assert_response(&records[0], 3, 1, OK);
    assert_eq!(response["result_tag"], OPENED, "{response:#}");
    let none = json!({ "null": true, "count": 0 });
    assert_eq!(response["result"]["selected"], none, "{response:#}");
    assert_header(&records[1], INITIAL_METADATA, 3);
    let title_record = &records[1]["metadata"]["title"];
    assert_eq!(title_record, &title(UNAVAILABLE, None), "{title_record:#}");
    let (rest, terminal) = finished(step(&report, 5));
    assert!(rest.is_empty(), "{rest:#?}");
    assert_left(terminal, 3);
    assert_all_released(&report["ledger"]);
    session.end();
    package.assert_clean();
}
