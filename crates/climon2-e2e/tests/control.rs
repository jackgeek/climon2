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

/// The release when control ends, by Leave or revocation: one Display, or
/// its `chunk_bytes` records, under the held authority that resets the
/// modes the application set, focus reporting (`ESC [ ? 1004 l`) among
/// them, before the Leave's confirmation or the Control revoked.
fn assert_released(records: &[Value], generation: u64, authority: [u64; 2]) {
    assert!(!records.is_empty());
    assert!(
        records.iter().all(|record| record["tag"] == DISPLAY),
        "{records:#?}"
    );
    let released = display(records, generation, authority);
    assert!(released.contains("1b5b3f313030346c"), "{released}");
}

/// COM-04 Library (binary bytes survive the C ABI unchanged both ways:
/// Input of `00 ff 41`, NUL and invalid UTF-8 reaches the raw application,
/// whose unchanged echo returns as Display bytes with no C-string
/// truncation, and with `chunk_bytes` of 16, far below the daemon's frame
/// size, every Display is at most 16 bytes and the stream stays healthy),
/// COM-05 Library (an interactive Open is answered, then the
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
    let mut startup = session.startup(&package, 3);
    startup["limits"] = json!({ "chunk_bytes": 16 });
    let steps = vec![
        json!({ "do": "create", "startup": startup }),
        json!({ "do": "submit", "request": interactive(1, 100, 30) }),
        json!({ "do": "run" }),
        json!({ "do": "drain", "until": { "tag": RESTORATION_COMPLETE }, "as": "attached" }),
        json!({ "do": "submit", "request": ready(2, 3, CURRENT) }),
        json!({ "do": "drain", "until": { "response": 2 }, "as": "readied" }),
        json!({ "do": "submit", "request": input(3, 3, CURRENT, BINARY) }),
        json!({ "do": "drain", "until": { "display": BINARY }, "as": "binary" }),
        json!({ "do": "submit", "request": input(4, 3, CURRENT, "454e44") }),
        json!({ "do": "drain", "until": { "display": "454e44" }, "as": "end" }),
        json!({ "do": "shutdown" }),
        json!({ "do": "finish", "as": "finished" }),
    ];
    let report = script(&package, &root, json!(steps));
    assert_submitted(&report, &steps);
    let authority = assert_attached(&drained(&report, &steps, "attached"), 1, 3, [100, 30]);
    let readied = drained(&report, &steps, "readied");
    assert_authority(
        assert_ok(answer(&readied, 2), 3, 2, READY_RESULT),
        1,
        authority,
    );
    let echoed = records(&report, &steps, &["binary", "end"]);
    assert_eq!(display(&echoed, 3, authority), format!("{BINARY}454e44"));
    for (id, length) in [(3, 9), (4, 3)] {
        let accepted = assert_ok(answer(&echoed, id), 3, id, INPUT_ACCEPTED);
        assert_eq!(accepted["accepted_bytes"], length, "{accepted:#}");
    }
    let (records_of_finish, terminal) = finished(labelled(&report, &steps, "finished"));
    assert_released(&records_of_finish, 3, authority);
    assert_detached(terminal, 3);
    let mut every = records(&report, &steps, &["attached", "readied", "binary", "end"]);
    every.extend(records_of_finish);
    let chunks = tagged(&every, DISPLAY);
    assert!(chunks.len() > 2, "{every:#?}");
    for chunk in chunks {
        let bytes = chunk["presentation"]["bytes"].as_str().unwrap().len() / 2;
        assert!((1..=16).contains(&bytes), "{chunk:#}");
    }
    assert_all_released(&report["ledger"]);
    session.end();
    assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
    package.assert_clean();
}

/// The records the steps labelled `labels` drained, in order.
fn records(report: &Value, steps: &[Value], labels: &[&str]) -> Vec<Value> {
    labels
        .iter()
        .flat_map(|label| drained(report, steps, label))
        .collect()
}

