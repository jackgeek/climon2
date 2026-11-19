//! Library layer (#95): status, viewing and acknowledgement through the
//! installed client library's frozen C ABI, against real session daemons
//! started through the packaged CLI with the daemon's controllable status
//! clock (`CLIMON2_DAEMON_CLOCK`, a test hook), so every status change
//! happens at an exact, hand-computed instant: the first sample is due at
//! most 1,500 ms after start, and ten seconds without change after the
//! sample that saw agent work request attention. Expected values come from
//! the native header, the native contract and the daemon wire reference,
//! never from another run of the library. Fault cases against the host's
//! fake daemon peer are in `status_faults.rs`. Release archives have no
//! test hooks, so these cases run only on the workspace's hooked build.
//! The ordinary client's own viewing and acknowledgement (#102) is observed
//! here too, through such a library instance watching the session.

#[allow(dead_code)]
mod support;

use climon2_test_term::Terminal;
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
use support::{
    Package, TIMEOUT,
    control::*,
    library::{archive, observe_within, scenario_command},
    passive::*,
    status::*,
};

/// The controllable clock's start, the first sample, and ten seconds later.
const AT_START: &str = "2027-01-15T08:00:00.000Z";
const AT_SAMPLE: &str = "2027-01-15T08:00:01.500Z";
const AT_DEADLINE: &str = "2027-01-15T08:00:11.500Z";
/// Ten seconds after the sample at 13 s saw more agent work (#102).
const AT_SECOND_DEADLINE: &str = "2027-01-15T08:00:23.000Z";

/// What a monitor that requires status asks for, and what it selects from a
/// real daemon.
const WATCHING: [&str; 3] = ["daemon-wire-v1", "monitor-v1", "status-v1"];
const WATCHED: [&str; 4] = ["daemon-wire-v1", "monitor-v1", "status-v1", "title-v1"];

/// `step` for instance `name`.
fn on(name: &str, mut step: Value) -> Value {
    step["instance"] = json!(name);
    step
}

/// The steps that register instance `name` with `generation`, requiring
/// status, and drain through its Initial metadata, labelled
/// `<name>-initial`.
fn watch(name: &str, session: &Session, package: &Package, generation: u64) -> Vec<Value> {
    let startup = session.startup(package, generation);
    vec![
        json!({ "do": "create", "instance": name, "startup": startup }),
        on(
            name,
            json!({ "do": "submit", "request": open(1, &WATCHING, &["title-v1"]) }),
        ),
        on(name, json!({ "do": "run" })),
        on(
            name,
            json!({ "do": "drain", "until": { "tag": INITIAL_METADATA },
                    "as": format!("{name}-initial") }),
        ),
    ]
}

/// The steps that attach instance `name` interactively at the session's
/// 120x30 with `generation`, asking for status, and drain through its first
/// Restoration complete, labelled `<name>-attached`.
fn attach(name: &str, session: &Session, package: &Package, generation: u64) -> Vec<Value> {
    let startup = session.startup(package, generation);
    vec![
        json!({ "do": "create", "instance": name, "startup": startup }),
        on(
            name,
            json!({ "do": "submit", "request": interactive_status(1, 120, 30) }),
        ),
        on(name, json!({ "do": "run" })),
        on(
            name,
            json!({ "do": "drain", "until": { "tag": RESTORATION_COMPLETE },
                    "as": format!("{name}-attached") }),
        ),
    ]
}

/// Each instance in `names` drains through its next Status, labelled
/// `<name>-<label>`.
fn statuses(names: &[&str], label: &str) -> Vec<Value> {
    names
        .iter()
        .map(|name| {
            on(
                name,
                json!({ "do": "drain", "until": { "tag": STATUS },
                        "as": format!("{name}-{label}") }),
            )
        })
        .collect()
}

/// The selected tokens of an Opened result, sorted.
fn selection(result: &Value) -> Vec<String> {
    let mut selected: Vec<String> = result["selected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|token| token.as_str().unwrap().to_owned())
        .collect();
    selected.sort_unstable();
    selected
}

/// The Initial metadata among `records`, after the Open's success
/// selecting `selected`: its status.
fn initial_status<'a>(records: &'a [Value], generation: u64, selected: &[&str]) -> &'a Value {
    let opened = assert_ok(&records[0], generation, 1, OPENED);
    assert_eq!(selection(opened), selected, "{records:#?}");
    assert_header(&records[1], INITIAL_METADATA, generation);
    &records[1]["metadata"]["status"]
}

