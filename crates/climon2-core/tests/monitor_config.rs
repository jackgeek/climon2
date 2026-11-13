//! Spec #73 Configuration model: the monitor's one startup snapshot of
//! `config.jsonc` (CFG-01 and INP-01, Contract layer), the `monitor` section
//! `exec` validates too, and the README that documents it (CFG-01 Docs).
//! Every expectation is written here from docs/monitor-operational-limits.md,
//! never taken from the code under test.
use climon2_core::{
    config::{AttachConfig, ConfigError, ConfigSource, ExecConfig, MonitorConfig},
    keys::ControlKey,
    layout::HomeLayout,
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

/// A setting's accepted range and the native type it converts to.
#[derive(Clone, Copy)]
struct Kind {
    min: u64,
    max: u64,
    native: &'static str,
}

/// Byte budgets and ceilings held in `uint64_t`, positive.
const BYTES64: Kind = Kind {
    min: 1,
    max: u64::MAX,
    native: "u64",
};
/// Counts and single-item byte ceilings held in `uint32_t`, positive.
const COUNT32: Kind = Kind {
    min: 1,
    max: u32::MAX as u64,
    native: "u32",
};
/// `resources.max_workers_per_client`, the one count that may be zero.
const WORKERS: Kind = Kind {
    min: 0,
    max: u32::MAX as u64,
    native: "u32",
};
const MILLIS: Kind = Kind {
    min: 1,
    max: 3_600_000,
    native: "u32",
};
const QUANTUM: Kind = Kind {
    min: 1,
    max: 50,
    native: "u32",
};
const FPS: Kind = Kind {
    min: 1,
    max: 120,
    native: "u32",
};
const JITTER: Kind = Kind {
    min: 0,
    max: 50,
    native: "u32",
};
const ROWS: Kind = Kind {
    min: 200,
    max: 100_000,
    native: "u32",
};

/// Every `monitor.limits` setting, in its group, with its range.
const FIELDS: [(&str, Kind); 62] = [
    ("memory.total_bytes", BYTES64),
    ("memory.client_bytes", BYTES64),
    ("memory.history_bytes_per_session", BYTES64),
    ("memory.history_bytes_total", BYTES64),
    ("memory.queue_bytes_total", BYTES64),
    ("history.max_rows", ROWS),
    ("resources.stack_bytes", BYTES64),
    ("resources.max_workers_per_client", WORKERS),
    ("resources.descriptors_per_client", COUNT32),
    ("resources.descriptors_per_load", COUNT32),
    ("resources.descriptor_headroom", COUNT32),
    ("resources.release_bytes", BYTES64),
    ("queues.request_bytes", BYTES64),
    ("queues.request_records", COUNT32),
    ("queues.event_bytes", BYTES64),
    ("queues.event_records", COUNT32),
    ("queues.progress_bytes", BYTES64),
    ("queues.progress_records", COUNT32),
    ("queues.deliveries", COUNT32),
    ("queues.operations", COUNT32),
    ("transfer.chunk_bytes", COUNT32),
    ("transfer.drain_records", COUNT32),
    ("transfer.drain_payload_bytes", BYTES64),
    ("payload.wire_frame_bytes", COUNT32),
    ("payload.metadata_record_bytes", COUNT32),
    ("payload.title_bytes", COUNT32),
    ("payload.notification_title_bytes", COUNT32),
    ("payload.notification_body_bytes", COUNT32),
    ("payload.assembly_records", COUNT32),
    ("payload.assembly_bytes", BYTES64),
    ("scheduling.probes", COUNT32),
    ("scheduling.loads", COUNT32),
    ("scheduling.starts", COUNT32),
    ("scheduling.newer_start_burst", COUNT32),
    ("discovery.pass_ms", MILLIS),
    ("discovery.stall_ms", MILLIS),
    ("discovery.batch_records", COUNT32),
    ("discovery.batch_bytes", BYTES64),
    ("discovery.metadata_file_bytes", BYTES64),
    ("discovery.metadata_depth", COUNT32),
    ("deadlines.load_ms", MILLIS),
    ("deadlines.start_ms", MILLIS),
    ("deadlines.operation_ms", MILLIS),
    ("deadlines.kill_ms", MILLIS),
    ("deadlines.restoration_stall_ms", MILLIS),
    ("deadlines.restoration_total_ms", MILLIS),
    ("deadlines.delivery_stall_ms", MILLIS),
    ("deadlines.leave_ms", MILLIS),
    ("deadlines.cleanup_ms", MILLIS),
    ("deadlines.shutdown_ms", MILLIS),
    ("deadlines.final_host_ms", MILLIS),
    ("deadlines.assembly_ms", MILLIS),
    ("retry.initial_ms", MILLIS),
    ("retry.cap_ms", MILLIS),
    ("retry.jitter_percent", JITTER),
    ("retry.admission_ms", MILLIS),
    ("ui.fps", FPS),
    ("ui.client_quantum_ms", QUANTUM),
    ("ui.background_quantum_ms", QUANTUM),
    ("ui.toast_records", COUNT32),
    ("ui.toast_bytes", BYTES64),
    ("ui.search_bytes", BYTES64),
];

const GROUPS: [&str; 11] = [
    "memory",
    "history",
    "resources",
    "queues",
    "transfer",
    "payload",
    "scheduling",
    "discovery",
    "deadlines",
    "retry",
    "ui",
];

/// The agreed defaults, as the monitor reports its effective configuration.
fn defaults() -> Value {
    json!({
        "client": {"escape_prefix": "Ctrl+]", "take_control_key": "Ctrl+T"},
        "monitor": {
            "show_key_hints": true,
            "bell": true,
            "limits": {
                "memory": {
                    "total_bytes": 536_870_912,
                    "client_bytes": 67_108_864,
                    "history_bytes_per_session": 16_777_216,
                    "history_bytes_total": 268_435_456,
                    "queue_bytes_total": 67_108_864
                },
                "history": {"max_rows": 10_000},
                "resources": {
                    "stack_bytes": 2_097_152,
                    "max_workers_per_client": 2,
                    "descriptors_per_client": 8,
                    "descriptors_per_load": 4,
                    "descriptor_headroom": 64,
                    "release_bytes": 67_108_864
                },
                "queues": {
                    "request_bytes": 262_144,
                    "request_records": 64,
                    "event_bytes": 1_048_576,
                    "event_records": 1024,
                    "progress_bytes": 65_536,
                    "progress_records": 32,
                    "deliveries": 8,
                    "operations": 16
                },
                "transfer": {
                    "chunk_bytes": 65_536,
                    "drain_records": 64,
                    "drain_payload_bytes": 65_536
                },
                "payload": {
                    "wire_frame_bytes": 1_048_576,
                    "metadata_record_bytes": 16_384,
                    "title_bytes": 4096,
                    "notification_title_bytes": 4096,
                    "notification_body_bytes": 8192,
                    "assembly_records": 32,
                    "assembly_bytes": 65_536
                },
                "scheduling": {"probes": 8, "loads": 2, "starts": 8, "newer_start_burst": 8},
                "discovery": {
                    "pass_ms": 30_000,
                    "stall_ms": 5000,
                    "batch_records": 128,
                    "batch_bytes": 1_048_576,
                    "metadata_file_bytes": 262_144,
                    "metadata_depth": 32
                },
                "deadlines": {
                    "load_ms": 10_000,
                    "start_ms": 10_000,
                    "operation_ms": 5000,
                    "kill_ms": 10_000,
                    "restoration_stall_ms": 5000,
                    "restoration_total_ms": 60_000,
                    "delivery_stall_ms": 5000,
                    "leave_ms": 5000,
                    "cleanup_ms": 10_000,
                    "shutdown_ms": 10_000,
                    "final_host_ms": 1000,
                    "assembly_ms": 5000
                },
                "retry": {
                    "initial_ms": 1000,
                    "cap_ms": 30_000,
                    "jitter_percent": 20,
                    "admission_ms": 1000
                },
                "ui": {
                    "fps": 30,
                    "client_quantum_ms": 2,
                    "background_quantum_ms": 8,
                    "toast_records": 128,
                    "toast_bytes": 524_288,
                    "search_bytes": 4096
                }
            }
        }
    })
}

fn monitor(text: &str) -> Result<MonitorConfig, ConfigError> {
    MonitorConfig::parse(text.as_bytes())
}

/// The effective values the monitor reports in its diagnostics.
fn reported(config: &MonitorConfig) -> Value {
    serde_json::from_str(&config.to_string()).unwrap()
}

fn effective(text: &str) -> Value {
    reported(&monitor(text).unwrap_or_else(|error| panic!("{text}: {error}")))
}

fn invalid(text: &str) -> String {
    match monitor(text) {
        Err(ConfigError::Invalid(detail)) => detail,
        other => panic!("{text}: expected invalid, got {other:?}"),
    }
}

/// Asserts that `text` is invalid with a detail containing `expected`.
fn rejects(text: &str, expected: &str) {
    let detail = invalid(text);
    assert!(detail.contains(expected), "{text}: {detail}");
}

/// Puts `value` at a dotted path, creating the objects above it.
fn set(root: &mut Value, path: &str, value: Value) {
    let (parents, leaf) = path.rsplit_once('.').unwrap_or(("", path));
    let mut node = root;
    for name in parents.split('.').filter(|name| !name.is_empty()) {
        node = node
            .as_object_mut()
            .unwrap()
            .entry(name)
            .or_insert_with(|| json!({}));
    }
    node.as_object_mut().unwrap().insert(leaf.to_owned(), value);
}

/// A configuration holding each value at its `monitor.limits` path.
fn limits(settings: &[(&str, Value)]) -> Value {
    let mut root = json!({});
    for (path, value) in settings {
        set(&mut root, &format!("monitor.limits.{path}"), value.clone());
    }
    root
}

/// The defaults with each value at its `monitor.limits` path.
fn defaults_with(settings: &[(&str, Value)]) -> Value {
    let mut expected = defaults();
    for (path, value) in settings {
        set(
            &mut expected,
            &format!("monitor.limits.{path}"),
            value.clone(),
        );
    }
    expected
}

/// A configuration with `literal`, written verbatim, at one limits path.
fn literal(path: &str, literal: &str) -> String {
    limits(&[(path, json!("@"))])
        .to_string()
        .replace("\"@\"", literal)
}

/// CFG-01 (Contract): a missing file, object or field means the documented
/// default for every setting, and the monitor reports each effective value.
#[test]
fn cfg01_missing_objects_and_fields_mean_every_documented_default() {
    let empty_groups = format!(
        "{{\"monitor\":{{\"limits\":{{{}}}}}}}",
        GROUPS.map(|group| format!("\"{group}\":{{}}")).join(",")
    );
    for text in [
        "{}",
        r#"{"client":{}}"#,
        r#"{"monitor":{}}"#,
        r#"{"monitor":{"limits":{}}}"#,
        &empty_groups,
    ] {
        assert_eq!(effective(text), defaults(), "{text}");
    }
    let config = monitor("{}").unwrap();
    assert_eq!(
        config.escape_prefix(),
        ControlKey::from_name("Ctrl+]").unwrap()
    );
    assert!(config.settings().show_key_hints);
}

/// CFG-01 (Contract): `show_key_hints` and `bell` are independent booleans,
/// each defaulting to true, and the effective values report them.
#[test]
fn cfg01_hints_and_bell_are_independent_booleans() {
    for (hints, bell) in [(false, true), (true, false), (false, false), (true, true)] {
        let text = json!({"monitor": {"show_key_hints": hints, "bell": bell}}).to_string();
        let config = monitor(&text).unwrap();
        assert_eq!(config.settings().show_key_hints, hints);
        let mut expected = defaults();
        set(&mut expected, "monitor.show_key_hints", json!(hints));
        set(&mut expected, "monitor.bell", json!(bell));
        assert_eq!(reported(&config), expected, "{text}");
    }
}

/// CFG-01 (Contract): every setting accepts the lower limit of its range,
/// except where the rules between settings forbid it: each event allowance
/// must exceed its progress reserve, so its smallest valid value is 2, and
/// the event bytes must fit the aggregate queue, client and total budgets.
#[test]
fn cfg01_every_setting_accepts_the_lower_limit_of_its_range() {
    let settings: Vec<_> = FIELDS
        .iter()
        .map(|(path, kind)| match *path {
            "queues.event_bytes"
            | "queues.event_records"
            | "memory.total_bytes"
            | "memory.client_bytes"
            | "memory.queue_bytes_total" => (*path, json!(2)),
            _ => (*path, json!(kind.min)),
        })
        .collect();
    accepted(&settings);
}

/// CFG-01 (Contract): every setting accepts the upper limit of its range,
/// the largest value of its native type for budgets and counts. The progress
/// reserves stay below their allowances (see the reserve rule), and the bytes
/// reserve leaves the largest chunk its ordinary capacity.
#[test]
fn cfg01_every_setting_accepts_the_upper_limit_of_its_range() {
    let settings: Vec<_> = FIELDS
        .iter()
        .map(|(path, kind)| match *path {
            "queues.progress_bytes" => (*path, json!(u64::MAX - u64::from(u32::MAX))),
            "queues.progress_records" => (*path, json!(u32::MAX - 1)),
            _ => (*path, json!(kind.max)),
        })
        .collect();
    accepted(&settings);
}

/// CFG-01 (Contract): one past either end of every range is an explicit
/// error, including overflow of the native type.
#[test]
fn cfg01_one_past_every_range_is_rejected() {
    for (path, kind) in FIELDS {
        let range = format!("expected an integer from {} to {}", kind.min, kind.max);
        let below = match kind.min {
            0 => (
                String::from("-1"),
                format!("integer `-1`, expected {}", kind.native),
            ),
            min => (
                (min - 1).to_string(),
                format!("integer `{}`, {range}", min - 1),
            ),
        };
        let above = match kind.native {
            "u64" if kind.max == u64::MAX => (
                String::from("18446744073709551616"),
                String::from("invalid type: floating point `1.8446744073709552e+19`, expected u64"),
            ),
            "u32" if kind.max == u64::from(u32::MAX) => (
                String::from("4294967296"),
                String::from("integer `4294967296`, expected u32"),
            ),
            _ => (
                (kind.max + 1).to_string(),
                format!("integer `{}`, {range}", kind.max + 1),
            ),
        };
        for (value, expected) in [below, above] {
            rejects(&literal(path, &value), &expected);
        }
    }
}

/// CFG-01 (Contract): null is an error for every known setting except the
/// take-control key, whatever its level.
#[test]
fn cfg01_null_is_an_error_for_every_known_setting() {
    for (path, kind) in FIELDS {
        rejects(
            &limits(&[(path, Value::Null)]).to_string(),
            &format!("invalid type: null, expected {}", kind.native),
        );
    }
    for group in GROUPS {
        rejects(
            &limits(&[(group, Value::Null)]).to_string(),
            "invalid type: null, expected a JSON object",
        );
    }
    for (text, expected) in [
        (r#"{"monitor":null}"#, "expected a JSON object"),
        (r#"{"monitor":{"limits":null}}"#, "expected a JSON object"),
        (
            r#"{"monitor":{"show_key_hints":null}}"#,
            "expected a boolean",
        ),
        (r#"{"monitor":{"bell":null}}"#, "expected a boolean"),
        (r#"{"client":null}"#, "expected a JSON object"),
    ] {
        rejects(text, &format!("invalid type: null, {expected}"));
    }
}

/// CFG-01 (Contract): wrong types are errors for every known setting: only
/// integers count, only booleans switch and only objects group.
#[test]
fn cfg01_wrong_types_are_errors() {
    for (path, _) in FIELDS {
        for wrong in ["\"1\"", "1.5", "1e3", "1.0", "true", "[]", "{}"] {
            rejects(&literal(path, wrong), "invalid type");
        }
    }
    for group in GROUPS {
        for wrong in ["1", "\"x\"", "true", "[]", "[1,2]"] {
            rejects(&literal(group, wrong), "expected a JSON object");
        }
    }
    for wrong in ["1", "\"x\"", "[]", "true"] {
        rejects(
            &format!(r#"{{"monitor":{wrong}}}"#),
            "expected a JSON object",
        );
        rejects(
            &format!(r#"{{"monitor":{{"limits":{wrong}}}}}"#),
            "expected a JSON object",
        );
    }
    for setting in ["show_key_hints", "bell"] {
        for wrong in ["\"true\"", "1", "0", "[]", "{}"] {
            rejects(
                &format!(r#"{{"monitor":{{"{setting}":{wrong}}}}}"#),
                "expected a boolean",
            );
        }
    }
}

/// CFG-01 (Contract): a known field given twice is an error at every level.
#[test]
fn cfg01_duplicate_known_fields_are_errors() {
    for (path, kind) in FIELDS {
        let (group, field) = path.split_once('.').unwrap();
        let value = kind.min;
        rejects(
            &format!(
                r#"{{"monitor":{{"limits":{{"{group}":{{"{field}":{value},"{field}":{value}}}}}}}}}"#
            ),
            &format!("duplicate field `{field}`"),
        );
    }
    for group in GROUPS {
        rejects(
            &format!(r#"{{"monitor":{{"limits":{{"{group}":{{}},"{group}":{{}}}}}}}}"#),
            &format!("duplicate field `{group}`"),
        );
    }
    for (text, field) in [
        (r#"{"monitor":{},"monitor":{}}"#, "monitor"),
        (r#"{"monitor":{"limits":{},"limits":{}}}"#, "limits"),
        (
            r#"{"monitor":{"show_key_hints":true,"show_key_hints":true}}"#,
            "show_key_hints",
        ),
        (r#"{"monitor":{"bell":true,"bell":true}}"#, "bell"),
        (r#"{"client":{},"client":{}}"#, "client"),
    ] {
        rejects(text, &format!("duplicate field `{field}`"));
    }
}

/// CFG-01 (Contract): unknown and unrelated settings are ignored whatever
/// their value, at every level, and the monitor ignores `scrollback`
/// entirely.
#[test]
fn cfg01_unknown_and_unrelated_settings_are_ignored() {
    let text = r#"{
        "future": null, "future": [1, {"x": null}],
        "scrollback": {"max_rows": "many"}, "scrollback": null,
        "client": {"other": null, "other": 2},
        "monitor": {
            "future": null, "future": "x",
            "limits": {
                "future": {"memory": null},
                "memory": {"future_bytes": -1, "future_bytes": "x"},
                "deadlines": {"later_ms": null},
                "ui": {"theme": [], "theme": {}}
            }
        }
    }"#;
    assert_eq!(effective(text), defaults());
}

/// The detail of the cross-field rule that refuses `settings`.
fn refused(settings: &[(&str, Value)]) -> String {
    invalid(&limits(settings).to_string())
}

fn accepted(settings: &[(&str, Value)]) {
    assert_eq!(
        effective(&limits(settings).to_string()),
        defaults_with(settings),
        "{settings:?}"
    );
}

const TOTAL: u64 = 536_870_912;

fn at(path: &str, value: u64) -> (&str, Value) {
    (path, json!(value))
}

/// CFG-01 (Contract): each sub-budget fits the total that includes it:
/// accepted at the limit, refused one past it, naming both settings.
#[test]
fn cfg01_sub_budgets_fit_their_totals() {
    accepted(&[at("memory.client_bytes", TOTAL)]);
    assert!(
        refused(&[at("memory.client_bytes", TOTAL + 1)]).contains(
            "monitor.limits.memory.client_bytes (536870913) may not exceed monitor.limits.memory.total_bytes (536870912)"
        )
    );
    accepted(&[at("memory.history_bytes_total", TOTAL)]);
    assert!(
        refused(&[at("memory.history_bytes_total", TOTAL + 1)]).contains(
            "monitor.limits.memory.history_bytes_total (536870913) may not exceed monitor.limits.memory.total_bytes (536870912)"
        )
    );
    accepted(&[at("memory.queue_bytes_total", TOTAL)]);
    assert!(
        refused(&[at("memory.queue_bytes_total", TOTAL + 1)]).contains(
            "monitor.limits.memory.queue_bytes_total (536870913) may not exceed monitor.limits.memory.total_bytes (536870912)"
        )
    );
    let history_total = 268_435_456;
    accepted(&[
        at("memory.client_bytes", TOTAL),
        at("memory.history_bytes_per_session", history_total),
    ]);
    assert!(
        refused(&[
            at("memory.client_bytes", TOTAL),
            at("memory.history_bytes_per_session", history_total + 1),
        ])
        .contains(
            "monitor.limits.memory.history_bytes_per_session (268435457) may not exceed monitor.limits.memory.history_bytes_total (268435456)"
        )
    );
    let per_session = 16_777_216;
    accepted(&[at("memory.client_bytes", per_session)]);
    assert!(
        refused(&[at("memory.client_bytes", per_session - 1)]).contains(
            "monitor.limits.memory.history_bytes_per_session (16777216) may not exceed monitor.limits.memory.client_bytes (16777215)"
        )
    );
}

/// CFG-01 (Contract): reservations consume the total too: a thread stack and
/// a release reservation each fit `memory.total_bytes`.
#[test]
fn cfg01_reservations_fit_the_total() {
    for reservation in ["stack_bytes", "release_bytes"] {
        let path = format!("resources.{reservation}");
        accepted(&[at(&path, TOTAL)]);
        assert!(refused(&[at(&path, TOTAL + 1)]).contains(&format!(
            "monitor.limits.resources.{reservation} (536870913) may not exceed monitor.limits.memory.total_bytes (536870912)"
        )));
    }
}

/// CFG-01 (Contract): each client's request and event queues fit both the
/// aggregate queue budget and the client's own budget.
#[test]
fn cfg01_client_queues_fit_the_queue_and_client_budgets() {
    let budget = 67_108_864;
    let roomy_total = at("memory.queue_bytes_total", TOTAL);
    let roomy_client = at("memory.client_bytes", TOTAL);
    for queue in ["request_bytes", "event_bytes"] {
        let path = format!("queues.{queue}");
        accepted(&[at(&path, budget)]);
        assert!(
            refused(&[roomy_client.clone(), at(&path, budget + 1)]).contains(&format!(
                "monitor.limits.queues.{queue} (67108865) may not exceed monitor.limits.memory.queue_bytes_total (67108864)"
            ))
        );
        assert!(
            refused(&[roomy_total.clone(), at(&path, budget + 1)]).contains(&format!(
                "monitor.limits.queues.{queue} (67108865) may not exceed monitor.limits.memory.client_bytes (67108864)"
            ))
        );
    }
}

/// CFG-01 (Contract): each progress reserve sits inside its event allowance
/// and leaves room for ordinary work, up to the native maximum.
#[test]
fn cfg01_progress_reserves_leave_room_in_their_event_allowances() {
    // The largest event allowance, with nothing else in its way: every budget
    // that contains it at its maximum, and the smallest chunk and record.
    let largest_allowance = [
        ("memory.total_bytes", json!(u64::MAX)),
        ("memory.client_bytes", json!(u64::MAX)),
        ("memory.queue_bytes_total", json!(u64::MAX)),
        ("transfer.chunk_bytes", json!(1)),
        ("payload.metadata_record_bytes", json!(1)),
        ("queues.event_bytes", json!(u64::MAX)),
    ];
    let mut room = largest_allowance.to_vec();
    room.push(("queues.progress_bytes", json!(u64::MAX - 1)));
    accepted(&room);
    let mut full = largest_allowance.to_vec();
    full.push(("queues.progress_bytes", json!(u64::MAX)));
    assert!(refused(&full).contains(
        "monitor.limits.queues.progress_bytes (18446744073709551615) must be less than monitor.limits.queues.event_bytes (18446744073709551615)"
    ));
    accepted(&[("queues.progress_records", json!(1023))]);
    assert!(
        refused(&[("queues.progress_records", json!(1024))]).contains(
            "monitor.limits.queues.progress_records (1024) must be less than monitor.limits.queues.event_records (1024)"
        )
    );
    assert!(
        refused(&[
            ("queues.event_records", json!(1)),
            ("queues.progress_records", json!(1))
        ])
        .contains("must be less than monitor.limits.queues.event_records (1)")
    );
}

const ORDINARY: &str = "monitor.limits.queues.event_bytes - monitor.limits.queues.progress_bytes";

/// CFG-01 (Contract): the largest chunk fits one request, the ordinary event
/// capacity beside the progress reserve, and one delivery.
#[test]
fn cfg01_the_largest_chunk_fits_usable_capacity() {
    let chunk = 65_536;
    accepted(&[("queues.request_bytes", json!(chunk))]);
    assert!(
        refused(&[("queues.request_bytes", json!(chunk - 1))]).contains(
            "monitor.limits.transfer.chunk_bytes (65536) may not exceed monitor.limits.queues.request_bytes (65535)"
        )
    );
    accepted(&[("queues.event_bytes", json!(65_536 + chunk))]);
    assert!(
        refused(&[("queues.event_bytes", json!(65_536 + chunk - 1))]).contains(&format!(
            "monitor.limits.transfer.chunk_bytes (65536) may not exceed {ORDINARY} (65535)"
        ))
    );
    accepted(&[("transfer.drain_payload_bytes", json!(chunk))]);
    assert!(
        refused(&[("transfer.drain_payload_bytes", json!(chunk - 1))]).contains(
            "monitor.limits.transfer.chunk_bytes (65536) may not exceed monitor.limits.transfer.drain_payload_bytes (65535)"
        )
    );
}

/// CFG-01 (Contract): the largest metadata record fits the progress reserve,
/// the ordinary event capacity and one delivery.
#[test]
fn cfg01_the_largest_record_fits_usable_capacity() {
    let record = 16_384;
    accepted(&[("queues.progress_bytes", json!(record))]);
    assert!(
        refused(&[("queues.progress_bytes", json!(record - 1))]).contains(
            "monitor.limits.payload.metadata_record_bytes (16384) may not exceed monitor.limits.queues.progress_bytes (16383)"
        )
    );
    let small_chunk = ("transfer.chunk_bytes", json!(1));
    accepted(&[
        small_chunk.clone(),
        ("queues.event_bytes", json!(65_536 + record)),
    ]);
    assert!(
        refused(&[
            small_chunk.clone(),
            ("queues.event_bytes", json!(65_536 + record - 1))
        ])
        .contains(&format!(
            "monitor.limits.payload.metadata_record_bytes (16384) may not exceed {ORDINARY} (16383)"
        ))
    );
    accepted(&[
        small_chunk.clone(),
        ("transfer.drain_payload_bytes", json!(record)),
    ]);
    assert!(
        refused(&[small_chunk, ("transfer.drain_payload_bytes", json!(record - 1))]).contains(
            "monitor.limits.payload.metadata_record_bytes (16384) may not exceed monitor.limits.transfer.drain_payload_bytes (16383)"
        )
    );
}

/// CFG-01 (Contract): the per-client quantum may not exceed the aggregate
/// background quantum, and the initial retry delay may not exceed its cap.
#[test]
fn cfg01_quanta_and_retry_delays_are_ordered() {
    accepted(&[("ui.client_quantum_ms", json!(8))]);
    assert!(refused(&[("ui.client_quantum_ms", json!(9))]).contains(
        "monitor.limits.ui.client_quantum_ms (9) may not exceed monitor.limits.ui.background_quantum_ms (8)"
    ));
    accepted(&[("retry.initial_ms", json!(30_000))]);
    assert!(refused(&[("retry.initial_ms", json!(30_001))]).contains(
        "monitor.limits.retry.initial_ms (30001) may not exceed monitor.limits.retry.cap_ms (30000)"
    ));
}

/// INP-01 (Contract): the monitor shares the client bindings and their rules,
/// refuses a null escape prefix, the only way to reach its actions, and
/// accepts a null take-control key.
#[test]
fn inp01_the_monitor_needs_an_escape_prefix_but_not_a_take_control_key() {
    assert_eq!(
        invalid(r#"{"client":{"escape_prefix":null}}"#),
        "client.escape_prefix may not be null for climon2 monitor"
    );
    let config = monitor(r#"{"client":{"take_control_key":null}}"#).unwrap();
    let mut expected = defaults();
    set(&mut expected, "client.take_control_key", Value::Null);
    assert_eq!(reported(&config), expected);
    let config =
        monitor(r#"{"client":{"escape_prefix":"Ctrl+[","take_control_key":"Ctrl+\\"}}"#).unwrap();
    assert_eq!(config.escape_prefix().byte(), 0x1B);
    set(&mut expected, "client.escape_prefix", json!("Ctrl+["));
    set(&mut expected, "client.take_control_key", json!("Ctrl+\\"));
    assert_eq!(reported(&config), expected);
    for (text, expected) in [
        (
            r#"{"client":{"take_control_key":"Ctrl+["}}"#,
            "take_control_key may not be Ctrl+[",
        ),
        (
            r#"{"client":{"escape_prefix":"Ctrl+T"}}"#,
            "escape_prefix and take_control_key are the same key",
        ),
        (
            r#"{"client":{"escape_prefix":"Alt+x"}}"#,
            r#"invalid key string "Alt+x""#,
        ),
    ] {
        rejects(text, expected);
    }
}

/// CFG-01 (Contract): `exec` validates the whole file, now including the
/// `monitor` section, while `attach` still validates only `client`; neither
/// refuses the null escape prefix that only the monitor needs.
#[test]
fn cfg01_exec_validates_the_monitor_section_and_attach_ignores_it() {
    for (text, expected) in [
        (r#"{"monitor":{"bell":null}}"#, "expected a boolean"),
        (r#"{"monitor":[]}"#, "expected a JSON object"),
        (
            r#"{"monitor":{"limits":{"history":{"max_rows":199}}}}"#,
            "expected an integer from 200 to 100000",
        ),
        (
            r#"{"monitor":{"limits":{"retry":{"initial_ms":30001}}}}"#,
            "monitor.limits.retry.initial_ms (30001) may not exceed monitor.limits.retry.cap_ms (30000)",
        ),
        (
            r#"{"monitor":{"show_key_hints":true,"show_key_hints":false}}"#,
            "duplicate field `show_key_hints`",
        ),
    ] {
        match ExecConfig::parse(text.as_bytes()) {
            Err(ConfigError::Invalid(detail)) => assert!(detail.contains(expected), "{detail}"),
            other => panic!("{text}: {other:?}"),
        }
        assert_eq!(
            AttachConfig::parse(text.as_bytes()).unwrap(),
            AttachConfig::default()
        );
    }
    let valid = r#"{"client":{"escape_prefix":null},"monitor":{"show_key_hints":false,"limits":{"history":{"max_rows":200}}}}"#;
    assert_eq!(
        ExecConfig::parse(valid.as_bytes())
            .unwrap()
            .client
            .escape_prefix(),
        None
    );
    assert_eq!(
        AttachConfig::parse(valid.as_bytes())
            .unwrap()
            .client
            .escape_prefix(),
        None
    );
}

fn home(name: &str) -> HomeLayout {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("monitor-config-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    HomeLayout::resolve(Some(root.as_os_str()), &root).unwrap()
}

/// CFG-01 (Contract): the monitor's snapshot comes from the one read of
/// `config.jsonc` in the climon2 home: a missing file means every default,
/// a later edit changes nothing already read, and malformed JSONC is an
/// explicit error.
#[test]
fn cfg01_the_snapshot_is_one_read_of_the_home_configuration() {
    let home = home("snapshot");
    let missing = ConfigSource::read(&home).unwrap();
    assert_eq!(reported(&missing.monitor().unwrap()), defaults());
    fs::write(home.config(), r#"{"monitor":{"show_key_hints":false}}"#).unwrap();
    let source = ConfigSource::read(&home).unwrap();
    fs::write(home.config(), r#"{"monitor":{"show_key_hints":null}}"#).unwrap();
    assert!(!source.monitor().unwrap().settings().show_key_hints);
    fs::write(home.config(), "{\"monitor\":").unwrap();
    match ConfigSource::read(&home).unwrap().monitor() {
        Err(ConfigError::Malformed(detail)) => assert!(detail.contains("EOF"), "{detail}"),
        other => panic!("{other:?}"),
    }
    fs::remove_dir_all(home.root()).unwrap();
}

/// The README's documented configuration: the first JSONC block of its
/// Configuration section.
fn documented_configuration() -> &'static str {
    let readme = include_str!("../../../README.md");
    let section = readme.split_once("\n## Configuration\n").unwrap().1;
    let block = section.split_once("```jsonc\n").unwrap().1;
    block.split_once("\n```").unwrap().0
}

/// The documented JSONC as plain JSON: its line comments and trailing commas
/// removed. The README's strings hold no `//`.
fn plain(jsonc: &str) -> Value {
    let code: Vec<_> = jsonc
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
        .collect();
    let code = code.join("\n");
    let mut json = String::new();
    for (index, character) in code.char_indices() {
        let next = code[index + 1..].trim_start().chars().next();
        if character != ',' || !matches!(next, Some('}' | ']')) {
            json.push(character);
        }
    }
    serde_json::from_str(&json).unwrap()
}

/// CFG-01 (Docs): the README spells out every client and monitor setting with
/// its default, as JSONC that both `exec` and the monitor accept, and keeps
/// the Escape and Alt warning on the line above `escape_prefix`.
#[test]
fn cfg01_the_readme_documents_every_default_and_the_escape_warning() {
    let documented = documented_configuration();
    let spelled = plain(documented);
    assert_eq!(spelled["client"], defaults()["client"]);
    assert_eq!(spelled["monitor"], defaults()["monitor"]);
    assert_eq!(effective(documented), defaults(), "{documented}");
    assert_eq!(
        ExecConfig::parse(documented.as_bytes()).unwrap(),
        ExecConfig::default()
    );
    let lines: Vec<_> = documented.lines().map(str::trim).collect();
    let prefix = lines
        .iter()
        .position(|line| line.starts_with("\"escape_prefix\""))
        .unwrap();
    assert_eq!(
        lines[prefix - 1],
        "// Ctrl+[ is Escape: it delays bare Escape, and in the monitor legacy Alt+s/x/j/? may trigger the shortcuts."
    );
}
