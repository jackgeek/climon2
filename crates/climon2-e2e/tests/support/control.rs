//! The control and display suites' vocabulary (#94): native constants from
//! the header, control requests, a real session whose application echoes
//! its raw input, and the records the native contract specifies. Expected
//! values are written here by hand from the header, the native contract and
//! the daemon wire reference.

use super::{Package, TIMEOUT, passive::*};
use climon2_test_term::Terminal;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const INPUT_ACCEPTED: u64 = 2;
pub const RESIZE_ACCEPTED: u64 = 3;
pub const ACQUIRED: u64 = 4;
pub const READY_RESULT: u64 = 5;
pub const FOCUS_RESULT: u64 = 10;
pub const GRANTED: u64 = 7;
pub const REVOKED: u64 = 8;
pub const RESTORATION_BEGIN: u64 = 9;
pub const DISPLAY: u64 = 10;
pub const RESTORATION_COMPLETE: u64 = 11;
pub const FINAL: u64 = 12;
pub const GEOMETRY: u64 = 13;
pub const ATTACH: u64 = 1;
pub const TAKE_CONTROL: u64 = 2;
pub const HAND_BACK: u64 = 3;
pub const SUCCESSOR: u64 = 4;
pub const TAKEOVER: u64 = 1;
pub const SESSION_ENDING: u64 = 2;
pub const INTERACTIVE: u64 = 2;

/// The latest and the earlier authority the host drained for an instance.
pub const CURRENT: (&str, &str) = ("$period", "$restoration");
pub const STALE: (&str, &str) = ("$stale_period", "$stale_restoration");

/// An interactive Open with public `id` at `cols` x `rows`, requiring core
/// and asking for titles and focus.
pub fn interactive(id: u64, cols: u64, rows: u64) -> Value {
    json!({
        "tag": "open",
        "request_id": id,
        "open": {
            "mode": INTERACTIVE,
            "geometry": [cols, rows],
            "required": ["daemon-wire-v1"],
            "optional": ["title-v1", "focus-v1"],
        },
    })
}

pub fn acquire(id: u64, generation: u64) -> Value {
    json!({ "tag": "acquire", "request_id": id, "generation": generation })
}

fn authority(generation: u64, (period, restoration): (&str, &str)) -> Value {
    json!([generation, period, restoration])
}

/// Input of the bytes `hex` under the instance's `which` authority.
pub fn input(id: u64, generation: u64, which: (&str, &str), hex: &str) -> Value {
    json!({
        "tag": "input",
        "request_id": id,
        "authority": authority(generation, which),
        "bytes": { "hex": hex },
    })
}

pub fn resize(id: u64, generation: u64, which: (&str, &str), cols: u64, rows: u64) -> Value {
    json!({
        "tag": "resize",
        "request_id": id,
        "authority": authority(generation, which),
        "geometry": [cols, rows],
    })
}

pub fn ready(id: u64, generation: u64, which: (&str, &str)) -> Value {
    json!({ "tag": "ready", "request_id": id, "authority": authority(generation, which) })
}

pub fn focus(id: u64, generation: u64, which: (&str, &str), value: u64) -> Value {
    json!({
        "tag": "focus",
        "request_id": id,
        "authority": authority(generation, which),
        "value": value,
    })
}

/// A real session whose application enables focus reporting, prints
/// `ECHO-READY`, then writes back every raw input byte unchanged until its
/// input contains `END`, and exits 0 once the `end` file exists.
pub struct Session {
    pub terminal: Terminal,
    pub id: String,
    metadata: PathBuf,
    socket: PathBuf,
    end: PathBuf,
}

impl Session {
    pub fn start(package: &Package, root: &Path) -> Self {
        Self::running(package, root, &["echo-until", "END"])
    }

    /// A session whose application only waits for the `end` file after
    /// `ECHO-READY`, whoever controls it.
    pub fn idle(package: &Package, root: &Path) -> Self {
        Self::running(package, root, &[])
    }

