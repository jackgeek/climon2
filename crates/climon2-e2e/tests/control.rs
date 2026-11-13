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

/// The departure's release: one Display under the held authority that
/// resets the modes the application set, focus reporting (`ESC [ ? 1004 l`)
/// among them, before the Leave's confirmation.
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
        &assert_ok(answer(readied, 2), 3, 2, READY_RESULT),
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
