//! #502 (spec #488): the Pair layer of ADV-01, ADV-03, ADV-04, ADV-05 and
//! SEL-01. Packaged relays, each in its own climon2 home with real session
//! daemons, advertise their sessions to a packaged hub monitor over
//! loopback, and the hub monitor lists them in its selector beside its
//! local sessions, with a machine column. Link peer stand-ins play relays
//! whose sessions share an ID with a local one. Expectations come from the
//! advertisement (#195), selector (#199) and limits (#412) resolutions.
#![cfg(any(target_os = "macos", target_os = "linux"))]

#[allow(dead_code)]
mod support;

use climon2_link::{Advertisement, Session};
use climon2_test_term::Terminal;
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use serde_json::Value;
use std::{fs, thread, time::Duration};
use support::{
    Package, TIMEOUT, kill_session,
    pair::{RelayHome, headless, links, links_file, pair_piped},
    stand_in::StandInRelay,
};

const ROWS: u16 = 30;
const COLS: u16 = 120;
/// The preference file a release writes once Don't show again is checked.
const SUPPRESSED: &str = "{\"version\":1,\"suppress_key_introduction\":true}\n";
/// The headless session every relay home starts with.
const RELAYED: &str = "/bin/bash -c until";

/// Pairs `hub` with `relay`, as `c2 monitor pair` does.
fn pair(hub: &Package, relay: &mut RelayHome) {
    let pin = relay.mint();
    let output = pair_piped(hub, &relay.listen().to_string(), &format!("{pin}\n"), &[]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

/// Renames the relays `hub` remembers, in the order they were paired: the
/// name a hub monitor's user chose for each.
fn rename(hub: &Package, names: &[&str]) {
    let mut file = links_file(hub, "relays.json").unwrap();
    let relays = file["relays"].as_array_mut().unwrap();
    assert_eq!(relays.len(), names.len());
    for (relay, name) in relays.iter_mut().zip(names) {
        relay["name"] = Value::from(*name);
    }
    fs::write(links(hub).join("relays.json"), file.to_string()).unwrap();
}

/// The packaged hub monitor in a terminal of its own, without its key
/// introduction or key hints, with its selector open.
fn hub_monitor(hub: &Package) -> Terminal {
    let home = hub.isolation.climon2_home();
    fs::write(home.join("monitor-preferences.json"), SUPPRESSED).unwrap();
    let config = r#"{"monitor": {"show_key_hints": false}}"#;
    fs::write(home.join("config.jsonc"), config).unwrap();
    let mut command = hub.command("c2");
    command.arg("monitor");
    let mut terminal = Terminal::spawn(command, ROWS, COLS).unwrap();
    terminal.send("\x1ds").unwrap();
    terminal.wait_for_text("╟", TIMEOUT).unwrap();
    terminal
}

/// The selector's rows on `screen`: what lies between the side borders of
/// each line below its separator.
fn rows(screen: &str) -> Vec<String> {
    let below = screen
        .lines()
        .skip_while(|line| !line.contains('╟'))
        .skip(1);
    let inside = |line: &str| {
        let (start, end) = (line.find('║')?, line.rfind('║')?);
        Some(line[start + '║'.len_utf8()..end].trim_end().to_owned())
    };
    below
        .take_while(|line| !line.contains('╚'))
        .filter_map(inside)
        .filter(|row| !row.is_empty())
        .collect()
}

/// Waits until the selector lists exactly one row with each of `wanted`'s
/// texts, each row's texts in order, and no other row.
fn lists(terminal: &mut Terminal, wanted: &[&[&str]]) {
    let matches = |row: &str, texts: &[&str]| {
        let mut rest = row;
        texts.iter().all(|text| match rest.find(text) {
            Some(at) => {
                rest = &rest[at + text.len()..];
                true
            }
            None => false,
        })
    };
    let listed = |screen: &str| {
        let rows = rows(screen);
        rows.len() == wanted.len()
            && wanted
                .iter()
                .all(|texts| rows.iter().filter(|row| matches(row, texts)).count() == 1)
    };
    if terminal.wait_for_screen(TIMEOUT, listed).is_err() {
        panic!("wanted {wanted:?}, listed {:?}", rows(&terminal.screen()));
    }
}

/// Leaves the hub monitor with prefix+x.
fn leave(mut terminal: Terminal) {
    terminal.send("\x1dx").unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
}

/// ADV-01, ADV-03 and SEL-01 at the Pair layer: two relays, one of them
/// running inside a session of its own, advertise every live session in
/// their homes but that one. The hub monitor lists them in one list beside
/// its local session, with a machine column between status and title:
/// `local`, or the name it knows each relay by. A session started on a
/// relay's machine after the link appears, and one ended there goes.
/// Searching a relay's name lists only its sessions.
#[test]
fn adv_01_two_relays_sessions_join_the_hub_monitors_list_by_machine() {
    let hub = Package::new();
    let local = headless(&hub, &["/bin/sleep", "120"]);
    let mut zebra = RelayHome::start_in_session();
    let mut yak = RelayHome::start();
    pair(&hub, &mut zebra);
    pair(&hub, &mut yak);
    rename(&hub, &["zebra", "yak"]);

    let mut monitor = hub_monitor(&hub);
    lists(
        &mut monitor,
        &[
            &["local", "/bin/sleep 120"],
            &["Unavailable", "zebra", RELAYED],
            &["Unavailable", "yak", RELAYED],
        ],
    );
    assert!(!monitor.screen().contains("relay --listen"), "own session");

    let later = headless(&yak.package, &["/bin/sleep", "121"]);
    lists(
        &mut monitor,
        &[
            &["local", "/bin/sleep 120"],
            &["zebra", RELAYED],
            &["yak", RELAYED],
            &["yak", "/bin/sleep 121"],
        ],
    );
    kill_session(&yak.package, &later);
    lists(
        &mut monitor,
        &[
            &["local", "/bin/sleep 120"],
            &["zebra", RELAYED],
            &["yak", RELAYED],
        ],
    );

    monitor.send("zebra").unwrap();
    lists(&mut monitor, &[&["zebra", RELAYED]]);
    monitor.send("\x7f\x7f\x7f\x7f\x7fyak").unwrap();
    lists(&mut monitor, &[&["yak", RELAYED]]);

    leave(monitor);
    kill_session(&hub, &local);
    zebra.stop();
    yak.stop();
    hub.assert_clean();
}

/// The process ID of the daemon of `package`'s session `id`.
fn daemon(package: &Package, id: &str) -> Pid {
    let path = package
        .isolation
        .climon2_home()
        .join("sessions")
        .join(format!("{id}.pty"));
    let metadata: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    Pid::from_raw(metadata["daemon_pid"].as_i64().unwrap() as i32)
}

/// ADV-04 at the Pair layer: a session whose daemon stops answering is not
/// withdrawn, and goes once discovery proves it stale; the row was never
/// connected, so it is dropped. After the relay restarts, the hub monitor
/// redials, discards what the relay held and takes its fresh snapshot, so
/// a session ended while the link was down is gone.
#[test]
fn adv_04_only_proof_withdraws_a_session_and_a_reconnect_takes_a_fresh_snapshot() {
    let hub = Package::new();
    let mut relay = RelayHome::start();
    pair(&hub, &mut relay);
    rename(&hub, &["yak"]);
    let stopped = headless(&relay.package, &["/bin/sleep", "121"]);
    let ended = headless(&relay.package, &["/bin/sleep", "122"]);
    let mut monitor = hub_monitor(&hub);
    let all: [&[&str]; 3] = [
        &["yak", RELAYED],
        &["yak", "/bin/sleep 121"],
        &["yak", "/bin/sleep 122"],
    ];
    lists(&mut monitor, &all);

    let paused = daemon(&relay.package, &stopped);
    kill(paused, Signal::SIGSTOP).unwrap();
    thread::sleep(Duration::from_secs(6));
    lists(&mut monitor, &all);
    kill(paused, Signal::SIGCONT).unwrap();
    kill_session(&relay.package, &stopped);
    lists(
        &mut monitor,
        &[&["yak", RELAYED], &["yak", "/bin/sleep 122"]],
    );

    relay.restart(|package| kill_session(package, &ended));
    lists(&mut monitor, &[&["yak", RELAYED]]);

    leave(monitor);
    relay.stop();
    hub.assert_clean();
}

/// A session a stand-in advertises: `id` running `command`.
fn advertised(id: &str, command: &str) -> Advertisement {
    let id: climon2_core::SessionId = id.parse().unwrap();
    Advertisement::Session(Session {
        id: id.bytes(),
        encoding: 1,
        command: command.as_bytes().to_vec(),
        arguments: Vec::new(),
        working_directory: b"/".to_vec(),
        depth: 1,
        created_at: *b"2026-11-27T09:30:00.000Z",
        release: "0.0.0-stand-in".to_owned(),
    })
}

/// ADV-05 at the Pair layer: two stand-in relays each advertise a session
/// with the session ID of the hub monitor's local session. The three are
/// three rows, never merged.
#[test]
fn adv_05_one_session_id_locally_and_on_two_relays_is_three_rows() {
    let hub = Package::new();
    let local = headless(&hub, &["/bin/sleep", "120"]);
    let alpha = StandInRelay::advertising(vec![advertised(&local, "alpha-copy")]);
    let beta = StandInRelay::advertising(vec![advertised(&local, "beta-copy")]);
    for stand_in in [&alpha, &beta] {
        let pin = format!("{}\n", stand_in.pin());
        let output = pair_piped(&hub, &stand_in.address().to_string(), &pin, &[]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }
    rename(&hub, &["alpha", "beta"]);
    let mut monitor = hub_monitor(&hub);
    lists(
        &mut monitor,
        &[
            &["local", "/bin/sleep 120"],
            &["alpha", "alpha-copy"],
            &["beta", "beta-copy"],
        ],
    );
    leave(monitor);
    alpha.finish();
    beta.finish();
    kill_session(&hub, &local);
    hub.assert_clean();
}