    fn running(package: &Package, root: &Path, then: &[&str]) -> Self {
        let end = root.join("end");
        let mut command = package.command("climon2");
        command.args(["exec", "climon2-fixture", "raw", "print", "\x1b[?1004h"]);
        command.args(["print", "ECHO-READY"]);
        command.args(then);
        command.arg("await-file");
        command.arg(&end);
        let mut terminal = Terminal::spawn(command, 30, 120).unwrap();
        terminal.wait_for_text("ECHO-READY", TIMEOUT).unwrap();
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
            end,
        }
    }

    /// The same session, detached by its client's escape prefix.
    pub fn detached(package: &Package, root: &Path) -> Self {
        let mut session = Self::idle(package, root);
        session.terminal.send(super::DETACH).unwrap();
        assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
        let notice = format!("climon2: detached from session {}", session.id);
        session
            .terminal
            .wait_for_bytes(notice.as_bytes(), TIMEOUT)
            .unwrap();
        session
    }

    /// Startup overrides naming this session.
    pub fn startup(&self, package: &Package, generation: u64) -> Value {
        let path = |path: &Path| json!({ "bytes": path.to_str().unwrap() });
        json!({
            "session_id": self.id.replace('-', ""),
            "generation": generation,
            "metadata_path": path(&self.metadata),
            "home_path": path(&package.isolation.climon2_home()),
            "runtime_path": path(self.socket.parent().unwrap()),
        })
    }

    pub fn socket(&self) -> &str {
        self.socket.to_str().unwrap()
    }

    /// Lets the application exit 0 once its input held `END`.
    pub fn end(&self) {
        fs::write(&self.end, b"").unwrap();
    }

    /// The file whose creation lets the application exit, for a `touch`
    /// step.
    pub fn end_path(&self) -> &str {
        self.end.to_str().unwrap()
    }
}

/// A record's active presentation member, after its header.
pub fn presented(record: &Value, tag: u64, generation: u64) -> &Value {
    assert_header(record, tag, generation);
    &record["presentation"]
}

/// `[period, restoration_id]` of a presentation record or authority result.
pub fn ids(value: &Value) -> [u64; 2] {
    [
        value["period"].as_u64().unwrap(),
        value["restoration_id"].as_u64().unwrap(),
    ]
}

/// A successful Response to `id` carrying `result_tag`: its result.
pub fn assert_ok(record: &Value, generation: u64, id: u64, result_tag: u64) -> &Value {
    let response = assert_response(record, generation, id, OK);
    assert_eq!(response["result_tag"], result_tag, "{record:#}");
    assert_eq!(response["error"], no_error(), "{record:#}");
    &response["result"]
}

/// An Acquire or Ready result: whether it changed, and the authority.
pub fn assert_authority(result: &Value, changed: u64, authority: [u64; 2]) {
    assert_eq!(result["changed"], changed, "{result:#}");
    assert_eq!(
        result["reserved"],
        json!([0, 0, 0, 0, 0, 0, 0]),
        "{result:#}"
    );
    assert_eq!(ids(result), authority, "{result:#}");
}

/// The records of `tag` among `records`.
pub fn tagged(records: &[Value], tag: u64) -> Vec<&Value> {
    records
        .iter()
        .filter(|record| record["tag"] == tag)
        .collect()
}

/// The Response to `id` among `records`.
pub fn answer(records: &[Value], id: u64) -> &Value {
    records
        .iter()
        .find(|record| record["response"]["request_id"] == id)
        .unwrap_or_else(|| panic!("no Response to {id}: {records:#?}"))
}

/// The Display bytes among `records`, as hex, after checking each names
/// `authority`.
pub fn display(records: &[Value], generation: u64, authority: [u64; 2]) -> String {
    tagged(records, DISPLAY)
        .into_iter()
        .map(|record| {
            let display = presented(record, DISPLAY, generation);
            assert_eq!(ids(display), authority, "{record:#}");
            display["bytes"].as_str().unwrap().to_owned()
        })
        .collect()
}

