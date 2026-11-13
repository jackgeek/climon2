//! Spec #73 / #74: the packaged monitor, reached as `c2 monitor`, `c2 mon` or
//! their `climon2` spellings through the installed launcher, driven through
//! real PTYs from outer shells with isolated homes (the Monitor layer), on
//! macOS and Linux. Every scenario ends with `assert_clean()`.
#![cfg(any(target_os = "macos", target_os = "linux"))]

#[allow(dead_code)]
mod support;

use climon2_test_term::{InputTerminal, Modes, Terminal, bin_path};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use serde_json::Value;
use std::{
    env, fs,
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
const USAGE: &str = "climon2: error: usage: c2 exec <command> [args...] | attach <session-id> | kill <session-id> | list [--json] | monitor | mon\r\n";
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
            "config.jsonc is invalid: invalid value: integer `121`, expected an integer from 1 to 120",
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
    terminal
        .wait_for_screen(TIMEOUT, |screen| screen == repainted)
        .unwrap();
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

/// The one general log the monitor wrote.
fn monitor_log(package: &Package) -> String {
    let logs = package.logs();
    let monitor: Vec<_> = logs
        .iter()
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("monitor-")
        })
        .collect();
    assert_eq!(monitor.len(), 1, "{logs:?}");
    fs::read_to_string(monitor[0]).unwrap()
}

/// CFG-01 (Monitor): the monitor reports every effective setting, the
/// configured ones and every default, in its general log.
#[test]
fn installed_monitor_logs_its_effective_configuration() {
    let package = Package::new();
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
/// state's `lines` centred in the rows below it; an odd spare row falls above
/// the text and an odd spare column after it.
fn empty_monitor(rows: u16, cols: u16, lines: &[&str]) -> String {
    let (rows, cols) = (usize::from(rows), usize::from(cols));
    let first = 1 + (rows - lines.len()) / 2;
    (0..rows)
        .map(|row| match row {
            0 => format!("{TOP_ROW:<cols$}"),
            _ if (first..first + lines.len()).contains(&row) => {
                format!("{:^cols$}", lines[row - first])
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

/// The monitor owns the host: its own alternate screen showing the top row and
/// the empty state, raw input, bracketed paste, SGR-encoded button and drag
/// mouse reports and focus reports, no cursor, every other mode at the
/// baseline, and the host's title untouched.
fn opened(terminal: &mut Terminal) {
    terminal.wait_for_text(EMPTY, TIMEOUT).unwrap();
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
    let mut terminal = outer(&package, "climon2 monitor");
    opened(&mut terminal);
    for (rows, cols) in [(12, 50), (40, 160)] {
        terminal.resize(rows, cols).unwrap();
        let expected = expected_screen(rows, cols, "Ctrl+]");
        terminal
            .wait_for_screen(TIMEOUT, |screen| screen == expected)
            .unwrap();
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
