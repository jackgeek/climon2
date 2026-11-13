//! The passive-connection suites' vocabulary (#93): native constants from
//! the header, the `script` scenario, requests, the fake daemon peer's
//! traffic, and the records and evidence the native contract specifies.
//! Expected values are written here by hand from the header, the native
//! contract and the daemon wire reference.

use super::{Package, library::scenario};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const SUCCESS: u64 = 0;
pub const INVALID_ARGUMENT: u64 = 2;
pub const UNSUPPORTED_OPERATION: u64 = 3;
pub const INVALID_REQUEST_ID: u64 = 4;
pub const FULL: u64 = 5;
pub const CLOSED: u64 = 6;
pub const ORIGIN_LIBRARY: u64 = 1;
pub const ORIGIN_DAEMON: u64 = 2;
pub const ORIGIN_TRANSPORT: u64 = 3;
pub const OK: u64 = 1;
pub const REJECTED: u64 = 2;
pub const FAILED: u64 = 3;
pub const INDETERMINATE: u64 = 4;
pub const UNAVAILABLE: u64 = 0;
pub const UNSET: u64 = 1;
pub const VALUE: u64 = 2;
pub const EXIT: u64 = 1;
pub const SIGNAL: u64 = 2;
pub const RESULT_NONE: u64 = 0;
pub const OPENED: u64 = 1;
pub const LEFT: u64 = 8;
pub const KNOWN_FINAL: u64 = 11;
pub const RESPONSE: u64 = 1;
pub const INITIAL_METADATA: u64 = 2;
pub const TITLE: u64 = 3;
pub const DIAGNOSTIC: u64 = 14;
pub const DELIVERY: u64 = 1;
pub const EMPTY: u64 = 2;
pub const TERMINAL: u64 = 3;
pub const CLIENT_STOPPED: u64 = 1;
pub const CLIENT_FAILED: u64 = 2;
pub const RETRY_NONE: u64 = 1;
pub const RETRY_PASSIVE: u64 = 2;
pub const RETRY_PERMANENT: u64 = 3;
pub const LEAVE_NOT_REQUIRED: u64 = 1;
pub const LEAVE_CONFIRMED: u64 = 2;
pub const LEAVE_REJECTED: u64 = 3;
pub const LEAVE_FAILED: u64 = 4;
pub const LEAVE_INDETERMINATE: u64 = 5;
pub const CLEANUP_COMPLETE: u64 = 1;
/// The descriptors one instance holds before any connection: its
/// notification descriptor and its waker, an `eventfd` each on Linux and a
/// pipe's two ends each on macOS.
#[cfg(target_os = "linux")]
pub const INSTANCE_DESCRIPTORS: u64 = 2;
#[cfg(target_os = "macos")]
pub const INSTANCE_DESCRIPTORS: u64 = 4;
/// `sizeof(cm1_request)`, `sizeof(cm1_bytes)` and `sizeof(cm1_record)`.
pub const REQUEST: u64 = size_of::<climon2_core::native::Request>() as u64;
pub const TOKEN: u64 = size_of::<climon2_core::native::Bytes>() as u64;
pub const RECORD: u64 = size_of::<climon2_core::native::Record>() as u64;
/// What monitor Open requires, and what the spec's monitor requests besides
/// (an unknown family too).
pub const REQUIRED: [&str; 2] = ["daemon-wire-v1", "monitor-v1"];
pub const OPTIONAL: [&str; 6] = [
    "title-v1",
    "status-v1",
    "notification-v1",
    "progress-v1",
    "focus-v1",
    "x-later-v9",
];
/// What a daemon that relays titles selects for an Open requiring
/// `REQUIRED` with `title-v1` optional.
pub const SELECTED: [&str; 3] = ["daemon-wire-v1", "monitor-v1", "title-v1"];
/// The fake daemon's connection GUID, and the bytes the library reports.
pub const CONNECTION: &str = "00112233-4455-4677-8899-aabbccddeeff";
pub const CONNECTION_BYTES: &str = "00112233445546778899aabbccddeeff";

/// A directory for one scenario's paths, trigger files and fake daemons.
pub fn root(package: &Package) -> PathBuf {
    let root = package.isolation.home().join("instances");
    fs::create_dir_all(&root).unwrap();
    root
}

/// One `script` run over the installed library.
pub fn script(package: &Package, root: &Path, steps: Value) -> Value {
    script_with(package, root, steps, json!({}))
}

/// One `script` run with host `options` (`"sigpipe": "default"`).
pub fn script_with(package: &Package, root: &Path, steps: Value, options: Value) -> Value {
    let mut input = json!({ "root": root.to_str().unwrap(), "steps": steps });
    input
        .as_object_mut()
        .unwrap()
        .extend(options.as_object().unwrap().clone());
    scenario(package, "script", &input, None)
}

