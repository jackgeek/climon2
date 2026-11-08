//! Spec #73 / #74: the packaged monitor, reached as `climon2 monitor` or
//! `climon2 mon` through the installed launcher, driven through real PTYs from
//! outer shells with isolated homes (the Monitor layer). Every scenario ends
//! with `assert_clean()`.
#![cfg(target_os = "macos")]

#[allow(dead_code)]
mod support;

use climon2_test_term::{Modes, Terminal, bin_path};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use std::{env, fs, path::Path, process::Command};
use support::{Package, TIMEOUT, quote};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const ROWS: u16 = 30;
const COLS: u16 = 120;
const TITLE: &str = "OUTER-TITLE";
const BEFORE: &[u8] = b"BEFORE-MONITOR\r\n";
const DONE: &[u8] = b"OUTER-DONE=";
const USAGE: &str = "climon2: error: usage: climon2 exec <command> [args...] | attach <session-id> | kill <session-id> | list [--json] | monitor | mon\r\n";
const NO_TERMINAL: &str = "climon2: error: climon2 monitor needs a terminal\r\n";
const TOP_ROW: &str = "No session selected";
const EMPTY: &str = "No sessions to show.";

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
/// exact termios, the outer title untouched, no OSC output beyond the outer
/// shell's own title, the host's earlier content and ordinary shell input.
fn assert_released(package: &Package, terminal: &mut Terminal, status: i32) {
    terminal
        .wait_for_text(&format!("OUTER-DONE={status}"), TIMEOUT)
        .unwrap();
    assert_eq!(terminal.modes(), Modes::BASELINE, "{}", terminal.screen());
    assert_title_untouched(terminal);
    assert!(terminal.screen().contains("BEFORE-MONITOR"));
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

/// The outer shell titled the host once; nothing else ever sent an OSC.
fn assert_title_untouched(terminal: &Terminal) {
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    let received = terminal.received();
    assert_eq!(
        received
            .windows(2)
            .filter(|window| window == b"\x1b]")
            .count(),
        1
    );
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

/// Derived scenario: monitor argument refusal (INP-01 refusals, Monitor layer).
/// Any argument after `monitor` or its alias `mon` exits 2 with the usage line
/// before a monitor starts.
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

/// Derived scenario: monitor non-TTY refusal (INP-01 refusals, Monitor layer).
/// Without a terminal on stdin or on stdout the monitor prints one diagnostic
/// and exits with the climon2 error code before any terminal mode change or
/// OSC output.
#[test]
fn installed_monitor_refuses_a_non_terminal_stdin_or_stdout() {
    let package = Package::new();
    let stdout = package.isolation.home().join("monitor stdout");
    for command in [
        "climon2 monitor < /dev/null".to_owned(),
        format!("climon2 mon > {}", quote(&stdout)),
    ] {
        let mut terminal = outer(&package, &command);
        assert_eq!(refusal(&mut terminal), NO_TERMINAL);
        assert_released(&package, &mut terminal, 125);
    }
    assert_eq!(fs::read(&stdout).unwrap(), b"");
    package.assert_clean();
}

/// INP-01 (Monitor layer): a null `client.escape_prefix`, like any invalid
/// client setting, is refused with one diagnostic line and the climon2 error
/// code before any terminal change.
#[test]
fn installed_monitor_refuses_a_null_escape_prefix_and_invalid_keys() {
    let package = Package::new();
    for (config, expected) in [
        (
            r#"{"client": {"escape_prefix": null}}"#,
            "climon2: error: config.jsonc is invalid: client.escape_prefix may not be null for climon2 monitor (logs: ",
        ),
        (
            r#"{"client": {"take_control_key": "Ctrl+\n"}}"#,
            r#"climon2: error: config.jsonc is invalid: invalid key string "Ctrl+\n""#,
        ),
    ] {
        fs::write(
            package.isolation.climon2_home().join("config.jsonc"),
            config,
        )
        .unwrap();
        let mut terminal = outer(&package, "climon2 monitor");
        let diagnostic = refusal(&mut terminal);
        assert!(diagnostic.starts_with(expected), "{diagnostic:?}");
        assert_released(&package, &mut terminal, 125);
    }
    package.assert_clean();
}

/// Spec #73 Entry and startup: the monitor resolves its executable home exactly
/// as the CLI does, so outside an install layout it refuses before touching the
/// terminal.
#[test]
fn a_monitor_outside_a_version_folder_refuses_to_start() {
    let package = Package::new();
    let mut terminal = outer(&package, &quote(&bin_path("climon2-monitor")));
    let diagnostic = refusal(&mut terminal);
    assert!(
        diagnostic.starts_with(
            "climon2: error: cannot find the climon2 executable home: invalid SemVer version (logs: "
        ),
        "{diagnostic:?}"
    );
    assert_released(&package, &mut terminal, 125);
    package.assert_clean();
}

/// The empty monitor as specified: the permanent top row, then the empty state
/// centred in the rows below it; an odd spare row falls above the text and an
/// odd spare column after it.
fn expected_screen(rows: u16, cols: u16, prefix: &str) -> String {
    let (rows, cols) = (usize::from(rows), usize::from(cols));
    let hint = format!("Press {prefix} then x to exit.");
    let first = 1 + (rows - 2) / 2;
    (0..rows)
        .map(|row| match row {
            0 => format!("{TOP_ROW:<cols$}"),
            _ if row == first => format!("{EMPTY:^cols$}"),
            _ if row == first + 1 => format!("{hint:^cols$}"),
            _ => " ".repeat(cols),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The monitor owns the host: its own alternate screen showing the top row and
/// the empty state, raw input, bracketed paste, SGR mouse and focus reporting,
/// no cursor, and the host's title untouched.
fn opened(terminal: &mut Terminal) {
    terminal.wait_for_text(EMPTY, TIMEOUT).unwrap();
    terminal.wait_for_input_ready(TIMEOUT).unwrap();
    let modes = terminal.modes();
    assert!(modes.alternate_screen, "{modes:?}");
    assert!(
        modes.bracketed_paste && modes.mouse && modes.focus,
        "{modes:?}"
    );
    assert!(!modes.cursor_visible, "{modes:?}");
    assert_title_untouched(terminal);
}

/// The executable of the one running monitor process this test owns.
fn monitor_process(package: &Package) -> (i32, String) {
    let processes: Vec<_> = package
        .isolation
        .owned_pids()
        .into_iter()
        .map(|pid| {
            let output = Command::new("/bin/ps")
                .args(["-p", &pid.to_string(), "-o", "comm="])
                .output()
                .unwrap();
            (
                pid,
                String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            )
        })
        .filter(|(_, command)| command.ends_with("/climon2-monitor"))
        .collect();
    assert_eq!(processes.len(), 1, "{processes:?}");
    processes.into_iter().next().unwrap()
}

/// ENT-01 (Monitor layer): from an installed release, `climon2 monitor` and
/// its alias `climon2 mon` open the same monitor through the launcher, the CLI
/// and the version's installed `climon2-monitor`: one permanent top row above
/// the explicit empty state, whose hint names the default prefix. Prefix+x,
/// even split across reads, exits 0 and releases the host.
#[test]
fn installed_monitor_and_mon_open_the_same_monitor() {
    let package = Package::new();
    let installed = package
        .executable_home()
        .canonicalize()
        .unwrap()
        .join(VERSION)
        .join("climon2-monitor");
    let mut screens = Vec::new();
    for command in ["climon2 monitor", "climon2 mon"] {
        let mut terminal = outer(&package, command);
        opened(&mut terminal);
        assert_eq!(monitor_process(&package).1, installed.to_str().unwrap());
        screens.push(terminal.screen());
        terminal.send(b"\x1d").unwrap();
        terminal.send(b"x").unwrap();
        assert_released(&package, &mut terminal, 0);
    }
    assert_eq!(screens[0], expected_screen(ROWS, COLS, "Ctrl+]"));
    assert_eq!(screens[1], screens[0]);
    package.assert_clean();
}

/// INP-01 (Monitor layer): the monitor reuses the configured client escape
/// prefix: the empty state's hint names it and prefix+x exits. A null
/// take-control key is accepted, and Ctrl+C, the default prefix and its x are
/// not exits.
#[test]
fn installed_monitor_hints_and_exits_with_the_configured_prefix() {
    let package = Package::new();
    fs::write(
        package.isolation.climon2_home().join("config.jsonc"),
        r#"{"client": {"escape_prefix": "Ctrl+B", "take_control_key": null}}"#,
    )
    .unwrap();
    let mut terminal = outer(&package, "climon2 monitor");
    opened(&mut terminal);
    assert_eq!(terminal.screen(), expected_screen(ROWS, COLS, "Ctrl+B"));
    terminal.send(b"\x03\x1dx").unwrap();
    terminal.send(b"\x02x").unwrap();
    assert_released(&package, &mut terminal, 0);
    package.assert_clean();
}

/// EXT-03 (Monitor layer, empty monitor): catchable SIGINT, SIGTERM and SIGHUP
/// leave through the same clean path as prefix+x with signal-style status: the
/// baseline and exact termios are restored and nothing is printed.
#[test]
fn signals_release_the_host_with_signal_style_status() {
    let package = Package::new();
    for (signal, status) in [
        (Signal::SIGINT, 130),
        (Signal::SIGTERM, 143),
        (Signal::SIGHUP, 129),
    ] {
        let mut terminal = outer(&package, "climon2 monitor");
        opened(&mut terminal);
        kill(Pid::from_raw(monitor_process(&package).0), signal).unwrap();
        terminal
            .wait_for_text(&format!("OUTER-DONE={status}"), TIMEOUT)
            .unwrap();
        assert!(!written(&mut terminal).contains("climon2: error"));
        assert_released(&package, &mut terminal, status);
    }
    package.assert_clean();
}

/// Spec #73 Host terminal: SIGWINCH redraws the top row and the empty state at
/// the new size, leaving nothing of the old frame behind.
#[test]
fn resizing_the_host_redraws_the_monitor_at_the_new_size() {
    let package = Package::new();
    let mut terminal = outer(&package, "climon2 monitor");
    opened(&mut terminal);
    for (rows, cols) in [(12, 50), (40, 160)] {
        terminal.resize(rows, cols).unwrap();
        let expected = expected_screen(rows, cols, "Ctrl+]");
        terminal
            .wait_for_screen(TIMEOUT, |screen| screen == expected)
            .unwrap();
    }
    terminal.send(b"\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    package.assert_clean();
}

/// EXT-03 (Fault layer, empty monitor): a render failure while the monitor owns
/// the host first leaves its alternate screen and restores the baseline and
/// exact termios, then prints one diagnostic, and exits with the climon2 error
/// code. Only hook-enabled debug siblings inject it.
#[test]
fn debug_hook_render_failure_restores_the_host_before_its_diagnostic() {
    if env::var_os("CLIMON2_E2E_ARCHIVE").is_some() {
        return;
    }
    let package = Package::new();
    let mut terminal = outer(&package, "CLIMON2_MONITOR_FAULT=draw climon2 monitor");
    let written = written(&mut terminal);
    let entered = written.find("\x1b[?1049h").unwrap();
    let left = written.rfind("\x1b[?1049l").unwrap();
    let diagnostic = written
        .find("climon2: error: injected draw failure (logs: ")
        .unwrap();
    assert!(entered < left && left < diagnostic, "{written:?}");
    assert!(written.ends_with(")\r\n"), "{written:?}");
    assert_eq!(written.matches("climon2: error:").count(), 1);
    assert_released(&package, &mut terminal, 125);
    package.assert_clean();
}

/// Spec #73 Exit (diagnostics): the monitor writes content-free records to its
/// own general log in the climon2 home; typed input reaches no log.
#[test]
fn installed_monitor_logs_content_free_records_to_its_general_log() {
    let package = Package::new();
    let mut terminal = outer(&package, "CLIMON2_LOG=trace climon2 monitor");
    opened(&mut terminal);
    terminal.send("typed-secret-not-for-logs-5e1").unwrap();
    terminal.send(b"\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    let logs = package.logs();
    let monitor: Vec<_> = logs
        .iter()
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("monitor-")
        })
        .collect();
    assert_eq!(monitor.len(), 1, "{logs:?}");
    let log = fs::read_to_string(monitor[0]).unwrap();
    assert!(log.contains(" role=monitor "), "{log}");
    assert!(
        log.contains(&format!("monitor started version={VERSION}")),
        "{log}"
    );
    assert!(log.contains("monitor stopped status=0"), "{log}");
    for path in &logs {
        assert!(!fs::read_to_string(path).unwrap().contains("typed-secret"));
    }
    package.assert_clean();
}
