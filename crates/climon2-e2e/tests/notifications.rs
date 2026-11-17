//! Library layer and Fault (#98): notifications and progress through the
//! installed client library's frozen C ABI, against a real session daemon
//! started through the packaged CLI with the daemon's controllable status
//! clock (`CLIMON2_DAEMON_CLOCK`, a test hook), which stands still at its
//! start, and against the host's fake daemon peer, which sends what a real
//! daemon never would. Expected values come from the native header, the
//! native contract and the daemon wire reference, never from another run
//! of the library. Release archives have no test hooks, so the real-daemon
//! case runs only on the workspace's hooked build.

#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use support::{
    Package, TIMEOUT,
    control::*,
    library::{CHARGE, archive},
    passive::*,
    status::*,
};

/// The controllable clock's start.
const AT_START: &str = "2027-01-15T08:00:00.000Z";
/// Status, notifications and progress.
const REPORTING: [&str; 3] = ["status-v1", "notification-v1", "progress-v1"];
/// What an Open requiring `REQUIRED` and asking for `REPORTING` selects.
const SELECTED: [&str; 5] = [
    "daemon-wire-v1",
    "monitor-v1",
    "status-v1",
    "notification-v1",
    "progress-v1",
];

/// What the session's application prints once told to: progress normal
/// 50, an OSC 9 notification, progress indeterminate, an OSC 99
/// notification in two chunks, an OSC 777 notification, then paused
/// progress, which reports the cached 50.
const SEQUENCES: &str = concat!(
    "\x1b]9;4;1;50\x1b\\",
    "\x1b]9;Build done\x1b\\",
    "\x1b]9;4;3\x1b\\",
    "\x1b]99;i=n:d=0;Tit\x1b\\",
    "\x1b]99;i=n:p=body;Body\x1b\\",
    "\x1b]777;notify;T;B;c\x1b\\",
    "\x1b]9;4;4\x1b\\",
);

/// `step` for instance `name`.
fn on(name: &str, mut step: Value) -> Value {
    step["instance"] = json!(name);
    step
}

/// The steps that create instance `name` with `generation`, Open it with
/// `request` and drain until `until`, labelled `<name>-opened`.
fn opening(
    name: &str,
    session: &Session,
    package: &Package,
    generation: u64,
    (request, until): (Value, u64),
) -> Vec<Value> {
    let startup = session.startup(package, generation);
    vec![
        json!({ "do": "create", "instance": name, "startup": startup }),
        on(name, json!({ "do": "submit", "request": request })),
        on(name, json!({ "do": "run" })),
        on(
            name,
            json!({ "do": "drain", "until": { "tag": until }, "as": format!("{name}-opened") }),
        ),
    ]
}

/// A passive Open asking for status, notifications and progress, through
/// its Initial metadata.
fn watching() -> (Value, u64) {
    (open(1, &REQUIRED, &REPORTING), INITIAL_METADATA)
}

/// An interactive Open at the session's 120x30 asking for status,
/// notifications and progress, through its first restoration.
fn attaching() -> (Value, u64) {
    let request = json!({
        "tag": "open",
        "request_id": 1,
        "open": {
            "mode": INTERACTIVE,
            "geometry": [120, 30],
            "required": ["daemon-wire-v1"],
            "optional": REPORTING,
        },
    });
    (request, RESTORATION_COMPLETE)
}

fn optional_text(text: Option<&str>) -> Value {
    let value = match text {
        Some(text) if !text.is_empty() => json!(text),
        _ => json!({ "null": true, "length": 0 }),
    };
    json!({ "present": u64::from(text.is_some()), "reserved": 0, "value": value })
}

/// A native notification from `source` with the source identifier, title
/// and body given and `episode`, if any (0 for none).
fn notification(
    source: u64,
    source_id: Option<&str>,
    (title, body): (Option<&str>, Option<&str>),
    episode: u64,
) -> Value {
    json!({ "tag": NOTIFICATION, "notification": {
        "source": source,
        "episode_present": u64::from(episode != 0),
        "episode_id": episode,
        "source_id": optional_text(source_id),
        "title": optional_text(title),
        "body": optional_text(body),
    } })
}

/// A native progress in `state`, with `percent` when given.
fn progress(state: u64, percent: Option<u64>) -> Value {
    json!({
        "state": state,
        "percent_present": u64::from(percent.is_some()),
        "percent": percent.unwrap_or(0),
        "reserved": 0,
    })
}

fn progressed(state: u64, percent: Option<u64>) -> Value {
    json!({ "tag": PROGRESS, "progress": progress(state, percent) })
}

/// The native available progress in `state`.
fn available_progress(state: u64, percent: Option<u64>) -> Value {
    json!({ "available": 1, "reserved": 0, "value": progress(state, percent) })
}