/// The one Status among `records` is `expected`.
fn assert_status(records: &[Value], generation: u64, expected: &Value) {
    let found = tagged(records, STATUS);
    assert_eq!(found.len(), 1, "{records:#?}");
    assert_header(found[0], STATUS, generation);
    assert_eq!(&found[0]["status"], expected, "{records:#?}");
}

/// A session that ended with exit 0 while the instance was open: its
/// records hold one `CM1_FINAL` with the final status at `changed_at`,
/// completed and with no episode, and no Status at all, and the daemon's
/// close stopped the stream cleanly.
fn assert_completed(
    report: &Value,
    steps: &[Value],
    label: &str,
    generation: u64,
    changed_at: &str,
) {
    let (records, terminal) = finished(labelled(report, steps, label));
    assert!(tagged(&records, STATUS).is_empty(), "{records:#?}");
    let finals = tagged(&records, FINAL);
    assert_eq!(finals.len(), 1, "{records:#?}");
    assert_header(finals[0], FINAL, generation);
    assert_eq!(
        finals[0]["final"]["command_result"],
        native_command(EXIT, 0, None)
    );
    assert_eq!(
        finals[0]["final"]["status"],
        available(status(COMPLETED, None, changed_at)),
        "{records:#?}"
    );
    assert_terminal(
        terminal,
        generation,
        (CLIENT_STOPPED, RETRY_NONE, LEAVE_NOT_REQUIRED),
        None,
    );
}

/// STA-07 Library (status records keep their episode and time identity
/// across a reconnect, and nothing is fabricated: each Initial metadata
/// reports the stored status and time, idle since the daemon started, not
/// the connection's time; the agent work the first sample saw is running
/// at that sample's time, and ten seconds later attention is pending as
/// episode 1 at that time, with no idle in between; a fresh instance of a
/// new generation then reports exactly that episode and time in its fresh
/// Initial metadata, never idle or cleared; the final status is only the
/// Command final's, completed at the time the result was published, and no
/// Status follows it), CTL-03 Library (status case: the same status reaches
/// a passive registration, an attachment revoked by takeover and a monitor
/// controlling in the background without Ready or viewing, whatever the
/// control) and STA-08 Library (viewing false is accepted from a background
/// controller during its restoration, before readiness: no change, since a
/// new period starts not viewing).
#[test]
fn installed_library_relays_status_to_every_role_and_keeps_its_identity_across_instances() {
    if archive() {
        return;
    }
    let package = Package::new();
    let root = root(&package);
    let clock = root.join("clock");
    start_clock(&clock);
    let mut session = Session::clocked(&package, &root, &clock);
    let mut steps = watch("p", &session, &package, 1);
    steps.extend(attach("c", &session, &package, 2));
    steps.extend(watch("b", &session, &package, 3));
    steps.extend([
        on("b", json!({ "do": "submit", "request": acquire(2, 3) })),
        on(
            "b",
            json!({ "do": "drain", "until": { "all": [
                { "response": 2 }, { "tag": RESTORATION_COMPLETE }] }, "as": "b-acquired" }),
        ),
        on(
            "c",
            json!({ "do": "drain", "until": { "tag": REVOKED }, "as": "c-revoked" }),
        ),
        on("b", json!({ "do": "submit", "request": view(3, 3, 0) })),
        on(
            "b",
            json!({ "do": "drain", "until": { "response": 3 }, "as": "b-background" }),
        ),
        advance(&clock, 1_500),
    ]);
    steps.extend(statuses(&["p", "c", "b"], "running"));
    steps.push(advance(&clock, 11_500));
    steps.extend(statuses(&["p", "c", "b"], "pending"));
    steps.extend([
        on("p", json!({ "do": "shutdown" })),
        on("p", json!({ "do": "finish", "as": "p-left" })),
    ]);
    steps.extend(watch("q", &session, &package, 4));
    steps.extend([
        on(
            "b",
            json!({ "do": "submit", "request": ready(4, 3, CURRENT) }),
        ),
        on("b", json!({ "do": "drain", "until": { "response": 4 } })),
        json!({ "do": "touch", "path": session.end_path() }),
        on("q", json!({ "do": "finish", "as": "q-ended" })),
        on("b", json!({ "do": "finish", "as": "b-ended" })),
        on("c", json!({ "do": "finish", "as": "c-ended" })),
    ]);
    let report = script(&package, &root, json!(steps));
    for (index, submitted) in steps.iter().enumerate() {
        if submitted["do"] == "submit" {
            assert_success(step(&report, index));
        }
        if submitted["do"] == "clock" {
            assert_eq!(
                step(&report, index),
                &json!(true),
                "the daemon read the clock"
            );
        }
    }
    let idle = available(status(IDLE, None, AT_START));
    let p = drained(&report, &steps, "p-initial");
    assert_eq!(initial_status(&p, 1, &WATCHED), &idle);
    let c = drained(&report, &steps, "c-attached");
    let mut selected = ["daemon-wire-v1", "focus-v1", "status-v1", "title-v1"];
    selected.sort_unstable();
    assert_eq!(initial_status(&c, 2, &selected), &idle);
    let b = drained(&report, &steps, "b-initial");
    assert_eq!(initial_status(&b, 3, &WATCHED), &idle);
    let acquired = drained(&report, &steps, "b-acquired");
    let authority = assert_restored(&acquired, 3, TAKE_CONTROL, [120, 30]);
    assert_authority(
        assert_ok(answer(&acquired, 2), 3, 2, ACQUIRED),
        1,
        authority,
    );
    let revoked = drained(&report, &steps, "c-revoked");
    assert_eq!(
        presented(tagged(&revoked, REVOKED)[0], REVOKED, 2)["reason"],
        TAKEOVER
    );
    let background = drained(&report, &steps, "b-background");
    assert_eq!(
        assert_ok(answer(&background, 3), 3, 3, VIEWING_RESULT),
        &viewed(0, 0)
    );
    let running = status(RUNNING, None, AT_SAMPLE);
    let pending = status(NEEDS_ATTENTION, Some(1), AT_DEADLINE);
    for (name, generation) in [("p", 1), ("c", 2), ("b", 3)] {
        let label = format!("{name}-running");
        assert_status(&drained(&report, &steps, &label), generation, &running);
        let label = format!("{name}-pending");
        assert_status(&drained(&report, &steps, &label), generation, &pending);
    }
    let (left, terminal) = finished(labelled(&report, &steps, "p-left"));
    assert!(left.is_empty(), "{left:#?}");
    assert_left(terminal, 1);
    let q = drained(&report, &steps, "q-initial");
    assert_eq!(initial_status(&q, 4, &WATCHED), &available(pending));
    for (name, generation) in [("q", 4), ("b", 3), ("c", 2)] {
        let label = format!("{name}-ended");
        assert_completed(&report, &steps, &label, generation, AT_DEADLINE);
    }
    assert_all_released(&report["ledger"]);
    assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
    package.assert_clean();
}

