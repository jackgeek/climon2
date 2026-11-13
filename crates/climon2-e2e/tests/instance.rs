//! Library layer (#85): client instances through the installed client
//! library's frozen C ABI, driven by the `climon2-library-host` fixture's
//! scenarios from the threads the contract names. The host's caller startup
//! memory is scrambled and unmapped as soon as `create` returns, its
//! ownership ledger shows how every library object was released, and its
//! timeline shows which thread made each call. Expected values come from the
//! native header and contract, never from another run of the library.
//!
//! Fault cases select the hooked library's `CLIMON2_LIBRARY_FAULT`; the
//! no-hooks release archive runs every other case.

#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use std::{
    ffi::OsStr,
    fs,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};
use support::{
    Package,
    library::{archive, scenario},
};

const SUCCESS: u64 = 0;
const INVALID_ARGUMENT: u64 = 2;
const UNSUPPORTED_OPERATION: u64 = 3;
const CLOSED: u64 = 6;
const BUSY: u64 = 7;
const WRONG_THREAD: u64 = 8;
const UNSUPPORTED_LIMIT: u64 = 9;
const RESOURCE_LIMIT: u64 = 10;
const INTERNAL_ERROR: u64 = 11;
const TIMEOUT: u64 = 13;
const RELEASE_MISMATCH: u64 = 14;
const ORIGIN_LIBRARY: u64 = 1;
const DELIVERY: u64 = 1;
const EMPTY: u64 = 2;
const TERMINAL: u64 = 3;
const DIAGNOSTIC: u64 = 14;
const CLIENT_STOPPED: u64 = 1;
const CLIENT_FAILED: u64 = 2;
const RETRY_NONE: u64 = 1;
const RETRY_PASSIVE: u64 = 2;
const LEAVE_NOT_REQUIRED: u64 = 1;
const CLEANUP_COMPLETE: u64 = 1;
const CLEANUP_INCOMPLETE: u64 = 2;
/// The description's instance allocation and worker requirements.
const INSTANCE_BYTES: u64 = 1024 * 1024;
const WORKERS: u64 = 2;
/// `sizeof(cm1_record)` on every supported target.
const RECORD: u64 = size_of::<climon2_core::native::Record>() as u64;
/// The notification descriptors one instance holds: an `eventfd` on Linux,
/// a pipe's two ends on macOS.
#[cfg(target_os = "linux")]
const NOTIFIER_DESCRIPTORS: u64 = 1;
#[cfg(target_os = "macos")]
const NOTIFIER_DESCRIPTORS: u64 = 2;
const PATH_MAX: u64 = nix::libc::PATH_MAX as u64;

/// The documented default `monitor.limits` values the host's startup uses.
fn default_limits() -> Value {
    json!({
        "client_bytes": 67_108_864,
        "request_bytes": 262_144,
        "event_bytes": 1_048_576,
        "progress_bytes": 65_536,
        "drain_payload_bytes": 65_536,
        "request_records": 64,
        "event_records": 1024,
        "progress_records": 32,
        "deliveries": 8,
        "operations": 16,
        "chunk_bytes": 65_536,
        "drain_records": 64,
        "wire_frame_bytes": 1_048_576,
        "metadata_record_bytes": 16_384,
        "title_bytes": 4096,
        "notification_title_bytes": 4096,
        "notification_body_bytes": 8192,
        "max_workers": 2,
        "operation_ms": 5000,
        "kill_ms": 10_000,
        "restoration_stall_ms": 5000,
        "restoration_total_ms": 60_000,
        "delivery_stall_ms": 5000,
        "leave_ms": 5000,
        "cleanup_ms": 10_000,
        "start_ms": 10_000,
        "reserved": 0,
    })
}

/// A directory for one scenario's instance paths and logs.
fn root(package: &Package) -> PathBuf {
    let root = package.isolation.home().join("instances");
    fs::create_dir_all(root.join("logs")).unwrap();
    root
}

fn input(root: &Path, fields: Value) -> Value {
    let mut input = json!({ "root": root.to_str().unwrap() });
    input
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    input
}

/// A successful call: code `CM1_SUCCESS`, no origin, empty error buffers.
fn assert_success(call: &Value) {
    let empty = json!({ "null": true, "length": 0 });
    assert_eq!(
        call,
        &json!({
            "code": SUCCESS,
            "reserved": 0,
            "origin": 0,
            "error_reserved": 0,
            "error_code": empty,
            "message": empty,
        }),
        "{call:#}"
    );
}

/// An explicit library failure with its stable error code and a message.
fn assert_failure(call: &Value, code: u64, error_code: &str) {
    assert_eq!(call["code"], code, "{call:#}");
    assert_eq!(call["reserved"], 0, "{call:#}");
    assert_eq!(call["origin"], ORIGIN_LIBRARY, "{call:#}");
    assert_eq!(call["error_reserved"], 0, "{call:#}");
    assert_eq!(call["error_code"], error_code, "{call:#}");
    assert!(!call["message"].as_str().unwrap().is_empty(), "{call:#}");
}

/// Terminal evidence of a never-opened instance that was shut down: stopped,
/// no retry, Leave not required, nothing detached, every connection closed.
fn stopped(generation: u64) -> Value {
    let empty = json!({ "null": true, "length": 0 });
    json!({
        "generation": generation,
        "client_outcome": CLIENT_STOPPED,
        "retry": RETRY_NONE,
        "leave": LEAVE_NOT_REQUIRED,
        "attachment_detached": 0,
        "connection_closed": 1,
        "reserved": [0, 0],
        "error": { "origin": 0, "reserved": 0, "code": empty, "message": empty },
    })
}

/// Terminal evidence of a never-opened instance that failed with a contained
/// panic: failed with `internal_error`, retried as `retry` says.
fn assert_failed(terminal: &Value, generation: u64, retry: u64) {
    assert_eq!(terminal["generation"], generation, "{terminal:#}");
    assert_eq!(terminal["client_outcome"], CLIENT_FAILED, "{terminal:#}");
    assert_eq!(terminal["retry"], retry, "{terminal:#}");
    assert_eq!(terminal["leave"], LEAVE_NOT_REQUIRED, "{terminal:#}");
    assert_eq!(terminal["attachment_detached"], 0, "{terminal:#}");
    assert_eq!(terminal["connection_closed"], 1, "{terminal:#}");
    assert_eq!(terminal["reserved"], json!([0, 0]), "{terminal:#}");
    let error = &terminal["error"];
    assert_eq!(error["origin"], ORIGIN_LIBRARY, "{terminal:#}");
    assert_eq!(error["reserved"], 0, "{terminal:#}");
    assert_eq!(error["code"], "internal_error", "{terminal:#}");
    assert!(
        !error["message"].as_str().unwrap().is_empty(),
        "{terminal:#}"
    );
}