/// Every `submit` among `steps` was accepted.
fn assert_submitted(report: &Value, steps: &[Value]) {
    for (index, submitted) in steps.iter().enumerate() {
        if submitted["do"] == "submit" {
            assert_success(step(report, index));
        }
    }
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
        json!({ "do": "drain", "instance": name, "until": { "tag": RESTORATION_COMPLETE },
                "as": format!("attached-{name}") }),
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
                { "response": 2 }, { "tag": RESTORATION_COMPLETE }] }, "as": "resized" },
            { "do": "submit", "request": input(3, 4, STALE, "42") },
            { "do": "submit", "request": ready(4, 4, STALE) },
            { "do": "submit", "request": input(5, 4, CURRENT, "43") },
            { "do": "submit", "request": input(6, 9, CURRENT, "44") },
            { "do": "submit", "request": ready(7, 4, CURRENT) },
            { "do": "drain", "until": { "response": 7 }, "as": "readied" },
        ])
        .as_array()
        .unwrap()
        .clone(),
    );
    steps.extend(attach("b", &session, &package, 5));
    steps.extend(
        json!([
            { "do": "drain", "until": { "tag": REVOKED }, "as": "taken" },
            { "do": "submit", "request": input(8, 4, CURRENT, "45") },
            { "do": "submit", "request": acquire(9, 4) },
            { "do": "drain", "until": { "all": [
                { "response": 9 }, { "tag": RESTORATION_COMPLETE }] }, "as": "regained" },
            { "do": "submit", "request": input(10, 4, STALE, "46") },
            { "do": "submit", "request": input(11, 4, CURRENT, "47") },
            { "do": "submit", "request": ready(12, 4, CURRENT) },
            { "do": "submit", "request": input(13, 4, CURRENT, "454e44") },
            { "do": "drain", "until": { "display": "454e44" }, "as": "last" },
            { "do": "shutdown" },
            { "do": "finish", "as": "finished-a" },
            { "do": "shutdown", "instance": "b" },
            { "do": "finish", "instance": "b", "as": "finished-b" },
        ])
        .as_array()
        .unwrap()
        .clone(),
    );
    let report = script(&package, &root, json!(steps));
    assert_submitted(&report, &steps);
    let first = assert_attached(&drained(&report, &steps, "attached-a"), 1, 4, [100, 30]);
    let resized = drained(&report, &steps, "resized");
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
    let readied = drained(&report, &steps, "readied");
    assert_refused(&readied, 4, 3, ORIGIN_DAEMON, "stale_restoration");
    assert_refused(&readied, 4, 4, ORIGIN_DAEMON, "stale_restoration");
    assert_refused(&readied, 4, 5, ORIGIN_DAEMON, "not_ready");
    assert_refused(&readied, 4, 6, ORIGIN_LIBRARY, "stale_generation");
    assert_authority(
        assert_ok(answer(&readied, 7), 4, 7, READY_RESULT),
        1,
        second,
    );
    let taken = drained(&report, &steps, "taken");
    assert_eq!(taken.len(), 2, "{taken:#?}");
    assert_released(&taken[..1], 4, second);
    let revoked = presented(&taken[1], REVOKED, 4);
    assert_eq!(ids(revoked), second, "{revoked:#}");
    assert_eq!(revoked["reason"], TAKEOVER, "{revoked:#}");
    let regained = drained(&report, &steps, "regained");
    assert_refused(&regained, 4, 8, ORIGIN_DAEMON, "not_controller");
    let third = assert_restored(&regained, 4, TAKE_CONTROL, [90, 25]);
    assert!(third[0] > second[0], "a new period");
    assert_authority(assert_ok(answer(&regained, 9), 4, 9, ACQUIRED), 1, third);
    let last = drained(&report, &steps, "last");
    assert_refused(&last, 4, 10, ORIGIN_DAEMON, "stale_period");
    assert_refused(&last, 4, 11, ORIGIN_DAEMON, "not_ready");
    assert_authority(assert_ok(answer(&last, 12), 4, 12, READY_RESULT), 1, third);
    assert!(tagged(&readied, DISPLAY).is_empty(), "nothing echoed");
    assert_eq!(display(&last, 4, third), "454e44");
    let (rest, terminal) = finished(labelled(&report, &steps, "finished-a"));
    assert_released(&rest, 4, third);
    assert_detached(terminal, 4);
    let (_, terminal) = finished(labelled(&report, &steps, "finished-b"));
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
            { "do": "drain", "until": { "all": [{ "response": 5 }, { "display": "1b5b49" }] },
              "as": "focused" },
        ])
        .as_array()
        .unwrap()
        .clone(),
    );
    steps.extend(attach("b", &session, &package, 6));
    steps.extend(
        json!([
            { "do": "drain", "until": { "tag": REVOKED }, "as": "taken" },
            { "do": "submit", "request": acquire(6, 5) },
            { "do": "drain", "until": { "all": [
                { "response": 6 }, { "tag": RESTORATION_COMPLETE }] }, "as": "regained" },
            { "do": "submit", "request": focus(7, 5, STALE, 1) },
            { "do": "submit", "request": focus(8, 5, CURRENT, 1) },
            { "do": "submit", "request": ready(9, 5, CURRENT) },
            { "do": "submit", "request": focus(10, 5, CURRENT, 1) },
            { "do": "submit", "request": focus(11, 5, CURRENT, 0) },
            { "do": "submit", "request": input(12, 5, CURRENT, "454e44") },
            { "do": "drain", "until": { "display": "1b5b491b5b4f454e44" }, "as": "late" },
            { "do": "shutdown" },
            { "do": "finish", "as": "finished-a" },
            { "do": "shutdown", "instance": "b" },
            { "do": "finish", "instance": "b", "as": "finished-b" },
        ])
        .as_array()
        .unwrap()
        .clone(),
    );
    let report = script(&package, &root, json!(steps));
    assert_submitted(&report, &steps);
    let first = assert_attached(&drained(&report, &steps, "attached-a"), 1, 5, [100, 30]);
    let focused = drained(&report, &steps, "focused");
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
    let regained = drained(&report, &steps, "regained");
    let second = assert_restored(&regained, 5, TAKE_CONTROL, [100, 30]);
    let late = drained(&report, &steps, "late");
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
    let (rest, terminal) = finished(labelled(&report, &steps, "finished-a"));
    assert_released(&rest, 5, second);
    assert_detached(terminal, 5);
    let (_, terminal) = finished(labelled(&report, &steps, "finished-b"));
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
            { "response": 2 }, { "tag": RESTORATION_COMPLETE }] }, "as": "a1" }),
        ),
        on("a", json!({ "do": "submit", "request": acquire(3, 1) })),
        on(
            "a",
            json!({ "do": "drain", "until": { "response": 3 }, "as": "again" }),
        ),
        on("b", json!({ "do": "submit", "request": acquire(2, 2) })),
        on(
            "b",
            json!({ "do": "drain", "until": { "all": [
            { "response": 2 }, { "tag": RESTORATION_COMPLETE }] }, "as": "b1" }),
        ),
        on(
            "a",
            json!({ "do": "drain", "until": { "tag": REVOKED }, "as": "a1-revoked" }),
        ),
        on("a", json!({ "do": "submit", "request": acquire(4, 1) })),
        on(
            "a",
            json!({ "do": "drain", "until": { "all": [
            { "response": 4 }, { "tag": RESTORATION_COMPLETE }] }, "as": "a2" }),
        ),
        on(
            "b",
            json!({ "do": "drain", "until": { "tag": REVOKED }, "as": "b1-revoked" }),
        ),
    ]);
    steps.extend(attach("c", &session, &package, 3));
    steps.extend([
        on(
            "a",
            json!({ "do": "drain", "until": { "tag": REVOKED }, "as": "a2-revoked" }),
        ),
        on("a", json!({ "do": "shutdown" })),
        on("a", json!({ "do": "finish", "as": "finished-a" })),
        on("b", json!({ "do": "shutdown" })),
        on("b", json!({ "do": "finish", "as": "finished-b" })),
        on("c", json!({ "do": "shutdown" })),
        on("c", json!({ "do": "finish", "as": "finished-c" })),
    ]);
    let report = script(&package, &root, json!(steps));
    assert_submitted(&report, &steps);
    let granted = |label: &str, generation: u64, id: u64| {
        let found = drained(&report, &steps, label);
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
    let a1 = granted("a1", 1, 2);
    let again = drained(&report, &steps, "again");
    assert_eq!(again.len(), 1, "no new restoration: {again:#?}");
    assert_authority(assert_ok(&again[0], 1, 3, ACQUIRED), 0, a1);
    let b1 = granted("b1", 2, 2);
    assert!(b1[0] > a1[0], "a new period");
    let revoked = |label: &str, generation: u64, authority: [u64; 2]| {
        let found = drained(&report, &steps, label);
        assert_eq!(found.len(), 2, "{found:#?}");
        assert_released(&found[..1], generation, authority);
        let revoked = presented(&found[1], REVOKED, generation);
        assert_eq!(ids(revoked), authority, "{revoked:#}");
        assert_eq!(revoked["reason"], TAKEOVER, "{revoked:#}");
    };
    revoked("a1-revoked", 1, a1);
    let a2 = granted("a2", 1, 4);
    assert!(a2[0] > b1[0], "a new period");
    revoked("b1-revoked", 2, b1);
    assert_attached(&drained(&report, &steps, "attached-c"), 1, 3, [100, 30]);
    revoked("a2-revoked", 1, a2);
    for (label, generation) in [("finished-a", 1), ("finished-b", 2)] {
        let (rest, terminal) = finished(labelled(&report, &steps, label));
        assert!(rest.is_empty(), "{rest:#?}");
        assert_detached(terminal, generation);
    }
    let (_, terminal) = finished(labelled(&report, &steps, "finished-c"));
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
        on("i", json!({ "do": "finish", "as": "finished-i" })),
        on(
            "p2",
            json!({ "do": "drain", "until": { "tag": RESTORATION_COMPLETE }, "as": "promoted-p2" }),
        ),
        on(
            "p2",
            json!({ "do": "submit", "request": ready(2, 2, CURRENT) }),
        ),
        on(
            "p2",
            json!({ "do": "drain", "until": { "response": 2 }, "as": "readied" }),
        ),
        on("p1", json!({ "do": "drain", "as": "quiet" })),
        on("p2", json!({ "do": "shutdown" })),
        on("p2", json!({ "do": "finish", "as": "finished-p2" })),
        on(
            "p1",
            json!({ "do": "drain", "until": { "tag": RESTORATION_COMPLETE }, "as": "promoted-p1" }),
        ),
        on("p1", json!({ "do": "shutdown" })),
        on("p1", json!({ "do": "finish", "as": "finished-p1" })),
    ]);
    let report = script(&package, &root, json!(steps));
    assert_submitted(&report, &steps);
    let held = assert_attached(&drained(&report, &steps, "attached-i"), 1, 3, [100, 30]);
    let (rest, terminal) = finished(labelled(&report, &steps, "finished-i"));
    assert_released(&rest, 3, held);
    assert_detached(terminal, 3);
    let promoted = drained(&report, &steps, "promoted-p2");
    let p2 = assert_restored(&promoted, 2, SUCCESSOR, [100, 30]);
    assert!(p2[0] > held[0], "a new period");
    let readied = drained(&report, &steps, "readied");
    assert_authority(assert_ok(answer(&readied, 2), 2, 2, READY_RESULT), 1, p2);
    let quiet = &labelled(&report, &steps, "quiet")["drains"][0]["drained"];
    assert_eq!(quiet["kind"], EMPTY, "p1 is not promoted: {quiet:#}");
    let (rest, terminal) = finished(labelled(&report, &steps, "finished-p2"));
    assert_released(&rest, 2, p2);
    assert_detached(terminal, 2);
    let p1 = assert_restored(
        &drained(&report, &steps, "promoted-p1"),
        1,
        SUCCESSOR,
        [100, 30],
    );
    assert!(p1[0] > p2[0], "a new period");
    let (rest, terminal) = finished(labelled(&report, &steps, "finished-p1"));
    assert_released(&rest, 1, p1);
    assert_detached(terminal, 1);
    assert_all_released(&report["ledger"]);
    session.end();
    package.assert_clean();
}

