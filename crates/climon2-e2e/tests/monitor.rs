//! Spec #73 / #74: the packaged monitor, reached as `c2 monitor`, `c2 mon` or
//! their `climon2` spellings through the installed launcher, driven through
//! real PTYs from outer shells with isolated homes (the Monitor layer), on
//! macOS and Linux. Every scenario ends with `assert_clean()`.
#![cfg(any(target_os = "macos", target_os = "linux"))]

#[allow(dead_code)]
mod support;

use climon2_test_term::{AnsiColor, Color, InputTerminal, Modes, Terminal, Width, bin_path};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use serde_json::Value;
use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};
use support::{Package, TIMEOUT, quote};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const ROWS: u16 = 30;
const COLS: u16 = 120;
const TITLE: &str = "OUTER-TITLE";
const BEFORE: &[u8] = b"BEFORE-MONITOR\r\n";
const DONE: &[u8] = b"OUTER-DONE=";
const USAGE: &str = "climon2: error: usage: c2 exec [--headless] <command> [args...] | attach <session-id> | kill <session-id> | list [--json] | monitor | mon\r\n";
const NO_TERMINAL: &str = "climon2: error: climon2 monitor needs a terminal\r\n";
/// Three bracketed pastes with the default prefix: one holding prefix+x, one
/// after a typed prefix and followed by a typed x, and one holding the prefix
/// alone, followed by a typed x.
const PASTES: &[u8] = b"\x1b[200~\x1dx\x1b[201~\x1d\x1b[200~x\x1b[201~x\x1b[200~\x1d\x1b[201~x";
const TOP_ROW: &str = "No session selected";
const EMPTY: &str = "No sessions to show.";

/// The [`support::outer`] shell, first titling its host and marking where
/// `command`'s output begins.
fn outer(package: &Package, command: &str) -> Terminal {
    support::outer(
        package,
        Path::new("/bin/bash"),
        &format!("printf '\\033]2;{TITLE}\\007'; printf 'BEFORE-MONITOR\\n'; {command}"),
        ROWS,
        COLS,
    )
}

/// After `command` returned `status`: the host released as
/// [`support::assert_released`] checks, with the outer title untouched, no OSC
/// output beyond the outer shell's own title, and the host's earlier content.
fn assert_released(package: &Package, terminal: &mut Terminal, status: i32) {
    support::assert_released(package, terminal, status);
    assert_title_untouched(terminal);
    assert!(terminal.screen().contains("BEFORE-MONITOR"));
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
    for command in ["c2 monitor extra", "climon2 mon --help"] {
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
        "c2 monitor < /dev/null".to_owned(),
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
        fs::write(config_file(&package), config).unwrap();
        let mut terminal = outer(&package, "climon2 monitor");
        let diagnostic = refusal(&mut terminal);
        assert!(diagnostic.starts_with(expected), "{diagnostic:?}");
        assert_released(&package, &mut terminal, 125);
    }
    package.assert_clean();
}

fn config_file(package: &Package) -> PathBuf {
    package.isolation.climon2_home().join("config.jsonc")
}

/// The preference file a release writes once Don't show again is checked.
const SUPPRESSED: &str = "{\"version\":1,\"suppress_key_introduction\":true}\n";

fn preferences_file(package: &Package) -> PathBuf {
    package
        .isolation
        .climon2_home()
        .join("monitor-preferences.json")
}

/// Saves Don't show again in the test's climon2 home, as an earlier run
/// would, so the monitor opens on its empty state without the key
/// introduction.
fn without_introduction(package: &Package) {
    fs::write(preferences_file(package), SUPPRESSED).unwrap();
}

/// The climon2 home holds the choice `suppressed` exactly as a release saves
/// it, readable and writable only by its owner, and nothing else a save could
/// have left there.
fn assert_saved(package: &Package, suppressed: bool) {
    let path = preferences_file(package);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("{{\"version\":1,\"suppress_key_introduction\":{suppressed}}}\n")
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o600
    );
    assert_eq!(home_entries(package), ["logs", "monitor-preferences.json"]);
}

/// The names in the test's climon2 home, sorted.
fn home_entries(package: &Package) -> Vec<String> {
    let mut entries: Vec<_> = fs::read_dir(package.isolation.climon2_home())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    entries.sort();
    entries
}

/// CFG-01 (Monitor): every invalid configuration, unreadable and malformed
/// JSONC included, prints one explicit diagnostic and exits with the climon2
/// error code before any terminal change: a null, a wrong type, a duplicate
/// known field, an overflowing count, an out-of-range value and inconsistent
/// limits.
#[test]
fn installed_monitor_refuses_every_invalid_configuration_before_the_terminal_changes() {
    let package = Package::new();
    let config = config_file(&package);
    fs::create_dir(&config).unwrap();
    let mut terminal = outer(&package, "climon2 monitor");
    let diagnostic = refusal(&mut terminal);
    assert!(
        diagnostic.starts_with("climon2: error: cannot read config.jsonc: "),
        "{diagnostic:?}"
    );
    assert!(diagnostic.contains(" (logs: "), "{diagnostic:?}");
    assert_released(&package, &mut terminal, 125);
    fs::remove_dir(&config).unwrap();
    for (text, expected) in [
        (
            "{\"monitor\":",
            "config.jsonc is malformed: EOF while parsing a value at line 1 column 11",
        ),
        (
            r#"{"monitor": {"bell": null}}"#,
            "config.jsonc is invalid: invalid type: null, expected a boolean",
        ),
        (
            r#"{"monitor": {"show_key_hints": "no"}}"#,
            r#"config.jsonc is invalid: invalid type: string "no", expected a boolean"#,
        ),
        (
            r#"{"monitor": {"limits": {}, "limits": {}}}"#,
            "config.jsonc is invalid: duplicate field `limits`",
        ),
        (
            r#"{"monitor": {"limits": {"queues": {"deliveries": 4294967296}}}}"#,
            "config.jsonc is invalid: invalid value: integer `4294967296`, expected u32",
        ),
        (
            r#"{"monitor": {"limits": {"ui": {"fps": 121}}}}"#,
            "config.jsonc is invalid: monitor.limits.ui.fps must be an integer from 1 to 120",
        ),
        (
            r#"{"monitor": {"limits": {"retry": {"initial_ms": 30001}}}}"#,
            "config.jsonc is invalid: monitor.limits.retry.initial_ms (30001) may not exceed monitor.limits.retry.cap_ms (30000)",
        ),
    ] {
        fs::write(&config, text).unwrap();
        let mut terminal = outer(&package, "climon2 monitor");
        let diagnostic = refusal(&mut terminal);
        assert!(
            diagnostic.starts_with(&format!("climon2: error: {expected}")),
            "{text}: {diagnostic:?}"
        );
        assert!(diagnostic.contains(" (logs: "), "{diagnostic:?}");
        assert_released(&package, &mut terminal, 125);
    }
    package.assert_clean();
}

/// CFG-01 (Monitor): `monitor.show_key_hints: false` removes the empty state's
/// hint but no binding, so prefix+x still exits 0. Unknown and unrelated
/// settings, an invalid `scrollback` among them, are ignored.
#[test]
fn installed_monitor_without_key_hints_shows_none_and_still_exits() {
    let package = Package::new();
    fs::write(
        config_file(&package),
        r#"{"monitor": {"show_key_hints": false, "future": null}, "scrollback": {"max_rows": "many"}, "future": []}"#,
    )
    .unwrap();
    let mut terminal = outer(&package, "climon2 monitor");
    opened(&mut terminal);
    assert_eq!(terminal.screen(), empty_monitor(ROWS, COLS, &[EMPTY]));
    terminal.send(b"\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    package.assert_clean();
}

/// CFG-01 (Monitor): the monitor reads its configuration once, before the
/// terminal changes, and never reloads it. After the file is edited, a repaint
/// still shows no hint, and the edited prefix and x, once read, do not exit:
/// the monitor still runs and leaves on SIGTERM with that signal's status.
#[test]
fn editing_the_configuration_while_the_monitor_runs_changes_nothing() {
    let package = Package::new();
    let config = config_file(&package);
    fs::write(
        &config,
        r#"{"client": {"escape_prefix": "Ctrl+B"}, "monitor": {"show_key_hints": false}}"#,
    )
    .unwrap();
    let mut terminal = outer(&package, "climon2 monitor");
    opened(&mut terminal);
    assert_eq!(terminal.screen(), empty_monitor(ROWS, COLS, &[EMPTY]));
    fs::write(
        &config,
        r#"{"client": {"escape_prefix": "Ctrl+A"}, "monitor": {"show_key_hints": true}}"#,
    )
    .unwrap();
    terminal.resize(12, 50).unwrap();
    let repainted = empty_monitor(12, 50, &[EMPTY]);
    shows(&mut terminal, &repainted);
    let monitor = monitor_process(&package).0;
    read_alone(&mut terminal, monitor, b"\x01x");
    kill(Pid::from_raw(monitor), Signal::SIGTERM)
        .expect("the monitor still runs after reading the edited prefix and x");
    assert_released(&package, &mut terminal, 143);
    package.assert_clean();
}

/// The effective configuration expected from the file in
/// [`installed_monitor_logs_its_effective_configuration`]: its four settings
/// and every other default, written out from the operational limits document.
const EFFECTIVE: &str = r#"{
  "client": {"escape_prefix": "Ctrl+]", "take_control_key": null},
  "monitor": {
    "show_key_hints": true,
    "bell": false,
    "limits": {
      "memory": {"total_bytes": 536870912, "client_bytes": 67108864, "history_bytes_per_session": 16777216, "history_bytes_total": 268435456, "queue_bytes_total": 67108864},
      "history": {"max_rows": 200},
      "resources": {"stack_bytes": 2097152, "max_workers_per_client": 2, "descriptors_per_client": 8, "descriptors_per_load": 4, "descriptor_headroom": 64, "release_bytes": 67108864},
      "queues": {"request_bytes": 262144, "request_records": 64, "event_bytes": 1048576, "event_records": 1024, "progress_bytes": 65536, "progress_records": 32, "deliveries": 8, "operations": 16},
      "transfer": {"chunk_bytes": 65536, "drain_records": 64, "drain_payload_bytes": 65536},
      "payload": {"wire_frame_bytes": 1048576, "metadata_record_bytes": 16384, "title_bytes": 4096, "notification_title_bytes": 4096, "notification_body_bytes": 8192, "assembly_records": 32, "assembly_bytes": 65536},
      "scheduling": {"probes": 8, "loads": 2, "starts": 8, "newer_start_burst": 8},
      "discovery": {"pass_ms": 30000, "stall_ms": 5000, "batch_records": 128, "batch_bytes": 1048576, "metadata_file_bytes": 262144, "metadata_depth": 32},
      "deadlines": {"load_ms": 10000, "start_ms": 10000, "operation_ms": 5000, "kill_ms": 10000, "restoration_stall_ms": 5000, "restoration_total_ms": 60000, "delivery_stall_ms": 5000, "leave_ms": 5000, "cleanup_ms": 10000, "shutdown_ms": 10000, "final_host_ms": 1000, "assembly_ms": 5000},
      "retry": {"initial_ms": 1000, "cap_ms": 40000, "jitter_percent": 20, "admission_ms": 1000},
      "ui": {"fps": 30, "client_quantum_ms": 2, "background_quantum_ms": 8, "toast_records": 128, "toast_bytes": 524288, "search_bytes": 4096}
    }
  }
}"#;

/// The general logs the monitor runs wrote.
fn monitor_log_files(package: &Package) -> Vec<PathBuf> {
    package
        .logs()
        .into_iter()
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("monitor-")
        })
        .collect()
}

/// The one general log the monitor wrote.
fn monitor_log(package: &Package) -> String {
    let files = monitor_log_files(package);
    assert_eq!(files.len(), 1, "{:?}", package.logs());
    fs::read_to_string(&files[0]).unwrap()
}

