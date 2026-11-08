//! Spec #73 / #74: the packaged monitor, reached as `climon2 monitor` or
//! `climon2 mon` through the installed launcher, driven through real PTYs from
//! outer shells with isolated homes (the Monitor layer). Every scenario ends
//! with `assert_clean()`.
#![cfg(target_os = "macos")]

#[allow(dead_code)]
mod support;

use climon2_test_term::{Modes, Terminal};
use std::{fs, path::Path};
use support::{Package, TIMEOUT};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const ROWS: u16 = 30;
const COLS: u16 = 120;
const TITLE: &str = "OUTER-TITLE";
const BEFORE: &[u8] = b"BEFORE-MONITOR\r\n";
const DONE: &[u8] = b"OUTER-DONE=";
const USAGE: &str = "climon2: error: usage: climon2 exec <command> [args...] | attach <session-id> | kill <session-id> | list [--json] | monitor | mon\r\n";

/// An outer non-interactive shell that titles its host, records termios around
/// `command`, reports its status, then echoes one ordinary typed line.
fn outer(package: &Package, command: &str) -> Terminal {
    let mut shell = package.command("/bin/bash");
    shell.args(["--noprofile", "--norc", "-c"]);
    shell.arg(format!(
        "printf '\\033]2;{TITLE}\\007'; stty -g > ~/before; printf 'BEFORE-MONITOR\\n'; \
         {command}; s=$?; stty -g > ~/after; printf 'OUTER-DONE=%s\\n' \"$s\"; \
         IFS= read -r line; printf 'TYPED=%s\\n' \"$line\""
    ));
    Terminal::spawn(shell, ROWS, COLS).unwrap()
}

/// After `command` returned `status`: the documented normal-shell baseline,
/// exact termios, the outer title untouched and ordinary shell input.
fn assert_released(package: &Package, terminal: &mut Terminal, status: i32) {
    terminal
        .wait_for_text(&format!("OUTER-DONE={status}"), TIMEOUT)
        .unwrap();
    assert_eq!(terminal.modes(), Modes::BASELINE, "{}", terminal.screen());
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    let home = package.isolation.home();
    assert_eq!(
        termios(&home.join("after")),
        termios(&home.join("before")),
        "termios restored"
    );
    terminal.send("plain words\n").unwrap();
    terminal
        .wait_for_text("TYPED=plain words", TIMEOUT)
        .unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
}

/// `stty -g` settings without macOS's transient `PENDIN`, which the kernel
/// sets whenever a terminal returns to canonical mode.
fn termios(path: &Path) -> String {
    const PENDIN: u64 = 0x2000_0000;
    fs::read_to_string(path)
        .unwrap()
        .trim_end()
        .split(':')
        .map(|field| match field.strip_prefix("lflag=") {
            Some(flags) => {
                let flags = u64::from_str_radix(flags, 16).unwrap() & !PENDIN;
                format!("lflag={flags:x}")
            }
            None => field.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(":")
}

/// Everything `command` wrote to the host, between the outer shell's markers.
fn written(terminal: &mut Terminal) -> String {
    terminal.wait_for_bytes(DONE, TIMEOUT).unwrap();
    let received = terminal.received();
    let start = position(&received, BEFORE) + BEFORE.len();
    let end = start + position(&received[start..], DONE);
    String::from_utf8(received[start..end].to_vec()).unwrap()
}

fn position(bytes: &[u8], needle: &[u8]) -> usize {
    bytes
        .windows(needle.len())
        .position(|window| window == needle)
        .unwrap_or_else(|| panic!("{:?} missing", String::from_utf8_lossy(needle)))
}

/// A refusal before any terminal mode change or OSC output: exactly one
/// diagnostic line and no escape sequence at all.
fn refusal(terminal: &mut Terminal) -> String {
    let written = written(terminal);
    assert!(!written.contains('\x1b'), "{written:?}");
    assert_eq!(written.matches("\r\n").count(), 1, "{written:?}");
    assert!(written.ends_with("\r\n"), "{written:?}");
    written
}

/// INP-01 (Monitor, derived refusal): any argument after `monitor` or its alias
/// `mon` exits 2 with the usage line before a monitor starts.
#[test]
fn installed_monitor_arguments_are_a_usage_error() {
    let package = Package::new();
    for command in ["climon2 monitor extra", "climon2 mon --help"] {
        let mut terminal = outer(&package, command);
        assert_eq!(refusal(&mut terminal), USAGE);
        assert_released(&package, &mut terminal, 2);
    }
    package.assert_clean();
}

/// Spec #73 Entry and startup: a missing monitor executable fails with the
/// climon2 error code, naming the version, before any terminal change.
#[test]
fn missing_installed_monitor_fails_naming_its_version() {
    let package = Package::private();
    fs::remove_file(
        package
            .executable_home()
            .join(VERSION)
            .join("climon2-monitor"),
    )
    .unwrap();
    let mut terminal = outer(&package, "climon2 monitor");
    let diagnostic = refusal(&mut terminal);
    assert!(
        diagnostic.starts_with(&format!(
            "climon2: error: cannot start the climon2 {VERSION} monitor: "
        )),
        "{diagnostic:?}"
    );
    assert!(diagnostic.contains(" (logs: "), "{diagnostic:?}");
    assert_released(&package, &mut terminal, 125);
    package.assert_clean();
}