pub fn step(report: &Value, index: usize) -> &Value {
    &report["steps"][index]
}

/// A monitor-mode Open with public `id`.
pub fn open(id: u64, required: &[&str], optional: &[&str]) -> Value {
    json!({
        "tag": "open",
        "request_id": id,
        "open": { "mode": 1, "required": required, "optional": optional },
    })
}

/// A Leave with public `id` of connection `generation`.
pub fn leave(id: u64, generation: u64) -> Value {
    json!({ "tag": "leave", "request_id": id, "generation": generation })
}

/// The charge of an Open naming `tokens`: the request and each token.
pub fn charge(tokens: &[&str]) -> u64 {
    REQUEST
        + tokens
            .iter()
            .map(|token| TOKEN + token.len() as u64)
            .sum::<u64>()
}

pub fn assert_success(call: &Value) {
    let empty = json!({ "null": true, "length": 0 });
    assert_eq!(call["code"], SUCCESS, "{call:#}");
    assert_eq!(call["reserved"], 0, "{call:#}");
    assert_eq!(call["origin"], 0, "{call:#}");
    assert_eq!(call["error_code"], empty, "{call:#}");
    assert_eq!(call["message"], empty, "{call:#}");
}

/// An explicit library failure with its stable error code and a message.
pub fn assert_failure(call: &Value, code: u64, error_code: &str) {
    assert_eq!(call["code"], code, "{call:#}");
    assert_eq!(call["reserved"], 0, "{call:#}");
    assert_eq!(call["origin"], ORIGIN_LIBRARY, "{call:#}");
    assert_eq!(call["error_reserved"], 0, "{call:#}");
    assert_eq!(call["error_code"], error_code, "{call:#}");
    assert!(!call["message"].as_str().unwrap().is_empty(), "{call:#}");
}

pub fn no_error() -> Value {
    let empty = json!({ "null": true, "length": 0 });
    json!({ "origin": 0, "reserved": 0, "code": empty, "message": empty })
}

/// An error with its origin and stable code, and a message.
pub fn assert_error(error: &Value, origin: u64, code: &str) {
    assert_eq!(error["origin"], origin, "{error:#}");
    assert_eq!(error["reserved"], 0, "{error:#}");
    assert_eq!(error["code"], code, "{error:#}");
    assert!(!error["message"].as_str().unwrap().is_empty(), "{error:#}");
}

/// A record's header: its tag, reserved zero and the startup generation.
pub fn assert_header(record: &Value, tag: u64, generation: u64) {
    assert_eq!(record["tag"], tag, "{record:#}");
    assert_eq!(record["reserved"], 0, "{record:#}");
    assert_eq!(record["generation"], generation, "{record:#}");
}

/// A title's native form: its state and, for a value, its text (an empty
/// value is null and zero, as every empty native buffer is).
pub fn title(state: u64, value: Option<&str>) -> Value {
    let empty = json!({ "null": true, "length": 0 });
    let value = value
        .filter(|value| !value.is_empty())
        .map_or(empty, |value| json!(value));
    json!({ "state": state, "reserved": 0, "value": value })
}

/// A Response with no result: an outcome, and an error unless OK.
pub fn assert_response(record: &Value, generation: u64, id: u64, outcome: u64) -> &Value {
    assert_header(record, RESPONSE, generation);
    let response = &record["response"];
    assert_eq!(response["request_id"], id, "{record:#}");
    assert_eq!(response["outcome"], outcome, "{record:#}");
    response
}

/// A Response rejecting request `id` with the library's `code`.
pub fn assert_rejected(record: &Value, generation: u64, id: u64, origin: u64, code: &str) {
    let response = assert_response(record, generation, id, REJECTED);
    assert_eq!(response["result_tag"], RESULT_NONE, "{record:#}");
    assert_error(&response["error"], origin, code);
}

/// A diagnostic record of `origin` and `code`.
pub fn assert_diagnostic(record: &Value, generation: u64, origin: u64, code: &str) {
    assert_header(record, DIAGNOSTIC, generation);
    assert_error(&record["diagnostic"], origin, code);
}

/// Terminal evidence: outcome, retry and Leave, nothing detached, every
/// connection closed, and the error or none.
pub fn assert_terminal(
    terminal: &Value,
    generation: u64,
    (outcome, retry, leave): (u64, u64, u64),
    error: Option<(u64, &str)>,
) {
    assert_eq!(terminal["generation"], generation, "{terminal:#}");
    assert_eq!(terminal["client_outcome"], outcome, "{terminal:#}");
    assert_eq!(terminal["retry"], retry, "{terminal:#}");
    assert_eq!(terminal["leave"], leave, "{terminal:#}");
    assert_eq!(terminal["attachment_detached"], 0, "{terminal:#}");
    assert_eq!(terminal["connection_closed"], 1, "{terminal:#}");
    assert_eq!(terminal["reserved"], json!([0, 0]), "{terminal:#}");
    match error {
        Some((origin, code)) => assert_error(&terminal["error"], origin, code),
        None => assert_eq!(terminal["error"], no_error(), "{terminal:#}"),
    }
}