/// The status, notification and progress records among `records`, each
/// as its tag and data, after checking their generation.
fn reports(records: &[Value], generation: u64) -> Vec<Value> {
    records
        .iter()
        .filter_map(|record| {
            let tag = record["tag"].as_u64().unwrap();
            let field = match tag {
                STATUS => "status",
                NOTIFICATION => "notification",
                PROGRESS => "progress",
                _ => return None,
            };
            assert_header(record, tag, generation);
            Some(json!({ "tag": tag, field: record[field] }))
        })
        .collect()
}

/// STA-06 Library (notification and progress records: a real daemon's
/// OSC 9, OSC 99 and OSC 777 notifications arrive as native notifications
/// with their source, identifier, inert title and body and the episode
/// they requested or joined, after the needs_attention Status the first
/// one caused; OSC 9;4 progress arrives as native progress, normal 50,
/// indeterminate, then paused with the cached 50, never requesting
/// attention; Initial metadata reports progress none, and a later
/// instance's reports the retained paused 50) and CTL-03 Library
/// (notification and progress cases: the same records, in the same order,
/// reach a passive registration, an attachment revoked by takeover and a
/// monitor controlling in the background before its Ready).
#[test]
fn installed_library_relays_notifications_and_progress_to_every_role() {
    if archive() {
        return;
    }
    let package = Package::new();
    let root = root(&package);
    let clock = root.join("clock");
    start_clock(&clock);
    let go = root.join("go");
    let mut session = Session::clocked_printing(&package, &root, &clock, &go, SEQUENCES);
    let mut steps = opening("p", &session, &package, 1, watching());
    steps.extend(opening("c", &session, &package, 2, attaching()));
    steps.extend(opening("b", &session, &package, 3, watching()));
    steps.extend([
        on("b", json!({ "do": "submit", "request": acquire(2, 3) })),
        on(
            "b",
            json!({ "do": "drain", "until": { "all": [
                { "response": 2 }, { "tag": RESTORATION_COMPLETE }] } }),
        ),
        on("c", json!({ "do": "drain", "until": { "tag": REVOKED } })),
        json!({ "do": "touch", "path": go.to_str().unwrap() }),
    ]);
    for name in ["p", "c", "b"] {
        steps.push(on(
            name,
            json!({ "do": "drain", "until": { "tag": PROGRESS, "count": 3 },
                    "as": format!("{name}-reported") }),
        ));
    }
    steps.extend(opening("q", &session, &package, 4, watching()));
    steps.extend([
        on(
            "b",
            json!({ "do": "submit", "request": ready(3, 3, CURRENT) }),
        ),
        on("b", json!({ "do": "drain", "until": { "response": 3 } })),
        json!({ "do": "touch", "path": session.end_path() }),
    ]);
    for name in ["q", "p", "b", "c"] {
        steps.push(on(
            name,
            json!({ "do": "finish", "as": format!("{name}-ended") }),
        ));
    }
    let report = script(&package, &root, json!(steps));
    for (index, submitted) in steps.iter().enumerate() {
        if submitted["do"] == "submit" {
            assert_success(step(&report, index));
        }
    }
    let none = available_progress(PROGRESS_NONE, None);
    let p = drained(&report, &steps, "p-opened");
    assert_eq!(p[1]["metadata"]["progress"], none, "{p:#?}");
    let pending = status(NEEDS_ATTENTION, Some(1), AT_START);
    let expected = [
        progressed(PROGRESS_NORMAL, Some(50)),
        json!({ "tag": STATUS, "status": pending }),
        notification(OSC9, None, (None, Some("Build done")), 1),
        progressed(PROGRESS_INDETERMINATE, None),
        notification(OSC99, Some("n"), (Some("Tit"), Some("Body")), 1),
        notification(OSC777, None, (Some("T"), Some("B;c")), 1),
        progressed(PROGRESS_PAUSED, Some(50)),
    ];
    for (name, generation) in [("p", 1), ("c", 2), ("b", 3)] {
        let records = drained(&report, &steps, &format!("{name}-reported"));
        assert_eq!(reports(&records, generation), expected, "{name}");
    }
    let q = drained(&report, &steps, "q-opened");
    let paused = available_progress(PROGRESS_PAUSED, Some(50));
    assert_eq!(q[1]["metadata"]["progress"], paused, "{q:#?}");
    assert_eq!(q[1]["metadata"]["status"], available(pending), "{q:#?}");
    for (name, generation) in [("q", 4), ("p", 1), ("b", 3), ("c", 2)] {
        let (records, _) = finished(labelled(&report, &steps, &format!("{name}-ended")));
        assert_eq!(tagged(&records, FINAL).len(), 1, "{name}: {records:#?}");
        assert!(
            reports(&records, generation).is_empty(),
            "{name}: {records:#?}"
        );
    }
    assert_all_released(&report["ledger"]);
    assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
    package.assert_clean();
}