/// A drain's call and observation, from the host's `[call, observed]`.
fn drained(pair: &Value) -> &Value {
    assert_success(&pair[0]);
    &pair[1]
}

fn assert_empty(drained: &Value) {
    assert_eq!(
        drained,
        &json!({
            "kind": EMPTY,
            "reserved": 0,
            "delivery_null": true,
            "records_null": true,
            "count": 0,
            "terminal_null": true,
        }),
        "{drained:#}"
    );
}

/// Terminal evidence, alone: no delivery and no records.
fn terminal_of(drained: &Value) -> &Value {
    assert_eq!(drained["kind"], TERMINAL, "{drained:#}");
    assert_eq!(drained["reserved"], 0, "{drained:#}");
    assert_eq!(drained["delivery_null"], true, "{drained:#}");
    assert_eq!(drained["records_null"], true, "{drained:#}");
    assert_eq!(drained["count"], 0, "{drained:#}");
    assert_eq!(drained["terminal_null"], false, "{drained:#}");
    &drained["terminal"]
}

/// One delivery holding the failure's library diagnostic.
fn assert_diagnostic(drained: &Value, generation: u64) {
    assert_eq!(drained["kind"], DELIVERY, "{drained:#}");
    assert_eq!(drained["reserved"], 0, "{drained:#}");
    assert_eq!(drained["delivery_null"], false, "{drained:#}");
    assert_eq!(drained["terminal_null"], true, "{drained:#}");
    assert_eq!(drained["count"], 1, "{drained:#}");
    let record = &drained["records"][0];
    assert_eq!(record["tag"], DIAGNOSTIC, "{drained:#}");
    assert_eq!(record["reserved"], 0, "{drained:#}");
    assert_eq!(record["generation"], generation, "{drained:#}");
    let diagnostic = &record["diagnostic"];
    assert_eq!(diagnostic["origin"], ORIGIN_LIBRARY, "{drained:#}");
    assert_eq!(diagnostic["reserved"], 0, "{drained:#}");
    assert_eq!(diagnostic["code"], "internal_error", "{drained:#}");
    assert!(!diagnostic["message"].as_str().unwrap().is_empty());
}

/// The drains the host made until terminal evidence: each delivery and its
/// release, then the terminal evidence.
fn drains_to_terminal(drains: &Value) -> (Vec<&Value>, &Value) {
    let drains = drains.as_array().unwrap();
    let (last, earlier) = drains.split_last().unwrap();
    assert_success(&last["call"]);
    let mut deliveries = Vec::new();
    for step in earlier {
        if let Some(released) = step.get("release_delivery") {
            assert_success(released.as_array().unwrap().last().unwrap());
        } else {
            assert_success(&step["call"]);
            if step["drained"]["kind"] == DELIVERY {
                deliveries.push(&step["drained"]);
            } else {
                assert_empty(&step["drained"]);
            }
        }
    }
    (deliveries, terminal_of(&last["drained"]))
}

/// The ledger: every library object was released through its own library's
/// matching call, and none is outstanding.
fn assert_all_released(ledger: &Value, deliveries: usize) {
    assert_eq!(ledger["outstanding"], 0, "{ledger:#}");
    let objects = ledger["objects"].as_array().unwrap();
    for (kind, by) in [
        ("instance", "destroy"),
        ("communication", "release_communication"),
        ("control", "release_control"),
        ("delivery", "release_delivery"),
    ] {
        let released: Vec<_> = objects
            .iter()
            .filter(|object| object["kind"] == kind)
            .collect();
        let expected = if kind == "delivery" { deliveries } else { 1 };
        assert_eq!(released.len(), expected, "{kind}: {ledger:#}");
        assert!(
            released.iter().all(|object| object["released_by"] == by),
            "{ledger:#}"
        );
    }
}