/// STA-08 Library (Set viewing and Acknowledge with their authority-checked
/// results from a real daemon: before readiness viewing true and
/// Acknowledge are `not_ready`, while viewing false is accepted during the
/// restoration; once ready, Acknowledge needs the latest viewing true
/// (`not_viewing`); viewing true then changes once and repeats unchanged;
/// with nothing pending Acknowledge is an explicit no-change success, with
/// no Status; pending attention becomes acknowledged with the same episode
/// and time, its Status before the changed success, and a second
/// Acknowledge changes nothing; viewing false changes viewing but never
/// the status; after another instance takes over, the old authority is
/// `not_controller` for both; a new period after Acquire starts not
/// viewing, so Acknowledge is `not_viewing` again until viewing true;
/// accepted real Input then clears the acknowledged attention, its Status
/// running before its acceptance) and COM-03 Library (viewing and
/// acknowledgement cases: a request naming another generation is rejected
/// locally, `stale_generation`).
#[test]
fn installed_library_sets_viewing_and_acknowledges_attention_through_a_real_daemon() {
    if archive() {
        return;
    }
    let package = Package::new();
    let root = root(&package);
    let clock = root.join("clock");
    start_clock(&clock);
    let mut session = Session::clocked(&package, &root, &clock);
    let mut steps = attach("a", &session, &package, 5);
    let before = json!([
        { "do": "submit", "request": view(2, 5, 1) },
        { "do": "submit", "request": acknowledge(3, 5, CURRENT) },
        { "do": "submit", "request": view(4, 5, 0) },
        { "do": "submit", "request": ready(5, 5, CURRENT) },
        { "do": "submit", "request": acknowledge(6, 5, CURRENT) },
        { "do": "submit", "request": view(7, 5, 1) },
        { "do": "submit", "request": view(8, 5, 1) },
        { "do": "submit", "request": acknowledge(9, 5, CURRENT) },
        { "do": "submit", "request": view(10, 9, 1) },
        { "do": "drain", "until": { "all": [{ "response": 9 }, { "response": 10 }] },
          "as": "before" },
    ]);
    steps.extend(
        before
            .as_array()
            .unwrap()
            .iter()
            .map(|step| on("a", step.clone())),
    );
    steps.extend([
        advance(&clock, 1_500),
        on(
            "a",
            json!({ "do": "drain", "until": { "tag": STATUS }, "as": "running" }),
        ),
        advance(&clock, 11_500),
        on(
            "a",
            json!({ "do": "drain", "until": { "tag": STATUS }, "as": "pending" }),
        ),
    ]);
    let pending = json!([
        { "do": "submit", "request": acknowledge(11, 5, CURRENT) },
        { "do": "drain", "until": { "response": 11 }, "as": "acknowledged" },
        { "do": "submit", "request": acknowledge(12, 5, CURRENT) },
        { "do": "submit", "request": view(13, 5, 0) },
        { "do": "drain", "until": { "response": 13 }, "as": "again" },
    ]);
    steps.extend(
        pending
            .as_array()
            .unwrap()
            .iter()
            .map(|step| on("a", step.clone())),
    );
    steps.extend(attach("t", &session, &package, 6));
    let taken = json!([
        { "do": "drain", "until": { "tag": REVOKED }, "as": "taken" },
        { "do": "submit", "request": view(14, 5, 1) },
        { "do": "submit", "request": acknowledge(15, 5, CURRENT) },
        { "do": "submit", "request": acquire(16, 5) },
        { "do": "drain", "until": { "all": [
            { "response": 16 }, { "tag": RESTORATION_COMPLETE }] }, "as": "regained" },
        { "do": "submit", "request": ready(17, 5, CURRENT) },
        { "do": "submit", "request": acknowledge(18, 5, CURRENT) },
        { "do": "submit", "request": view(19, 5, 1) },
        { "do": "submit", "request": input(20, 5, CURRENT, "41") },
        { "do": "drain", "until": { "response": 20 }, "as": "typed" },
        { "do": "shutdown" },
        { "do": "finish", "as": "finished-a" },
    ]);
    steps.extend(
        taken
            .as_array()
            .unwrap()
            .iter()
            .map(|step| on("a", step.clone())),
    );
    steps.extend([
        on("t", json!({ "do": "shutdown" })),
        on("t", json!({ "do": "finish", "as": "finished-t" })),
    ]);
    let report = script(&package, &root, json!(steps));
    for (index, submitted) in steps.iter().enumerate() {
        if submitted["do"] == "submit" {
            assert_success(step(&report, index));
        }
    }
    let attached = drained(&report, &steps, "a-attached");
    let first = assert_restored(&attached[2..], 5, ATTACH, [120, 30]);
    let early = drained(&report, &steps, "before");
    assert_rejected(answer(&early, 2), 5, 2, ORIGIN_DAEMON, "not_ready");
    assert_rejected(answer(&early, 3), 5, 3, ORIGIN_DAEMON, "not_ready");
    assert_eq!(
        assert_ok(answer(&early, 4), 5, 4, VIEWING_RESULT),
        &viewed(0, 0)
    );
    assert_authority(assert_ok(answer(&early, 5), 5, 5, READY_RESULT), 1, first);
    assert_rejected(answer(&early, 6), 5, 6, ORIGIN_DAEMON, "not_viewing");
    assert_eq!(
        assert_ok(answer(&early, 7), 5, 7, VIEWING_RESULT),
        &viewed(1, 1)
    );
    assert_eq!(
        assert_ok(answer(&early, 8), 5, 8, VIEWING_RESULT),
        &viewed(0, 1)
    );
    assert_eq!(
        assert_ok(answer(&early, 9), 5, 9, ACKNOWLEDGEMENT),
        &acknowledged(0)
    );
    assert_rejected(
        answer(&early, 10),
        5,
        10,
        ORIGIN_LIBRARY,
        "stale_generation",
    );
    assert!(tagged(&early, STATUS).is_empty(), "{early:#?}");
    let running = status(RUNNING, None, AT_SAMPLE);
    assert_status(&drained(&report, &steps, "running"), 5, &running);
    let needs = status(NEEDS_ATTENTION, Some(1), AT_DEADLINE);
    assert_status(&drained(&report, &steps, "pending"), 5, &needs);
    let acknowledging = drained(&report, &steps, "acknowledged");
    assert_eq!(
        tags(&acknowledging),
        [STATUS, RESPONSE],
        "{acknowledging:#?}"
    );
    let kept = status(ACKNOWLEDGED, Some(1), AT_DEADLINE);
    assert_status(&acknowledging, 5, &kept);
    assert_eq!(
        assert_ok(&acknowledging[1], 5, 11, ACKNOWLEDGEMENT),
        &acknowledged(1)
    );
    let again = drained(&report, &steps, "again");
    assert_eq!(tags(&again), [RESPONSE, RESPONSE], "{again:#?}");
    assert_eq!(
        assert_ok(answer(&again, 12), 5, 12, ACKNOWLEDGEMENT),
        &acknowledged(0)
    );
    assert_eq!(
        assert_ok(answer(&again, 13), 5, 13, VIEWING_RESULT),
        &viewed(1, 0)
    );
    let taken = drained(&report, &steps, "taken");
    assert_eq!(
        ids(presented(tagged(&taken, REVOKED)[0], REVOKED, 5)),
        first
    );
    let regained = drained(&report, &steps, "regained");
    assert_rejected(
        answer(&regained, 14),
        5,
        14,
        ORIGIN_DAEMON,
        "not_controller",
    );
    assert_rejected(
        answer(&regained, 15),
        5,
        15,
        ORIGIN_DAEMON,
        "not_controller",
    );
    let second = assert_restored(&regained, 5, TAKE_CONTROL, [120, 30]);
    assert!(second[0] > first[0], "a new period");
    let typed = drained(&report, &steps, "typed");
    assert_authority(
        assert_ok(answer(&typed, 17), 5, 17, READY_RESULT),
        1,
        second,
    );
    assert_rejected(answer(&typed, 18), 5, 18, ORIGIN_DAEMON, "not_viewing");
    assert_eq!(
        assert_ok(answer(&typed, 19), 5, 19, VIEWING_RESULT),
        &viewed(1, 1)
    );
    let cleared = status(RUNNING, None, AT_DEADLINE);
    assert_status(&typed, 5, &cleared);
    let order: Vec<u64> = typed
        .iter()
        .filter(|record| record["tag"] == STATUS || record["response"]["request_id"] == 20)
        .map(|record| record["tag"].as_u64().unwrap())
        .collect();
    assert_eq!(order, [STATUS, RESPONSE], "{typed:#?}");
    let accepted = assert_ok(answer(&typed, 20), 5, 20, INPUT_ACCEPTED);
    assert_eq!(accepted["accepted_bytes"], 1, "{accepted:#}");
    let (_, terminal) = finished(labelled(&report, &steps, "finished-a"));
    assert_detached(terminal, 5);
    let (_, terminal) = finished(labelled(&report, &steps, "finished-t"));
    assert_eq!(terminal["leave"], LEAVE_CONFIRMED, "{terminal:#}");
    assert_all_released(&report["ledger"]);
    session.end();
    assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
    package.assert_clean();
}