/// Terminal evidence of an instance that shut down after a confirmed Leave
/// of its attachment.
pub fn assert_detached(terminal: &Value, generation: u64) {
    assert_eq!(terminal["generation"], generation, "{terminal:#}");
    assert_eq!(terminal["client_outcome"], CLIENT_STOPPED, "{terminal:#}");
    assert_eq!(terminal["retry"], RETRY_NONE, "{terminal:#}");
    assert_eq!(terminal["leave"], LEAVE_CONFIRMED, "{terminal:#}");
    assert_eq!(terminal["attachment_detached"], 1, "{terminal:#}");
    assert_eq!(terminal["connection_closed"], 1, "{terminal:#}");
    assert_eq!(terminal["error"], no_error(), "{terminal:#}");
}

/// The presentation records from a grant through its Restoration complete,
/// whatever Responses or metadata interleave: Control granted for `reason`
/// at `geometry`, its Restoration begin at the same authority and geometry,
/// Display chunks of that authority, and the matching complete. The
/// authority, nonzero.
pub fn assert_restored(
    records: &[Value],
    generation: u64,
    reason: u64,
    geometry: [u64; 2],
) -> [u64; 2] {
    let presented_only: Vec<&Value> = records
        .iter()
        .filter(|record| {
            let tag = record["tag"].as_u64().unwrap();
            (GRANTED..=GEOMETRY).contains(&tag) && tag != FINAL
        })
        .collect();
    let granted = presented(presented_only[0], GRANTED, generation);
    let authority = ids(granted);
    assert!(authority.iter().all(|id| *id > 0), "{granted:#}");
    assert_eq!(granted["geometry"], json!(geometry), "{granted:#}");
    assert_eq!(granted["reason"], reason, "{granted:#}");
    assert_eq!(granted["reserved"], 0, "{granted:#}");
    let begin = presented(presented_only[1], RESTORATION_BEGIN, generation);
    assert_eq!(ids(begin), authority, "{begin:#}");
    assert_eq!(begin["geometry"], json!(geometry), "{begin:#}");
    let complete = presented_only
        .iter()
        .position(|record| record["tag"] == RESTORATION_COMPLETE)
        .unwrap();
    for chunk in &presented_only[2..complete] {
        let display = presented(chunk, DISPLAY, generation);
        assert_eq!(ids(display), authority, "{chunk:#}");
        assert!(!display["bytes"].as_str().unwrap().is_empty());
    }
    assert_eq!(
        ids(presented(
            presented_only[complete],
            RESTORATION_COMPLETE,
            generation
        )),
        authority
    );
    authority
}

/// The report of the step `steps` labels `label` with `"as"`, a member the
/// host ignores.
pub fn labelled<'a>(report: &'a Value, steps: &[Value], label: &str) -> &'a Value {
    let index = steps
        .iter()
        .position(|step| step["as"] == label)
        .unwrap_or_else(|| panic!("no step labelled {label}"));
    &report["steps"][index]
}

/// The records the step `steps` labels `label` drained.
pub fn drained(report: &Value, steps: &[Value], label: &str) -> Vec<Value> {
    labelled(report, steps, label)["records"]
        .as_array()
        .unwrap()
        .clone()
}

/// A `CM1_FINAL` record of `generation`: its command result, with status
/// unavailable, since `status-v1` is never selected.
pub fn assert_final(record: &Value, generation: u64, command: Value) {
    assert_header(record, FINAL, generation);
    assert_eq!(record["final"]["command_result"], command, "{record:#}");
    assert_eq!(
        record["final"]["status"],
        json!({ "available": 0, "reserved": 0 }),
        "{record:#}"
    );
}

/// An exit or signal command result as the native record carries it,
/// with no signal name.
pub fn command_result(kind: u64, value: u64) -> Value {
    let unnamed = json!({ "present": 0, "reserved": 0, "value": { "null": true, "length": 0 } });
    json!({ "kind": kind, "value": value, "signal_name": unnamed })
}

/// The tags of `records`.
pub fn tags(records: &[Value]) -> Vec<u64> {
    records
        .iter()
        .map(|record| record["tag"].as_u64().unwrap())
        .collect()
}