/// COM-05 Library (a session that ends while an instance controls it keeps
/// its causal order through the library: the Command final arrives first,
/// as `CM1_FINAL` with exit 0 and status unavailable, then the release
/// Display under the held authority and Control revoked `session_ending`;
/// the stream then ends with retry none, since the final result is known,
/// and no Leave required) and COM-06 Library (the final result reaches an
/// attached controller). Exit and signal detail, repeats and evidence that
/// outlives the connection belong to #96.
#[test]
fn installed_library_relays_the_command_final_before_the_release_and_session_ending() {
    let package = Package::new();
    let root = root(&package);
    let mut session = Session::start(&package, &root);
    let mut steps = attach("a", &session, &package, 7);
    steps.extend([
        on("a", json!({ "do": "submit", "request": ready(2, 7, CURRENT) })),
        on("a", json!({ "do": "drain", "until": { "response": 2 } })),
        on(
            "a",
            json!({ "do": "submit", "request": input(3, 7, CURRENT, "454e44") }),
        ),
        on(
            "a",
            json!({ "do": "drain", "until": { "all": [{ "response": 3 }, { "display": "454e44" }] } }),
        ),
        json!({ "do": "touch", "path": session.end_path() }),
        json!({ "do": "finish", "instance": "a", "as": "ended" }),
    ]);
    let report = script(&package, &root, json!(steps));
    let authority = assert_attached(&drained(&report, &steps, "attached-a"), 1, 7, [100, 30]);
    let (records, terminal) = finished(labelled(&report, &steps, "ended"));
    assert_eq!(
        tags(&records),
        [FINAL, DISPLAY, REVOKED, DIAGNOSTIC],
        "{records:#?}"
    );
    assert_final(&records[0], 7, command_result(EXIT, 0));
    assert_released(&records[1..2], 7, authority);
    let ended = presented(&records[2], REVOKED, 7);
    assert_eq!(ids(ended), authority, "{ended:#}");
    assert_eq!(ended["reason"], SESSION_ENDING, "{ended:#}");
    let closed = (ORIGIN_TRANSPORT, "io_error");
    assert_diagnostic(&records[3], 7, closed.0, closed.1);
    assert_terminal(
        terminal,
        7,
        (CLIENT_FAILED, RETRY_NONE, LEAVE_NOT_REQUIRED),
        Some(closed),
    );
    assert_all_released(&report["ledger"]);
    assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
    package.assert_clean();
}