/// What the fake daemon advertises: the real one's families.
const ADVERTISED: [&str; 9] = [
    "attach",
    "kill",
    "state-v1",
    "client-startup-v1",
    "daemon-wire-v1",
    "monitor-v1",
    "status-v1",
    "notification-v1",
    "progress-v1",
];
/// What a fake daemon that does not report progress selects.
const UNPROGRESSED: [&str; 4] = [
    "daemon-wire-v1",
    "monitor-v1",
    "status-v1",
    "notification-v1",
];
const TIME: &str = "2026-02-28T23:59:59.999Z";
/// Notification text bounds small enough for multi-byte cases: a title of
/// seven bytes and a body of nine.
const LIMITS: [(&str, u64); 2] = [
    ("notification_title_bytes", 7),
    ("notification_body_bytes", 9),
];

/// The fake daemon's Initial metadata, idle and untitled, with `progress`
/// (an `Available<Progress>`).
fn initial(progress: Value) -> Value {
    send(
        0x41,
        json!({
            "title": { "available": false, "value": null },
            "status": { "available": true, "value": {
                "state": "idle", "changed_at": TIME, "episode_id": null } },
            "progress": progress,
            "command_result": null,
        }),
    )
}

fn wire_none() -> Value {
    json!({ "available": true, "value": { "state": "none", "percent": null } })
}

fn wire_unavailable() -> Value {
    json!({ "available": false, "value": null })
}

/// A wire OSC 99 Notification `n` of episode 2 with `title` and `body`.
fn notified(title: Value, body: Value) -> Value {
    send(
        0x44,
        json!({ "source": "osc99", "source_id": "n", "title": title, "body": body,
                "episode_id": "2" }),
    )
}

/// A wire OSC 777 Notification with the title `t` and no episode, as when
/// attention could not be requested.
fn unattended() -> Value {
    send(
        0x44,
        json!({ "source": "osc777", "source_id": null, "title": "t", "body": null,
                "episode_id": null }),
    )
}

fn progress_changed(state: &str, percent: Value) -> Value {
    send(
        0x45,
        json!({ "progress": { "state": state, "percent": percent } }),
    )
}

/// A fake daemon that opens selecting `selected` and then sends `after`.
fn peer(selected: &[&str], after: Vec<Value>) -> Value {
    let mut open = vec![opened(selected)];
    open.extend(after);
    json!({
        "hello": { "json": { "capabilities": ADVERTISED } },
        "open": open,
        "leave": [leave_answer("ok", false, Value::Null)],
    })
}

fn limits() -> Value {
    let [(title, title_bytes), (body, body_bytes)] = LIMITS;
    json!({ "limits": { title: title_bytes, body: body_bytes } })
}

/// STA-06 Library (exact native mapping and bounds at the limit: with
/// `notification_title_bytes` 7 and `notification_body_bytes` 9, a
/// notification whose title and body fill them with multi-byte characters
/// arrives whole, as do one with a body alone and one with no episode;
/// progress maps state and percentage exactly, an unknown error percentage
/// absent rather than zero; while queued, a notification is charged its
/// record and its text, and a progress its record alone).
#[test]
fn installed_library_maps_notifications_and_progress_exactly_at_their_bounds() {
    let package = Package::new();
    let root = root(&package);
    let fake = peer(
        &SELECTED,
        vec![
            initial(wire_none()),
            notified(
                json!("\u{20ac}\u{20ac}e"),
                json!("\u{20ac}\u{20ac}\u{20ac}"),
            ),
            progress_changed("normal", json!(50)),
            progress_changed("error", Value::Null),
            progress_changed("paused", json!(7)),
            notified(Value::Null, json!("b")),
            unattended(),
        ],
    );
    let until =
        json!({ "all": [{ "tag": PROGRESS, "count": 3 }, { "tag": NOTIFICATION, "count": 3 }] });
    let steps = json!([
        { "do": "create", "peer": fake, "startup": limits() },
        { "do": "submit", "request": open(1, &REQUIRED, &REPORTING) },
        { "do": "run" },
        { "do": "await_snapshot", "gauge": "event_records", "value": 8, "as": "charged" },
        { "do": "drain", "until": until },
        { "do": "shutdown" },
        { "do": "finish" },
    ]);
    let report = script(&package, &root, steps.clone());
    let charges = &labelled(&report, steps.as_array().unwrap(), "charged")["output"];
    let selected = TOKEN * SELECTED.len() as u64;
    // Each notification's source identifier, title and body: "n", seven
    // and nine bytes; "n" and "b"; and "t".
    let text = (1 + 7 + 9) + (1 + 1) + 1;
    assert_eq!(
        charges["event_bytes"],
        8 * CHARGE + selected + text,
        "{charges:#}"
    );
    let records = step(&report, 4)["records"].as_array().unwrap();
    assert_eq!(assert_response(&records[0], 1, 1, OK)["result_tag"], OPENED);
    assert_header(&records[1], INITIAL_METADATA, 1);
    assert_eq!(
        records[1]["metadata"]["progress"],
        available_progress(PROGRESS_NONE, None)
    );
    let euros = "\u{20ac}\u{20ac}e";
    let body = "\u{20ac}\u{20ac}\u{20ac}";
    let expected = [
        notification(OSC99, Some("n"), (Some(euros), Some(body)), 2),
        progressed(PROGRESS_NORMAL, Some(50)),
        progressed(PROGRESS_ERROR, None),
        progressed(PROGRESS_PAUSED, Some(7)),
        notification(OSC99, Some("n"), (None, Some("b")), 2),
        notification(OSC777, None, (Some("t"), None), 0),
    ];
    assert_eq!(reports(&records[2..], 1), expected, "{records:#?}");
    let (_, terminal) = finished(step(&report, 6));
    assert_left(terminal, 1);
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}