/// Terminal evidence of a passive instance that shut down after a confirmed
/// Leave.
pub fn assert_left(terminal: &Value, generation: u64) {
    assert_terminal(
        terminal,
        generation,
        (CLIENT_STOPPED, RETRY_NONE, LEAVE_CONFIRMED),
        None,
    );
}

/// The terminal evidence `finish` drained, after every record (returned).
pub fn finished(finish: &Value) -> (Vec<Value>, &Value) {
    let drained = &finish["drained"];
    let drains = drained["drains"].as_array().unwrap();
    let last = &drains.last().unwrap()["drained"];
    assert_eq!(last["kind"], TERMINAL, "{last:#}");
    let records = drained["records"].as_array().unwrap().clone();
    assert_eq!(finish["run"]["report"]["terminal"], last["terminal"]);
    assert_eq!(
        finish["run"]["report"]["workers_cleanup"], CLEANUP_COMPLETE,
        "{finish:#}"
    );
    for call in ["release_communication", "release_control", "destroy"] {
        assert_success(&finish[call]);
    }
    assert_eq!(finish["joined"], true, "{finish:#}");
    (records, &last["terminal"])
}

/// Every library object released through its own library, none outstanding.
pub fn assert_all_released(ledger: &Value) {
    assert_eq!(ledger["outstanding"], 0, "{ledger:#}");
    for object in ledger["objects"].as_array().unwrap() {
        let by = match object["kind"].as_str().unwrap() {
            "instance" => "destroy",
            "communication" => "release_communication",
            "control" => "release_control",
            _ => "release_delivery",
        };
        assert_eq!(object["released_by"], by, "{ledger:#}");
    }
}

/// The fake daemon's frame of `type` with a JSON `body`.
pub fn send(code: u8, body: Value) -> Value {
    json!({ "send": { "type": code, "json": body } })
}

/// A Response to the library's request `id` (`"$open"` or `"$leave"`).
pub fn response(id: &str, outcome: &str, result: Value, error: Value) -> Value {
    send(
        0x40,
        json!({ "request_id": id, "outcome": outcome, "result": result, "error": error }),
    )
}

/// The fake daemon's success for the Open, selecting `selected`.
pub fn opened(selected: &[&str]) -> Value {
    response(
        "$open",
        "ok",
        json!({
            "kind": "opened",
            "connection_id": CONNECTION,
            "selected_capabilities": selected,
        }),
        Value::Null,
    )
}

/// The fake daemon's Initial metadata with an available `title`.
pub fn initial(title: Value) -> Value {
    send(
        0x41,
        json!({
            "title": { "available": true, "value": title },
            "status": { "available": false, "value": null },
            "progress": { "available": false, "value": null },
            "command_result": null,
        }),
    )
}

pub fn title_changed(title: Value) -> Value {
    send(0x42, json!({ "title": title }))
}

/// The fake daemon's answer to a Leave.
pub fn leave_answer(outcome: &str, detached: bool, error: Value) -> Value {
    let result = if outcome == "ok" {
        json!({ "kind": "left", "attachment_detached": detached })
    } else {
        Value::Null
    };
    response("$leave", outcome, result, error)
}

pub fn daemon_error(code: &str) -> Value {
    json!({ "code": code, "message": "fake daemon" })
}

/// A fake daemon that opens with `title-v1`, reports the title `t`, and
/// answers each Leave with `leave`.
pub fn passive_peer(leave: Value) -> Value {
    json!({
        "open": [opened(&SELECTED), initial(json!("t"))],
        "leave": leave,
    })
}

/// The fake daemon's received frames: hello, then each frame's kind (and
/// its marks, apart from the `closed` it records after a scripted close).
pub fn kinds(peer: &Value) -> Vec<String> {
    peer["frames"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|frame| frame.get("closed").is_none())
        .map(|frame| {
            if frame.get("hello").is_some() {
                "hello".to_owned()
            } else if frame.get("deaf").is_some() {
                "deaf".to_owned()
            } else if frame.get("wait_expired").is_some() {
                "wait_expired".to_owned()
            } else if frame.get("held").is_some() {
                format!("held:{}", frame["type"])
            } else {
                frame["kind"].as_str().unwrap().to_owned()
            }
        })
        .collect()
}
