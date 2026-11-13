//! Library layer (#94): control and display through the installed client
//! library's frozen C ABI, against real session daemons started through
//! the packaged CLI. The `climon2-library-host` fixture's `script` scenario
//! drives each instance step by step; a request names the authority the
//! host drained last (`$period`, `$restoration`) or the one before it.
//! Expected values come from the native header, the native contract and the
//! daemon wire reference, never from another run of the library. Fault
//! cases against the host's fake daemon peer are in `control_faults.rs`.

#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use support::{Package, TIMEOUT, control::*, passive::*};

/// Literal `00 ff 41`, a NUL, invalid UTF-8 (`c3 28`, `fe`, a lone
/// continuation `80`) and `ff` again.
const BINARY: &str = "00ff4100c328fe80ff";

/// The records an interactive Open delivers through its Restoration
/// complete: OK `opened` selecting core, titles and focus, the Initial
/// metadata, then the attach grant at the requested geometry and its
/// restoration. The authority.
fn assert_attached(records: &[Value], id: u64, generation: u64, geometry: [u64; 2]) -> [u64; 2] {
    let opened = assert_ok(&records[0], generation, id, OPENED);
    let mut selected: Vec<&str> = opened["selected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|token| token.as_str().unwrap())
        .collect();
    selected.sort_unstable();
    assert_eq!(selected, ["daemon-wire-v1", "focus-v1", "title-v1"]);
    assert_header(&records[1], INITIAL_METADATA, generation);
    assert_restored(&records[2..], generation, ATTACH, geometry)
}

/// The release when control ends, by Leave or revocation: one Display under
/// the held authority that resets the modes the application set, focus
/// reporting (`ESC [ ? 1004 l`) among them, before the Leave's
/// confirmation or the Control revoked.
fn assert_released(records: &[Value], generation: u64, authority: [u64; 2]) {
    assert_eq!(records.len(), 1, "{records:#?}");
    let released = display(records, generation, authority);
    assert!(released.contains("1b5b3f313030346c"), "{released}");
}

/// COM-04 Library (binary bytes survive the C ABI unchanged both ways:
/// Input of `00 ff 41`, NUL and invalid UTF-8 reaches the raw application,
/// whose unchanged echo returns as Display bytes with no C-string
/// truncation), COM-05 Library (an interactive Open is answered, then the
/// Initial metadata, then the attach grant, its Restoration begin, Display
/// chunks and Restoration complete, all under one immutable authority and
/// the startup generation), COM-02 and COM-03 Library acceptance cases for
/// interactive Open, Ready and Input (`opened`, `ready` with the granted
/// authority, `accepted_bytes` the whole payload) and EXT-01 Library
/// (shutting down an attached instance attempts one Leave: confirmed, the
/// attachment detached after the release Display, reported in terminal
/// evidence apart from cleanup).
#[test]
fn installed_library_attaches_and_relays_binary_input_and_display_unchanged() {
    let package = Package::new();
    let root = root(&package);
    let mut session = Session::start(&package, &root);
    let steps = json!([
        { "do": "create", "startup": session.startup(&package, 3) },
        { "do": "submit", "request": interactive(1, 100, 30) },
        { "do": "run" },
        { "do": "drain", "until": { "tag": RESTORATION_COMPLETE } },
        { "do": "submit", "request": ready(2, 3, CURRENT) },
        { "do": "drain", "until": { "response": 2 } },
        { "do": "submit", "request": input(3, 3, CURRENT, BINARY) },
        { "do": "drain", "until": { "display": BINARY } },
        { "do": "submit", "request": input(4, 3, CURRENT, "454e44") },
        { "do": "drain", "until": { "display": "454e44" } },
        { "do": "shutdown" },
        { "do": "finish" },
    ]);
    let report = script(&package, &root, steps);
    for index in [1, 4, 6, 8, 10] {
        assert_success(step(&report, index));
    }
    let attached = step(&report, 3)["records"].as_array().unwrap();
    let authority = assert_attached(attached, 1, 3, [100, 30]);
    let readied = step(&report, 5)["records"].as_array().unwrap();
    assert_authority(
        assert_ok(answer(readied, 2), 3, 2, READY_RESULT),
        1,
        authority,
    );
    let echoed: Vec<Value> = [7, 9]
        .into_iter()
        .flat_map(|index| step(&report, index)["records"].as_array().unwrap().clone())
        .collect();
    assert_eq!(display(&echoed, 3, authority), format!("{BINARY}454e44"));
    for (id, length) in [(3, 9), (4, 3)] {
        let accepted = assert_ok(answer(&echoed, id), 3, id, INPUT_ACCEPTED);
        assert_eq!(accepted["accepted_bytes"], length, "{accepted:#}");
    }
    let (records, terminal) = finished(step(&report, 11));
    assert_released(&records, 3, authority);
    assert_detached(terminal, 3);
    assert_all_released(&report["ledger"]);
    session.end();
    assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
    package.assert_clean();
}

/// The records of `steps` of `report`, in order.
fn records(report: &Value, steps: &[usize]) -> Vec<Value> {
    steps
        .iter()
        .flat_map(|index| step(report, *index)["records"].as_array().unwrap().clone())
        .collect()
}

/// A rejected Response to `id` with the daemon's `code`.
fn assert_refused(records: &[Value], generation: u64, id: u64, origin: u64, code: &str) {
    assert_rejected(answer(records, id), generation, id, origin, code);
}

/// The steps that attach instance `name` interactively at 100x30 with
/// generation `generation` and drain through its first Restoration
/// complete.
fn attach(name: &str, session: &Session, package: &Package, generation: u64) -> Vec<Value> {
    vec![
        json!({ "do": "create", "instance": name, "startup": session.startup(package, generation) }),
        json!({ "do": "submit", "instance": name, "request": interactive(1, 100, 30) }),
        json!({ "do": "run", "instance": name }),
        json!({ "do": "drain", "instance": name, "until": { "tag": RESTORATION_COMPLETE } }),
    ]
}

/// CTL-05 Library (Input and Ready with a superseded restoration or a stale
/// period are rejected by the daemon, and late authority never re-enables
/// input: a resize before readiness replaces the restoration in the same
/// period, after which the old restoration is `stale_restoration` and the
/// new one `not_ready` until its own Ready; once `b` takes over, `a`'s held
/// authority is `not_controller`; after `a` acquires again, its old period
/// is `stale_period` and its new one `not_ready` until Ready; its new grant
/// is at its own last geometry. No rejected
/// byte ever reaches the application, whose only echo is the final `END`),
/// COM-02 and COM-03 Library (Resize accepted with no result before
/// readiness, then a Restoration begin at the requested size; a request
/// naming another generation rejected locally, `stale_generation`, and
/// never sent; Acquire by an interactive attachment) and EXT-01 Library.
#[test]
fn installed_library_stale_authority_never_re_enables_input() {
    let package = Package::new();
    let root = root(&package);
    let mut session = Session::start(&package, &root);
    let mut steps = attach("a", &session, &package, 4);
    steps.extend(
        json!([
            { "do": "submit", "request": resize(2, 4, CURRENT, 90, 25) },
            { "do": "drain", "until": { "all": [
                { "response": 2 }, { "tag": RESTORATION_COMPLETE }] } },
            { "do": "submit", "request": input(3, 4, STALE, "42") },
            { "do": "submit", "request": ready(4, 4, STALE) },
            { "do": "submit", "request": input(5, 4, CURRENT, "43") },
            { "do": "submit", "request": input(6, 9, CURRENT, "44") },
            { "do": "submit", "request": ready(7, 4, CURRENT) },
            { "do": "drain", "until": { "response": 7 } },
        ])
        .as_array()
        .unwrap()
        .clone(),
    );
    steps.extend(attach("b", &session, &package, 5));
    steps.extend(
        json!([
            { "do": "drain", "until": { "tag": REVOKED } },
            { "do": "submit", "request": input(8, 4, CURRENT, "45") },
            { "do": "submit", "request": acquire(9, 4) },
            { "do": "drain", "until": { "all": [
                { "response": 9 }, { "tag": RESTORATION_COMPLETE }] } },
            { "do": "submit", "request": input(10, 4, STALE, "46") },
            { "do": "submit", "request": input(11, 4, CURRENT, "47") },
            { "do": "submit", "request": ready(12, 4, CURRENT) },
            { "do": "submit", "request": input(13, 4, CURRENT, "454e44") },
            { "do": "drain", "until": { "display": "454e44" } },
            { "do": "shutdown" },
            { "do": "finish" },
            { "do": "shutdown", "instance": "b" },
            { "do": "finish", "instance": "b" },
        ])
        .as_array()
        .unwrap()
        .clone(),
    );
    let report = script(&package, &root, json!(steps));
    let first = assert_attached(
        step(&report, 3)["records"].as_array().unwrap(),
        1,
        4,
        [100, 30],
    );
    let resized = records(&report, &[5]);
    assert_eq!(
        assert_ok(answer(&resized, 2), 4, 2, RESIZE_ACCEPTED),
        &Value::Null
    );
    let begun = tagged(&resized, RESTORATION_BEGIN);
    assert_eq!(begun.len(), 1, "{resized:#?}");
    let second = ids(presented(begun[0], RESTORATION_BEGIN, 4));
    assert_eq!(second[0], first[0], "the same period");
    assert_ne!(second[1], first[1], "a new restoration");
    assert_eq!(begun[0]["presentation"]["geometry"], json!([90, 25]));
    let readied = records(&report, &[11]);
    assert_refused(&readied, 4, 3, ORIGIN_DAEMON, "stale_restoration");
    assert_refused(&readied, 4, 4, ORIGIN_DAEMON, "stale_restoration");
    assert_refused(&readied, 4, 5, ORIGIN_DAEMON, "not_ready");
    assert_refused(&readied, 4, 6, ORIGIN_LIBRARY, "stale_generation");
    assert_authority(
        assert_ok(answer(&readied, 7), 4, 7, READY_RESULT),
        1,
        second,
    );
    let taken = records(&report, &[16]);
    assert_eq!(taken.len(), 2, "{taken:#?}");
    assert_released(&taken[..1], 4, second);
    let revoked = presented(&taken[1], REVOKED, 4);
    assert_eq!(ids(revoked), second, "{revoked:#}");
    assert_eq!(revoked["reason"], TAKEOVER, "{revoked:#}");
    let regained = records(&report, &[19]);
    assert_refused(&regained, 4, 8, ORIGIN_DAEMON, "not_controller");
    let third = assert_restored(&regained, 4, TAKE_CONTROL, [90, 25]);
    assert!(third[0] > second[0], "a new period");
    assert_authority(assert_ok(answer(&regained, 9), 4, 9, ACQUIRED), 1, third);
    let last = records(&report, &[24]);
    assert_refused(&last, 4, 10, ORIGIN_DAEMON, "stale_period");
    assert_refused(&last, 4, 11, ORIGIN_DAEMON, "not_ready");
    assert_authority(assert_ok(answer(&last, 12), 4, 12, READY_RESULT), 1, third);
    assert!(tagged(&readied, DISPLAY).is_empty(), "nothing echoed");
    assert_eq!(display(&records(&report, &[24]), 4, third), "454e44");
    let (rest, terminal) = finished(step(&report, 26));
    assert_released(&rest, 4, third);
    assert_detached(terminal, 4);
    let (_, terminal) = finished(step(&report, 28));
    assert_eq!(terminal["leave"], LEAVE_CONFIRMED, "{terminal:#}");
    assert_all_released(&report["ledger"]);
    session.end();
    assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
    package.assert_clean();
}

/// FOC-01 Library (Focus produces no duplicates and synchronizes on new
/// readiness: the application, which enabled focus reporting, receives one
/// focus-in for two Focus requests of the same state, the second unchanged
/// with no bytes forwarded; after `b` takes over and `a` acquires again,
/// Focus under `a`'s old period is `stale_period` and under its new one
/// `not_ready` until Ready, after which the new period's first Focus is
/// forwarded again, then focus-out once) and COM-02 Library (Focus
/// accepted: changed, value and 0 or 3 bytes; Focus before readiness
/// rejected `not_ready`).
#[test]
fn installed_library_focus_has_no_duplicates_and_resynchronizes_on_new_readiness() {
    let package = Package::new();
    let root = root(&package);
    let mut session = Session::start(&package, &root);
    let mut steps = attach("a", &session, &package, 5);
    steps.extend(
        json!([
            { "do": "submit", "request": focus(2, 5, CURRENT, 1) },
            { "do": "submit", "request": ready(3, 5, CURRENT) },
            { "do": "submit", "request": focus(4, 5, CURRENT, 1) },
            { "do": "submit", "request": focus(5, 5, CURRENT, 1) },
            { "do": "drain", "until": { "all": [{ "response": 5 }, { "display": "1b5b49" }] } },
        ])
        .as_array()
        .unwrap()
        .clone(),
    );
    steps.extend(attach("b", &session, &package, 6));
    steps.extend(
        json!([
            { "do": "drain", "until": { "tag": REVOKED } },
            { "do": "submit", "request": acquire(6, 5) },
            { "do": "drain", "until": { "all": [
                { "response": 6 }, { "tag": RESTORATION_COMPLETE }] } },
            { "do": "submit", "request": focus(7, 5, STALE, 1) },
            { "do": "submit", "request": focus(8, 5, CURRENT, 1) },
            { "do": "submit", "request": ready(9, 5, CURRENT) },
            { "do": "submit", "request": focus(10, 5, CURRENT, 1) },
            { "do": "submit", "request": focus(11, 5, CURRENT, 0) },
            { "do": "submit", "request": input(12, 5, CURRENT, "454e44") },
            { "do": "drain", "until": { "display": "1b5b491b5b4f454e44" } },
            { "do": "shutdown" },
            { "do": "finish" },
            { "do": "shutdown", "instance": "b" },
            { "do": "finish", "instance": "b" },
        ])
        .as_array()
        .unwrap()
        .clone(),
    );
    let report = script(&package, &root, json!(steps));
    let first = assert_attached(
        step(&report, 3)["records"].as_array().unwrap(),
        1,
        5,
        [100, 30],
    );
    let focused = records(&report, &[8]);
    assert_refused(&focused, 5, 2, ORIGIN_DAEMON, "not_ready");
    let flag = |changed: u64, value: u64, bytes: u64| json!({ "changed": changed, "value": value, "reserved": [0, 0], "accepted_bytes": bytes });
    assert_eq!(
        assert_ok(answer(&focused, 4), 5, 4, FOCUS_RESULT),
        &flag(1, 1, 3)
    );
    assert_eq!(
        assert_ok(answer(&focused, 5), 5, 5, FOCUS_RESULT),
        &flag(0, 1, 0)
    );
    assert_eq!(display(&focused, 5, first), "1b5b49", "one focus-in only");
    let regained = records(&report, &[15]);
    let second = assert_restored(&regained, 5, TAKE_CONTROL, [100, 30]);
    let late = records(&report, &[22]);
    assert_refused(&late, 5, 7, ORIGIN_DAEMON, "stale_period");
    assert_refused(&late, 5, 8, ORIGIN_DAEMON, "not_ready");
    assert_eq!(
        assert_ok(answer(&late, 10), 5, 10, FOCUS_RESULT),
        &flag(1, 1, 3)
    );
    assert_eq!(
        assert_ok(answer(&late, 11), 5, 11, FOCUS_RESULT),
        &flag(1, 0, 3)
    );
    assert_eq!(display(&late, 5, second), "1b5b491b5b4f454e44");
    let (rest, terminal) = finished(step(&report, 24));
    assert_released(&rest, 5, second);
    assert_detached(terminal, 5);
    let (_, terminal) = finished(step(&report, 26));
    assert_eq!(terminal["leave"], LEAVE_CONFIRMED, "{terminal:#}");
    assert_all_released(&report["ledger"]);
    session.end();
    assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
    package.assert_clean();
}

/// The steps that register monitor instance `name` passively with
/// generation `generation` and drain its Initial metadata.
fn register(name: &str, session: &Session, package: &Package, generation: u64) -> Vec<Value> {
    vec![
        json!({ "do": "create", "instance": name, "startup": session.startup(package, generation) }),
        json!({ "do": "submit", "instance": name, "request": open(1, &REQUIRED, &[]) }),
        json!({ "do": "run", "instance": name }),
        json!({ "do": "drain", "instance": name, "until": { "tag": INITIAL_METADATA } }),
    ]
}

fn on(name: &str, step: Value) -> Value {
    let mut step = step;
    step["instance"] = json!(name);
    step
}

/// CTL-02 Library (explicit Acquire, switching between instances and
/// another client's takeover: monitor `a` acquires a session its terminal
/// client controls and is granted `take_control` at the session's effective
/// geometry; acquiring again changes nothing; `b` acquiring revokes `a`
/// (takeover) under `a`'s exact authority; `a` acquiring again wins back
/// control in a new period, so the last accepted acquisition wins; and
/// control is retained in the background, without Ready, until an
/// interactive instance `c` takes over), COM-02 Library (Acquire accepted:
/// changed and the granted authority, its grant first) and EXT-01 Library
/// (each instance's shutdown leaves, confirmed; a monitor that acquired
/// is an attachment, so its Leave detaches one).
#[test]
fn installed_library_acquisition_switches_and_takeover_keep_the_last_accepted() {
    let package = Package::new();
    let root = root(&package);
    let mut session = Session::idle(&package, &root);
    let mut steps = register("a", &session, &package, 1);
    steps.extend(register("b", &session, &package, 2));
    steps.extend([
        on("a", json!({ "do": "submit", "request": acquire(2, 1) })),
        on(
            "a",
            json!({ "do": "drain", "until": { "all": [
            { "response": 2 }, { "tag": RESTORATION_COMPLETE }] } }),
        ),
        on("a", json!({ "do": "submit", "request": acquire(3, 1) })),
        on("a", json!({ "do": "drain", "until": { "response": 3 } })),
        on("b", json!({ "do": "submit", "request": acquire(2, 2) })),
        on(
            "b",
            json!({ "do": "drain", "until": { "all": [
            { "response": 2 }, { "tag": RESTORATION_COMPLETE }] } }),
        ),
        on("a", json!({ "do": "drain", "until": { "tag": REVOKED } })),
        on("a", json!({ "do": "submit", "request": acquire(4, 1) })),
        on(
            "a",
            json!({ "do": "drain", "until": { "all": [
            { "response": 4 }, { "tag": RESTORATION_COMPLETE }] } }),
        ),
        on("b", json!({ "do": "drain", "until": { "tag": REVOKED } })),
    ]);
    steps.extend(attach("c", &session, &package, 3));
    steps.extend([
        on("a", json!({ "do": "drain", "until": { "tag": REVOKED } })),
        on("a", json!({ "do": "shutdown" })),
        on("a", json!({ "do": "finish" })),
        on("b", json!({ "do": "shutdown" })),
        on("b", json!({ "do": "finish" })),
        on("c", json!({ "do": "shutdown" })),
        on("c", json!({ "do": "finish" })),
    ]);
    let report = script(&package, &root, json!(steps));
    let granted = |index: usize, generation: u64, id: u64| {
        let found = records(&report, &[index]);
        let at = found
            .iter()
            .position(|record| record["tag"] == GRANTED)
            .unwrap();
        let authority = assert_restored(&found[at..], generation, TAKE_CONTROL, [120, 30]);
        let answered = found
            .iter()
            .position(|record| record["response"]["request_id"] == id)
            .unwrap();
        assert!(at < answered, "the grant precedes its Acquire's success");
        let result = assert_ok(&found[answered], generation, id, ACQUIRED);
        assert_authority(result, 1, authority);
        authority
    };
    let a1 = granted(9, 1, 2);
    let again = records(&report, &[11]);
    assert_eq!(again.len(), 1, "no new restoration: {again:#?}");
    assert_authority(assert_ok(&again[0], 1, 3, ACQUIRED), 0, a1);
    let b1 = granted(13, 2, 2);
    assert!(b1[0] > a1[0], "a new period");
    let revoked = |index: usize, generation: u64, authority: [u64; 2]| {
        let found = records(&report, &[index]);
        assert_eq!(found.len(), 2, "{found:#?}");
        assert_released(&found[..1], generation, authority);
        let revoked = presented(&found[1], REVOKED, generation);
        assert_eq!(ids(revoked), authority, "{revoked:#}");
        assert_eq!(revoked["reason"], TAKEOVER, "{revoked:#}");
    };
    revoked(14, 1, a1);
    let a2 = granted(16, 1, 4);
    assert!(a2[0] > b1[0], "a new period");
    revoked(17, 2, b1);
    let attached = step(&report, 21)["records"].as_array().unwrap();
    assert_attached(attached, 1, 3, [100, 30]);
    revoked(22, 1, a2);
    for (index, generation) in [(24, 1), (26, 2)] {
        let (rest, terminal) = finished(step(&report, index));
        assert!(rest.is_empty(), "{rest:#?}");
        assert_detached(terminal, generation);
    }
    let (_, terminal) = finished(step(&report, 28));
    assert_detached(terminal, 3);
    assert_all_released(&report["ledger"]);
    session.end();
    assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
    package.assert_clean();
}

/// CTL-04 Library (when the controller departs, the newest passive
/// registration is promoted: of `p1` and then `p2`, registered on a
/// detached session that interactive `i` then attaches, `p2` receives a
/// background `successor` grant with a canonical restoration at the
/// session's effective geometry, and Ready without selection succeeds;
/// `p1` receives nothing until `p2` leaves too, when it is promoted in
/// turn), COM-05 Library (a fresh attempt never sees an old one's
/// deliveries: every record carries its own instance's generation, and each
/// promotion starts a new period).
#[test]
fn installed_library_promotes_the_newest_registration_with_a_canonical_restoration() {
    let package = Package::new();
    let root = root(&package);
    let session = Session::detached(&package, &root);
    let mut steps = register("p1", &session, &package, 1);
    steps.extend(register("p2", &session, &package, 2));
    steps.extend(attach("i", &session, &package, 3));
    steps.extend([
        on("i", json!({ "do": "shutdown" })),
        on("i", json!({ "do": "finish" })),
        on(
            "p2",
            json!({ "do": "drain", "until": { "tag": RESTORATION_COMPLETE } }),
        ),
        on(
            "p2",
            json!({ "do": "submit", "request": ready(2, 2, CURRENT) }),
        ),
        on("p2", json!({ "do": "drain", "until": { "response": 2 } })),
        on("p1", json!({ "do": "drain" })),
        on("p2", json!({ "do": "shutdown" })),
        on("p2", json!({ "do": "finish" })),
        on(
            "p1",
            json!({ "do": "drain", "until": { "tag": RESTORATION_COMPLETE } }),
        ),
        on("p1", json!({ "do": "shutdown" })),
        on("p1", json!({ "do": "finish" })),
    ]);
    let report = script(&package, &root, json!(steps));
    let attached = step(&report, 11)["records"].as_array().unwrap();
    let held = assert_attached(attached, 1, 3, [100, 30]);
    let (rest, terminal) = finished(step(&report, 13));
    assert_released(&rest, 3, held);
    assert_detached(terminal, 3);
    let promoted = records(&report, &[14]);
    let p2 = assert_restored(&promoted, 2, SUCCESSOR, [100, 30]);
    assert!(p2[0] > held[0], "a new period");
    let readied = records(&report, &[16]);
    assert_authority(assert_ok(answer(&readied, 2), 2, 2, READY_RESULT), 1, p2);
    let quiet = &step(&report, 17)["drains"][0]["drained"];
    assert_eq!(quiet["kind"], EMPTY, "p1 is not promoted: {quiet:#}");
    let (rest, terminal) = finished(step(&report, 19));
    assert_released(&rest, 2, p2);
    assert_detached(terminal, 2);
    let p1 = assert_restored(&records(&report, &[20]), 1, SUCCESSOR, [100, 30]);
    assert!(p1[0] > p2[0], "a new period");
    let (rest, terminal) = finished(step(&report, 22));
    assert_released(&rest, 1, p1);
    assert_detached(terminal, 1);
    assert_all_released(&report["ledger"]);
    session.end();
    package.assert_clean();
}