/// STA-06 Fault and LIM-01 Library (notification text one byte past its
/// bound, multi-byte characters included, fails the connection with
/// `resource_limit` instead of being truncated; a notification with
/// neither title nor body, a notification or a progress before the Initial
/// metadata, progress when `progress-v1` is not selected, an
/// Initial metadata reporting progress nobody selected, and one omitting
/// selected progress are `invalid_record`): each fails the connection with
/// a diagnostic and failed terminal evidence with passive retry.
#[test]
fn installed_library_fails_the_connection_on_notification_and_progress_faults() {
    let package = Package::new();
    let root = root(&package);
    let library = (ORIGIN_LIBRARY, "resource_limit");
    let invalid = (ORIGIN_TRANSPORT, "invalid_record");
    let ready = || initial(wire_none());
    let cases = [
        (
            "title",
            peer(
                &SELECTED,
                vec![ready(), notified(json!("\u{20ac}\u{20ac}ee"), Value::Null)],
            ),
            library,
        ),
        (
            "body",
            peer(
                &SELECTED,
                vec![
                    ready(),
                    notified(Value::Null, json!("\u{20ac}\u{20ac}\u{20ac}e")),
                ],
            ),
            library,
        ),
        (
            "blank",
            peer(&SELECTED, vec![ready(), notified(json!(""), Value::Null)]),
            invalid,
        ),
        (
            "early-notification",
            peer(&SELECTED, vec![notified(json!("t"), Value::Null)]),
            invalid,
        ),
        (
            "early-progress",
            peer(&SELECTED, vec![progress_changed("none", Value::Null)]),
            invalid,
        ),
        (
            "unselected",
            peer(
                &UNPROGRESSED,
                vec![
                    initial(wire_unavailable()),
                    progress_changed("none", Value::Null),
                ],
            ),
            invalid,
        ),
        (
            "reported",
            peer(&UNPROGRESSED, vec![initial(wire_none())]),
            invalid,
        ),
        (
            "omitted",
            peer(&SELECTED, vec![initial(wire_unavailable())]),
            invalid,
        ),
    ];
    let mut steps = Vec::new();
    for (name, fake, _) in &cases {
        steps.extend([
            json!({ "do": "create", "instance": name, "peer": fake, "startup": limits() }),
            json!({ "do": "submit", "instance": name, "request": open(1, &REQUIRED, &REPORTING) }),
            json!({ "do": "run", "instance": name }),
            json!({ "do": "wait_run", "instance": name }),
            json!({ "do": "finish", "instance": name }),
        ]);
    }
    let report = script(&package, &root, json!(steps));
    for (index, (name, _, (origin, code))) in cases.iter().enumerate() {
        let (records, terminal) = finished(step(&report, index * 5 + 4));
        assert_eq!(assert_response(&records[0], 1, 1, OK)["result_tag"], OPENED);
        assert_diagnostic(records.last().unwrap(), 1, *origin, code);
        let broken = (CLIENT_FAILED, RETRY_PASSIVE, LEAVE_NOT_REQUIRED);
        assert_terminal(terminal, 1, broken, Some((*origin, code)));
        assert!(reports(&records, 1).is_empty(), "{name}: {records:#?}");
    }
    assert_all_released(&report["ledger"]);
    package.assert_clean();
}