/// Every record the monitor runs wrote to their general logs.
fn monitor_logs(package: &Package) -> String {
    monitor_log_files(package)
        .iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect()
}

/// CFG-01 (Monitor): the monitor reports every effective setting, the
/// configured ones and every default, in its general log.
#[test]
fn installed_monitor_logs_its_effective_configuration() {
    let package = Package::new();
    without_introduction(&package);
    fs::write(
        config_file(&package),
        r#"{"client": {"take_control_key": null}, "monitor": {"bell": false, "limits": {"history": {"max_rows": 200}, "retry": {"cap_ms": 40000}}}}"#,
    )
    .unwrap();
    let mut terminal = outer(&package, "climon2 monitor");
    opened(&mut terminal);
    terminal.send(b"\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    let log = monitor_log(&package);
    let reported = log
        .lines()
        .find_map(|line| line.split_once(" monitor configuration effective="))
        .unwrap_or_else(|| panic!("{log}"))
        .1;
    assert_eq!(
        serde_json::from_str::<Value>(reported).unwrap(),
        serde_json::from_str::<Value>(EFFECTIVE).unwrap()
    );
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

/// The empty monitor as specified: the permanent top row, then the empty
/// state's `lines` centred in the rows below it, an odd spare row above the
/// text. Each line is centred on the middle column: it starts half the
/// columns less half its width from the left, both halves rounded down.
fn empty_monitor(rows: u16, cols: u16, lines: &[&str]) -> String {
    let (rows, cols) = (usize::from(rows), usize::from(cols));
    let first = 1 + (rows - lines.len()) / 2;
    (0..rows)
        .map(|row| match row {
            0 => format!("{TOP_ROW:<cols$}"),
            _ if (first..first + lines.len()).contains(&row) => {
                let line = lines[row - first];
                let left = cols / 2 - line.chars().count() / 2;
                format!("{:left$}{line:<width$}", "", width = cols - left)
            }
            _ => " ".repeat(cols),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The empty monitor with its key hint naming `prefix`.
fn expected_screen(rows: u16, cols: u16, prefix: &str) -> String {
    empty_monitor(
        rows,
        cols,
        &[EMPTY, &format!("Press {prefix} then x to exit.")],
    )
}

/// The key introduction's dialog as the selector document specifies it: the
/// configured `prefix` followed by x, s and ?, the Don't show again checkbox,
/// any `problem` with the saved choice, and the Continue action that Enter
/// takes, in a box titled Keys with one column of space inside its border.
fn key_introduction(prefix: &str, checked: bool, problem: Option<&str>) -> Vec<String> {
    let mut lines = vec![
        format!("{prefix} then x  Exit the monitor"),
        format!("{prefix} then s  Open the session list"),
        format!("{prefix} then ?  Show this dialog"),
        String::new(),
        format!(
            "[{}] Don't show again (Space)",
            if checked { 'x' } else { ' ' }
        ),
    ];
    lines.extend(problem.map(str::to_owned));
    lines.extend([String::new(), "Continue (Enter)".to_owned()]);
    let width = lines.iter().map(|line| line.chars().count()).max().unwrap();
    let mut boxed = vec![format!("┌ Keys {}┐", "─".repeat(width - 4))];
    boxed.extend(lines.iter().map(|line| format!("│ {line:<width$} │")));
    boxed.push(format!("└{}┘", "─".repeat(width + 2)));
    boxed
}

/// `screen` with `dialog` centred in the rows below its top row: an odd spare
/// row falls above the dialog and an odd spare column to its left.
fn overlaid(screen: &str, dialog: &[String]) -> String {
    let mut rows: Vec<Vec<char>> = screen
        .split('\n')
        .map(|row| row.chars().collect())
        .collect();
    let (height, width) = (dialog.len(), dialog[0].chars().count());
    let top = 1 + (rows.len() - 1 - height).div_ceil(2);
    let left = (rows[0].len() - width).div_ceil(2);
    for (row, line) in rows[top..top + height].iter_mut().zip(dialog) {
        row.splice(left..left + width, line.chars());
    }
    rows.iter()
        .map(|row| row.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The monitor owns the host: its own alternate screen showing the top row and
/// the empty state, raw input, bracketed paste, SGR-encoded button and drag
/// mouse reports and focus reports, no cursor, every other mode at the
/// baseline, and the host's title untouched.
fn opened(terminal: &mut Terminal) {
    terminal.wait_for_text(EMPTY, TIMEOUT).unwrap();
    owns_host(terminal);
}

/// The monitor owns the host as [`opened`] checks, showing exactly `expected`.
fn opened_on(terminal: &mut Terminal, expected: &str) {
    shows(terminal, expected);
    owns_host(terminal);
}

/// Waits until the host shows exactly `expected`.
fn shows(terminal: &mut Terminal, expected: &str) {
    terminal
        .wait_for_screen(TIMEOUT, |screen| screen == expected)
        .unwrap();
}

fn owns_host(terminal: &mut Terminal) {
    terminal.wait_for_input_ready(TIMEOUT).unwrap();
    assert_eq!(
        terminal.modes(),
        Modes {
            alternate_screen: true,
            cursor_visible: false,
            mouse: true,
            mouse_drag: true,
            sgr_mouse: true,
            focus: true,
            bracketed_paste: true,
            ..Modes::BASELINE
        }
    );
    assert_title_untouched(terminal);
}

/// The executable a process runs: the kernel's record on Linux, `ps`'s full
/// command path on macOS. Empty once the process is gone.
fn executable(pid: i32) -> String {
    if cfg!(target_os = "linux") {
        return fs::read_link(format!("/proc/{pid}/exe"))
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    let output = Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The executable of the one running monitor process this test owns.
fn monitor_process(package: &Package) -> (i32, String) {
    let processes: Vec<_> = package
        .isolation
        .owned_pids()
        .into_iter()
        .map(|pid| (pid, executable(pid)))
        .filter(|(_, command)| command.ends_with("/climon2-monitor"))
        .collect();
    assert_eq!(processes.len(), 1, "{processes:?}");
    processes.into_iter().next().unwrap()
}

/// ENT-01 (Monitor layer): from an installed release, `c2 monitor`, `c2 mon`,
/// `climon2 monitor` and `climon2 mon` all open the same monitor through the
/// launcher, the CLI and the version's installed `climon2-monitor`: one
/// permanent top row above the explicit empty state, whose hint names the
/// default prefix. Prefix+x exits 0 and releases the host even split across
/// reads: the monitor has read the prefix alone before x is typed.
#[test]
fn installed_monitor_and_mon_open_the_same_monitor() {
    let package = Package::new();
    without_introduction(&package);
    let installed = package
        .executable_home()
        .canonicalize()
        .unwrap()
        .join(VERSION)
        .join("climon2-monitor");
    let expected = expected_screen(ROWS, COLS, "Ctrl+]");
    for command in ["c2 monitor", "c2 mon", "climon2 monitor", "climon2 mon"] {
        let mut terminal = outer(&package, command);
        opened(&mut terminal);
        let (monitor, executable) = monitor_process(&package);
        assert_eq!(executable, installed.to_str().unwrap());
        assert_eq!(terminal.screen(), expected, "{command}");
        read_alone(&mut terminal, monitor, b"\x1d");
        terminal.send(b"x").unwrap();
        assert_released(&package, &mut terminal, 0);
    }
    package.assert_clean();
}

/// INP-01 (Monitor layer): the monitor reuses the configured client escape
/// prefix: the empty state's hint names it and prefix+x exits 0. A null
/// take-control key is accepted. Ctrl+C, the default prefix and its x are not
/// exits: a monitor that has read them still runs, and leaves on SIGTERM with
/// that signal's status, not prefix+x's.
#[test]
fn installed_monitor_hints_and_exits_with_the_configured_prefix() {
    let package = Package::new();
    without_introduction(&package);
    fs::write(
        config_file(&package),
        r#"{"client": {"escape_prefix": "Ctrl+B", "take_control_key": null}}"#,
    )
    .unwrap();
    let mut ignoring = outer(&package, "climon2 monitor");
    opened(&mut ignoring);
    assert_eq!(ignoring.screen(), expected_screen(ROWS, COLS, "Ctrl+B"));
    let monitor = monitor_process(&package).0;
    read_alone(&mut ignoring, monitor, b"\x03\x1dx");
    kill(Pid::from_raw(monitor), Signal::SIGTERM)
        .expect("the monitor still runs after reading Ctrl+C and the default prefix+x");
    assert_released(&package, &mut ignoring, 143);
    let mut exiting = outer(&package, "climon2 monitor");
    opened(&mut exiting);
    exiting.send(b"\x02x").unwrap();
    assert_released(&package, &mut exiting, 0);
    package.assert_clean();
}

/// The process state `ps` reports, such as `T` once stopped.
fn state(pid: i32) -> String {
    let output = Command::new("/bin/ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Stops `pid`, and returns once it no longer runs.
fn stop(pid: i32) {
    kill(Pid::from_raw(pid), Signal::SIGSTOP).unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while !state(pid).starts_with('T') {
        assert!(Instant::now() < deadline, "{pid} never stopped");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Has `monitor` read `bytes` in one read of their own: they all reach its
/// terminal while it is stopped, then it resumes and reads every one before
/// this returns.
fn read_alone(terminal: &mut Terminal, monitor: i32, bytes: &[u8]) {
    stop(monitor);
    terminal.send(bytes).unwrap();
    terminal
        .wait_for_unread_input(bytes.len(), TIMEOUT)
        .unwrap();
    kill(Pid::from_raw(monitor), Signal::SIGCONT).unwrap();
    terminal.wait_for_unread_input(0, TIMEOUT).unwrap();
}

/// INP-03 (Monitor layer, empty monitor): a paste never exits, never completes
/// a pending prefix and leaves none pending, so typed x after it is not an
/// exit either. Once the monitor has read every pasted byte it is still
/// running, and leaves on SIGTERM, not with prefix+x's status.
#[test]
fn pastes_never_exit_or_leave_a_pending_prefix() {
    let package = Package::new();
    without_introduction(&package);
    let mut terminal = outer(&package, "climon2 monitor");
    opened(&mut terminal);
    let monitor = monitor_process(&package).0;
    read_alone(&mut terminal, monitor, PASTES);
    kill(Pid::from_raw(monitor), Signal::SIGTERM)
        .expect("the monitor still runs after reading every paste");
    assert!(!written(&mut terminal).contains("climon2: error"));
    assert_released(&package, &mut terminal, 143);
    package.assert_clean();
}

/// EXT-03 (Monitor layer, empty monitor): catchable SIGINT, SIGTERM and SIGHUP
/// leave through the same clean path as prefix+x with signal-style status: the
/// baseline and exact termios are restored and nothing is printed.
#[test]
fn signals_release_the_host_with_signal_style_status() {
    let package = Package::new();
    without_introduction(&package);
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

/// Spec #73 Host terminal; TRM-04 (Monitor layer, empty monitor): SIGWINCH
/// redraws the top row and the empty state at the new size, leaving nothing of
/// the old frame behind, and one frame replaces the other without blanking or
/// flashing: until its exit, the monitor never erases the screen or shows the
/// cursor.
#[test]
fn resizing_the_host_redraws_the_monitor_at_the_new_size() {
    let package = Package::new();
    without_introduction(&package);
    let mut terminal = outer(&package, "climon2 monitor");
    opened(&mut terminal);
    for (rows, cols) in [(12, 50), (40, 160)] {
        terminal.resize(rows, cols).unwrap();
        let expected = expected_screen(rows, cols, "Ctrl+]");
        shows(&mut terminal, &expected);
        assert!(!terminal.modes().cursor_visible);
    }
    let received = terminal.received();
    terminal.send(b"\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    let frames = &received[position(&received, b"\x1b[?1049h")..];
    assert!(!contains(frames, b"\x1b[?25h"), "cursor shown");
    assert!(!erases(frames), "screen erased");
    package.assert_clean();
}

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

/// Whether `bytes` hold an erase in display or in line: CSI, optional digits,
/// then `J` or `K`.
fn erases(bytes: &[u8]) -> bool {
    bytes.windows(2).enumerate().any(|(start, window)| {
        window == b"\x1b["
            && bytes[start + 2..]
                .iter()
                .find(|byte| !byte.is_ascii_digit())
                .is_some_and(|last| matches!(last, b'J' | b'K'))
    })
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
    assert_diagnosed_after_release(
        &mut terminal,
        "climon2: error: injected draw failure (logs: ",
    );
    assert_released(&package, &mut terminal, 125);
    package.assert_clean();
}

/// Everything the monitor wrote: its own alternate screen entered and left,
/// then exactly one diagnostic line, beginning `diagnostic`, with its log.
fn assert_diagnosed_after_release(terminal: &mut Terminal, diagnostic: &str) {
    let written = written(terminal);
    let entered = written.find("\x1b[?1049h").unwrap();
    let left = written.rfind("\x1b[?1049l").unwrap();
    let reported = written.find(diagnostic).unwrap();
    assert!(entered < left && left < reported, "{written:?}");
    assert!(written.ends_with(")\r\n"), "{written:?}");
    assert_eq!(written.matches("climon2: error:").count(), 1, "{written:?}");
}

/// EXT-03 (Fault layer, empty monitor): a failed wait for host input ends the
/// monitor at once instead of spinning: it leaves its alternate screen and
/// restores the baseline and exact termios, then prints one diagnostic, and
/// exits with the climon2 error code. Only hook-enabled debug siblings inject
/// it, by giving the kernel no descriptor table to poll (EFAULT).
#[test]
fn debug_hook_input_wait_failure_restores_the_host_before_its_diagnostic() {
    if env::var_os("CLIMON2_E2E_ARCHIVE").is_some() {
        return;
    }
    let package = Package::new();
    let mut terminal = outer(&package, "CLIMON2_MONITOR_FAULT=poll climon2 monitor");
    assert_diagnosed_after_release(
        &mut terminal,
        "climon2: error: cannot wait for host input: Bad address (os error 14) (logs: ",
    );
    assert_released(&package, &mut terminal, 125);
    package.assert_clean();
}

/// EXT-03 (Monitor layer, empty monitor): when its host input hangs up, the
/// monitor ends at once instead of spinning: it releases the host it still
/// writes to, then reports the closed terminal, also in its general log, with
/// the climon2 error code. Linux wakes readers before it finishes hanging a
/// terminal up, so the monitor is stopped until the hang-up is complete.
#[test]
fn a_hung_up_host_input_releases_the_host_before_its_diagnostic() {
    let package = Package::new();
    without_introduction(&package);
    let input = InputTerminal::open(ROWS, COLS).unwrap();
    let mut terminal = outer(
        &package,
        &format!("climon2 monitor < {}", quote(input.path())),
    );
    terminal.wait_for_text(EMPTY, TIMEOUT).unwrap();
    let monitor = monitor_process(&package).0;
    stop(monitor);
    input.hang_up();
    kill(Pid::from_raw(monitor), Signal::SIGCONT).unwrap();
    assert_diagnosed_after_release(
        &mut terminal,
        "climon2: error: the host terminal closed (logs: ",
    );
    assert_released(&package, &mut terminal, 125);
    let log = monitor_log(&package);
    assert!(
        log.contains("monitor failed error=the host terminal closed"),
        "{log}"
    );
    package.assert_clean();
}

/// Spec #73 Exit (diagnostics): the monitor writes content-free records to its
/// own general log in the climon2 home; typed input reaches no log.
#[test]
fn installed_monitor_logs_content_free_records_to_its_general_log() {
    let package = Package::new();
    without_introduction(&package);
    let mut terminal = outer(&package, "CLIMON2_LOG=trace climon2 monitor");
    opened(&mut terminal);
    terminal.send("typed-secret-not-for-logs-5e1").unwrap();
    terminal.send(b"\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    let log = monitor_log(&package);
    assert!(log.contains(" role=monitor "), "{log}");
    assert!(
        log.contains(&format!("monitor started version={VERSION}")),
        "{log}"
    );
    assert!(log.contains("monitor stopped status=0"), "{log}");
    for path in package.logs() {
        assert!(!fs::read_to_string(path).unwrap().contains("typed-secret"));
    }
    package.assert_clean();
}

/// HLP-01 (Monitor): with key hints on and nothing saved, every start opens
/// the modal key introduction over the empty monitor: the actual configured
/// prefix followed by x, s and ?, and an unchecked Don't show again checkbox.
/// Enter continues to the empty monitor. Closing without checking the box
/// saves nothing, so the next start opens it again, and prefix+x exits from
/// it.
#[test]
fn the_key_introduction_opens_at_every_start_until_dont_show_again_is_saved() {
    let package = Package::new();
    fs::write(
        config_file(&package),
        r#"{"client": {"escape_prefix": "Ctrl+B"}}"#,
    )
    .unwrap();
    let empty = expected_screen(ROWS, COLS, "Ctrl+B");
    let introduction = overlaid(&empty, &key_introduction("Ctrl+B", false, None));
    let mut first = outer(&package, "climon2 monitor");
    opened_on(&mut first, &introduction);
    first.send(b"\r").unwrap();
    shows(&mut first, &empty);
    first.send(b"\x02x").unwrap();
    assert_released(&package, &mut first, 0);
    let mut again = outer(&package, "climon2 monitor");
    opened_on(&mut again, &introduction);
    again.send(b"\x02x").unwrap();
    assert_released(&package, &mut again, 0);
    assert!(!preferences_file(&package).exists());
    package.assert_clean();
}

/// HLP-01 (Monitor): the key introduction opens at startup only when key
/// hints are on and Don't show again is not saved, while prefix+? opens the
/// same dialog whatever either says, showing the saved choice. Prefix+?
/// while it is open changes nothing: once the monitor has read it, a resize
/// repaints the dialog still open, centred with an odd spare row above it and
/// an odd spare column to its left. A later release's preference file, with
/// a higher version and fields this release does not know, is honoured and
/// left as it is.
#[test]
fn the_key_introduction_opens_on_prefix_question_mark_whatever_hints_and_the_saved_choice_say() {
    let package = Package::new();
    fs::write(
        config_file(&package),
        r#"{"monitor": {"show_key_hints": false}}"#,
    )
    .unwrap();
    let mut hidden = outer(&package, "climon2 monitor");
    let quiet = empty_monitor(ROWS, COLS, &[EMPTY]);
    opened_on(&mut hidden, &quiet);
    hidden.send(b"\x1d?").unwrap();
    let opened = overlaid(&quiet, &key_introduction("Ctrl+]", false, None));
    shows(&mut hidden, &opened);
    let monitor = monitor_process(&package).0;
    read_alone(&mut hidden, monitor, b"\x1d?");
    hidden.resize(21, 71).unwrap();
    let small = empty_monitor(21, 71, &[EMPTY]);
    let still_open = overlaid(&small, &key_introduction("Ctrl+]", false, None));
    shows(&mut hidden, &still_open);
    hidden.send(b"\r").unwrap();
    shows(&mut hidden, &small);
    hidden.send(b"\x1dx").unwrap();
    assert_released(&package, &mut hidden, 0);
    assert!(!preferences_file(&package).exists());

    fs::remove_file(config_file(&package)).unwrap();
    let later = r#"{"version":2,"suppress_key_introduction":true,"tips":{"seen":["x"]}}"#;
    fs::write(preferences_file(&package), later).unwrap();
    let mut suppressed = outer(&package, "climon2 monitor");
    let empty = expected_screen(ROWS, COLS, "Ctrl+]");
    opened_on(&mut suppressed, &empty);
    suppressed.send(b"\x1d?").unwrap();
    let checked = overlaid(&empty, &key_introduction("Ctrl+]", true, None));
    shows(&mut suppressed, &checked);
    suppressed.send(b"\x1dx").unwrap();
    assert_released(&package, &mut suppressed, 0);
    assert_eq!(
        fs::read_to_string(preferences_file(&package)).unwrap(),
        later
    );
    package.assert_clean();
}

/// The monitor's temporary file for its preference file
/// (`monitor-preferences.json.<pid>.tmp`), which it renames over that file.
fn temporary_file(package: &Package) -> PathBuf {
    let monitor = monitor_process(package).0;
    package
        .isolation
        .climon2_home()
        .join(format!("monitor-preferences.json.{monitor}.tmp"))
}

/// HLP-01 (Monitor): Space checks Don't show again and saves it in the
/// climon2 home's preference file, private to its owner even under umask 000,
/// and atomically: a new temporary file renamed over the old one, so a hard
/// link to the old file keeps its bytes, and no temporary file is left
/// behind. A longer, readable file already under the temporary name, as an
/// interrupted save by a process with the same pid would leave, is replaced.
/// The next start opens on the empty monitor; prefix+? shows the box checked,
/// and Space reverses the choice, so the following start shows the
/// introduction again. Another climon2 home keeps its own choice.
#[test]
fn dont_show_again_is_saved_privately_and_atomically_per_home_and_reversible() {
    let package = Package::new();
    let empty = expected_screen(ROWS, COLS, "Ctrl+]");
    let introduction = overlaid(&empty, &key_introduction("Ctrl+]", false, None));
    let checked = overlaid(&empty, &key_introduction("Ctrl+]", true, None));
    let mut first = outer(&package, "umask 000; climon2 monitor");
    opened_on(&mut first, &introduction);
    let interrupted = temporary_file(&package);
    fs::write(
        &interrupted,
        "left by an interrupted save, longer than any saved choice\n",
    )
    .unwrap();
    fs::set_permissions(&interrupted, fs::Permissions::from_mode(0o644)).unwrap();
    first.send(b" ").unwrap();
    shows(&mut first, &checked);
    assert_saved(&package, true);
    first.send(b"\r").unwrap();
    shows(&mut first, &empty);
    first.send(b"\x1dx").unwrap();
    assert_released(&package, &mut first, 0);

    let old = package.isolation.home().join("old preferences");
    fs::hard_link(preferences_file(&package), &old).unwrap();
    let mut second = outer(&package, "umask 000; climon2 monitor");
    opened_on(&mut second, &empty);
    second.send(b"\x1d?").unwrap();
    shows(&mut second, &checked);
    second.send(b" ").unwrap();
    shows(&mut second, &introduction);
    assert_saved(&package, false);
    assert_eq!(fs::read_to_string(&old).unwrap(), SUPPRESSED);
    second.send(b"\x1dx").unwrap();
    assert_released(&package, &mut second, 0);

    let mut third = outer(&package, "climon2 monitor");
    opened_on(&mut third, &introduction);
    third.send(b" ").unwrap();
    shows(&mut third, &checked);
    third.send(b"\x1dx").unwrap();
    assert_released(&package, &mut third, 0);
    assert_saved(&package, true);

    let other = package.isolation.home().join("other home");
    let mut elsewhere = outer(
        &package,
        &format!("CLIMON2_HOME={} climon2 monitor", quote(&other)),
    );
    opened_on(&mut elsewhere, &introduction);
    elsewhere.send(b"\x1dx").unwrap();
    assert_released(&package, &mut elsewhere, 0);
    assert!(!other.join("monitor-preferences.json").exists());
    assert_saved(&package, true);
    let logs = monitor_logs(&package);
    for record in [
        "monitor preference saved suppressed=true",
        "monitor preference saved suppressed=false",
    ] {
        assert!(logs.contains(record), "{logs}");
    }
    package.assert_clean();
}

/// HLP-01 (Monitor; derived scenario: a save interrupted before its rename):
/// before it saves, the monitor removes the temporary preference files that
/// processes no longer running left in its climon2 home, so a crash before
/// the rename leaves nothing for good. It keeps those of running processes,
/// which may still be saving, and any other file. No process ever runs as
/// 2147483647, the largest pid: Linux keeps pids below 2^22 and macOS below
/// 100000.
#[test]
fn saving_removes_temporary_files_that_ended_processes_left_behind() {
    let package = Package::new();
    let home = package.isolation.climon2_home();
    let ended = home.join(format!("monitor-preferences.json.{}.tmp", i32::MAX));
    let running = home.join(format!(
        "monitor-preferences.json.{}.tmp",
        std::process::id()
    ));
    let other = home.join("monitor-preferences.json.saved.tmp");
    for path in [&ended, &running, &other] {
        fs::write(path, "left by a save interrupted before its rename\n").unwrap();
    }
    let empty = expected_screen(ROWS, COLS, "Ctrl+]");
    let mut terminal = outer(&package, "climon2 monitor");
    opened_on(
        &mut terminal,
        &overlaid(&empty, &key_introduction("Ctrl+]", false, None)),
    );
    terminal.send(b" ").unwrap();
    shows(
        &mut terminal,
        &overlaid(&empty, &key_introduction("Ctrl+]", true, None)),
    );
    assert!(!ended.exists());
    assert!(running.exists() && other.exists());
    terminal.send(b"\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    fs::remove_file(running).unwrap();
    fs::remove_file(other).unwrap();
    assert_saved(&package, true);
    package.assert_clean();
}

/// HLP-01 (Monitor; derived scenario: an unreadable or malformed preference
/// file): such a file means not suppressed, so the introduction opens with
/// the reason under its unchecked box, and the general log records it. The
/// monitor never rewrites the file implicitly: Enter and prefix+x leave it
/// as it was. A directory in place of the file is unreadable.
#[test]
fn an_unreadable_or_malformed_preference_file_shows_why_beside_the_introduction() {
    let package = Package::new();
    let empty = expected_screen(ROWS, COLS, "Ctrl+]");
    for (text, reason) in [
        (
            r#"{"suppress_key_introduction":true}"#,
            "monitor-preferences.json is malformed: missing field `version` at line 1 column 34",
        ),
        (
            r#"{"version":1,"suppress_key_introduction":"yes"}"#,
            "monitor-preferences.json is malformed: invalid type: string \"yes\", expected a boolean at line 1 column 46",
        ),
    ] {
        fs::write(preferences_file(&package), text).unwrap();
        let mut terminal = outer(&package, "climon2 monitor");
        opened_on(
            &mut terminal,
            &overlaid(&empty, &key_introduction("Ctrl+]", false, Some(reason))),
        );
        terminal.send(b"\r").unwrap();
        shows(&mut terminal, &empty);
        terminal.send(b"\x1dx").unwrap();
        assert_released(&package, &mut terminal, 0);
        assert_eq!(
            fs::read_to_string(preferences_file(&package)).unwrap(),
            text
        );
        assert!(
            monitor_logs(&package)
                .contains(&format!("monitor preference unusable problem={reason}")),
            "{}",
            monitor_logs(&package)
        );
    }
    fs::remove_file(preferences_file(&package)).unwrap();
    fs::create_dir(preferences_file(&package)).unwrap();
    let mut terminal = outer(&package, "climon2 monitor");
    let unreadable = "cannot read monitor-preferences.json: Is a directory (os error 21)";
    opened_on(
        &mut terminal,
        &overlaid(&empty, &key_introduction("Ctrl+]", false, Some(unreadable))),
    );
    terminal.send(b"\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    assert!(preferences_file(&package).is_dir());
    package.assert_clean();
}

/// HLP-01 (Monitor, Fault; derived scenario: an unreadable or malformed
/// preference file, with key hints off): the key introduction stays closed,
/// and the empty state shows why the saved choice is unknown where its key
/// hint would be: a visible diagnostic, not a hint. Prefix+? shows the reason
/// under the box too. A save that fails replaces it with the failure in both
/// places, and once a save succeeds over the file no problem is shown.
#[test]
fn without_key_hints_the_empty_state_shows_problems_with_the_saved_choice() {
    let package = Package::new();
    fs::write(
        config_file(&package),
        r#"{"monitor": {"show_key_hints": false}}"#,
    )
    .unwrap();
    fs::write(
        preferences_file(&package),
        r#"{"suppress_key_introduction":true}"#,
    )
    .unwrap();
    let reason =
        "monitor-preferences.json is malformed: missing field `version` at line 1 column 34";
    let diagnosed = empty_monitor(ROWS, COLS, &[EMPTY, reason]);
    let mut terminal = outer(&package, "climon2 monitor");
    opened_on(&mut terminal, &diagnosed);
    terminal.send(b"\x1d?").unwrap();
    shows(
        &mut terminal,
        &overlaid(&diagnosed, &key_introduction("Ctrl+]", false, Some(reason))),
    );
    let temporary = temporary_file(&package);
    fs::create_dir(&temporary).unwrap();
    terminal.send(b" ").unwrap();
    let exists = "cannot save monitor-preferences.json: File exists (os error 17)";
    let unsaved = empty_monitor(ROWS, COLS, &[EMPTY, exists]);
    shows(
        &mut terminal,
        &overlaid(&unsaved, &key_introduction("Ctrl+]", false, Some(exists))),
    );
    terminal.send(b"\r").unwrap();
    shows(&mut terminal, &unsaved);
    fs::remove_dir(&temporary).unwrap();
    terminal.send(b"\x1d? ").unwrap();
    let quiet = empty_monitor(ROWS, COLS, &[EMPTY]);
    shows(
        &mut terminal,
        &overlaid(&quiet, &key_introduction("Ctrl+]", true, None)),
    );
    terminal.send(b"\r").unwrap();
    shows(&mut terminal, &quiet);
    terminal.send(b"\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    assert_eq!(
        fs::read_to_string(preferences_file(&package)).unwrap(),
        SUPPRESSED
    );
    package.assert_clean();
}

/// HLP-01 (Fault: a temporary file or a preference file that cannot be
/// written): a save that fails is reported in the key introduction, never
/// claimed: the
/// box stays as saved, nothing is left behind, and the next start still opens
/// the introduction. First a directory occupies the monitor's temporary file,
/// so the save cannot create it; then a directory in place of the preference
/// file, which is unreadable and cannot be replaced, so the temporary file the
/// save wrote is removed again.
#[test]
fn a_failing_save_is_reported_in_the_key_introduction_and_saves_nothing() {
    let package = Package::new();
    let empty = expected_screen(ROWS, COLS, "Ctrl+]");
    let introduction = overlaid(&empty, &key_introduction("Ctrl+]", false, None));
    let mut blocked = outer(&package, "climon2 monitor");
    opened_on(&mut blocked, &introduction);
    let temporary = temporary_file(&package);
    fs::create_dir(&temporary).unwrap();
    blocked.send(b" ").unwrap();
    let exists = "cannot save monitor-preferences.json: File exists (os error 17)";
    let not_created = overlaid(&empty, &key_introduction("Ctrl+]", false, Some(exists)));
    shows(&mut blocked, &not_created);
    blocked.send(b"\x1dx").unwrap();
    assert_released(&package, &mut blocked, 0);
    assert!(!preferences_file(&package).exists());
    assert!(
        monitor_logs(&package).contains(&format!("monitor preference not saved problem={exists}"))
    );
    fs::remove_dir(&temporary).unwrap();
    let mut again = outer(&package, "climon2 monitor");
    opened_on(&mut again, &introduction);
    again.send(b"\x1dx").unwrap();
    assert_released(&package, &mut again, 0);

    fs::create_dir(preferences_file(&package)).unwrap();
    let mut replaced = outer(&package, "climon2 monitor");
    let unreadable = "cannot read monitor-preferences.json: Is a directory (os error 21)";
    opened_on(
        &mut replaced,
        &overlaid(&empty, &key_introduction("Ctrl+]", false, Some(unreadable))),
    );
    replaced.send(b" ").unwrap();
    let not_replaced = "cannot save monitor-preferences.json: Is a directory (os error 21)";
    let reported = overlaid(
        &empty,
        &key_introduction("Ctrl+]", false, Some(not_replaced)),
    );
    shows(&mut replaced, &reported);
    assert_eq!(home_entries(&package), ["logs", "monitor-preferences.json"]);
    replaced.send(b"\x1dx").unwrap();
    assert_released(&package, &mut replaced, 0);
    assert!(preferences_file(&package).is_dir());
    package.assert_clean();
}

/// Keys, reports and a paste that the open key introduction owns and that
/// change nothing: Ctrl+C, plain x, s, j and ?, Tab and Backspace, prefix+s,
/// prefix+j, prefix+d, prefix+Space, prefix+Enter, a doubled (literal)
/// prefix, a focus report, an SGR mouse click, and a paste holding Space,
/// Enter, prefix+x and prefix+?.
const INERT: &[u8] = b"\x03xsj?\t\x7f\x1ds\x1dj\x1dd\x1d \x1d\r\x1d\x1d\x1b[I\x1b[<0;6;6M\x1b[<0;6;6m\x1b[200~ \r\x1dx\x1d?\x1b[201~";

/// HLP-02 (Monitor, over the empty monitor): the open key introduction owns
/// every typed key and paste, and only its own actions and prefix+x act. Once
/// the monitor has read [`INERT`], a resize repaints it still open and
/// unchanged, and no preference was written. Enter closes it onto the empty
/// monitor; prefix+? opens it again, and prefix+x exits from it with status 0,
/// releasing the host with its title untouched. Over the empty monitor there
/// is no session to send input to, resize, acquire or acknowledge.
#[test]
fn the_key_introduction_is_modal_and_prefix_x_still_exits_from_it() {
    let package = Package::new();
    let empty = expected_screen(ROWS, COLS, "Ctrl+]");
    let mut terminal = outer(&package, "climon2 monitor");
    opened_on(
        &mut terminal,
        &overlaid(&empty, &key_introduction("Ctrl+]", false, None)),
    );
    let monitor = monitor_process(&package).0;
    read_alone(&mut terminal, monitor, INERT);
    terminal.resize(25, 81).unwrap();
    let small = expected_screen(25, 81, "Ctrl+]");
    let unchanged = overlaid(&small, &key_introduction("Ctrl+]", false, None));
    shows(&mut terminal, &unchanged);
    assert!(!preferences_file(&package).exists());
    terminal.send(b"\r").unwrap();
    shows(&mut terminal, &small);
    terminal.send(b"\x1d?").unwrap();
    shows(&mut terminal, &unchanged);
    terminal.send(b"\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    assert!(!preferences_file(&package).exists());
    package.assert_clean();
}

/// HLP-02 (Monitor, over the empty monitor): with Ctrl+[ (Escape) as the
/// prefix, Escape cannot close the key introduction, the accepted limitation.
/// Escape ? (prefix+?, also legacy Alt+?) opens it; a doubled Escape is one
/// literal Escape, which it owns, so a resize repaints it still open; a lone
/// Escape waits as the prefix, and x after it exits with status 0.
#[test]
fn with_escape_as_the_prefix_escape_cannot_close_the_key_introduction_and_prefix_x_still_exits() {
    let package = Package::new();
    fs::write(
        config_file(&package),
        r#"{"client": {"escape_prefix": "Ctrl+["}, "monitor": {"show_key_hints": false}}"#,
    )
    .unwrap();
    let mut terminal = outer(&package, "climon2 monitor");
    let quiet = empty_monitor(ROWS, COLS, &[EMPTY]);
    opened_on(&mut terminal, &quiet);
    terminal.send(b"\x1b?").unwrap();
    shows(
        &mut terminal,
        &overlaid(&quiet, &key_introduction("Ctrl+[", false, None)),
    );
    let monitor = monitor_process(&package).0;
    read_alone(&mut terminal, monitor, b"\x1b\x1b");
    terminal.resize(25, 81).unwrap();
    let still_open = overlaid(
        &empty_monitor(25, 81, &[EMPTY]),
        &key_introduction("Ctrl+[", false, None),
    );
    shows(&mut terminal, &still_open);
    read_alone(&mut terminal, monitor, b"\x1b");
    terminal.send(b"x").unwrap();
    assert_released(&package, &mut terminal, 0);
    package.assert_clean();
}

/// HLP-02 (Monitor, over the empty monitor): a doubled prefix reaches the
/// open key introduction as one literal prefix, the key it is. With Ctrl+M
/// (Enter) as the prefix, a lone Enter waits as the prefix, so Enter and a
/// key it does not complete, Space, reach nothing: once the monitor has read
/// them, a resize repaints the introduction still open and unchecked. Enter
/// Enter then continues as Enter does, saving nothing, and prefix+x still
/// exits.
#[test]
fn with_enter_as_the_prefix_a_doubled_enter_continues_from_the_key_introduction() {
    let package = Package::new();
    fs::write(
        config_file(&package),
        r#"{"client": {"escape_prefix": "Ctrl+M"}}"#,
    )
    .unwrap();
    let mut terminal = outer(&package, "climon2 monitor");
    opened_on(
        &mut terminal,
        &overlaid(
            &expected_screen(ROWS, COLS, "Ctrl+M"),
            &key_introduction("Ctrl+M", false, None),
        ),
    );
    let monitor = monitor_process(&package).0;
    read_alone(&mut terminal, monitor, b"\r ");
    terminal.resize(25, 81).unwrap();
    let small = expected_screen(25, 81, "Ctrl+M");
    shows(
        &mut terminal,
        &overlaid(&small, &key_introduction("Ctrl+M", false, None)),
    );
    terminal.send(b"\r\r").unwrap();
    shows(&mut terminal, &small);
    assert!(!preferences_file(&package).exists());
    terminal.send(b"\rx").unwrap();
    assert_released(&package, &mut terminal, 0);
    package.assert_clean();
}

/// A headless session running `script` in bash: its full session ID. Each
/// script holds until `$HOME/<finish>` exists, giving up after two minutes,
/// so a failed test leaves nothing running for long.
fn headless(package: &Package, script: &str, finish: &str) -> String {
    let script = format!(
        "{script}\nuntil [ -e \"$HOME/{finish}\" ] || [ \"$SECONDS\" -ge 120 ]; do sleep 0.05; done"
    );
    let output = support::scripted(
        package,
        "c2",
        &["exec", "--headless", "/bin/bash", "-c", &script],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Ends every headless session `headless` started, once `finish` exists, and
/// waits until their metadata is gone.
fn finish_sessions(package: &Package, finish: &str) {
    fs::write(package.isolation.home().join(finish), b"").unwrap();
    // Session metadata, not a listing, which could race an ending session.
    let sessions = package.isolation.climon2_home().join("sessions");
    let metadata = || {
        fs::read_dir(&sessions).map_or(0, |entries| {
            entries
                .filter(|entry| {
                    entry
                        .as_ref()
                        .unwrap()
                        .path()
                        .extension()
                        .is_some_and(|ext| ext == "pty")
                })
                .count()
        })
    };
    let deadline = Instant::now() + TIMEOUT;
    while metadata() > 0 {
        assert!(Instant::now() < deadline, "sessions never ended");
        thread::sleep(Duration::from_millis(20));
    }
}

/// Every `logs/<id>/library-*.log` the monitor's library instances created.
fn library_logs(package: &Package, id: &str) -> Vec<PathBuf> {
    let directory = package.isolation.climon2_home().join("logs").join(id);
    let mut logs: Vec<_> = fs::read_dir(directory)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("library-"))
                })
                .collect()
        })
        .unwrap_or_default();
    logs.sort();
    logs
}

/// Waits until a row of the host screen contains `text`.
fn shows_row(terminal: &mut Terminal, text: &str) {
    terminal
        .wait_for_screen(TIMEOUT, |screen| {
            screen.lines().any(|line| line.contains(text))
        })
        .unwrap_or_else(|_| panic!("{text:?} never shown:\n{}", terminal.screen()));
}

/// DSC-03 (Monitor), CTL-01 (Monitor), COM-01 (Monitor), LIB-01 (Monitor)
/// and HDL-01 (Monitor rows): a monitor started on an empty home lists
/// headless sessions as they are created, without being asked. Each gets one
/// passive client, reused across later passes: one start and one library log
/// in its own session log directory. Passive clients never attach or take
/// control. The daemon advertises no `status-v1`, so a connected row shows
/// Unavailable, never a fabricated status; its title arrives through
/// `title-v1`, and an untitled session shows its command and arguments. On
/// Linux the monitor maps exactly the session version's
/// `libclimon2_client.so`.
#[test]
fn sessions_appear_as_they_are_created_each_with_one_passive_client() {
    let package = Package::new();
    without_introduction(&package);
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    let titled = headless(&package, "printf '\\033]2;Alpha\\007'", "finish");
    shows(
        &mut terminal,
        &empty_monitor(
            ROWS,
            COLS,
            &["No session selected.", "Press Ctrl+] then x to exit."],
        ),
    );
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, "║Unavailable Alpha");
    let untitled = headless(&package, ": untitled", "finish");
    shows_row(&mut terminal, "║Unavailable /bin/bash -c : untitled");
    for id in [&titled, &untitled] {
        let logs = library_logs(&package, id);
        assert_eq!(logs.len(), 1, "{logs:?}");
    }
    for session in package.list() {
        assert_eq!(
            session["state"],
            serde_json::json!({"lifecycle": "detached", "attachments": 0, "controlled": false}),
            "{session}"
        );
    }
    if cfg!(target_os = "linux") {
        let monitor = monitor_process(&package).0;
        let maps = fs::read_to_string(format!("/proc/{monitor}/maps")).unwrap();
        let libraries: std::collections::BTreeSet<_> = maps
            .lines()
            .filter_map(|line| line.find('/').map(|start| &line[start..]))
            .filter(|path| path.ends_with("libclimon2_client.so"))
            .collect();
        let expected = package
            .executable_home()
            .canonicalize()
            .unwrap()
            .join(VERSION)
            .join("libclimon2_client.so");
        assert_eq!(
            libraries.into_iter().collect::<Vec<_>>(),
            [expected.to_str().unwrap()]
        );
    }
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// How a fake daemon answers each connection.
#[derive(Clone, Copy)]
enum Answer {
    /// The hello as `id`, advertising `daemon-wire-v1` and `monitor-v1`;
    /// then, if the peer goes on to send anything (a client's Open, never a
    /// probe), record it and hang up.
    Hello(&'static str),
    /// Read the hello and never answer, until the prober hangs up.
    Silent,
}

/// What a fake daemon heard, in the order its threads recorded it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Heard {
    /// A connection was accepted.
    Connected,
    /// After the hello, the peer sent more: a client, not a probe.
    Client,
    /// The daemon is about to hang up on that client.
    HangingUp,
    /// The peer hung up a silent connection.
    Closed,
}

/// Every fake daemon's record of one test: when, which session, what.
type Ear = std::sync::Arc<std::sync::Mutex<Vec<(Instant, &'static str, Heard)>>>;

fn hear(ear: &Ear, id: &'static str, heard: Heard) {
    ear.lock().unwrap().push((Instant::now(), id, heard));
}

/// What `ear` recorded of kind `heard`, in order.
fn heard(ear: &Ear, heard: Heard) -> Vec<(Instant, &'static str)> {
    ear.lock()
        .unwrap()
        .iter()
        .filter(|(_, _, kind)| *kind == heard)
        .map(|(at, id, _)| (*at, *id))
        .collect()
}

/// A fake daemon on its own socket, answering every connection until
/// dropped; its metadata file goes with it.
struct FakeDaemon {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    socket: PathBuf,
    metadata: PathBuf,
}

/// Valid metadata for `id` created at `created`, of this package's version,
/// whose daemon socket is `socket`.
fn fake_metadata(package: &Package, id: &str, created: &str, socket: &Path) -> PathBuf {
    let sessions = package.isolation.climon2_home().join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let path = sessions.join(format!("{id}.pty"));
    let metadata = serde_json::json!({
        "format_version": 1, "session_id": id, "socket_path": socket,
        "daemon_pid": std::process::id(), "daemon_start_time": 1,
        "daemon_linux_identity": {"boot_id": "00000000-0000-4000-8000-000000000000", "start_ticks": 1},
        "command": "fake", "args": [id], "cwd": package.isolation.home(), "depth": 1,
        "created_at": created, "climon2_version": VERSION,
    });
    fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    path
}

impl FakeDaemon {
    fn start(
        package: &Package,
        id: &'static str,
        created: &str,
        answer: Answer,
        ear: &Ear,
    ) -> Self {
        let socket = package
            .isolation
            .home()
            .join(format!("{}.sock", &id[id.len() - 4..]));
        let metadata = fake_metadata(package, id, created, &socket);
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        let ear = ear.clone();
        let thread = thread::spawn(move || {
            while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                let Ok((mut connection, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                hear(&ear, id, Heard::Connected);
                connection.set_nonblocking(false).unwrap();
                let ear = ear.clone();
                thread::spawn(move || answer_once(&mut connection, id, answer, &ear));
            }
        });
        Self {
            stop,
            thread: Some(thread),
            socket,
            metadata,
        }
    }
}

fn answer_once(
    connection: &mut std::os::unix::net::UnixStream,
    id: &'static str,
    answer: Answer,
    ear: &Ear,
) {
    use climon2_core::{
        hello::{Lifecycle, SessionState, advertise_revision1},
        protocol::{HelloReply, read_frame, write_frame},
        wire::{Advertisement, Family, FrameLimit, FrameLimits},
    };
    use std::io::Read;
    let _ = connection.set_read_timeout(Some(TIMEOUT));
    let _ = read_frame(connection);
    match answer {
        Answer::Hello(answered) => {
            let reply = HelloReply {
                session_id: answered.parse().unwrap(),
                climon2_version: VERSION.parse().unwrap(),
                protocol: 1,
            };
            let limit = FrameLimit::new(1_048_576).unwrap();
            let revision1 = Advertisement::new(
                [Family::DaemonWireV1, Family::MonitorV1],
                FrameLimits {
                    max_request_frame_bytes: limit,
                    max_server_frame_bytes: limit,
                },
            );
            let state = SessionState::new(Lifecycle::Detached, 0, false).unwrap();
            let hello = advertise_revision1(&reply, state, &revision1).unwrap();
            let _ = write_frame(connection, &hello);
            if connection.read(&mut [0; 1]).is_ok_and(|read| read > 0) {
                hear(ear, id, Heard::Client);
                hear(ear, id, Heard::HangingUp);
            }
        }
        Answer::Silent => {
            let _ = connection.read_to_end(&mut Vec::new());
            hear(ear, id, Heard::Closed);
        }
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = fs::remove_file(&self.socket);
        let _ = fs::remove_file(&self.metadata);
    }
}

/// Writes `config.jsonc` with these `monitor.limits` values.
fn limits_config(package: &Package, limits: serde_json::Value) {
    fs::write(
        config_file(package),
        serde_json::json!({"monitor": {"limits": limits}}).to_string(),
    )
    .unwrap();
}

/// DSC-04 (Monitor, Fault): malformed and oversized metadata add no row and
/// stay; a mismatched hello and a silent daemon show Unavailable rows, the
/// silent one only after the frozen hello deadline, and get no client. An
/// inaccessible sessions directory shows incomplete discovery while every
/// row and the live session's client stay, and a live session's metadata
/// disappearing never removes its row or tears its client down. No metadata
/// is ever deleted.
#[test]
fn uncertain_discovery_is_explicit_and_never_deletes_metadata_or_clients() {
    let package = Package::new();
    without_introduction(&package);
    limits_config(
        &package,
        serde_json::json!({"discovery": {"metadata_file_bytes": 4096}}),
    );
    let live = headless(&package, "printf '\\033]2;Live\\007'", "finish");
    let sessions = package.isolation.climon2_home().join("sessions");
    let malformed = sessions.join("not-a-session.pty");
    fs::write(&malformed, b"{}").unwrap();
    let oversized_id = "0d5c0000-0000-4000-8000-0000000000f1";
    let oversized = fake_metadata(
        &package,
        oversized_id,
        "2026-01-01T00:00:00.000Z",
        Path::new("/tmp/none"),
    );
    let mut padded = fs::read(&oversized).unwrap();
    padded.resize(4097, b' ');
    fs::write(&oversized, &padded).unwrap();
    let ear = Ear::default();
    let mismatched = FakeDaemon::start(
        &package,
        "0d5c0000-0000-4000-8000-0000000000f2",
        "2026-01-01T00:00:00.000Z",
        Answer::Hello("0d5c0000-0000-4000-8000-0000000000ff"),
        &ear,
    );
    let silent = FakeDaemon::start(
        &package,
        "0d5c0000-0000-4000-8000-0000000000f3",
        "2025-01-01T00:00:00.000Z",
        Answer::Silent,
        &ear,
    );
    let mut terminal = outer(&package, "c2 monitor");
    terminal
        .wait_for_text("No session selected.", TIMEOUT)
        .unwrap();
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, "║Unavailable Live");
    shows_row(
        &mut terminal,
        "║Unavailable fake 0d5c0000-0000-4000-8000-0000000000f2",
    );
    shows_row(
        &mut terminal,
        "║Unavailable fake 0d5c0000-0000-4000-8000-0000000000f3",
    );
    assert!(
        !terminal.screen().contains("0000000000f1"),
        "{}",
        terminal.screen()
    );

    terminal.send("\x1b").unwrap();
    terminal
        .wait_for_text("No session selected.", TIMEOUT)
        .unwrap();
    let parked = package.isolation.home().join("parked.pty");
    let live_metadata = sessions.join(format!("{live}.pty"));
    fs::rename(&live_metadata, &parked).unwrap();
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o000)).unwrap();
    shows_row(
        &mut terminal,
        "Discovery incomplete: cannot list sessions: Permission denied (os error 13)",
    );
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, "║Unavailable Live");
    terminal.send("\x1b").unwrap();
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();
    terminal
        .wait_for_screen(TIMEOUT, |screen| !screen.contains("Discovery incomplete"))
        .unwrap();
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, "║Unavailable Live");
    fs::rename(&parked, &live_metadata).unwrap();
    assert_eq!(library_logs(&package, &live).len(), 1);
    assert_eq!(heard(&ear, Heard::Client), []);
    for file in [
        &malformed,
        &oversized,
        &mismatched.metadata,
        &silent.metadata,
    ] {
        assert!(file.exists(), "{}", file.display());
    }
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    drop((mismatched, silent));
    fs::remove_file(&malformed).unwrap();
    fs::remove_file(&oversized).unwrap();
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// DSC-01 (Monitor, start-order case): with `scheduling.starts` at 1, clients
/// start one at a time, newest created first, with ascending full session ID
/// breaking ties; each start waits for the one before it to open or fail.
/// The fake daemons are the oracle: each records a client (a peer that sends
/// Open after the hello, which a probe never does) before hanging up on it.
#[test]
fn clients_start_one_at_a_time_newest_first_with_ascending_id_ties() {
    let package = Package::new();
    without_introduction(&package);
    limits_config(&package, serde_json::json!({"scheduling": {"starts": 1}}));
    let ids = [
        (
            "0d5c0000-0000-4000-8000-0000000000a2",
            "2026-01-01T00:00:02.000Z",
        ),
        (
            "0d5c0000-0000-4000-8000-0000000000b0",
            "2026-01-01T00:00:01.000Z",
        ),
        (
            "0d5c0000-0000-4000-8000-0000000000a3",
            "2026-01-01T00:00:03.000Z",
        ),
        (
            "0d5c0000-0000-4000-8000-0000000000a1",
            "2026-01-01T00:00:02.000Z",
        ),
    ];
    let ear = Ear::default();
    let fakes: Vec<_> = ids
        .iter()
        .map(|&(id, created)| FakeDaemon::start(&package, id, created, Answer::Hello(id), &ear))
        .collect();
    let mut terminal = outer(&package, "c2 monitor");
    terminal
        .wait_for_text("No session selected.", TIMEOUT)
        .unwrap();
    terminal.send("\x1ds").unwrap();
    for (id, _) in ids {
        shows_row(&mut terminal, &format!("║Unavailable fake {id}"));
    }
    let order = [ids[2].0, ids[3].0, ids[0].0, ids[1].0];
    let events: Vec<_> = ear
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, _, heard)| matches!(heard, Heard::Client | Heard::HangingUp))
        .map(|(_, id, heard)| (*id, *heard))
        .collect();
    let expected: Vec<_> = order
        .iter()
        .flat_map(|id| [(*id, Heard::Client), (*id, Heard::HangingUp)])
        .collect();
    assert_eq!(events, expected);
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    drop(fakes);
    package.assert_clean();
}

/// DSC-03 (Monitor, cadence cases): discovery starts at once, and each later
/// pass starts a second after the previous one finished, never overlapping
/// it, even when the selector is opened during a pass. A silent daemon holds
/// each pass's probe for the frozen two-second hello deadline, so its
/// probes are at least three seconds apart; a zero cadence, or one measured
/// from the pass's start, would put them two seconds apart. The bound
/// allows half a second of observer lag.
#[test]
fn discovery_passes_start_a_second_after_the_previous_one_ends_without_overlap() {
    let package = Package::new();
    without_introduction(&package);
    let ear = Ear::default();
    let silent = FakeDaemon::start(
        &package,
        "0d5c0000-0000-4000-8000-0000000000c1",
        "2026-01-01T00:00:00.000Z",
        Answer::Silent,
        &ear,
    );
    let started = Instant::now();
    let mut terminal = outer(&package, "c2 monitor");
    let probes = |count: usize| {
        let deadline = Instant::now() + TIMEOUT;
        while heard(&ear, Heard::Connected).len() < count {
            assert!(Instant::now() < deadline, "{:?}", ear.lock().unwrap());
            thread::sleep(Duration::from_millis(10));
        }
    };
    probes(1);
    // The first pass is still probing: open the selector during it.
    assert_eq!(heard(&ear, Heard::Closed), []);
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, "║Unavailable fake");
    probes(3);
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    drop(silent);
    let ear = ear.lock().unwrap().clone();
    assert!(ear[0].0 - started < TIMEOUT);
    // The third probe may still be open when the monitor exits.
    let kinds: Vec<_> = ear.iter().map(|(_, _, heard)| *heard).take(5).collect();
    assert_eq!(
        kinds,
        [Heard::Connected, Heard::Closed].repeat(3)[..5],
        "passes overlapped: {ear:?}"
    );
    let connected: Vec<_> = ear
        .iter()
        .filter(|(_, _, heard)| *heard == Heard::Connected)
        .map(|(at, _, _)| *at)
        .collect();
    for pair in connected.windows(2) {
        let apart = pair[1] - pair[0];
        assert!(apart >= Duration::from_millis(2500), "{apart:?}: {ear:?}");
    }
    package.assert_clean();
}

/// Bash that sets its session's title to `title`, then holds as
/// [`headless`]'s scripts do.
fn titled_hold(title: &str) -> String {
    format!(
        "printf '\\033]2;{title}\\007'; until [ -e \"$HOME/finish\" ] || [ \"$SECONDS\" -ge 120 ]; do sleep 0.05; done"
    )
}

/// The listed session whose arguments contain `text`.
fn listed(package: &Package, text: &str) -> Value {
    package
        .list()
        .into_iter()
        .find(|session| session["args"].to_string().contains(text))
        .unwrap_or_else(|| panic!("no session runs {text:?}"))
}

/// Own-session scenario (confirmed 2026-11-08), DSC-06 (Monitor), HDL-01
/// (Monitor rows) and CTL-01 (Monitor): a monitor running inside a session
/// shows that session as This monitor and starts no client for it, while a
/// nested headless session and a session controlled elsewhere appear with
/// their titles, wide text included, in the home they share; labels keep the
/// selector's border whole however wide or long (TRM-06 label cases, SEL-01
/// cell-safe clipping). Passive clients leave the controller in control.
#[test]
fn the_own_session_is_this_monitor_and_nested_and_controlled_sessions_appear() {
    let package = Package::new();
    without_introduction(&package);
    let mut elsewhere = package.command("c2");
    elsewhere.args(["exec", "/bin/bash", "-c", &titled_hold("Elsewhere")]);
    let mut elsewhere = Terminal::spawn(elsewhere, ROWS, COLS).unwrap();
    // The controlled session's metadata, not a listing that could race its
    // creation.
    let sessions = package.isolation.climon2_home().join("sessions");
    let deadline = Instant::now() + TIMEOUT;
    while fs::read_dir(&sessions).map_or(true, |mut entries| entries.next().is_none()) {
        assert!(
            Instant::now() < deadline,
            "the controlled session never started"
        );
        thread::sleep(Duration::from_millis(20));
    }
    let nested = format!(
        "c2 exec --headless /bin/bash -c {}",
        quote(Path::new(&titled_hold("Nested 日本")))
    );
    headless(&package, &nested, "finish");
    let mut terminal = outer(&package, "c2 exec /bin/bash -c 'c2 monitor'");
    terminal
        .wait_for_text("No session selected.", TIMEOUT)
        .unwrap();
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, "║This monitor /bin/bash -c c2 monitor ");
    // The host's screen text follows each wide character with its covered cell.
    shows_row(&mut terminal, "║Unavailable  Nested 日 本 ");
    shows_row(&mut terminal, "║Unavailable  Elsewhere");
    shows_row(
        &mut terminal,
        "║Unavailable  /bin/bash -c c2 exec --headless",
    );
    // Wide labels, and the long one clipped to the list, keep the border whole.
    for row in terminal.screen().lines().filter(|row| row.contains('║')) {
        assert!(row.ends_with("║ "), "{row:?}");
    }

    let own = listed(&package, "c2 monitor");
    let own = own["session_id"].as_str().unwrap();
    assert!(library_logs(&package, own).is_empty());
    // The session that started the nested one names it in its arguments too.
    let inner = package
        .list()
        .into_iter()
        .find(|session| {
            let args = session["args"].to_string();
            args.contains("Nested") && !args.contains("--headless")
        })
        .unwrap();
    assert_eq!(inner["depth"], 2);
    assert_eq!(
        listed(&package, "Elsewhere")["state"],
        serde_json::json!({"lifecycle": "attached", "attachments": 1, "controlled": true})
    );
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    assert_eq!(elsewhere.wait_exit(TIMEOUT).unwrap(), 0);
    package.assert_clean();
}

/// The basic selector over `screen` as SEL-01 specifies it for #106: a
/// captionless double border one cell in from the rows below the top row,
/// and each `(status, label)` row with the status column as wide as its
/// widest status.
fn with_selector(screen: &str, rows: &[(&str, &str)]) -> String {
    let mut lines: Vec<String> = screen.split('\n').map(str::to_owned).collect();
    let (height, width) = (lines.len(), lines[0].chars().count());
    let inner = width - 4;
    let status = rows
        .iter()
        .map(|(status, _)| status.len())
        .max()
        .unwrap_or(0);
    let mut boxed = vec![format!(" ╔{}╗ ", "═".repeat(inner))];
    for index in 0..height - 5 {
        let text = rows
            .get(index)
            .map(|(state, label)| format!("{state:status$} {label}"))
            .unwrap_or_default();
        boxed.push(format!(" ║{text:<inner$}║ "));
    }
    boxed.push(format!(" ╚{}╝ ", "═".repeat(inner)));
    lines.splice(2..height - 1, boxed);
    lines.join("\n")
}

/// The rows of the open selector's list whose cells are reversed.
fn highlighted_rows(terminal: &Terminal) -> Vec<usize> {
    (3..usize::from(ROWS) - 2)
        .filter(|&row| terminal.cells(row)[2].rendition.inverse)
        .collect()
}

/// SEL-01 (Monitor, basic cases): prefix+s overlays the captionless double
/// border with status and title columns and the first row highlighted,
/// reversed; Up and Down move the highlight without wrapping, prefix+s while
/// open changes nothing, and Escape closes it, back to the body. Every step
/// is followed by a key with a known effect, so a step that wrapped or
/// reopened the selector ends on another row. Opening it
/// never resizes or acquires a session: the sessions keep their size and
/// stay detached.
#[test]
fn the_selector_lists_rows_moves_its_highlight_and_closes_without_resizing() {
    let package = Package::new();
    without_introduction(&package);
    let sized = |name: &str| {
        format!(
            "stty size > \"$HOME/{name}.size\"; trap 'touch \"$HOME/{name}.winch\"' WINCH; printf '\\033]2;{name}\\007'"
        )
    };
    for name in ["A1", "B2", "C3"] {
        headless(&package, &sized(name), "finish");
    }
    let mut terminal = outer(&package, "c2 monitor");
    let body = empty_monitor(
        ROWS,
        COLS,
        &["No session selected.", "Press Ctrl+] then x to exit."],
    );
    terminal
        .wait_for_text("No session selected.", TIMEOUT)
        .unwrap();
    terminal.send("\x1ds").unwrap();
    let open = with_selector(
        &body,
        &[
            ("Unavailable", "C3"),
            ("Unavailable", "B2"),
            ("Unavailable", "A1"),
        ],
    );
    shows(&mut terminal, &open);
    assert_eq!(highlighted_rows(&terminal), [3]);
    // Each step's keys end on a row that differs from where the step
    // started, and that no wrapping, dropped or reopening step could reach:
    // Up at the top then Down; Down at the bottom then Up; prefix+s while
    // open, which must not reopen on the first row, then Down.
    for (keys, expected) in [
        (&b"\x1b[A\x1b[B"[..], 4),
        (b"\x1b[B", 5),
        (b"\x1b[B\x1b[A", 4),
        (b"\x1ds\x1b[B", 5),
        (b"\x1b[A\x1b[A", 3),
    ] {
        terminal.send(keys).unwrap();
        let deadline = Instant::now() + TIMEOUT;
        while highlighted_rows(&terminal) != [expected] {
            assert!(
                Instant::now() < deadline,
                "{keys:?}: {:?}",
                highlighted_rows(&terminal)
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(terminal.screen(), open);
    }
    terminal.send("\x1b").unwrap();
    shows(&mut terminal, &body);
    for session in package.list() {
        assert_eq!(session["state"]["lifecycle"], "detached", "{session}");
    }
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    let home = package.isolation.home();
    for name in ["A1", "B2", "C3"] {
        assert_eq!(
            fs::read_to_string(home.join(format!("{name}.size")))
                .unwrap()
                .trim(),
            "24 80"
        );
        assert!(
            !home.join(format!("{name}.winch")).exists(),
            "{name} was resized"
        );
    }
    package.assert_clean();
}

/// The top row's key hint beside a selected session, with the default
/// prefix.
const SESSIONS_HINT: &str = "Ctrl+] then s: sessions";
/// The status a selected session shows until status reaches the monitor
/// (#95, #107), and the control the monitor holds once input may go.
const STATUS: &str = "Unavailable";
const CONTROLLING: &str = "Controlling";

/// The top row with a session selected, as #108 lays it out (owner
/// decision, 2026-11-14): the label from the left, then fixed slots at the
/// right edge, each after ` | `: the status (`fields[0]`) in 18 cells, the
/// control (`fields[1]`) in 15 and any key hint (`fields[2]`).
fn session_top_row(cols: usize, label: &str, fields: &[&str]) -> String {
    let mut slots = format!(" | {:<18} | {:<15}", fields[0], fields[1]);
    if let Some(hint) = fields.get(2) {
        slots.push_str(&format!(" | {hint}"));
    }
    let width = cols - slots.chars().count();
    format!("{label:<width$}{slots}")
}

/// Waits until the host's top row is exactly `expected`.
fn top_row_is(terminal: &mut Terminal, expected: &str) {
    terminal
        .wait_for_screen(TIMEOUT, |screen| screen.lines().next() == Some(expected))
        .unwrap_or_else(|_| panic!("top row never {expected:?}:\n{}", terminal.screen()));
}

/// Waits until the host shows its cursor at `row` and `column`: a frame's
/// cursor follows its cells, and a host read can split them.
fn cursor_at(terminal: &Terminal, row: usize, column: usize) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let cursor = terminal.cursor();
        if (cursor.row, cursor.column, cursor.visible) == (row, column, true) {
            return;
        }
        assert!(Instant::now() < deadline, "cursor {cursor:?}");
        thread::sleep(Duration::from_millis(10));
    }
}

/// Opens the selector once `label`'s session is connected (listed as
/// Unavailable without `status-v1`), moves the highlight `downs` rows down
/// from the first and selects that row with Enter.
fn select(terminal: &mut Terminal, label: &str, downs: usize) {
    terminal.send("\x1ds").unwrap();
    shows_row(terminal, &format!("║{STATUS} {label}"));
    let mut keys = "\x1b[B".repeat(downs);
    keys.push('\r');
    terminal.send(keys).unwrap();
}

/// Bash that titles its session `title`, then, until `$HOME/finish` exists,
/// appends each new `stty size` of its terminal to `$HOME/<title>.sizes`.
/// A `$HOME/<title>.probe` seen before a read is renamed `.probed` after it.
fn sized(title: &str) -> String {
    format!(
        "printf '\\033]2;{title}\\007'; p=; \
         until [ -e \"$HOME/finish\" ] || [ \"$SECONDS\" -ge 120 ]; do \
         q=; [ -e \"$HOME/{title}.probe\" ] && q=1; \
         s=$(stty size); if [ \"$s\" != \"$p\" ]; then echo \"$s\" >> \"$HOME/{title}.sizes\"; p=$s; fi; \
         [ -n \"$q\" ] && mv \"$HOME/{title}.probe\" \"$HOME/{title}.probed\"; \
         sleep 0.05; done"
    )
}

/// The sizes [`sized`]'s session `title` has had once it has read its size
/// after this call: every resize applied before the call is among them.
fn settled_sizes(package: &Package, title: &str) -> Vec<String> {
    let home = package.isolation.home();
    let probed = home.join(format!("{title}.probed"));
    let _ = fs::remove_file(&probed);
    fs::write(home.join(format!("{title}.probe")), b"").unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while !probed.exists() {
        assert!(Instant::now() < deadline, "{title} never read its size");
        thread::sleep(Duration::from_millis(20));
    }
    sizes(package, title)
}

/// The sizes [`sized`]'s session `title` has had, oldest first.
fn sizes(package: &Package, title: &str) -> Vec<String> {
    let path = package.isolation.home().join(format!("{title}.sizes"));
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Waits until session `title` has had exactly the sizes `expected`.
fn sized_to(package: &Package, title: &str, expected: &[&str]) {
    let deadline = Instant::now() + TIMEOUT;
    while sizes(package, title) != expected {
        assert!(
            Instant::now() < deadline,
            "{title} sizes {:?}, not {expected:?}",
            sizes(package, title)
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// CTL-05 (Monitor) and TRM-01 (Monitor, basic cases), with spec #73's Enter
/// and initial selection before status (#108): nothing is selected at
/// startup however many sessions exist. Enter on a connected row selects and
/// acquires it, and its screen is painted directly below the top row from
/// the monitor's own model: plain, ANSI-coloured, bold-on-blue, wide and
/// combining text, each cell as written, and the cursor where the
/// application left it. Later output is painted live. A session created
/// later never moves the selection, and the monitor controls the session.
#[test]
fn enter_selects_and_acquires_a_session_and_paints_it_live() {
    let package = Package::new();
    without_introduction(&package);
    let script = r#"printf '\033]2;Paint\007plain \033[31mred\033[0m \033[1;44mbold\033[0m \xe7\x95\x8ce\xcc\x81\r\n\033[3;5H'; until [ -e "$HOME/more" ] || [ "$SECONDS" -ge 120 ]; do sleep 0.05; done; printf 'MORE'"#;
    headless(&package, script, "finish");
    let mut terminal = outer(&package, "c2 monitor");
    let unselected = empty_monitor(
        ROWS,
        COLS,
        &["No session selected.", "Press Ctrl+] then x to exit."],
    );
    shows(&mut terminal, &unselected);
    select(&mut terminal, "Paint", 0);
    let top = session_top_row(
        usize::from(COLS),
        "Paint",
        &[STATUS, CONTROLLING, SESSIONS_HINT],
    );
    top_row_is(&mut terminal, &top);
    // The host's row text has each cell's character alone, and a blank for
    // the column a wide character covers.
    assert_eq!(terminal.row(1).trim_end(), "plain red bold 界 e");
    let cells = terminal.cells(1);
    let red = Color::Ansi(AnsiColor::Red);
    assert_eq!(
        (cells[5].text.as_str(), cells[5].foreground),
        (" ", Color::Default)
    );
    for cell in &cells[6..9] {
        assert_eq!(cell.foreground, red, "{cell:?}");
        assert!(!cell.rendition.bold);
    }
    assert_eq!(cells[9].foreground, Color::Default);
    for cell in &cells[10..14] {
        assert!(cell.rendition.bold, "{cell:?}");
        assert_eq!(
            (cell.foreground, cell.background),
            (Color::Default, Color::Ansi(AnsiColor::Blue))
        );
    }
    assert_eq!(cells[14].background, Color::Default);
    assert_eq!(
        (cells[15].text.as_str(), cells[15].width),
        ("界", Width::Wide)
    );
    assert_eq!(cells[16].width, Width::Spacer);
    assert_eq!(
        (cells[17].text.as_str(), cells[17].width),
        ("e\u{301}", Width::Single)
    );
    cursor_at(&terminal, 3, 4);
    for row in 2..usize::from(ROWS) {
        assert_eq!(terminal.row(row).trim_end(), "", "row {row}");
    }

    fs::write(package.isolation.home().join("more"), b"").unwrap();
    terminal
        .wait_for_screen(TIMEOUT, |screen| {
            screen
                .lines()
                .nth(3)
                .is_some_and(|row| row.starts_with("    MORE"))
        })
        .unwrap();
    cursor_at(&terminal, 3, 8);

    headless(&package, &titled_hold("Later"), "finish");
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║{STATUS} Later"));
    // The selector covers the canvas: input could not reach a painted
    // frame of the session, so it is not Controlling until it is uncovered.
    top_row_is(
        &mut terminal,
        &session_top_row(
            usize::from(COLS),
            "Paint",
            &[STATUS, "Restoring", SESSIONS_HINT],
        ),
    );
    terminal.send("\x1b").unwrap();
    terminal
        .wait_for_screen(TIMEOUT, |screen| !screen.contains('║'))
        .unwrap();
    assert_eq!(terminal.row(0), top);
    assert_eq!(
        listed(&package, "Paint")["state"],
        serde_json::json!({"lifecycle": "attached", "attachments": 1, "controlled": true})
    );
    assert_eq!(listed(&package, "Later")["state"]["controlled"], false);
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// TRM-02 (Monitor): the session daemon alone answers a session's queries;
/// the monitor never asks the host anything, never forwards a session's
/// bytes (its queries and a title it sets) to the host, and shows the title
/// only as an inert label, so the host title stays the outer shell's.
#[test]
fn only_the_daemon_answers_session_queries_and_no_session_byte_reaches_the_host() {
    let package = Package::new();
    without_introduction(&package);
    let script = r#"printf '\033]2;Queries\007\033[6n'; IFS= read -rs -t 10 -d R reply; printf 'GOT %s\r\n\033[c' "${reply#*[}""#;
    headless(&package, script, "finish");
    let mut terminal = outer(&package, "c2 monitor");
    terminal
        .wait_for_text("No session selected.", TIMEOUT)
        .unwrap();
    select(&mut terminal, "Queries", 0);
    top_row_is(
        &mut terminal,
        &session_top_row(
            usize::from(COLS),
            "Queries",
            &[STATUS, CONTROLLING, SESSIONS_HINT],
        ),
    );
    assert_eq!(terminal.row(1).trim_end(), "GOT 1;1");
    assert_eq!(terminal.host_requests(), Vec::<String>::new());
    let received = terminal.received();
    for forwarded in [&b"\x1b[6n"[..], b"\x1b[c", b"\x1b]2;Queries"] {
        assert!(
            !contains(&received, forwarded),
            "{forwarded:?} reached the host"
        );
    }
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// TRM-03 (Monitor) at 80x24, 200x60, the 500x200 boundary and a larger
/// host: one top row; the selected session is asked for the host width by
/// its height less one, which the daemon clamps to 500x200; the session's
/// bottom-right cell, written by absolute position at its own geometry,
/// lands there below the top row; and the margins beyond it stay blank.
#[test]
fn the_selected_session_fills_the_host_below_one_top_row_at_its_effective_geometry() {
    let hosts: [(u16, u16, (u16, u16)); 4] = [
        (24, 80, (23, 80)),
        (60, 200, (59, 200)),
        (201, 500, (200, 500)),
        (204, 503, (200, 500)),
    ];
    for (rows, cols, session) in hosts {
        let package = Package::new();
        without_introduction(&package);
        let corner = format!(
            "printf '\\033]2;Corner\\007'; until [ \"$(stty size)\" = '{} {}' ] || [ \"$SECONDS\" -ge 60 ]; do sleep 0.05; done; printf '\\033[{};{}HZ'",
            session.0, session.1, session.0, session.1
        );
        headless(&package, &corner, "finish");
        let mut terminal =
            support::outer(&package, Path::new("/bin/bash"), "c2 monitor", rows, cols);
        terminal
            .wait_for_text("No session selected.", TIMEOUT)
            .unwrap();
        select(&mut terminal, "Corner", 0);
        let (bottom, right) = (usize::from(session.0), usize::from(session.1) - 1);
        terminal
            .wait_for_screen(TIMEOUT, |screen| {
                screen
                    .lines()
                    .nth(bottom)
                    .and_then(|row| row.chars().nth(right))
                    == Some('Z')
            })
            .unwrap_or_else(|_| panic!("{rows}x{cols}: no corner"));
        let top = session_top_row(
            usize::from(cols),
            "Corner",
            &[STATUS, CONTROLLING, SESSIONS_HINT],
        );
        top_row_is(&mut terminal, &top);
        let screen = terminal.screen();
        let lines: Vec<&str> = screen.lines().collect();
        assert_eq!(lines.len(), usize::from(rows));
        for (index, line) in lines.iter().enumerate().skip(1) {
            let expected = if index == bottom {
                format!("{}Z", " ".repeat(right))
            } else {
                String::new()
            };
            assert_eq!(line.trim_end(), expected, "{rows}x{cols} row {index}");
        }
        cursor_at(&terminal, bottom, right);
        terminal.send("\x1dx").unwrap();
        support::assert_released(&package, &mut terminal, 0);
        finish_sessions(&package, "finish");
        package.assert_clean();
    }
}

/// CTL-05 (Monitor) and TRM-03 (real host resizes): Resize goes only to the
/// selected controller. Selecting a session sizes it to the host less the
/// top row; after another is selected, host resizes reach only that one,
/// although the first stays controlled in the background; selecting the
/// first again sizes it to the host as it is then.
#[test]
fn host_resizes_reach_only_the_selected_controller() {
    let package = Package::new();
    without_introduction(&package);
    headless(&package, &sized("First"), "finish");
    headless(&package, &sized("Second"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    terminal
        .wait_for_text("No session selected.", TIMEOUT)
        .unwrap();
    // Rows list newest first: Second, then First.
    select(&mut terminal, "First", 1);
    sized_to(&package, "First", &["24 80", "29 120"]);
    select(&mut terminal, "Second", 0);
    sized_to(&package, "Second", &["24 80", "29 120"]);
    terminal.resize(40, 100).unwrap();
    sized_to(&package, "Second", &["24 80", "29 120", "39 100"]);
    terminal.resize(35, 90).unwrap();
    sized_to(&package, "Second", &["24 80", "29 120", "39 100", "34 90"]);
    select(&mut terminal, "First", 1);
    sized_to(&package, "First", &["24 80", "29 120", "34 90"]);
    for session in package.list() {
        assert_eq!(session["state"]["controlled"], true, "{session}");
    }
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    assert_eq!(sizes(&package, "First"), ["24 80", "29 120", "34 90"]);
    package.assert_clean();
}

/// The host shows only `Terminal too small` in its top row, clipped to
/// its width, above blank rows, with no cursor.
fn shows_too_small(terminal: &mut Terminal, rows: u16, cols: u16) {
    let width = usize::from(cols);
    let shown: String = "Terminal too small".chars().take(width).collect();
    let blank = format!("\n{:width$}", "").repeat(usize::from(rows) - 1);
    shows(terminal, &format!("{shown:<width$}{blank}"));
    assert!(!terminal.cursor().visible);
}

/// TRM-04 (Monitor, too-small cases; owner decision 2026-11-14): a host
/// under four rows or five columns shows `Terminal too small` and sends no
/// resize, at each limit and one below it: once the too-small frame shows,
/// the session reads its size, and never had the one a resize would have
/// given it (2x120, 29x4). Four rows and five columns are room enough, and
/// each recovery resizes the selected session to the host less its top
/// row. The client and control survive throughout, and prefix+x still exits
/// while too small.
#[test]
fn a_host_too_small_for_a_session_says_so_sends_no_resize_and_recovers() {
    let package = Package::new();
    without_introduction(&package);
    headless(&package, &sized("Small"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    terminal
        .wait_for_text("No session selected.", TIMEOUT)
        .unwrap();
    select(&mut terminal, "Small", 0);
    sized_to(&package, "Small", &["24 80", "29 120"]);
    let fields = [STATUS, CONTROLLING, SESSIONS_HINT];
    let mut expected = vec!["24 80", "29 120"];
    for (rows, cols, size) in [
        (3, 120, None),
        (4, 120, Some("3 120")),
        (30, 4, None),
        (30, 5, Some("29 5")),
        (30, 120, Some("29 120")),
    ] {
        terminal.resize(rows, cols).unwrap();
        match size {
            None => {
                shows_too_small(&mut terminal, rows, cols);
                assert_eq!(settled_sizes(&package, "Small"), expected);
            }
            Some(size) => {
                expected.push(size);
                sized_to(&package, "Small", &expected);
            }
        }
    }
    top_row_is(
        &mut terminal,
        &session_top_row(usize::from(COLS), "Small", &fields),
    );
    assert_eq!(listed(&package, "Small")["state"]["controlled"], true);
    terminal.resize(3, 120).unwrap();
    shows_too_small(&mut terminal, 3, 120);
    terminal.send("\x1dx").unwrap();
    terminal.wait_for_bytes(b"OUTER-DONE=0", TIMEOUT).unwrap();
    // The outer shell's lines need more than three rows to be read back.
    terminal.resize(ROWS, COLS).unwrap();
    support::assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    assert_eq!(sizes(&package, "Small"), expected);
    package.assert_clean();
}

/// Top-row key hints (#108): with `monitor.show_key_hints` false, the top
/// row beside a selected session shows its label, status and control and
/// no hint.
#[test]
fn without_key_hints_the_top_row_shows_no_hint() {
    let package = Package::new();
    fs::write(
        config_file(&package),
        r#"{"monitor": {"show_key_hints": false}}"#,
    )
    .unwrap();
    headless(&package, &titled_hold("Plain"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    terminal
        .wait_for_text("No session selected.", TIMEOUT)
        .unwrap();
    select(&mut terminal, "Plain", 0);
    top_row_is(
        &mut terminal,
        &session_top_row(usize::from(COLS), "Plain", &[STATUS, CONTROLLING]),
    );
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// Spec #73 Enter ("never exposing cached output") and CTL-05 (revoked):
/// when another client takes control of the selected session, the monitor
/// shows it is no longer controlling and no longer paints the session, so
/// its now-cached screen never shows. The session's screen then changes
/// while the other client controls it; Enter takes control back, and only
/// the new restoration is painted, never the cache.
#[test]
fn a_revoked_session_shows_no_cached_output_until_enter_takes_control_again() {
    let package = Package::new();
    without_introduction(&package);
    let script = "printf '\\033]2;Taken\\007VISIBLE\\r\\n'; \
         until [ -e \"$HOME/change\" ] || [ \"$SECONDS\" -ge 120 ]; do sleep 0.05; done; \
         printf '\\033[2J\\033[HCHANGED\\r\\n'";
    let id = headless(&package, script, "finish");
    let mut terminal = outer(&package, "c2 monitor");
    terminal
        .wait_for_text("No session selected.", TIMEOUT)
        .unwrap();
    select(&mut terminal, "Taken", 0);
    let fields = |control| [STATUS, control, SESSIONS_HINT];
    let controlling = session_top_row(usize::from(COLS), "Taken", &fields(CONTROLLING));
    top_row_is(&mut terminal, &controlling);
    assert_eq!(terminal.row(1).trim_end(), "VISIBLE");

    let mut attach = package.command("c2");
    attach.args(["attach", &id]);
    let mut attach = Terminal::spawn(attach, ROWS, COLS).unwrap();
    attach.wait_for_text("VISIBLE", TIMEOUT).unwrap();
    let revoked = session_top_row(usize::from(COLS), "Taken", &fields("Not controlling"));
    // The whole frame, since a host read can split it after its top row.
    let blank = format!(
        "{revoked}{}",
        format!("\n{:width$}", "", width = usize::from(COLS)).repeat(usize::from(ROWS) - 1)
    );
    shows(&mut terminal, &blank);
    assert!(!terminal.cursor().visible);
    fs::write(package.isolation.home().join("change"), b"").unwrap();
    attach.wait_for_text("CHANGED", TIMEOUT).unwrap();

    select(&mut terminal, "Taken", 0);
    top_row_is(&mut terminal, &controlling);
    assert_eq!(terminal.row(1).trim_end(), "CHANGED");
    assert_eq!(terminal.row(2).trim_end(), "");
    attach
        .wait_for_text("This session is being viewed elsewhere.", TIMEOUT)
        .unwrap();
    attach.send("\x1dd").unwrap();
    assert_eq!(attach.wait_exit(TIMEOUT).unwrap(), 0);
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// The #94 rule for #108: a finished session's view freezes at its Command
/// final. Its last screen stays painted, the top row says it ended and,
/// once its connection closes, that it is disconnected; Enter on its row,
/// now known final, neither selects nor acquires anything.
#[test]
fn a_finished_session_keeps_its_last_screen_and_cannot_be_acquired() {
    let package = Package::new();
    without_introduction(&package);
    let script = "printf '\\033]2;Done\\007FINAL OUTPUT'; until [ -e \"$HOME/end\" ] || [ \"$SECONDS\" -ge 120 ]; do sleep 0.05; done; exit 3";
    headless(&package, script, "finish");
    let mut terminal = outer(&package, "c2 monitor");
    terminal
        .wait_for_text("No session selected.", TIMEOUT)
        .unwrap();
    select(&mut terminal, "Done", 0);
    top_row_is(
        &mut terminal,
        &session_top_row(
            usize::from(COLS),
            "Done",
            &[STATUS, CONTROLLING, SESSIONS_HINT],
        ),
    );
    fs::write(package.isolation.home().join("end"), b"").unwrap();
    let ended = session_top_row(
        usize::from(COLS),
        "Done",
        &["Disconnected", "Ended", SESSIONS_HINT],
    );
    top_row_is(&mut terminal, &ended);
    assert_eq!(terminal.row(1).trim_end(), "FINAL OUTPUT");
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, "║Disconnected Done");
    terminal.send("\r").unwrap();
    terminal.send("\x1b").unwrap();
    terminal
        .wait_for_screen(TIMEOUT, |screen| !screen.contains('║'))
        .unwrap();
    assert_eq!(terminal.row(0), ended);
    assert_eq!(terminal.row(1).trim_end(), "FINAL OUTPUT");
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}