/// CTL-04 and EXT-01 Library (Leave racing a promotion, against a real
/// daemon; owner decision on #94): registration `p` shuts down after the
/// controller `i` left, which promoted `p`, but before `p`'s caller drained
/// anything of it, so `p`'s one Leave reaches the daemon as a controller's.
/// The library delivers the `successor` grant and its canonical
/// restoration at the session's effective geometry, then the release
/// Display under that authority, and reports the Leave confirmed with the
/// attachment detached; nobody is left to promote, so the session stays
/// detached with no attachment.
#[test]
fn installed_library_leave_racing_a_promotion_leaves_as_the_attachment() {
    let package = Package::new();
    let root = root(&package);
    let session = Session::detached(&package, &root);
    let mut steps = register("p", &session, &package, 1);
    steps.extend(attach("i", &session, &package, 2));
    steps.extend([
        on("i", json!({ "do": "shutdown" })),
        json!({ "do": "finish", "instance": "i", "as": "departed" }),
        on("p", json!({ "do": "shutdown" })),
        json!({ "do": "finish", "instance": "p", "as": "left" }),
        json!({ "do": "probe", "socket": session.socket(), "as": "after" }),
    ]);
    let report = script(&package, &root, json!(steps));
    let held = assert_attached(&drained(&report, &steps, "attached-i"), 1, 2, [100, 30]);
    let (rest, terminal) = finished(labelled(&report, &steps, "departed"));
    assert_released(&rest, 2, held);
    assert_detached(terminal, 2);
    let (records, terminal) = finished(labelled(&report, &steps, "left"));
    let promoted = assert_restored(&records, 1, SUCCESSOR, [100, 30]);
    assert!(promoted[0] > held[0], "a new period");
    let complete = records
        .iter()
        .position(|record| record["tag"] == RESTORATION_COMPLETE)
        .unwrap();
    assert_released(&records[complete + 1..], 1, promoted);
    assert_detached(terminal, 1);
    let detached = json!({ "lifecycle": "detached", "attachments": 0, "controlled": false });
    assert_eq!(labelled(&report, &steps, "after")["state"], detached);
    assert_all_released(&report["ledger"]);
    session.end();
    package.assert_clean();
}