/// How long a host whose steps wait on the test's clients may run: each of
/// its waits is bounded on its own, by 30 s at most.
const HOST_BOUND: Duration = Duration::from_secs(120);

/// A host step that waits until the test creates `path`.
fn await_step(path: &Path) -> Value {
    json!({ "do": "await_file", "path": path.to_str().unwrap() })
}

fn touch_step(path: &Path) -> Value {
    json!({ "do": "touch", "path": path.to_str().unwrap() })
}

/// Waits, within [`TIMEOUT`], until the host creates `path`.
fn await_host(path: &Path) {
    let deadline = Instant::now() + TIMEOUT;
    while !path.exists() {
        assert!(Instant::now() < deadline, "the host never made {path:?}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// An ordinary `climon2 attach` client of session `id` at the session's own
/// 120x30, so attaching never resizes the session.
fn ordinary_attach(package: &Package, id: &str) -> Terminal {
    let mut command = package.command("climon2");
    command.args(["attach", id]);
    Terminal::spawn(command, 30, 120).unwrap()
}

/// Derived scenario (spec #73, Client executable: viewing and
/// acknowledgement, #102), STA-08 ordinary-attach case against a real
/// daemon, watched by a passive library instance: pending attention
/// (episode 1) stays pending while the session's own ready client merely
/// shows it; an explicit `climon2 attach` moves it to acknowledged, same
/// episode, once that client is ready; new agent work then requests
/// attention again (episode 2), which the attached client, viewing, does
/// not acknowledge; the first client's take-control key (a takeover) then
/// acknowledges it once ready. Both clients end with the command's status.
#[test]
fn ordinary_attach_and_takeover_acknowledge_pending_attention_once_ready() {
    if archive() {
        return;
    }
    let package = Package::new();
    let root = root(&package);
    let clock = root.join("clock");
    start_clock(&clock);
    let work = root.join("work");
    let mut session = Session::clocked_working(&package, &root, &clock, &work);
    let [pending, attached, worked, again, taken] =
        ["pending", "attached", "worked", "again", "taken"].map(|name| root.join(name));
    let mut steps = watch("w", &session, &package, 1);
    steps.push(advance(&clock, 1_500));
    steps.extend(statuses(&["w"], "running"));
    steps.push(advance(&clock, 11_500));
    steps.extend(statuses(&["w"], "pending"));
    steps.extend([touch_step(&pending), await_step(&attached)]);
    steps.extend(statuses(&["w"], "attached"));
    steps.extend([
        await_step(&worked),
        advance(&clock, 13_000),
        advance(&clock, 23_000),
    ]);
    steps.extend(statuses(&["w"], "again"));
    steps.extend([touch_step(&again), await_step(&taken)]);
    steps.extend(statuses(&["w"], "taken"));
    steps.extend([
        on("w", json!({ "do": "shutdown" })),
        on("w", json!({ "do": "finish", "as": "w-left" })),
    ]);
    let (report, mut second) = thread::scope(|scope| {
        let input = json!({ "root": root.to_str().unwrap(), "steps": steps });
        let command = scenario_command(&package, "script", &input);
        let host = scope.spawn(|| observe_within(command, HOST_BOUND));
        await_host(&pending);
        let mut second = ordinary_attach(&package, &session.id);
        second.wait_for_text("ECHO-READY", TIMEOUT).unwrap();
        session
            .terminal
            .wait_for_text("take control", TIMEOUT)
            .unwrap();
        fs::write(&attached, b"").unwrap();
        fs::write(&work, b"").unwrap();
        second.wait_for_text("MORE", TIMEOUT).unwrap();
        fs::write(&worked, b"").unwrap();
        await_host(&again);
        session.terminal.send(b"\x14").unwrap();
        session.terminal.wait_for_text("MORE", TIMEOUT).unwrap();
        second.wait_for_text("take control", TIMEOUT).unwrap();
        fs::write(&taken, b"").unwrap();
        (host.join().unwrap(), second)
    });
    for (index, done) in steps.iter().enumerate() {
        if ["clock", "await_file"].contains(&done["do"].as_str().unwrap()) {
            assert_eq!(step(&report, index), &json!(true), "{done}");
        }
    }
    let initial = drained(&report, &steps, "w-initial");
    assert_eq!(
        initial_status(&initial, 1, &WATCHED),
        &available(status(IDLE, None, AT_START))
    );
    let expected = [
        ("running", status(RUNNING, None, AT_SAMPLE)),
        ("pending", status(NEEDS_ATTENTION, Some(1), AT_DEADLINE)),
        ("attached", status(ACKNOWLEDGED, Some(1), AT_DEADLINE)),
        (
            "again",
            status(NEEDS_ATTENTION, Some(2), AT_SECOND_DEADLINE),
        ),
        ("taken", status(ACKNOWLEDGED, Some(2), AT_SECOND_DEADLINE)),
    ];
    for (label, status) in expected {
        let records = drained(&report, &steps, &format!("w-{label}"));
        assert_status(&records, 1, &status);
    }
    let (left, terminal) = finished(labelled(&report, &steps, "w-left"));
    assert!(left.is_empty(), "{left:#?}");
    assert_left(terminal, 1);
    assert_all_released(&report["ledger"]);
    session.end();
    assert_eq!(session.terminal.wait_exit(TIMEOUT).unwrap(), 0);
    assert_eq!(second.wait_exit(TIMEOUT).unwrap(), 0);
    package.assert_clean();
}