/// LIB-03 Library (create returns all three handles; the caller's startup
/// memory is scrambled and unmapped at once), LIB-04 Library (destroy busy
/// while run or handles are outstanding; terminal evidence lives until
/// destroy; the descriptor stays borrowed and is closed by destroy; every
/// object is released through the library) and
/// LIB-05 Library (create, run and destroy on the owner thread, shutdown,
/// drain and handle release elsewhere, then join; terminal evidence and the
/// worker-cleanup report stay distinct), plus explicit rejection of run and
/// destroy on the wrong thread and of a repeated run: create, run, shut
/// down, drain to terminal evidence, release both handles, destroy, join.
#[test]
fn installed_library_instance_runs_create_run_shutdown_drain_release_destroy_join() {
    let package = Package::new();
    let root = root(&package);
    let report = scenario(
        &package,
        "lifecycle",
        &input(&root, json!({ "startup": { "generation": 7 } })),
        None,
    );
    let create = &report["create"];
    assert_success(&create["call"]);
    assert_eq!(create["output"], "modified");
    assert_eq!(create["effective_limits"], default_limits());
    assert_failure(&report["run_wrong_thread"][0], WRONG_THREAD, "wrong_thread");
    assert_eq!(report["run_wrong_thread"][1], json!({ "output": "zeroed" }));
    assert_failure(
        &report["destroy_wrong_thread"],
        WRONG_THREAD,
        "wrong_thread",
    );
    assert_failure(&report["destroy_before_run"], BUSY, "busy");
    assert_success(&report["notification"]);
    assert_eq!(report["readable_after_create"], false);
    assert_empty(drained(&report["drain_before_shutdown"]));
    assert_failure(
        &report["release_communication_before_shutdown"],
        BUSY,
        "busy",
    );
    assert_failure(&report["release_control_before_shutdown"], BUSY, "busy");
    assert_success(&report["shutdown"]);
    assert_success(&report["shutdown_again"]);
    assert_eq!(report["readable_after_shutdown"], true);
    let (deliveries, terminal) = drains_to_terminal(&report["drains"]);
    assert!(deliveries.is_empty());
    assert_eq!(terminal, &stopped(7));
    let repeated = drained(&report["drain_after_terminal"]);
    assert_eq!(terminal_of(repeated), &stopped(7));
    assert_eq!(report["readable_after_terminal"], true);
    let owner = &report["owner"];
    assert_success(&owner["run"][0]);
    let run = &owner["run"][1];
    assert_eq!(run["workers_cleanup"], CLEANUP_COMPLETE, "{run:#}");
    assert_eq!(run["reserved"], 0);
    assert_eq!(run["terminal"], stopped(7));
    assert_failure(&owner["run_again"][0], CLOSED, "closed");
    assert_eq!(owner["run_again"][1], json!({ "output": "zeroed" }));
    assert_failure(&owner["destroy_before_release"], BUSY, "busy");
    assert_success(&report["release_communication"]);
    assert_success(&report["release_control"]);
    // Terminal evidence lives until destroy, past both handle releases.
    assert_eq!(terminal_of(&report["terminal_before_destroy"]), &stopped(7));
    assert_success(&report["destroy"]);
    assert_eq!(report["descriptor"]["open"], true);
    assert_eq!(report["descriptor_after_destroy"]["open"], false);
    assert_eq!(report["joined"], true);
    assert_all_released(&report["ledger"], 0);
    let timeline = report["timeline"].as_array().unwrap();
    let calls = |thread: &str| -> Vec<(&str, u64)> {
        timeline
            .iter()
            .filter(|step| step["thread"] == thread)
            .map(|step| {
                (
                    step["call"].as_str().unwrap(),
                    step["code"].as_u64().unwrap(),
                )
            })
            .collect()
    };
    assert_eq!(
        calls("owner"),
        [
            ("create", SUCCESS),
            ("destroy", BUSY),
            ("run", SUCCESS),
            ("run", CLOSED),
            ("destroy", BUSY),
            ("destroy", SUCCESS),
        ]
    );
    assert_eq!(
        calls("main"),
        [
            ("run", WRONG_THREAD),
            ("destroy", WRONG_THREAD),
            ("notification", SUCCESS),
            ("snapshot", SUCCESS),
            ("drain", SUCCESS),
            ("release_communication", BUSY),
            ("release_control", BUSY),
            ("shutdown", SUCCESS),
            ("shutdown", SUCCESS),
            ("drain", SUCCESS),
            ("drain", SUCCESS),
            ("snapshot", SUCCESS),
            ("release_communication", SUCCESS),
            ("release_control", SUCCESS),
        ]
    );
    let sequence = |thread: &str, call: &str, code: u64| -> Vec<u64> {
        timeline
            .iter()
            .filter(|step| step["thread"] == thread && step["call"] == call && step["code"] == code)
            .map(|step| step["seq"].as_u64().unwrap())
            .collect()
    };
    // Only the host's own synchronization orders the two threads' records.
    let run = sequence("owner", "run", SUCCESS)[0];
    assert!(
        run < sequence("main", "snapshot", SUCCESS)[1],
        "joined to the run's return"
    );
    let destroyed = sequence("owner", "destroy", SUCCESS)[0];
    assert!(sequence("main", "release_control", SUCCESS)[0] < destroyed);
    assert!(sequence("main", "release_communication", SUCCESS)[0] < destroyed);
    package.assert_clean();
}

/// One create case per entry, each with its expected code and error code.
fn creates(package: &Package, root: &Path, cases: &[(&str, Value, u64, &str)]) -> Value {
    let cases_input: Vec<_> = cases.iter().map(|(_, case, _, _)| case.clone()).collect();
    let report = scenario(
        package,
        "create",
        &input(root, json!({ "cases": cases_input })),
        None,
    );
    for ((name, _, code, error_code), observed) in
        cases.iter().zip(report["cases"].as_array().unwrap())
    {
        if *code == SUCCESS {
            assert_success(&observed["call"]);
            let cleanup = &observed["cleanup"];
            for step in [
                "shutdown",
                "release_communication",
                "release_control",
                "destroy",
            ] {
                assert_success(&cleanup[step]);
            }
        } else {
            assert_failure(&observed["call"], *code, error_code);
            if *name != "null output" {
                assert_eq!(observed["output"], "zeroed", "{name}: {observed:#}");
            }
            assert!(observed.get("cleanup").is_none(), "{name}: no handles");
        }
        assert_eq!(
            observed["descriptors_unchanged"], true,
            "{name}: {observed:#}"
        );
        assert_eq!(observed["joined"], true, "{name}");
    }
    report
}

fn startup(overrides: Value) -> Value {
    json!({ "startup": overrides })
}

fn path(bytes: Value) -> Value {
    json!({ "bytes": bytes })
}

/// LIB-03 Library (a failed create returns no handles and leaves nothing
/// behind) and the mismatched-release rejection: null arguments, the
/// expected release, the generation and the native paths. A path keeps
/// non-UTF-8 bytes and may be `/` alone, but must be absolute, without NUL,
/// in the native Unix encoding and shorter than `PATH_MAX`.
#[test]
fn installed_library_create_rejects_invalid_identity_and_paths_without_handles() {
    let package = Package::new();
    let root = root(&package);
    let long = |length: u64| json!({ "prefix": "/", "fill": "a", "length": length });
    let mut cases = vec![
        (
            "null startup",
            json!({ "null": "startup" }),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "null output",
            json!({ "null": "output" }),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "another release",
            startup(json!({ "expected_release": "9.9.9" })),
            RELEASE_MISMATCH,
            "release_mismatch",
        ),
        (
            "empty release",
            startup(json!({ "expected_release": "" })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "non-UTF-8 release",
            startup(json!({ "expected_release": { "hex": "302e31ff" } })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "null release",
            startup(json!({ "expected_release": { "null": 5 } })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "zero generation",
            startup(json!({ "generation": 0 })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        ("valid", startup(json!({})), SUCCESS, ""),
    ];
    for field in ["metadata_path", "home_path", "runtime_path"] {
        for (case, value, code) in [
            (
                "relative",
                path(json!("instances/session")),
                INVALID_ARGUMENT,
            ),
            ("empty", path(json!("")), INVALID_ARGUMENT),
            (
                "dangling empty",
                path(json!({ "dangling": true })),
                INVALID_ARGUMENT,
            ),
            ("null", path(json!({ "null": 4 })), INVALID_ARGUMENT),
            (
                "NUL",
                path(json!({ "hex": "2f61002f62" })),
                INVALID_ARGUMENT,
            ),
            ("encoding", json!({ "encoding": 2 }), INVALID_ARGUMENT),
            ("reserved", json!({ "reserved": 1 }), INVALID_ARGUMENT),
            ("PATH_MAX bytes", path(long(PATH_MAX)), INVALID_ARGUMENT),
            ("PATH_MAX - 1 bytes", path(long(PATH_MAX - 1)), SUCCESS),
            ("root", path(json!("/")), SUCCESS),
            (
                "non-UTF-8",
                path(json!({ "hex": "2f636166c3a9ff" })),
                SUCCESS,
            ),
        ] {
            let name = Box::leak(format!("{field}: {case}").into_boxed_str());
            let error_code = if code == SUCCESS {
                ""
            } else {
                "invalid_argument"
            };
            cases.push((name, startup(json!({ field: value })), code, error_code));
        }
    }
    creates(&package, &root, &cases);
    package.assert_clean();
}

/// LIB-03 Library: client keys and diagnostics are validated before
/// anything is created. A key is enabled with a control byte or disabled
/// with zero; take-control is never ESC and never the escape key. The log
/// filter must be nonempty CLIMON2_LOG grammar even without a log file; a
/// log path that cannot be created as a new file rejects the create and
/// leaves no descriptor behind.
#[test]
fn installed_library_create_rejects_invalid_settings_and_diagnostics_without_handles() {
    let package = Package::new();
    let root = root(&package);
    let taken = root.join("logs/taken.log");
    fs::write(&taken, "").unwrap();
    let settings = |fields: Value| startup(json!({ "settings": fields }));
    let log = |fields: Value| startup(json!({ "diagnostics": { "log_path": fields } }));
    let filter = |value: Value| startup(json!({ "diagnostics": { "filter": value } }));
    let cases = [
        (
            "both keys disabled",
            settings(
                json!({ "escape_enabled": 0, "escape_byte": 0, "take_control_enabled": 0, "take_control_byte": 0 }),
            ),
            SUCCESS,
            "",
        ),
        (
            "escape flag 2",
            settings(json!({ "escape_enabled": 2 })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "take-control flag 2",
            settings(json!({ "take_control_enabled": 2 })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "disabled escape with a byte",
            settings(json!({ "escape_enabled": 0, "escape_byte": 0x1d })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "disabled take-control with a byte",
            settings(json!({ "take_control_enabled": 0 })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "escape beyond C0",
            settings(json!({ "escape_byte": 0x20 })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "take-control ESC",
            settings(json!({ "take_control_byte": 27 })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "same key twice",
            settings(json!({ "take_control_byte": 0x1d })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "settings reserved",
            settings(json!({ "reserved": 1 })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "log presence 2",
            log(json!({ "present": 2 })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "log reserved",
            log(json!({ "reserved": 1 })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "relative log path",
            log(json!({ "present": 1, "value": { "bytes": "logs/a.log" } })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "existing log file",
            log(json!({ "present": 1, "value": { "bytes": taken.to_str().unwrap() } })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "missing log directory",
            log(
                json!({ "present": 1, "value": { "bytes": root.join("missing/a.log").to_str().unwrap() } }),
            ),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "new log file",
            log(
                json!({ "present": 1, "value": { "bytes": root.join("logs/new.log").to_str().unwrap() } }),
            ),
            SUCCESS,
            "",
        ),
        (
            "empty filter",
            filter(json!("")),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "dangling empty filter",
            filter(json!({ "dangling": true })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "null filter",
            filter(json!({ "null": 4 })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "non-UTF-8 filter",
            filter(json!({ "hex": "696e666fff" })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "invalid filter",
            filter(json!("climon2=loud")),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "one-directive filter",
            filter(json!("climon2_client=trace,warn")),
            SUCCESS,
            "",
        ),
    ];
    creates(&package, &root, &cases);
    assert_eq!(
        fs::read(&taken).unwrap(),
        b"",
        "an existing file is never written"
    );
    package.assert_clean();
}

/// LIB-03 Library: limits are effective values the release can honour, or
/// `CM1_UNSUPPORTED_LIMIT`, never silently raised or lowered: every limit
/// but `max_workers` must be nonzero, `client_bytes` must cover the
/// described instance allocation and `max_workers` the described workers,
/// each exactly at its bound. With no descriptor left, create fails with
/// `CM1_RESOURCE_LIMIT` and no handles.
#[test]
fn installed_library_create_rejects_limits_it_cannot_honour_and_exhausted_descriptors() {
    let package = Package::new();
    let root = root(&package);
    let limits = |fields: Value| startup(json!({ "limits": fields }));
    let mut cases = vec![
        (
            "limits reserved",
            limits(json!({ "reserved": 1 })),
            INVALID_ARGUMENT,
            "invalid_argument",
        ),
        (
            "client_bytes below the instance",
            limits(json!({ "client_bytes": INSTANCE_BYTES - 1 })),
            UNSUPPORTED_LIMIT,
            "unsupported_limit",
        ),
        (
            "client_bytes at the instance",
            limits(json!({ "client_bytes": INSTANCE_BYTES })),
            SUCCESS,
            "",
        ),
        (
            "max_workers below the workers",
            limits(json!({ "max_workers": WORKERS - 1 })),
            UNSUPPORTED_LIMIT,
            "unsupported_limit",
        ),
        (
            "max_workers zero",
            limits(json!({ "max_workers": 0 })),
            UNSUPPORTED_LIMIT,
            "unsupported_limit",
        ),
        (
            "every limit at one",
            limits(json!({
                "request_bytes": 1, "event_bytes": 1, "progress_bytes": 1, "drain_payload_bytes": 1,
                "request_records": 1, "event_records": 1, "progress_records": 1, "deliveries": 1,
                "operations": 1, "chunk_bytes": 1, "drain_records": 1, "wire_frame_bytes": 1,
                "metadata_record_bytes": 1, "title_bytes": 1, "notification_title_bytes": 1,
                "notification_body_bytes": 1, "operation_ms": 1, "kill_ms": 1,
                "restoration_stall_ms": 1, "restoration_total_ms": 1, "delivery_stall_ms": 1,
                "leave_ms": 1, "cleanup_ms": 1, "start_ms": 1,
            })),
            SUCCESS,
            "",
        ),
        (
            "exhausted descriptors",
            json!({ "exhausted": true }),
            RESOURCE_LIMIT,
            "resource_limit",
        ),
    ];
    for field in default_limits().as_object().unwrap().keys() {
        if field != "max_workers" && field != "reserved" {
            let name = Box::leak(format!("zero {field}").into_boxed_str());
            cases.push((
                name,
                limits(json!({ field: 0 })),
                UNSUPPORTED_LIMIT,
                "unsupported_limit",
            ));
        }
    }
    let report = creates(&package, &root, &cases);
    assert_eq!(report["cases"].as_array().unwrap().len(), 7 + 25);
    package.assert_clean();
}

/// Instance `a`'s log file, at a non-UTF-8 path on Linux. macOS file systems
/// refuse names that are not UTF-8 (`EILSEQ`), so there it keeps non-ASCII
/// UTF-8; `a`'s home keeps an invalid byte on both systems.
#[cfg(target_os = "linux")]
const A_LOG: &[u8] = b"logs/a-caf\xc3\xa9-\xff.log";
#[cfg(target_os = "macos")]
const A_LOG: &[u8] = b"logs/a-caf\xc3\xa9.log";

/// LIB-03 Library and Fault (caller startup memory scrambled and unmapped
/// after create; native paths copied losslessly; independent homes,
/// settings, diagnostics and generations; instance-scoped, content-free
/// logs at a non-ASCII log path (not UTF-8 on Linux), filtered by each
/// instance's own grammar; allocation counts each instance's own copies).
/// Instance `a` ends while `b` keeps running, then `b` ends.
#[test]
fn installed_library_instances_keep_their_own_copies_paths_settings_and_logs() {
    let package = Package::new();
    let root = root(&package);
    let base = root.to_str().unwrap();
    let a_home = [base.as_bytes(), b"/caf\xc3\xa9/\xff"].concat();
    let b_home = format!("{base}/b-home-longer");
    let a_log = root.join(OsStr::from_bytes(A_LOG));
    let b_log = root.join("logs/b.log");
    let a = json!({
        "generation": 7,
        "home_path": path(json!({ "hex": hex(&a_home) })),
        "metadata_path": path(json!(format!("{base}/a/session.pty"))),
        "runtime_path": path(json!(format!("{base}/a/runtime"))),
        "settings": { "escape_byte": 0x1d, "take_control_byte": 0x14 },
        "diagnostics": {
            "log_path": { "present": 1, "value": path(json!({ "hex": hex(a_log.as_os_str().as_bytes()) })) },
            "filter": "trace",
        },
    });
    let b = json!({
        "generation": 9,
        "home_path": path(json!(b_home)),
        "metadata_path": path(json!(format!("{base}/b/session.pty"))),
        "runtime_path": path(json!(format!("{base}/b/runtime"))),
        "settings": { "escape_enabled": 0, "escape_byte": 0, "take_control_byte": 0x01 },
        "diagnostics": {
            "log_path": { "present": 1, "value": path(json!(b_log.to_str().unwrap())) },
            "filter": "info",
        },
    });
    let report = scenario(
        &package,
        "independence",
        &input(&root, json!({ "a": a, "b": b })),
        None,
    );
    assert_success(&report["create_a"]["call"]);
    assert_success(&report["create_b"]["call"]);
    let (snapshot_a, snapshot_b) = (&report["snapshot_a"][1], &report["snapshot_b"][1]);
    for snapshot in [snapshot_a, snapshot_b] {
        // The notification descriptors and the log file.
        assert_eq!(
            snapshot["descriptors"],
            NOTIFIER_DESCRIPTORS + 1,
            "{snapshot}"
        );
        assert_eq!(
            snapshot["peak_descriptors"],
            NOTIFIER_DESCRIPTORS + 1,
            "{snapshot}"
        );
    }
    let allocation = |snapshot: &Value| snapshot["allocation_bytes"].as_u64().unwrap();
    // Only the homes differ in length; their copies are charged byte for byte.
    assert_eq!(
        allocation(snapshot_b) - allocation(snapshot_a),
        (b_home.len() - a_home.len()) as u64,
        "{snapshot_a} {snapshot_b}"
    );
    let (_, terminal_a) = drains_to_terminal(&report["drains_a"]);
    assert_eq!(terminal_a, &stopped(7));
    assert_success(&report["run_a"][0]);
    assert_empty(drained(&report["drain_b_while_a_ended"]));
    assert_success(&report["snapshot_b_while_a_ended"][0]);
    let (_, terminal_b) = drains_to_terminal(&report["drains_b"]);
    assert_eq!(terminal_b, &stopped(9));
    assert_success(&report["run_b"][0]);
    for instance in ["release_a", "release_b"] {
        for step in ["release_communication", "release_control", "destroy"] {
            assert_success(&report[instance][step]);
        }
    }
    assert_eq!(report["joined"], json!([true, true]));
    assert_all_released_by_kind(&report["ledger"], 2);
    let a_lines = log_lines(&a_log);
    let b_lines = log_lines(&b_log);
    let home = format!("home=\"{base}/caf\u{e9}/\\xFF\"");
    let running = a_lines
        .iter()
        .find(|line| line.contains("client instance running"));
    let running = running.unwrap_or_else(|| panic!("{a_lines:#?}"));
    assert!(running.contains(" DEBUG "), "{running}");
    assert!(running.contains(&home), "{running}");
    assert!(
        running.contains(&format!("metadata=\"{base}/a/session.pty\"")),
        "{running}"
    );
    assert!(
        running.contains(&format!("runtime=\"{base}/a/runtime\"")),
        "{running}"
    );
    assert!(
        running.contains("escape=Some(29) take_control=Some(20)"),
        "{running}"
    );
    assert!(
        b_lines.iter().all(|line| !line.contains(" DEBUG ")),
        "{b_lines:#?}"
    );
    assert!(
        b_lines.iter().any(|line| line.contains(" INFO ")),
        "{b_lines:#?}"
    );
    assert!(
        a_lines.iter().all(|line| line.contains("generation=7")),
        "{a_lines:#?}"
    );
    assert!(
        b_lines.iter().all(|line| line.contains("generation=9")),
        "{b_lines:#?}"
    );
    package.assert_clean();
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Like [`assert_all_released`], for `instances` instances without deliveries.
fn assert_all_released_by_kind(ledger: &Value, instances: usize) {
    assert_eq!(ledger["outstanding"], 0, "{ledger:#}");
    let objects = ledger["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 3 * instances, "{ledger:#}");
}

/// Every record of an instance log, each a lifecycle fact and never content.
fn log_lines(path: &Path) -> Vec<String> {
    let text = fs::read_to_string(path).unwrap();
    let messages = [
        "client instance created",
        "client instance running",
        "client instance shutdown requested",
        "client instance terminal evidence published",
        "client instance worker cleanup complete",
        "client instance destroyed",
        "client library contained a panic",
    ];
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    assert!(!lines.is_empty(), "{path:?}");
    for line in &lines {
        assert!(
            messages.iter().any(|message| line.contains(message)),
            "{line}"
        );
    }
    lines
}

/// The `fault` scenario with `CLIMON2_LIBRARY_FAULT=panic:<calls>` and
/// generation 5, or `None` in the no-hooks release archive.
fn fault(package: &Package, calls: &str, limits: Value) -> Option<Value> {
    if archive() {
        return None;
    }
    let root = root(package);
    let input = input(
        &root,
        json!({ "startup": { "generation": 5, "limits": limits } }),
    );
    Some(scenario(
        package,
        "fault",
        &input,
        Some(&format!("panic:{calls}")),
    ))
}

/// A call's attempts: the injected panic's explicit failure, then the retry.
fn twice(attempts: &Value) -> (&Value, &Value) {
    let attempts = attempts.as_array().unwrap();
    assert_eq!(attempts.len(), 2, "{attempts:#?}");
    (&attempts[0], &attempts[1])
}

fn once(attempts: &Value) -> &Value {
    let attempts = attempts.as_array().unwrap();
    assert_eq!(attempts.len(), 1, "{attempts:#?}");
    &attempts[0]
}

/// The instance ended without a failure: its run completed, its stream holds
/// only stopped terminal evidence, and everything was released.
fn assert_unaffected(report: &Value) {
    let (deliveries, terminal) = drains_to_terminal(&report["drains"]);
    assert!(deliveries.is_empty(), "{report:#}");
    assert_eq!(terminal, &stopped(5));
    assert_success(&report["owner"]["run"][0]);
    assert_eq!(report["owner"]["run"][1]["terminal"], stopped(5));
    assert_eq!(report["joined"], true);
    assert_all_released(&report["ledger"], 0);
}

/// LIB-05 Fault: a panic injected at the bootstrap, at `create` or at
/// `submit` returns an explicit `CM1_INTERNAL_ERROR` without unwinding (the
/// host survives, nothing reaches its standard error), with cleared output
/// and no handles; the retry succeeds, and no instance is affected.
#[test]
fn installed_library_contains_a_panic_at_the_bootstrap_create_or_submit() {
    let package = Package::new();
    let Some(report) = fault(&package, "bootstrap", json!({})) else {
        return;
    };
    assert_eq!(
        report["bootstrap"],
        json!([
            { "status": INTERNAL_ERROR, "output": "null" },
            { "status": SUCCESS, "output": "set" },
        ])
    );
    assert_unaffected(&report);
    let report = fault(&package, "create", json!({})).unwrap();
    let (failed, retried) = twice(&report["create"]);
    assert_failure(&failed["call"], INTERNAL_ERROR, "internal_error");
    assert_eq!(failed["output"], "zeroed");
    assert_success(&retried["call"]);
    assert_unaffected(&report);
    let report = fault(&package, "submit", json!({})).unwrap();
    let (failed, retried) = twice(&report["submit"]);
    assert_failure(failed, INTERNAL_ERROR, "internal_error");
    assert_failure(retried, UNSUPPORTED_OPERATION, "unsupported_operation");
    assert_unaffected(&report);
    package.assert_clean();
}

/// LIB-05 Fault and LIB-04 Fault: a panic injected while an instance is in
/// use (at `notification`, `snapshot`, `drain`, `shutdown` or `run`)
/// returns an explicit `CM1_INTERNAL_ERROR` with cleared output (run still
/// reports its cleanup and terminal evidence) and stops the instance: a
/// library diagnostic is queued and raises the descriptor until drained, and
/// its delivery stays unchanged until released;
/// destroy and handle release stay busy while that delivery is outstanding;
/// terminal evidence follows it, failed with `internal_error`, retried
/// passively, or not at all when the panic is in `shutdown` or follows it
/// (the fault at `run` follows `shutdown`); the instance is still cleaned up
/// and joined.
#[test]
fn installed_library_contains_a_panic_in_a_running_instance_and_stops_it() {
    let package = Package::new();
    for call in ["notification", "snapshot", "drain", "shutdown", "run"] {
        let Some(report) = fault(&package, call, json!({})) else {
            return;
        };
        let early = ["notification", "snapshot", "drain"].contains(&call);
        if call == "notification" {
            let (failed, retried) = twice(&report["notification"]);
            assert_failure(&failed[0], INTERNAL_ERROR, "internal_error");
            assert_eq!(failed[1], -1, "a failed lookup writes -1");
            assert_success(&retried[0]);
        }
        if call == "snapshot" {
            let (failed, retried) = twice(&report["snapshot"]);
            assert_failure(&failed[0], INTERNAL_ERROR, "internal_error");
            assert_eq!(failed[1], json!({ "output": "zeroed" }));
            assert_success(&retried[0]);
            assert_eq!(retried[1]["event_records"], 1, "the diagnostic is queued");
        }
        if call == "drain" {
            let (failed, _) = twice(&report["drain"]);
            assert_failure(&failed[0], INTERNAL_ERROR, "internal_error");
            assert_eq!(failed[1], json!({ "output": "zeroed" }));
        }
        let failed_before_drain = call == "notification" || call == "snapshot";
        assert_eq!(
            report["readable_before_drain"], failed_before_drain,
            "{call}"
        );
        let drained_early = report["drain"].as_array().unwrap().last().unwrap();
        if early {
            assert_diagnostic(drained(drained_early), 5);
            // The delivery's records live, unchanged, until its release.
            assert_eq!(&report["delivery_before_release"], &drained_early[1]);
            assert_failure(&report["destroy_with_delivery"], BUSY, "busy");
            assert_failure(&report["release_communication_with_delivery"], BUSY, "busy");
            assert_success(once(&report["release_delivery"]));
            assert_eq!(
                report["readable_before_run"], false,
                "{call}: cleared once drained"
            );
        } else {
            assert_empty(drained(drained_early));
        }
        let (deliveries, terminal) = drains_to_terminal(&report["drains"]);
        assert_eq!(deliveries.len(), usize::from(!early), "{call}: {report:#}");
        for delivery in deliveries {
            assert_diagnostic(delivery, 5);
        }
        let retry = if call == "run" || call == "shutdown" {
            RETRY_NONE
        } else {
            RETRY_PASSIVE
        };
        assert_failed(terminal, 5, retry);
        let owner = &report["owner"];
        if call == "run" {
            assert_failure(&owner["run"][0], INTERNAL_ERROR, "internal_error");
        } else {
            assert_success(&owner["run"][0]);
        }
        assert_eq!(owner["run"][1]["workers_cleanup"], CLEANUP_COMPLETE);
        assert_eq!(&owner["run"][1]["terminal"], terminal);
        assert_failure(&owner["run_again"][0], CLOSED, "closed");
        if call == "shutdown" {
            let (failed, retried) = twice(&report["shutdown"]);
            assert_failure(failed, INTERNAL_ERROR, "internal_error");
            assert_success(retried);
        }
        let destroy = once(&report["destroy"]);
        assert_success(destroy);
        assert_eq!(report["joined"], true);
        assert_all_released(&report["ledger"], 1);
    }
    package.assert_clean();
}

/// LIB-04 Fault and LIB-05 Fault: a panic injected while releasing a
/// delivery, a handle, or the instance fails that call explicitly and leaves
/// the object valid and owned: the retry releases it. After terminal
/// evidence the panic changes nothing else: no new record appears and the
/// evidence is unchanged.
#[test]
fn installed_library_contains_a_panic_in_a_release_and_keeps_the_object() {
    let package = Package::new();
    let Some(report) = fault(&package, "snapshot,release_delivery", json!({})) else {
        return;
    };
    let (failed, retried) = twice(&report["release_delivery"]);
    assert_failure(failed, INTERNAL_ERROR, "internal_error");
    assert_success(retried);
    assert_all_released(&report["ledger"], 1);
    for call in ["release_communication", "release_control", "destroy"] {
        let report = fault(&package, call, json!({})).unwrap();
        let (failed, retried) = twice(&report[call]);
        assert_failure(failed, INTERNAL_ERROR, "internal_error");
        assert_success(retried);
        assert_unaffected(&report);
        let after = drained(&report["drain_after_release_control"]);
        assert_eq!(terminal_of(after), &stopped(5), "{call}");
    }
    package.assert_clean();
}

/// LIB-04 Fault: a failure diagnostic is best effort. It is queued only when
/// one record fits the instance's `event_bytes` (exactly at the limit), and
/// otherwise dropped while terminal evidence keeps the error (one byte
/// under).
#[test]
fn installed_library_queues_the_failure_diagnostic_only_within_event_bytes() {
    let package = Package::new();
    for (event_bytes, queued) in [(RECORD, true), (RECORD - 1, false)] {
        let limits = json!({ "event_bytes": event_bytes, "progress_bytes": 1 });
        let Some(report) = fault(&package, "snapshot", limits) else {
            return;
        };
        let drained_early = drained(report["drain"].as_array().unwrap().last().unwrap());
        if queued {
            assert_diagnostic(drained_early, 5);
        } else {
            assert_empty(drained_early);
        }
        let (deliveries, terminal) = drains_to_terminal(&report["drains"]);
        assert!(deliveries.is_empty());
        assert_failed(terminal, 5, RETRY_PASSIVE);
    }
    package.assert_clean();
}

/// LIB-04 Fault (allocator pairing through the owning library): a drained
/// delivery stays charged, by record, bytes, delivery and allocation, until
/// released through the library; the release returns every measurement to
/// its earlier value and only the peaks remember it.
#[test]
fn installed_library_charges_a_delivery_until_it_is_released() {
    let package = Package::new();
    let Some(report) = fault(&package, "drain", json!({})) else {
        return;
    };
    let before = &once(&report["snapshot"])[1];
    let during = &report["snapshot_with_delivery"][1];
    let after = &report["snapshot_after_run"][1];
    assert_eq!(before["event_records"], 0);
    assert_eq!(before["deliveries"], 0);
    assert_eq!(during["event_records"], 1, "{during}");
    assert_eq!(during["event_bytes"], RECORD, "{during}");
    assert_eq!(during["deliveries"], 1, "{during}");
    assert_eq!(during["records_drained"], 1, "{during}");
    assert_eq!(during["deliveries_released"], 0, "{during}");
    let allocation = |metrics: &Value| metrics["allocation_bytes"].as_u64().unwrap();
    assert!(allocation(during) > allocation(before), "{before} {during}");
    assert_eq!(allocation(after), allocation(before), "{before} {after}");
    assert_eq!(after["peak_allocation_bytes"], during["allocation_bytes"]);
    for (current, peak, value) in [
        ("event_records", "peak_event_records", 1),
        ("event_bytes", "peak_event_bytes", RECORD),
        ("deliveries", "peak_deliveries", 1),
    ] {
        assert_eq!(after[current], 0, "{current}: {after}");
        assert_eq!(after[peak], value, "{peak}: {after}");
    }
    assert_eq!(after["records_drained"], 1);
    assert_eq!(after["deliveries_released"], 1);
    for unused in [
        "release_allocation_bytes",
        "peak_release_allocation_bytes",
        "request_bytes",
        "peak_request_bytes",
        "input_bytes_accepted",
        "display_bytes_delivered",
        "request_records",
        "peak_request_records",
        "operations",
        "peak_operations",
        "workers",
        "peak_workers",
    ] {
        assert_eq!(after[unused], 0, "{unused}: {after}");
    }
    assert_eq!(after["descriptors"], NOTIFIER_DESCRIPTORS);
    assert_eq!(after["peak_descriptors"], NOTIFIER_DESCRIPTORS);
    package.assert_clean();
}

/// The `workers` scenario with `fault`, whose worker cleanup may take up to
/// `cleanup_ms`.
fn workers(package: &Package, fault: &str, cleanup_ms: u64) -> Option<Value> {
    if archive() {
        return None;
    }
    let root = root(package);
    let limits = json!({ "limits": { "cleanup_ms": cleanup_ms } });
    let input = input(&root, json!({ "startup": limits }));
    Some(scenario(package, "workers", &input, Some(fault)))
}

/// LIB-05 Library and Fault: a running worker is counted; terminal evidence
/// does not claim it finished; `run` reports its completion separately, as
/// complete; destroy waits for the handles and succeeds; the owner thread is
/// joined.
#[test]
fn installed_library_reports_worker_completion_apart_from_terminal_evidence() {
    let package = Package::new();
    let Some(report) = workers(&package, "worker", 10_000) else {
        return;
    };
    let running = &report["snapshot_running"][1];
    assert_eq!(running["workers"], 1, "{running}");
    assert_eq!(running["peak_workers"], 1, "{running}");
    assert_success(&report["shutdown"]);
    let (_, terminal) = drains_to_terminal(&report["drains"]);
    assert_eq!(terminal, &stopped(1));
    let owner = &report["owner"];
    assert_success(&owner["run"][0]);
    assert_eq!(owner["run"][1]["workers_cleanup"], CLEANUP_COMPLETE);
    assert_eq!(&owner["run"][1]["terminal"], terminal);
    assert_failure(&owner["destroy_after_run"], BUSY, "busy");
    let after = &report["snapshot_after_run"][1];
    assert_eq!(after["workers"], 0, "{after}");
    assert_eq!(after["peak_workers"], 1, "{after}");
    assert_success(&report["destroy"]);
    assert_eq!(report["instance_destroyed"], true);
    assert_eq!(report["joined"], true);
    assert_all_released(&report["ledger"], 0);
    package.assert_clean();
}

/// LIB-06 Library: a worker whose cleanup stalls is reported as incomplete,
/// never complete: `run` returns `CM1_TIMEOUT` once `cleanup_ms` passes,
/// with the stalled worker still counted, while terminal evidence was
/// published regardless. Handles can be released, but the instance can never
/// be destroyed; the host never claims the worker joined, and ends with it
/// (whole-process fail-stop).
#[test]
fn installed_library_reports_stalled_worker_cleanup_as_incomplete_and_never_destroys() {
    let package = Package::new();
    let Some(report) = workers(&package, "stall-cleanup", 200) else {
        return;
    };
    assert_eq!(report["snapshot_running"][1]["workers"], 1);
    let (_, terminal) = drains_to_terminal(&report["drains"]);
    assert_eq!(terminal, &stopped(1));
    let owner = &report["owner"];
    assert_failure(&owner["run"][0], TIMEOUT, "timeout");
    assert_eq!(owner["run"][1]["workers_cleanup"], CLEANUP_INCOMPLETE);
    assert_eq!(&owner["run"][1]["terminal"], terminal);
    assert!(owner["elapsed_ms"].as_u64().unwrap() >= 200, "{owner}");
    assert_failure(&owner["destroy_after_run"], BUSY, "busy");
    assert_eq!(report["snapshot_after_run"][1]["workers"], 1);
    assert_success(&report["release_communication"]);
    assert_success(&report["release_control"]);
    assert_failure(&report["destroy"], BUSY, "busy");
    assert_eq!(report["instance_destroyed"], false);
    assert_eq!(
        report["ledger"]["outstanding"], 1,
        "the instance stays owned"
    );
    package.assert_clean();
}

/// LIB-03 Library and Fault: a complete lifecycle, with a contained panic
/// in the hooked library, changes none of the process's environment,
/// working directory, signal dispositions, signal mask or standard streams.
#[test]
fn installed_library_leaves_environment_cwd_signals_and_streams_unchanged() {
    let package = Package::new();
    let root = root(&package);
    let log = root.join("logs/ambient.log");
    let startup = json!({ "diagnostics": {
        "log_path": { "present": 1, "value": path(json!(log.to_str().unwrap())) },
        "filter": "trace",
    } });
    let fault = (!archive()).then_some("panic:snapshot");
    let report = scenario(
        &package,
        "ambient",
        &input(&root, json!({ "startup": startup })),
        fault,
    );
    assert_success(&report["create"]["call"]);
    assert_eq!(
        report["snapshot"][0]["code"],
        if archive() { SUCCESS } else { INTERNAL_ERROR }
    );
    assert_success(&report["cleanup"]["destroy"]);
    assert_eq!(report["joined"], true);
    assert_eq!(report["before"], report["after"]);
    assert!(
        report["before"]["streams"]
            .as_array()
            .unwrap()
            .iter()
            .all(|stream| stream["status"] == 0)
    );
    assert!(!log_lines(&log).is_empty());
    package.assert_clean();
}

/// LIB-04 Library and Fault, and the Linux `eventfd`: the notification
/// descriptor is borrowed (the same descriptor on every call, open until
/// destroy closes it), close-on-exec, counted once in the instance's
/// descriptors (on Linux one nonblocking `eventfd`), not readable until
/// terminal evidence, and readable from then on until the library clears it.
/// A thread polling it sees the stream end while another uses the
/// communication handle throughout; every call succeeds.
#[test]
fn installed_library_notification_descriptor_is_borrowed_and_safe_to_poll_racing_the_end() {
    let package = Package::new();
    let root = root(&package);
    let report = scenario(
        &package,
        "race",
        &input(&root, json!({ "startup": {} })),
        None,
    );
    assert_success(&report["create"]["call"]);
    let notifications = &report["notifications"];
    assert_success(&notifications[0]);
    assert_success(&notifications[1]);
    assert_eq!(notifications[2], true, "the same descriptor on every call");
    let descriptor = &report["descriptor"];
    assert_eq!(descriptor["open"], true);
    assert_eq!(descriptor["close_on_exec"], true);
    #[cfg(target_os = "linux")]
    {
        assert_eq!(descriptor["nonblocking"], true);
        assert_eq!(descriptor["kind"]["link"], "anon_inode:[eventfd]");
        assert_eq!(descriptor["kind"]["eventfd_count"], "0");
    }
    #[cfg(target_os = "macos")]
    assert_eq!(descriptor["kind"]["fifo"], true);
    let snapshot = &report["snapshot"][1];
    assert_eq!(snapshot["descriptors"], NOTIFIER_DESCRIPTORS);
    assert_eq!(snapshot["peak_descriptors"], NOTIFIER_DESCRIPTORS);
    assert_eq!(report["readable_after_create"], false);
    assert_success(&report["shutdown"]);
    assert_eq!(report["poller_saw_end"], true);
    let racer = &report["racer"];
    assert_eq!(racer["failures"], 0, "{racer}");
    assert!(racer["calls"].as_u64().unwrap() > 0, "{racer}");
    assert_eq!(racer["descriptors"].as_array().unwrap().len(), 1, "{racer}");
    assert_eq!(report["readable_twice"], json!([true, true]));
    let (_, terminal) = drains_to_terminal(&report["drains"]);
    assert_eq!(terminal, &stopped(1));
    assert_eq!(report["readable_after_terminal"], true);
    assert_success(&report["run"][0]);
    assert_success(&report["release_communication"]);
    assert_success(&report["release_control"]);
    assert_eq!(report["descriptor_before_destroy"]["open"], true);
    assert_success(&report["destroy"]);
    assert_eq!(report["descriptor_after_destroy"]["open"], false);
    assert_eq!(report["joined"], true);
    assert_all_released(&report["ledger"], 0);
    package.assert_clean();
}
