//! Spec #73 / #74: the packaged monitor, reached as `c2 monitor`, `c2 mon` or
//! their `climon2` spellings through the installed launcher, driven through
//! real PTYs from outer shells with isolated homes (the Monitor layer), on
//! macOS and Linux. Every scenario ends with `assert_clean()`.
#![cfg(any(target_os = "macos", target_os = "linux"))]

#[allow(dead_code)]
mod support;

use climon2_test_term::{
    AnsiColor, Color, CursorShape, InputTerminal, Modes, Mouse, MouseButton, Terminal, Underline,
    Width, bin_path,
};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
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
/// Escape for the selector, then a focus report that nothing takes: a lone
/// Escape waits for the byte after it (owner decision on #109), so the
/// report makes it take effect at once.
const ESCAPE: &str = "\x1b\x1b[I";
const EMPTY: &str = "No sessions to show.";

/// The [`support::outer`] shell, first titling its host and marking where
/// `command`'s output begins.
fn outer(package: &Package, command: &str) -> Terminal {
    outer_sized(package, command, COLS)
}

/// [`outer`] on a host `cols` wide.
fn outer_sized(package: &Package, command: &str, cols: u16) -> Terminal {
    support::outer(
        package,
        Path::new("/bin/bash"),
        &format!("printf '\\033]2;{TITLE}\\007'; printf 'BEFORE-MONITOR\\n'; {command}"),
        ROWS,
        cols,
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

/// One 500x200 session's mandatory allocation, as documented in
/// docs/monitor-operational-limits.md: its 4,850,887-byte current model and
/// the 12,115,200-byte history chunk that holds its recent page, and beside
/// them, in the aggregate budget only, the monitor's one 2 MiB parser (#166).
const MODEL_AND_PAGE: u64 = 16_966_087;
const WITH_PARSER: u64 = 19_063_239;
const PAGE: u64 = 12_115_200;

/// Memory limits of `total` bytes, `client` bytes per client, `history`
/// bytes of history in all and `per_session` bytes of each session's, with
/// every other budget the total contains small enough to fit it.
fn memory([total, client, history, per_session]: [u64; 4]) -> String {
    json!({"monitor": {"limits": {
        "memory": {
            "total_bytes": total,
            "client_bytes": client,
            "history_bytes_total": history,
            "history_bytes_per_session": per_session,
            "queue_bytes_total": 1_048_576
        },
        "resources": {"release_bytes": 1_048_576}
    }}})
    .to_string()
}

/// CFG-01 (Monitor): limits that cannot hold one session's mandatory
/// allocation at the largest session geometry, 500x200, beside the monitor's
/// one parser, are refused like every other invalid configuration: one byte
/// short of the total, a client's
/// budget, the history total or a session's history budget, the monitor
/// prints one diagnostic naming that limit and exits with the climon2 error
/// code before any terminal change. At exactly those limits it opens, and
/// prefix+x exits 0.
#[test]
fn installed_monitor_refuses_limits_that_cannot_hold_the_largest_session_model() {
    let package = Package::new();
    without_introduction(&package);
    let config = config_file(&package);
    for (limits, expected) in [
        (
            [WITH_PARSER - 1, MODEL_AND_PAGE, PAGE, PAGE],
            "the parser and the current model and recent page of a 500x200 session (19063239) may not exceed monitor.limits.memory.total_bytes (19063238)",
        ),
        (
            [WITH_PARSER, MODEL_AND_PAGE - 1, PAGE, PAGE],
            "the current model and recent page of a 500x200 session (16966087) may not exceed monitor.limits.memory.client_bytes (16966086)",
        ),
        (
            [WITH_PARSER, MODEL_AND_PAGE, PAGE - 1, PAGE - 1],
            "the recent page of a 500x200 session (12115200) may not exceed monitor.limits.memory.history_bytes_total (12115199)",
        ),
        (
            [WITH_PARSER, MODEL_AND_PAGE, PAGE, PAGE - 1],
            "the recent page of a 500x200 session (12115200) may not exceed monitor.limits.memory.history_bytes_per_session (12115199)",
        ),
    ] {
        let text = memory(limits);
        fs::write(&config, &text).unwrap();
        let mut terminal = outer(&package, "climon2 monitor");
        let diagnostic = refusal(&mut terminal);
        assert!(
            diagnostic.starts_with(&format!(
                "climon2: error: config.jsonc is invalid: {expected} (logs: "
            )),
            "{text}: {diagnostic:?}"
        );
        assert_released(&package, &mut terminal, 125);
    }
    fs::write(&config, memory([WITH_PARSER, MODEL_AND_PAGE, PAGE, PAGE])).unwrap();
    let mut terminal = outer(&package, "climon2 monitor");
    opened(&mut terminal);
    terminal.send(b"\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
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
      "memory": {"total_bytes": 603979776, "client_bytes": 67108864, "history_bytes_per_session": 16777216, "history_bytes_total": 268435456, "queue_bytes_total": 67108864},
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
    let top = format!("{TOP_ROW:<width$}", width = usize::from(cols));
    centred_below(&top, rows, cols, lines)
}

/// A host `rows` by `cols` showing the top row `top` and below it `lines`
/// centred as [`empty_monitor`] centres them.
fn centred_below(top: &str, rows: u16, cols: u16, lines: &[&str]) -> String {
    let (rows, cols) = (usize::from(rows), usize::from(cols));
    let first = 1 + (rows - lines.len()) / 2;
    (0..rows)
        .map(|row| match row {
            0 => top.to_owned(),
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
/// configured `prefix` followed by x, s and ?, the selector's keys (#113), the
/// Don't show again checkbox,
/// any `problem` with the saved choice, and the Continue action that Enter
/// takes, in a box titled Keys with one column of space inside its border.
fn key_introduction(prefix: &str, checked: bool, problem: Option<&str>) -> Vec<String> {
    let mut lines = vec![
        format!("{prefix} then x  Exit the monitor"),
        format!("{prefix} then s  Open the session list"),
        format!("{prefix} then ?  Show this dialog"),
        String::new(),
        "In the session list:".to_owned(),
        "Type or paste  Search by status and title".to_owned(),
        "Left, Right, Home, End, Backspace  Edit the search".to_owned(),
        "Up, Down, PageUp, PageDown  Move the highlight".to_owned(),
    ];
    // Enter as the prefix selects when typed twice; Escape as the prefix
    // cannot cancel (owner decision on #113: help lists the selector's keys).
    lines.push(match prefix {
        "Ctrl+M" => "Ctrl+M then Ctrl+M  Select the highlighted session".to_owned(),
        _ => "Enter  Select the highlighted session".to_owned(),
    });
    lines.extend((prefix != "Ctrl+[").then(|| "Esc  Cancel".to_owned()));
    lines.extend([
        "Click a row  Highlight it".to_owned(),
        "Click outside  Cancel".to_owned(),
        "Wheel  Scroll the list".to_owned(),
        String::new(),
        format!(
            "[{}] Don't show again (Space)",
            if checked { 'x' } else { ' ' }
        ),
    ]);
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
    whole_frame_shows(terminal, EMPTY, |screen| screen.contains(EMPTY));
    owns_host(terminal);
}

/// The monitor owns the host as [`opened`] checks, showing exactly `expected`.
fn opened_on(terminal: &mut Terminal, expected: &str) {
    shows(terminal, expected);
    owns_host(terminal);
}

/// Whether `output` ends where a monitor frame ends: its closing SGR 0, or
/// the cursor shown after it. Neither occurs inside a frame, whose cells
/// set SGR as `0;...` and whose cursor is hidden while they are written.
fn frame_ended(output: &[u8]) -> bool {
    output.ends_with(b"\x1b[0m") || output.ends_with(b"\x1b[?25h")
}

/// Waits until `condition` holds of a screen that shows only whole frames:
/// the output ended where a frame ends and nothing arrived while the screen
/// was read. A host read can split a frame, so the screen alone may show one
/// half painted. `what` names the condition for a failure.
fn whole_frame_shows(terminal: &Terminal, what: &str, condition: impl Fn(&str) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let before = terminal.received();
        let screen = terminal.screen();
        let whole = frame_ended(&before) && terminal.received() == before;
        if whole && condition(&screen) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what} never shown in a whole frame:\n{screen}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// Waits until a whole frame shows no selector.
fn selector_closed(terminal: &Terminal) {
    whole_frame_shows(terminal, "no selector", |screen| !screen.contains('║'));
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
    let reported = written
        .find(diagnostic)
        .unwrap_or_else(|| panic!("{diagnostic:?} missing from {written:?}"));
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

/// The monitor released the host before its one closed-terminal diagnostic,
/// logged it and exited 125, leaving nothing behind.
fn assert_closed_host_terminal(package: &Package, terminal: &mut Terminal) {
    assert_diagnosed_after_release(terminal, "climon2: error: the host terminal closed (logs: ");
    assert_released(package, terminal, 125);
    let log = monitor_log(package);
    assert!(
        log.contains("monitor failed error=the host terminal closed"),
        "{log}"
    );
    package.assert_clean();
}

/// EXT-03 (Monitor layer, empty monitor): when its host input hangs up, the
/// monitor ends at once instead of spinning: it releases the host it still
/// writes to, then reports the closed terminal, also in its general log, with
/// the climon2 error code. The terminal hangs up while the monitor waits for
/// input, so the monitor may read either the end of input or, while Linux is
/// still hanging the terminal up, EIO: the last close of its master can come
/// later than this test's, from a child that another test is spawning.
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
    input.hang_up();
    assert_closed_host_terminal(&package, &mut terminal);
}

/// EXT-03 (Monitor layer, empty monitor): host input that fails with EIO is
/// the closed host terminal too, as Linux fails reads while it is still
/// hanging a terminal up: the monitor releases the host before the same one
/// diagnostic, also in its general log, with the climon2 error code. Here the
/// monitor is a background job of the outer shell with SIGTTIN ignored, so
/// its read of the typed newline fails with EIO; the shell then reads that
/// newline once the monitor has restored termios, with echo off for that
/// read alone and then exactly the termios the monitor left: macOS re-runs
/// input that arrived in raw mode through its input processing at the next
/// canonical read (`PENDIN`), which would echo the newline after the
/// diagnostic, where Linux echoed nothing on arrival. Only the monitor writes
/// errors to the host: older bash (macOS `/bin/bash` 3.2, Ubuntu 24.04's
/// 5.2) reports the finished job (`[1]+  Exit 125 ...`) on its own stderr,
/// which goes nowhere.
#[test]
fn host_input_failing_with_eio_releases_the_host_before_its_diagnostic() {
    let package = Package::new();
    without_introduction(&package);
    let mut terminal = outer(
        &package,
        "trap '' TTIN TTOU; set -m; exec 3>&2 2>/dev/null; \
         climon2 monitor 2>&3 3>&- & wait $!; s=$?; set +m; exec 3>&-; \
         r=$(stty -g); stty -echo; IFS= read -r _; stty \"$r\"; (exit $s)",
    );
    terminal.wait_for_text(EMPTY, TIMEOUT).unwrap();
    terminal.send(b"\n").unwrap();
    assert_closed_host_terminal(&package, &mut terminal);
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
/// so a failed test leaves nothing running for long. Its daemon reads the
/// status clock [`clock`] gives it.
fn headless(package: &Package, script: &str, finish: &str) -> String {
    let at = clock(package, None);
    headless_on(package, script, finish, &at)
}

/// The [`T0`] of the daemons' controllable status clock,
/// 2027-01-15T08:00:00.000Z.
const T0: u64 = 1_800_000_000_000;

/// A new controllable status clock for one session's daemon, set to
/// `millis` after [`T0`], or by default one second later than the clock
/// made before it, starting a second after [`T0`]: so idle sessions sort
/// newest created first. Nothing advances it, so no heuristic sample ever
/// changes a session's status; only what its application reports does, at
/// the clock's time.
fn clock(package: &Package, millis: Option<u64>) -> PathBuf {
    let home = package.isolation.home();
    let made = fs::read_dir(home)
        .unwrap()
        .filter(|entry| {
            // Not the `.ack` files the daemons write beside their clocks.
            let name = entry.as_ref().unwrap().file_name();
            let name = name.to_string_lossy();
            name.starts_with("clock-") && !name.contains('.')
        })
        .count() as u64;
    let path = home.join(format!("clock-{made}"));
    let millis = millis.unwrap_or((made + 1) * 1000);
    fs::write(&path, (T0 + millis).to_string()).unwrap();
    path
}

/// [`headless`], with the daemon reading its status clock from `clock`.
fn headless_on(package: &Package, script: &str, finish: &str, clock: &Path) -> String {
    let script = format!(
        "{script}\nuntil [ -e \"$HOME/{finish}\" ] || [ \"$SECONDS\" -ge 120 ]; do sleep 0.05; done"
    );
    let output = support::scripted_with(
        package,
        "c2",
        &["exec", "--headless", "/bin/bash", "-c", &script],
        std::process::Stdio::piped(),
        &[("CLIMON2_DAEMON_CLOCK", clock)],
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

/// Waits until a whole frame has a row that contains `text`.
fn shows_row(terminal: &mut Terminal, text: &str) {
    whole_frame_shows(terminal, text, |screen| {
        screen.lines().any(|line| line.contains(text))
    });
}

/// Whether the packaged binaries are an exact release archive's, which has
/// no test hooks.
fn archive() -> bool {
    env::var_os("CLIMON2_E2E_ARCHIVE").is_some()
}

/// The status a real session whose title or screen changed shows once it
/// has settled. Hooked daemons read the status clock [`headless`] gives
/// them, which nothing advances, so it stays Idle. A release archive's
/// daemon has no such hook: its heuristic sees the change, and ten quiet
/// seconds later requests attention, which stays without user input.
fn settled() -> &'static str {
    if archive() { "Needs attention" } else { "Idle" }
}

/// The start of a selector row showing `status` and `title`, its status
/// column as wide as the widest of `status` and `others`.
fn row(others: &[&str], status: &str, title: &str) -> String {
    let width = others
        .iter()
        .fold(status.len(), |widest, other| widest.max(other.len()));
    format!("║{status:width$} {title}")
}

/// DSC-03 (Monitor, including its selection case), CTL-01 (Monitor),
/// COM-01 (Monitor), LIB-01 (Monitor) and HDL-01 (Monitor rows), with the
/// empty-start scenario (confirmed 2026-11-08): a monitor started on an
/// empty home lists headless sessions as they are created, without being
/// asked. The first eligible session to appear becomes the one-time
/// initial selection and is acquired (#266); a later arrival never moves
/// it and is never acquired. Each session gets one passive client, reused
/// across later passes: one start and one library log in its own session
/// log directory. A connected row shows the status its daemon reports
/// through `status-v1`, never a fabricated one (#107): Idle, or once a
/// release archive's daemon has seen its title change, Needs attention
/// ([`settled`]); its title
/// arrives through `title-v1`, and an untitled session shows its command
/// and arguments. On Linux the monitor maps exactly the session version's
/// `libclimon2_client.so`.
#[test]
fn sessions_appear_as_they_are_created_each_with_one_passive_client() {
    let package = Package::new();
    without_introduction(&package);
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    let titled = headless(&package, "printf '\\033]2;Alpha\\007'", "finish");
    let alpha = controlled("Alpha", settled());
    top_row_is(&mut terminal, &alpha);
    // Its screen and title never change, so it stays Idle on any clock.
    let untitled = headless(&package, ": untitled", "finish");
    terminal.send("\x1ds").unwrap();
    shows_row(
        &mut terminal,
        &row(&[settled()], "Idle", "/bin/bash -c : untitled"),
    );
    shows_row(&mut terminal, &row(&["Idle"], settled(), "Alpha"));
    terminal.send(ESCAPE).unwrap();
    top_row_is(&mut terminal, &alpha);
    for id in [&titled, &untitled] {
        let logs = library_logs(&package, id);
        assert_eq!(logs.len(), 1, "{logs:?}");
    }
    let attached =
        serde_json::json!({"lifecycle": "attached", "attachments": 1, "controlled": true});
    listed_as(&package, "Alpha", &attached);
    let detached =
        serde_json::json!({"lifecycle": "detached", "attachments": 0, "controlled": false});
    listed_as(&package, "untitled", &detached);
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
#[derive(Clone)]
enum Answer {
    /// The hello as `id`, advertising `daemon-wire-v1` and `monitor-v1`;
    /// then, if the peer goes on to send anything (a client's Open, never a
    /// probe), record it and hang up.
    Hello(&'static str),
    /// Read the hello and never answer, until the prober hangs up.
    Silent,
    /// As [`Answer::Silent`]; while the first connection is still open,
    /// once this flag is set (by a test that set it after sending a
    /// request), record [`Heard::Requested`]. Later connections are
    /// [`Answer::Silent`].
    Watched(std::sync::Arc<std::sync::atomic::AtomicBool>),
    /// The hello as [`Answer::Hello`] gives it; then hold the connection,
    /// answering nothing, until the peer hangs up or the fake is dropped:
    /// a client's Open never completes while it holds.
    Held(&'static str),
    /// The hello as its own session, advertising `title-v1`, `status-v1`
    /// and `focus-v1` too, as the real daemon does; then, after a client's
    /// Open, these acts in turn, then a successful answer to its Leave.
    Serve(std::sync::Arc<Vec<Act>>),
    /// As [`Answer::Serve`], without advertising `focus-v1`.
    Unfocused(std::sync::Arc<Vec<Act>>),
    /// The hello as `id`, advertising `daemon-wire-v1` alone, so a monitor
    /// client never sends its Open; then hold until the peer hangs up.
    Bare(&'static str),
    /// As [`Answer::Serve`], each client taking its turn's acts and
    /// recording that it was served and each request it sent after them.
    Turns(std::sync::Arc<Turns>),
}

/// Each client's acts in turn, the last list for every later client.
struct Turns {
    taken: std::sync::atomic::AtomicUsize,
    acts: Vec<Vec<Act>>,
}

impl Turns {
    fn answer(acts: Vec<Vec<Act>>) -> Answer {
        Answer::Turns(std::sync::Arc::new(Self {
            taken: std::sync::atomic::AtomicUsize::new(0),
            acts,
        }))
    }

    /// The next client's acts.
    fn next(&self) -> Vec<Act> {
        let turn = self.taken.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.acts[turn.min(self.acts.len() - 1)].clone()
    }
}

/// What a serving fake daemon does after a client's Open, as the daemon
/// wire reference specifies its frames.
#[derive(Clone)]
enum Act {
    /// A frame of `code` with a JSON body, in which the string `"$open"` is
    /// the Open's request ID.
    Send(u8, Value),
    /// A frame of `code` with this binary body.
    Raw(u8, Vec<u8>),
    /// Wait until this file exists, for at most [`TIMEOUT`].
    Await(PathBuf),
    /// Hang up.
    Close,
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
    /// A turn-taking fake read a client's Open and began its turn.
    Served,
    /// After its acts, a turn-taking fake read a request with this code.
    Request(u8),
    /// A watched first connection was still open after the test's request
    /// was sent.
    Requested,
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
        let mut answer = answer;
        let thread = thread::spawn(move || {
            while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                let Ok((mut connection, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                hear(&ear, id, Heard::Connected);
                connection.set_nonblocking(false).unwrap();
                let ear = ear.clone();
                let answer = match answer {
                    Answer::Watched(_) => std::mem::replace(&mut answer, Answer::Silent),
                    _ => answer.clone(),
                };
                let stopped = stopped.clone();
                thread::spawn(move || answer_once(&mut connection, id, &answer, &ear, &stopped));
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
    answer: &Answer,
    ear: &Ear,
    stopped: &std::sync::atomic::AtomicBool,
) {
    use climon2_core::{protocol::read_frame, wire::Family};
    use std::io::Read;
    let _ = connection.set_read_timeout(Some(TIMEOUT));
    let _ = read_frame(connection);
    match answer {
        Answer::Serve(acts) => {
            let families = [
                Family::MonitorV1,
                Family::TitleV1,
                Family::StatusV1,
                Family::FocusV1,
            ];
            if hello(connection, id, families) {
                serve(connection, acts);
            }
        }
        Answer::Unfocused(acts) => {
            let families = [Family::MonitorV1, Family::TitleV1, Family::StatusV1];
            if hello(connection, id, families) {
                serve(connection, acts);
            }
        }
        Answer::Bare(answered) => {
            let _ = hello(connection, answered, []);
            let _ = connection.read_to_end(&mut Vec::new());
        }
        Answer::Turns(turns) => {
            let families = [Family::MonitorV1, Family::TitleV1, Family::StatusV1];
            if hello(connection, id, families) {
                serve_heard(connection, || turns.next(), Some((ear, id)));
            }
        }
        Answer::Hello(answered) => {
            let _ = hello(connection, answered, [Family::MonitorV1]);
            if connection.read(&mut [0; 1]).is_ok_and(|read| read > 0) {
                hear(ear, id, Heard::Client);
                hear(ear, id, Heard::HangingUp);
            }
        }
        Answer::Silent => {
            let _ = connection.read_to_end(&mut Vec::new());
            hear(ear, id, Heard::Closed);
        }
        Answer::Watched(sent) => {
            // A read that times out shows the prober has neither hung up nor
            // sent more (a probe never does); one that returns shows it has.
            let _ = connection.set_read_timeout(Some(Duration::from_millis(10)));
            let open = |mut connection: &std::os::unix::net::UnixStream| {
                connection.read(&mut [0; 1]).is_err_and(|error| {
                    matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    )
                })
            };
            while open(connection) {
                if sent.load(std::sync::atomic::Ordering::SeqCst) {
                    if open(connection) {
                        hear(ear, id, Heard::Requested);
                    }
                    break;
                }
            }
            let _ = connection.set_read_timeout(Some(TIMEOUT));
            let _ = connection.read_to_end(&mut Vec::new());
            hear(ear, id, Heard::Closed);
        }
        Answer::Held(answered) => {
            let _ = hello(connection, answered, [Family::MonitorV1]);
            let _ = connection.set_read_timeout(Some(Duration::from_millis(20)));
            while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                match connection.read(&mut [0; 256]) {
                    Ok(0) => return,
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(_) => return,
                }
            }
        }
    }
}

/// Answers the hello as session `id`, advertising `daemon-wire-v1` and
/// `families`; whether the answer was written.
fn hello(
    connection: &mut std::os::unix::net::UnixStream,
    id: &str,
    families: impl IntoIterator<Item = climon2_core::wire::Family>,
) -> bool {
    use climon2_core::{
        hello::{Lifecycle, SessionState, advertise_revision1},
        protocol::{HelloReply, write_frame},
        wire::{Advertisement, FrameLimit, FrameLimits},
    };
    let reply = HelloReply {
        session_id: id.parse().unwrap(),
        climon2_version: VERSION.parse().unwrap(),
        protocol: 1,
    };
    let limit = FrameLimit::new(1_048_576).unwrap();
    let limits = FrameLimits {
        max_request_frame_bytes: limit,
        max_server_frame_bytes: limit,
    };
    let state = SessionState::new(Lifecycle::Detached, 0, false).unwrap();
    let hello = advertise_revision1(&reply, state, &Advertisement::new(families, limits)).unwrap();
    write_frame(connection, &hello).is_ok()
}

/// Serves one client: reads its Open, performs `acts`, then answers its
/// Leave and hangs up.
fn serve(connection: &mut std::os::unix::net::UnixStream, acts: &[Act]) {
    serve_heard(connection, || acts.to_vec(), None);
}

/// [`serve`] with the acts `acts` gives once the Open is read, recording
/// in `heard`, if given, the Open and every later request.
fn serve_heard(
    connection: &mut std::os::unix::net::UnixStream,
    acts: impl FnOnce() -> Vec<Act>,
    heard: Option<(&Ear, &'static str)>,
) {
    use climon2_core::wire::{Frame, FrameLimit, read_frame, write_frame};
    let limit = FrameLimit::new(1_048_576).unwrap();
    let Ok(Some(open)) = read_frame(connection, limit) else {
        return;
    };
    if let Some((ear, id)) = heard {
        hear(ear, id, Heard::Served);
    }
    let acts = acts();
    // The client stays until it leaves or the monitor exits.
    let _ = connection.set_read_timeout(None);
    let open: Value = serde_json::from_slice(open.body()).unwrap();
    let request = open["request_id"].to_string();
    let send = |connection: &mut std::os::unix::net::UnixStream, code: u8, body: &Value| {
        let body = body.to_string().replace("\"$open\"", &request);
        write_frame(connection, &Frame::new(code, body.into_bytes()), limit).is_ok()
    };
    for act in &acts {
        let sent = match act {
            Act::Send(code, body) => send(connection, *code, body),
            Act::Raw(code, body) => {
                write_frame(connection, &Frame::new(*code, body.clone()), limit).is_ok()
            }
            Act::Await(path) => {
                let deadline = Instant::now() + TIMEOUT;
                while !path.exists() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(10));
                }
                true
            }
            Act::Close => false,
        };
        if !sent {
            return;
        }
    }
    while let Ok(Some(frame)) = read_frame(connection, limit) {
        if let Some((ear, id)) = heard {
            hear(ear, id, Heard::Request(frame.code()));
        }
        if frame.code() == LEAVE {
            let leave: Value = serde_json::from_slice(frame.body()).unwrap();
            let left = json!({
                "request_id": leave["request_id"], "outcome": "ok",
                "result": {"kind": "left", "attachment_detached": false}, "error": null,
            });
            send(connection, RESPONSE, &left);
            return;
        }
    }
}

/// Daemon wire revision 1 frame types, from the daemon wire reference.
const LEAVE: u8 = 0x37;
const RESPONSE: u8 = 0x40;
const INITIAL_METADATA: u8 = 0x41;
const STATUS_CHANGED: u8 = 0x43;
const COMMAND_FINAL: u8 = 0x4c;

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
/// disappearing never removes its row or tears its client down. A session
/// proven gone (its socket missing, its daemon identity not running) that no
/// client ever reached adds no row and gets no client (stale rows, owner
/// decision on #106). No metadata is ever deleted. With the live session
/// connected without `status-v1`, no row is eligible, so nothing is
/// selected (DSC-01 confirmed exclusion, #266) and the body stays in view.
#[test]
fn uncertain_discovery_is_explicit_and_never_deletes_metadata_or_clients() {
    let package = Package::new();
    without_introduction(&package);
    limits_config(
        &package,
        serde_json::json!({"discovery": {"metadata_file_bytes": 4096}}),
    );
    let ear = Ear::default();
    let live = FakeDaemon::start(
        &package,
        fake_id(0xf5),
        "2026-01-02T00:00:00.000Z",
        serving(vec![
            opened_with(false),
            initial_metadata("Live", None, None),
        ]),
        &ear,
    );
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
    let stale_id = "0d5c0000-0000-4000-8000-0000000000f4";
    let stale = fake_metadata(
        &package,
        stale_id,
        "2026-01-01T00:00:00.000Z",
        &package.isolation.home().join("gone.sock"),
    );
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

    terminal.send(ESCAPE).unwrap();
    terminal
        .wait_for_text("No session selected.", TIMEOUT)
        .unwrap();
    let parked = package.isolation.home().join("parked.pty");
    fs::rename(&live.metadata, &parked).unwrap();
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o000)).unwrap();
    shows_row(
        &mut terminal,
        "Discovery incomplete: cannot list sessions: Permission denied (os error 13)",
    );
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, "║Unavailable Live");
    terminal.send(ESCAPE).unwrap();
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();
    terminal
        .wait_for_screen(TIMEOUT, |screen| !screen.contains("Discovery incomplete"))
        .unwrap();
    terminal.send("\x1ds").unwrap();
    // The live session's client was never torn down: its row is still
    // connected, never Disconnected.
    shows_row(&mut terminal, "║Unavailable Live");
    // A complete pass has reported every batch, the stale session's included.
    assert!(
        !terminal.screen().contains("0000000000f4"),
        "{}",
        terminal.screen()
    );
    assert_eq!(library_logs(&package, stale_id), Vec::<PathBuf>::new());
    fs::rename(&parked, &live.metadata).unwrap();
    assert_eq!(heard(&ear, Heard::Client), []);
    for file in [
        &malformed,
        &oversized,
        &stale,
        &mismatched.metadata,
        &silent.metadata,
    ] {
        assert!(file.exists(), "{}", file.display());
    }
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    drop((live, mismatched, silent));
    fs::remove_file(&malformed).unwrap();
    fs::remove_file(&oversized).unwrap();
    fs::remove_file(&stale).unwrap();
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
/// it, even when the selector is opened during a pass. With
/// `scheduling.probes` at 1, each pass probes three silent daemons one at a
/// time, each for the frozen two-second hello deadline, so the first pass
/// lasts about six seconds: the selector is requested in it under any load
/// that lets the test act within that time. That the request came during the
/// first pass is proven, not assumed: a daemon's first probe, still open
/// after the request was sent, records it. Each daemon's nth probe belongs to
/// the nth pass. A pass starts when its first probe connects and ends when
/// its last probe hangs up; a zero cadence, one measured from the pass's
/// start, or an overlap would start the next pass at or before that end. The
/// bound allows half the cadence for observer lag.
#[test]
fn discovery_passes_start_a_second_after_the_previous_one_ends_without_overlap() {
    let package = Package::new();
    without_introduction(&package);
    limits_config(&package, serde_json::json!({"scheduling": {"probes": 1}}));
    let ear = Ear::default();
    let sent = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ids = [
        "0d5c0000-0000-4000-8000-0000000000c1",
        "0d5c0000-0000-4000-8000-0000000000c2",
        "0d5c0000-0000-4000-8000-0000000000c3",
    ];
    let silent: Vec<_> = ids
        .iter()
        .map(|id| {
            let answer = Answer::Watched(sent.clone());
            FakeDaemon::start(&package, id, "2026-01-01T00:00:00.000Z", answer, &ear)
        })
        .collect();
    let started = Instant::now();
    let mut terminal = outer(&package, "c2 monitor");
    let connections = |count: usize| {
        let deadline = Instant::now() + TIMEOUT;
        while heard(&ear, Heard::Connected).len() < count {
            assert!(Instant::now() < deadline, "{:?}", ear.lock().unwrap());
            thread::sleep(Duration::from_millis(10));
        }
    };
    // The first pass has started: request the selector during it.
    connections(1);
    terminal.send("\x1ds").unwrap();
    sent.store(true, std::sync::atomic::Ordering::SeqCst);
    shows_row(&mut terminal, "║Unavailable fake");
    // The third pass has started.
    connections(2 * ids.len() + 1);
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    drop(silent);
    let ear = ear.lock().unwrap().clone();
    assert!(ear[0].0 - started < TIMEOUT);
    assert!(
        ear.iter().any(|(_, _, heard)| *heard == Heard::Requested),
        "the selector was not requested during the first pass: {ear:?}"
    );
    // When each daemon heard its nth record of a kind, if it did.
    let nth = |kind: Heard, n: usize| {
        ids.map(|id| {
            ear.iter()
                .filter(|(_, heard_by, heard)| *heard_by == id && *heard == kind)
                .map(|(at, _, _)| *at)
                .nth(n)
        })
    };
    for pass in 0..2 {
        // Every probe of this pass hung up; the next pass's first connected.
        let ended = nth(Heard::Closed, pass).map(|at| at.unwrap_or_else(|| panic!("{ear:?}")));
        let ended = ended.into_iter().max().unwrap();
        let began = nth(Heard::Connected, pass + 1).into_iter().flatten().min();
        let after = began.and_then(|began| began.checked_duration_since(ended));
        assert!(
            after.is_some_and(|after| after >= Duration::from_millis(500)),
            "pass {pass} to the next: {after:?}: {ear:?}"
        );
    }
    package.assert_clean();
}

/// [`titled_hold`], reporting a root OSC 7501 `clear` after the title: the
/// session stays Idle, never needing attention, on any clock.
fn quiet_hold(title: &str) -> String {
    titled_hold(title).replacen("; until", &format!("; {}; until", report("clear")), 1)
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

/// Own-session scenario (confirmed 2026-11-08; owner decision on #110,
/// 2026-11-14: hidden), DSC-06 (Monitor), HDL-01 (Monitor rows) and CTL-01
/// (Monitor): a monitor running inside a session never lists that session
/// and starts no client for it, while a nested headless session and a
/// session controlled elsewhere appear with their titles, wide text
/// included, in the home they share; labels keep the selector's border
/// whole however wide or long (TRM-06 label cases, SEL-01 cell-safe
/// clipping). A session created once the monitor runs appears in a later
/// pass, so every earlier pass, which found the monitor's own session too,
/// has been shown. Passive clients leave the controller in control.
#[test]
fn the_own_session_is_hidden_while_nested_and_controlled_sessions_appear() {
    let package = Package::new();
    without_introduction(&package);
    let mut elsewhere = package.command("c2");
    elsewhere.env("CLIMON2_DAEMON_CLOCK", clock(&package, None));
    elsewhere.args(["exec", "/bin/bash", "-c", &quiet_hold("Elsewhere")]);
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
        quote(Path::new(&working(&titled_hold("Nested 日本"))))
    );
    headless(&package, &nested, "finish");
    let mut terminal = outer(&package, "c2 exec /bin/bash -c 'c2 monitor'");
    // Initial selection takes one of the headless pair: never the
    // monitor's own session, nor the session controlled elsewhere, which
    // stays Idle with the oldest change time below them on any clock.
    whole_frame_shows(&terminal, "an initial selection", |screen| {
        screen.lines().next().is_some_and(|top| {
            top.contains(&format!(" | {CONTROLLING:<15} |")) && !top.starts_with("Elsewhere")
        })
    });
    terminal.send("\x1ds").unwrap();
    // The host's screen text follows each wide character with its covered cell.
    // The headless session that started the nested one never reported,
    // so its status is the daemon's own: [`settled`].
    let shown = [RUNNING, "Idle", settled()];
    for (status, title) in [
        (RUNNING, "Nested 日 本 "),
        ("Idle", "Elsewhere"),
        (settled(), "/bin/bash -c c2 exec --headless"),
    ] {
        shows_row(&mut terminal, &row(&shown, status, title));
    }
    headless(&package, &working(&titled_hold("Later")), "finish");
    shows_row(&mut terminal, &row(&shown, RUNNING, "Later"));
    // Every pass so far is shown: four rows, and none is the monitor's own.
    let rows: Vec<_> = terminal
        .screen()
        .lines()
        .filter(|row| {
            shown
                .iter()
                .any(|status| row.starts_with(&format!(" ║{status} ")))
        })
        .map(str::to_owned)
        .collect();
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert!(
        !terminal.screen().contains("c2 monitor"),
        "{}",
        terminal.screen()
    );
    // Wide labels, and the long one clipped to the list, keep the border
    // whole once the frame is painted: one snapshot may land mid-paint.
    terminal
        .wait_for_screen(TIMEOUT, |screen| {
            let bordered: Vec<_> = screen.lines().filter(|row| row.contains('║')).collect();
            !bordered.is_empty() && bordered.iter().all(|row| row.ends_with("║ "))
        })
        .unwrap_or_else(|_| panic!("a bordered row stays broken:\n{}", terminal.screen()));

    let own = listed(&package, "c2 monitor");
    let own = own["session_id"].as_str().unwrap();
    // The own session's client executable logs its own library use there
    // (#101), but the monitor's process must have started no client for it.
    let monitor = format!("-{}.log", monitor_process(&package).0);
    let logs = library_logs(&package, own);
    assert!(
        logs.iter()
            .all(|log| !log.to_string_lossy().ends_with(&monitor)),
        "{logs:?}"
    );
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

/// The selector's footer with key hints on and the default prefix (owner
/// decision on #113: only select, cancel and help).
const FOOTER: &str = "Enter: select  Esc: cancel  Ctrl+] then ?: help";

/// The selector over `screen`, with an empty search and the default
/// footer, as [`selector_over`] draws it.
fn with_selector(screen: &str, rows: &[(&str, &str)]) -> String {
    selector_over(screen, "", rows, Some(FOOTER))
}

/// The selector over `screen` as SEL-01 specifies it (#106, #113): a
/// captionless double border one cell in from the rows below the top row;
/// `search` on its first line; one separator joined with ╟ and ╢; each
/// `(status, label)` row with the status column as wide as its widest
/// status; and any `footer` at the bottom inside the border. ASCII text
/// only: each character is one cell.
fn selector_over(
    screen: &str,
    search: &str,
    rows: &[(&str, &str)],
    footer: Option<&str>,
) -> String {
    let mut lines: Vec<String> = screen.split('\n').map(str::to_owned).collect();
    let (height, width) = (lines.len(), lines[0].chars().count());
    let inner = width - 4;
    let status = rows
        .iter()
        .map(|(status, _)| status.len())
        .max()
        .unwrap_or(0);
    let mut boxed = vec![
        format!(" ╔{}╗ ", "═".repeat(inner)),
        format!(" ║{search:<inner$}║ "),
        format!(" ╟{}╢ ", "─".repeat(inner)),
    ];
    let list = height - 7 - usize::from(footer.is_some());
    for index in 0..list {
        let text = rows
            .get(index)
            .map(|(state, label)| format!("{state:status$} {label}"))
            .unwrap_or_default();
        boxed.push(format!(" ║{text:<inner$}║ "));
    }
    boxed.extend(footer.map(|footer| format!(" ║{footer:<inner$}║ ")));
    boxed.push(format!(" ╚{}╝ ", "═".repeat(inner)));
    lines.splice(2..height - 1, boxed);
    lines.join("\n")
}

/// The rows of the open selector's list whose cells are reversed.
fn highlighted_rows(terminal: &Terminal) -> Vec<usize> {
    highlighted_in(terminal, ROWS)
}

/// SEL-01 (Monitor, basic cases): prefix+s overlays the captionless double
/// border with status and title columns and the first row highlighted,
/// reversed; Up and Down move the highlight without wrapping, prefix+s while
/// open changes nothing, and Escape closes it, back to the selected
/// session. Every step is followed by a key with a known effect, so a step
/// that wrapped or reopened the selector ends on another row. After an
/// empty start, initial selection acquires and resizes C3, the first to
/// appear (#266); opening the selector never resizes or acquires a session:
/// the others keep their size and stay detached. Each reports `working`, so
/// the rows list newest first, A1, B2, C3, on any clock.
#[test]
fn the_selector_lists_rows_moves_its_highlight_and_closes_without_resizing() {
    let package = Package::new();
    without_introduction(&package);
    let sized = |name: &str| {
        format!(
            "stty size > \"$HOME/{name}.size\"; trap 'touch \"$HOME/{name}.winch\"' WINCH; printf '\\033]2;{name}\\007'"
        )
    };
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    headless(&package, &working(&sized("C3")), "finish");
    let body = centred_below(&controlled("C3", RUNNING), ROWS, COLS, &[]);
    shows(&mut terminal, &body);
    for name in ["B2", "A1"] {
        headless(&package, &working(&sized(name)), "finish");
    }
    terminal.send("\x1ds").unwrap();
    let open = with_selector(&body, &[(RUNNING, "A1"), (RUNNING, "B2"), (RUNNING, "C3")]);
    shows(&mut terminal, &open);
    // Rows reorder as their statuses arrive, and the highlight keeps the row
    // that was first when the selector opened: reopen it on the settled list.
    terminal.send(ESCAPE).unwrap();
    shows(&mut terminal, &body);
    terminal.send("\x1ds").unwrap();
    shows(&mut terminal, &open);
    // It opens on the selected session, C3, the last row (#113).
    assert_eq!(highlighted_rows(&terminal), [7]);
    // Each step's keys end on a row that differs from where the step
    // started, and that no wrapping, dropped or reopening step could reach:
    // Down at the bottom then Up; Up; prefix+s while open, which must not
    // reopen on C3, then Down; Down past the bottom; Up past the top then
    // Down.
    for (keys, expected) in [
        (&b"\x1b[B\x1b[A"[..], 6),
        (b"\x1b[A", 5),
        (b"\x1ds\x1b[B", 6),
        (b"\x1b[B\x1b[B", 7),
        (b"\x1b[A\x1b[A\x1b[A\x1b[B", 6),
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
    terminal.send(ESCAPE).unwrap();
    shows(&mut terminal, &body);
    for name in ["A1", "B2"] {
        assert_eq!(listed(&package, name)["state"]["lifecycle"], "detached");
    }
    assert_eq!(listed(&package, "C3")["state"]["lifecycle"], "attached");
    let winched = package.isolation.home().join("C3.winch");
    let deadline = Instant::now() + TIMEOUT;
    while !winched.exists() {
        assert!(Instant::now() < deadline, "C3 was never resized");
        thread::sleep(Duration::from_millis(20));
    }
    terminal.send("\x1dx").unwrap();
    // Its many frames can outgrow the oracle's recent bytes under load, so
    // only the title itself is checked.
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    finish_sessions(&package, "finish");
    let home = package.isolation.home();
    for name in ["A1", "B2", "C3"] {
        assert_eq!(
            fs::read_to_string(home.join(format!("{name}.size")))
                .unwrap()
                .trim(),
            "24 80"
        );
        let resized = home.join(format!("{name}.winch")).exists();
        assert_eq!(resized, name == "C3", "{name}");
    }
    package.assert_clean();
}

/// The rows of the open selector's list on a host `rows` high whose cells
/// are reversed.
fn highlighted_in(terminal: &Terminal, rows: u16) -> Vec<usize> {
    (3..usize::from(rows) - 2)
        .filter(|&row| terminal.cells(row)[2].rendition.inverse)
        .collect()
}

/// A left click at the host cell `row`, `column`: a press and its release.
fn click(terminal: &mut Terminal, row: usize, column: usize) {
    terminal
        .mouse(Mouse::Press(MouseButton::Left), row, column)
        .unwrap();
    terminal
        .mouse(Mouse::Release(MouseButton::Left), row, column)
        .unwrap();
}

/// A blank host `rows` by `cols` with the selector over it, as
/// [`selector_over`] draws it.
fn selector_on(
    (rows, cols): (u16, u16),
    search: &str,
    list: &[(&str, &str)],
    footer: Option<&str>,
) -> String {
    let blank = vec![" ".repeat(usize::from(cols)); usize::from(rows)].join("\n");
    selector_over(&blank, search, list, footer)
}

/// The selector's rows of `screen`, from its top border to its bottom
/// border, without the margin columns at either edge, where the selected
/// session's canvas shows.
fn boxed(screen: &str) -> Vec<String> {
    let rows: Vec<_> = screen.split('\n').collect();
    rows[2..rows.len() - 1]
        .iter()
        .map(|row| {
            let cells = row.chars().count();
            row.chars().skip(1).take(cells - 2).collect()
        })
        .collect()
}

/// The selector on a blank host `size`, as [`boxed`] shows it.
fn selector_box(
    size: (u16, u16),
    search: &str,
    list: &[(&str, &str)],
    footer: Option<&str>,
) -> Vec<String> {
    boxed(&selector_on(size, search, list, footer))
}

/// Waits until a whole frame shows the selector, and whatever covers it,
/// as `expected` (from [`boxed`]), with its list highlighting exactly the
/// host rows `highlighted`, on a host `rows` high.
fn boxed_as(terminal: &Terminal, rows: u16, expected: &[String], highlighted: &[usize]) {
    whole_frame_shows(terminal, &expected.join("\n"), |screen| {
        boxed(screen) == expected && highlighted_in(terminal, rows) == highlighted
    });
}

/// Waits until a whole frame shows exactly `expected` and the selector's
/// list highlights exactly the host rows `highlighted`, on the default host.
fn shows_highlighted(terminal: &Terminal, expected: &str, highlighted: &[usize]) {
    whole_frame_shows(terminal, expected, |screen| {
        screen == expected && highlighted_rows(terminal) == highlighted
    });
}

/// Selector rows, each its status and label.
type Rows<'a> = &'a [(&'a str, &'a str)];

/// SEL-01 (Monitor: search line, separator, footer, empty matches) and
/// SEL-02 (Contract and Monitor: the matcher, editing and paste): the
/// selector opens with an empty search on the selected session, the
/// search's cursor at its start. Typed text filters the rows by
/// `SkimMatcherV2` ordered-subsequence matching over the displayed status,
/// a space and the label, ignoring case beyond ASCII: a short query keeps
/// every row containing it, a subsequence, a spaced query, one crossing
/// from status into label, a Unicode one and wide text each keep exactly
/// their rows, always in row order, never by score (Build Replica, the
/// newer, stays above Build, the closer match), and a query nothing matches
/// empties the list under the footer. Each edit keeps a highlight that
/// still matches, otherwise highlights the first match. Left, Home and End
/// move the search's cursor and typing inserts there. A bracketed paste is
/// one line of search text: its CRLF a space, its prefix byte discarded and
/// its x inserted, so neither its prefix+x nor its CR acts. Enter then
/// selects the highlighted row.
#[test]
fn the_selector_search_filters_by_status_and_label_in_row_order() {
    let package = Package::new();
    without_introduction(&package);
    for title in ["ÉTÉ 日本", "fox bar", "Build", "Build Replica"] {
        headless(&package, &working(&titled_hold(title)), "finish");
    }
    let mut terminal = outer(&package, "c2 monitor");
    let body = beneath(&controlled("Build Replica", RUNNING));
    top_row_is(&mut terminal, &controlled("Build Replica", RUNNING));
    terminal.send("\x1ds").unwrap();
    // The host's screen text follows each wide character with its covered
    // cell.
    let (replica, build, fox, wide) = (
        (RUNNING, "Build Replica"),
        (RUNNING, "Build"),
        (RUNNING, "fox bar"),
        (RUNNING, "ÉTÉ 日 本 "),
    );
    let searched =
        |search: &str, rows: &[(&str, &str)]| selector_over(&body, search, rows, Some(FOOTER));
    shows_highlighted(&terminal, &searched("", &[replica, build, fox, wide]), &[5]);
    cursor_at(&terminal, 3, 2);
    // Each step: keys typed, the search then shown, the rows it lists and
    // the search cursor's column.
    let steps: [(&str, &str, Rows, usize); 7] = [
        ("b", "b", &[replica, build, fox], 3),
        ("ld", "bld", &[replica, build], 5),
        ("\x7f\x7f\x7fo b", "o b", &[fox], 5),
        ("\x7f\x7f\x7fnBu", "nBu", &[replica, build], 5),
        ("\x7f\x7f\x7fété", "été", &[wide], 5),
        ("\x7f\x7f\x7f日本", "日 本 ", &[wide], 6),
        ("\x7f\x7fzzz", "zzz", &[], 5),
    ];
    for (typed, search, rows, column) in steps {
        terminal.send(typed).unwrap();
        let highlighted: &[usize] = if rows.is_empty() { &[] } else { &[5] };
        shows_highlighted(&terminal, &searched(search, rows), highlighted);
        cursor_at(&terminal, 3, column);
    }
    terminal.send("\x7f\x7f\x7fbr\x1b[Da").unwrap();
    shows_highlighted(&terminal, &searched("bar", &[fox]), &[5]);
    cursor_at(&terminal, 3, 4);
    terminal.send("\x1b[Ho \x1b[F").unwrap();
    shows_highlighted(&terminal, &searched("o bar", &[fox]), &[5]);
    cursor_at(&terminal, 3, 7);
    terminal.send("\x7f".repeat(5)).unwrap();
    shows_highlighted(&terminal, &searched("", &[replica, build, fox, wide]), &[7]);
    terminal.paste("f\x1dx\r\nb").unwrap();
    shows_highlighted(&terminal, &searched("fx b", &[fox]), &[5]);
    cursor_at(&terminal, 3, 6);
    terminal.send("\r").unwrap();
    top_row_is(&mut terminal, &controlled("fox bar", RUNNING));
    selector_closed(&terminal);
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// The narrow host of
/// [`selector_navigation_mouse_and_wheel_on_narrow_and_large_hosts`], rows
/// by columns.
const NARROW: (u16, u16) = (14, 40);

/// SEL-01 (Monitor: narrow and large hosts, scrolling, cell-safe clipping),
/// SEL-02 (Monitor: navigation, Tab, clicks and the wheel), INP-05 (Monitor,
/// selector case) and TRM-03 (Monitor, overlay case): on a 40x14 host the
/// list shows six of eight rows and the footer ends in "…"; a long label
/// and a wide one end in "…" without splitting a character (owner decision
/// on #112). Tab and Shift+Tab change nothing; Down, PageDown, PageUp and Up
/// move the highlight a row or a page, never wrapping, scrolling to keep it
/// in view. The wheel scrolls the list a row a notch, as far as it goes,
/// without moving the highlight; a left click on a row only highlights it;
/// clicks on the search line, the border or the footer, or with the right
/// button, do nothing. Enter acquires the clicked row; reopened, the
/// selector highlights it, scrolled into view, and a click outside the box
/// cancels. Resized to 200x60, the open selector lists every row whole.
#[test]
fn selector_navigation_mouse_and_wheel_on_narrow_and_large_hosts() {
    let package = Package::new();
    without_introduction(&package);
    let long = "S1 with a title far too long for this narrow host";
    let mut titles = vec![long.to_owned(), format!("S2 {}", "日".repeat(20))];
    titles.extend((3..=8).map(|index| format!("S{index}")));
    for title in &titles {
        headless(&package, &working(&titled_hold(title)), "finish");
    }
    let mut terminal = support::outer(
        &package,
        Path::new("/bin/bash"),
        &format!("printf '\\033]2;{TITLE}\\007'; printf 'BEFORE-MONITOR\\n'; c2 monitor"),
        NARROW.0,
        NARROW.1,
    );
    let controlled_here =
        serde_json::json!({"lifecycle": "attached", "attachments": 1, "controlled": true});
    // Initial selection takes S8, the newest, before the selector opens.
    listed_as(&package, "]2;S8", &controlled_here);
    terminal.send("\x1ds").unwrap();
    let names: Vec<String> = (3..=8).rev().map(|index| format!("S{index}")).collect();
    let clipped_wide = format!("S2 {}…", "日 ".repeat(12));
    let mut rows: Vec<(&str, &str)> = names.iter().map(|name| (RUNNING, name.as_str())).collect();
    rows.push((RUNNING, &clipped_wide));
    rows.push((RUNNING, "S1 with a title far too lon…"));
    let footer = Some("Enter: select  Esc: cancel  Ctrl+] …");
    let window = |first: usize| selector_box(NARROW, "", &rows[first..first + 6], footer);
    boxed_as(&terminal, NARROW.0, &window(0), &[5]);
    let steps: [(&[u8], usize, &[usize]); 7] = [
        (b"\t\x1b[Z", 0, &[5]),
        (b"\x1b[B\x1b[B", 0, &[7]),
        (b"\x1b[6~", 2, &[10]),
        (b"\x1b[6~\x1b[B", 2, &[10]),
        (b"\x1b[5~", 1, &[5]),
        (b"\x1b[A", 0, &[5]),
        (b"\x1b[A", 0, &[5]),
    ];
    for (keys, first, highlighted) in steps {
        terminal.send(keys).unwrap();
        boxed_as(&terminal, NARROW.0, &window(first), highlighted);
    }
    for _ in 0..3 {
        terminal.mouse(Mouse::WheelDown, 6, 5).unwrap();
    }
    boxed_as(&terminal, NARROW.0, &window(2), &[]);
    click(&mut terminal, 7, 5);
    boxed_as(&terminal, NARROW.0, &window(2), &[7]);
    terminal.mouse(Mouse::WheelUp, 6, 5).unwrap();
    boxed_as(&terminal, NARROW.0, &window(1), &[8]);
    for (row, column) in [(3, 5), (5, 1), (11, 5)] {
        click(&mut terminal, row, column);
    }
    terminal
        .mouse(Mouse::Press(MouseButton::Right), 5, 5)
        .unwrap();
    terminal
        .mouse(Mouse::Release(MouseButton::Right), 5, 5)
        .unwrap();
    // Down then Up lands back on S4 only if no click moved the highlight.
    terminal.send("\x1b[B\x1b[A").unwrap();
    boxed_as(&terminal, NARROW.0, &window(1), &[8]);
    terminal.send("\r").unwrap();
    listed_as(&package, "]2;S4", &controlled_here);
    selector_closed(&terminal);
    terminal.send("\x1ds").unwrap();
    boxed_as(&terminal, NARROW.0, &window(0), &[9]);
    click(&mut terminal, 0, 10);
    selector_closed(&terminal);
    terminal.send("\x1ds").unwrap();
    boxed_as(&terminal, NARROW.0, &window(0), &[9]);
    terminal.resize(60, 200).unwrap();
    let full_wide = format!("S2 {}", "日 ".repeat(20));
    let mut whole: Vec<(&str, &str)> = names.iter().map(|name| (RUNNING, name.as_str())).collect();
    whole.push((RUNNING, &full_wide));
    whole.push((RUNNING, long));
    boxed_as(
        &terminal,
        60,
        &selector_box((60, 200), "", &whole, Some(FOOTER)),
        &[9],
    );
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// SEL-02 (Monitor: asynchronous title and status updates) and SEL-03
/// (Monitor and Fault: reorder and takeover): after an empty start Beta is
/// the initial selection and Alpha appears above it. A highlighted row whose
/// new title stops matching the search loses the highlight, and matching
/// again does not bring it back; Enter then selects nothing until Down
/// highlights the first match. Clearing the search keeps the highlight on
/// its row; when Beta's report moves it above Alpha and another client
/// takes Alpha over, Enter still selects and acquires exactly Alpha, and no
/// selector key reached a session.
#[test]
fn async_updates_clear_the_highlight_and_enter_pins_the_highlighted_session() {
    let package = Package::new();
    without_introduction(&package);
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    headless(&package, &watched("Beta"), "finish");
    ready_at(&mut terminal, "Beta", RUNNING, "29 120");
    let alpha = headless(&package, &watched("Alpha"), "finish");
    terminal.send("\x1ds").unwrap();
    listed_with(&terminal, RUNNING, "Alpha");
    terminal.send("al").unwrap();
    let searched = |search: &str, rows: &[(&str, &str)]| {
        selector_box((ROWS, COLS), search, rows, Some(FOOTER))
    };
    let alpha_row = [(RUNNING, "Alpha")];
    boxed_as(&terminal, ROWS, &searched("al", &alpha_row), &[5]);
    retitle(&package, "Alpha", "Omega");
    boxed_as(&terminal, ROWS, &searched("al", &[]), &[]);
    retitle(&package, "Alpha", "Alpha");
    boxed_as(&terminal, ROWS, &searched("al", &alpha_row), &[]);
    // Enter selects nothing: the selector stays open over Beta, and the
    // Down after it highlights the first match.
    terminal.send("\r\x1b[B").unwrap();
    boxed_as(&terminal, ROWS, &searched("al", &alpha_row), &[5]);
    top_row_is(&mut terminal, &controlled("Beta", RUNNING));
    terminal.send("\x7f\x7f").unwrap();
    let both = [(RUNNING, "Alpha"), (RUNNING, "Beta")];
    boxed_as(&terminal, ROWS, &searched("", &both), &[5]);
    report_state(&package, "Beta", "idle");
    let moved = [("Needs attention", "Beta"), (RUNNING, "Alpha")];
    boxed_as(&terminal, ROWS, &searched("", &moved), &[6]);
    let mut other = attach(&package, &alpha);
    terminal.send("\r").unwrap();
    ready_at(&mut terminal, "Alpha", RUNNING, "29 120");
    other.wait_for_text(VIEWED_ELSEWHERE, TIMEOUT).unwrap();
    terminal.send("z").unwrap();
    received_by(&package, "Alpha", b"z");
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    received_by(&package, "Beta", b"");
    detach(other);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// SEL-03 (Monitor and Fault: loss): after an empty start Beta is the
/// initial selection, and Alpha then Gamma appear above it. A session lost
/// while its row is highlighted (Gamma's daemon killed) turns Disconnected
/// and moves below the connected rows, and the highlight moves with it.
/// Enter there acquires nothing, neither that session, which it selects
/// read-only (#116), nor Alpha, which the loss moved into the highlighted
/// row's old place. Reopened, the selector's Enter on Alpha acquires
/// exactly Alpha. No selector key reached a session.
#[test]
fn enter_pins_the_highlighted_session_while_it_is_lost() {
    let package = Package::new();
    without_introduction(&package);
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    headless(&package, &watched("Beta"), "finish");
    ready_at(&mut terminal, "Beta", RUNNING, "29 120");
    headless(&package, &watched("Alpha"), "finish");
    let gamma = headless(&package, &watched("Gamma"), "finish");
    terminal.send("\x1ds").unwrap();
    for label in ["Alpha", "Gamma"] {
        listed_with(&terminal, RUNNING, label);
    }
    let listing = |rows: &[(&str, &str)]| selector_box((ROWS, COLS), "", rows, Some(FOOTER));
    let connected = [(RUNNING, "Gamma"), (RUNNING, "Alpha"), (RUNNING, "Beta")];
    boxed_as(&terminal, ROWS, &listing(&connected), &[7]);
    terminal.send("\x1b[A\x1b[A").unwrap();
    boxed_as(&terminal, ROWS, &listing(&connected), &[5]);
    kill(Pid::from_raw(daemon_of(&package, &gamma)), Signal::SIGKILL).unwrap();
    let lost = [
        (RUNNING, "Alpha"),
        (RUNNING, "Beta"),
        ("Disconnected", "Gamma"),
    ];
    boxed_as(&terminal, ROWS, &listing(&lost), &[7]);
    terminal.send("\r").unwrap();
    whole_frame_shows(&terminal, "Gamma read-only", |screen| {
        screen.lines().next().is_some_and(|top| {
            top.starts_with("Gamma ") && top.contains(&format!(" | {NOT_CONTROLLING} "))
        })
    });
    terminal.send("\x1ds").unwrap();
    enter_on(&mut terminal, "Alpha");
    ready_at(&mut terminal, "Alpha", RUNNING, "29 120");
    terminal.send("z").unwrap();
    received_by(&package, "Alpha", b"z");
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    received_by(&package, "Beta", b"");
    // Listing removes the lost session's stale metadata.
    let detached =
        serde_json::json!({"lifecycle": "detached", "attachments": 0, "controlled": false});
    listed_as(&package, "Alpha", &detached);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// SEL-03 (Monitor: cancellation and a prefix that consumes Escape) and
/// INP-05 (Monitor, click outside, owner decision on #113): after an empty
/// start Beta is the initial selection. Escape, or a click outside the box,
/// cancels the selector after the search was edited and the highlight
/// moved to Alpha, leaving Beta controlled and Alpha unacquired, and none
/// of those keys reached a session. With Ctrl+[ as the prefix the footer
/// never advertises Escape, and a lone Escape waits as the prefix: the key
/// after it completes no action and reaches neither the search nor a
/// session, so the selector stays open with its highlight, nothing is
/// acquired, and prefix+x still exits.
#[test]
fn cancelling_keeps_the_selection_and_an_escape_prefix_cannot_cancel() {
    let package = Package::new();
    without_introduction(&package);
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    headless(&package, &watched("Beta"), "finish");
    ready_at(&mut terminal, "Beta", RUNNING, "29 120");
    headless(&package, &watched("Alpha"), "finish");
    terminal.send("\x1ds").unwrap();
    listed_with(&terminal, RUNNING, "Alpha");
    terminal.send("al").unwrap();
    let searched = |footer| selector_box((ROWS, COLS), "al", &[(RUNNING, "Alpha")], footer);
    boxed_as(&terminal, ROWS, &searched(Some(FOOTER)), &[5]);
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    top_row_is(&mut terminal, &controlled("Beta", RUNNING));
    terminal.send("z").unwrap();
    received_by(&package, "Beta", b"z");
    terminal.send("\x1dsal").unwrap();
    boxed_as(&terminal, ROWS, &searched(Some(FOOTER)), &[5]);
    click(&mut terminal, 29, 60);
    selector_closed(&terminal);
    terminal.send("y").unwrap();
    received_by(&package, "Beta", b"zy");
    let detached =
        serde_json::json!({"lifecycle": "detached", "attachments": 0, "controlled": false});
    listed_as(&package, "Alpha", &detached);
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);

    fs::write(
        config_file(&package),
        r#"{"client": {"escape_prefix": "Ctrl+["}}"#,
    )
    .unwrap();
    let mut terminal = outer(&package, "c2 monitor");
    // Whichever session the initial selection takes, its top row must not
    // change while the selector is open.
    let hint = "Ctrl+[ then s: sessions";
    whole_frame_shows(&terminal, "a controlled selection", |screen| {
        screen
            .lines()
            .next()
            .is_some_and(|top| top.contains(CONTROLLING) && top.ends_with(hint))
    });
    let top = terminal.screen().lines().next().unwrap().to_owned();
    terminal.send("\x1bs").unwrap();
    listed_with(&terminal, RUNNING, "Alpha");
    terminal.send("al").unwrap();
    let open = searched(Some("Enter: select  Ctrl+[ then ?: help"));
    boxed_as(&terminal, ROWS, &open, &[5]);
    let monitor = monitor_process(&package).0;
    read_alone(&mut terminal, monitor, b"\x1b");
    read_alone(&mut terminal, monitor, b"a");
    terminal.resize(25, 81).unwrap();
    terminal.resize(ROWS, COLS).unwrap();
    boxed_as(&terminal, ROWS, &open, &[5]);
    top_row_is(&mut terminal, &top);
    terminal.send("\x1bx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    received_by(&package, "Beta", b"zy");
    received_by(&package, "Alpha", b"");
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// HLP-02 (Monitor, over the selector): after an empty start Beta is the
/// initial selection and Alpha appears above it. Prefix+? over the selector
/// opens the key introduction above it, listing the selector's keys (owner
/// decision on #113) and hiding the search's cursor; keys, a paste and
/// clicks while it is open reach neither the selector nor a session, while
/// the selected session's output keeps reaching its model. Enter closes the
/// introduction back to the selector with its search and highlight as they
/// were and the cursor in the search; Escape then closes the selector and
/// the session shows the output it wrote meanwhile, still controlled, the
/// other session never acquired. Prefix+x exits from the introduction over
/// the selector.
#[test]
fn help_over_the_selector_isolates_input_and_restores_search_and_highlight() {
    let package = Package::new();
    without_introduction(&package);
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    headless(&package, &watched("Beta"), "finish");
    ready_at(&mut terminal, "Beta", RUNNING, "29 120");
    headless(&package, &watched("Alpha"), "finish");
    terminal.send("\x1ds").unwrap();
    listed_with(&terminal, RUNNING, "Alpha");
    terminal.send("a\x1b[A").unwrap();
    let rows = [(RUNNING, "Alpha"), (RUNNING, "Beta")];
    let searched = selector_on((ROWS, COLS), "a", &rows, Some(FOOTER));
    boxed_as(&terminal, ROWS, &boxed(&searched), &[5]);
    cursor_at(&terminal, 3, 3);
    terminal.send("\x1d?").unwrap();
    let helped = boxed(&overlaid(
        &searched,
        &key_introduction("Ctrl+]", true, None),
    ));
    boxed_as(&terminal, ROWS, &helped, &[5]);
    assert!(!terminal.cursor().visible);
    say(&package, "Beta");
    terminal.send("b\x1b[B\x7f").unwrap();
    terminal.paste("c").unwrap();
    click(&mut terminal, 6, 10);
    click(&mut terminal, 0, 10);
    terminal.resize(25, 81).unwrap();
    terminal.resize(ROWS, COLS).unwrap();
    boxed_as(&terminal, ROWS, &helped, &[5]);
    terminal.send("\r").unwrap();
    boxed_as(&terminal, ROWS, &boxed(&searched), &[5]);
    cursor_at(&terminal, 3, 3);
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    first_row_starts(&terminal, "RAW SAID");
    top_row_is(&mut terminal, &controlled("Beta", RUNNING));
    terminal.send("z").unwrap();
    received_by(&package, "Beta", b"z");
    let detached =
        serde_json::json!({"lifecycle": "detached", "attachments": 0, "controlled": false});
    listed_as(&package, "Alpha", &detached);
    terminal.send("\x1ds\x1d?").unwrap();
    whole_frame_shows(&terminal, "the introduction over the selector", |screen| {
        screen.contains("Exit the monitor") && screen.contains('║')
    });
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// Operational limits (`ui.search_bytes`, Monitor) and SEL-02 (Monitor,
/// paste): with the limit at 6 bytes, the search takes `abcdé` (6 bytes)
/// and refuses `f` visibly, the footer saying why until the next key;
/// typed `é` one byte past the limit is refused whole, as is a paste one
/// byte past it, while a paste reaching it exactly is taken, its tab a
/// space. Nothing typed or pasted reaches the session.
#[test]
fn search_text_past_its_byte_limit_is_refused_visibly() {
    let package = Package::new();
    without_introduction(&package);
    limits_config(&package, serde_json::json!({"ui": {"search_bytes": 6}}));
    headless(&package, &watched("Beta"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    ready_at(&mut terminal, "Beta", RUNNING, "29 120");
    let refused = Some("Search is limited to 6 bytes");
    let searched =
        |search: &str, footer: Option<&str>| selector_box((ROWS, COLS), search, &[], footer);
    let steps: [(&str, &str, Option<&str>, usize); 5] = [
        ("\x1dsabcdé", "abcdé", Some(FOOTER), 7),
        ("f", "abcdé", refused, 7),
        ("\x1b[D", "abcdé", Some(FOOTER), 6),
        ("\x7f\x1b[Fé", "abcé", refused, 6),
        ("\x7f", "abc", Some(FOOTER), 5),
    ];
    for (typed, search, footer, column) in steps {
        terminal.send(typed).unwrap();
        boxed_as(&terminal, ROWS, &searched(search, footer), &[]);
        cursor_at(&terminal, 3, column);
    }
    terminal.paste("xyzw").unwrap();
    boxed_as(&terminal, ROWS, &searched("abc", refused), &[]);
    terminal.paste("x\ty").unwrap();
    boxed_as(&terminal, ROWS, &searched("abcx y", Some(FOOTER)), &[]);
    cursor_at(&terminal, 3, 8);
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    terminal.send("q").unwrap();
    received_by(&package, "Beta", b"q");
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// The top row's key hint beside a selected session, with the default
/// prefix.
const SESSIONS_HINT: &str = "Ctrl+] then s: sessions";
/// The status a selected session shows: its daemon's status clock never
/// advances (#107), and the control the monitor holds once input may go.
const STATUS: &str = "Idle";
/// The status of a session that reported a root OSC 7501 `working`
/// ([`working`]), or that accepted typed input on a clock that never
/// advances.
const RUNNING: &str = "Running";

/// `script`, after a root OSC 7501 `working` report: explicit detection then
/// holds the session's status at Running on any clock, through output,
/// title changes and typed input (user work ends in agent work again), so
/// neither its label nor its order depends on the daemon's clock, a test
/// hook release archives lack.
fn working(script: &str) -> String {
    format!("{}; {script}", report("working"))
}
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

/// The top row of `label`'s session, selected and controlled by the
/// monitor, showing `status`, with the default prefix's hint.
fn controlled(label: &str, status: &str) -> String {
    session_top_row(
        usize::from(COLS),
        label,
        &[status, CONTROLLING, SESSIONS_HINT],
    )
}

/// Waits until a whole frame's top row is exactly `expected`.
fn top_row_is(terminal: &mut Terminal, expected: &str) {
    whole_frame_shows(terminal, expected, |screen| {
        screen.lines().next() == Some(expected)
    });
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

/// CTL-05 (Monitor) and TRM-01 (Monitor, basic cases), with spec #73's
/// initial selection (#266): the one session, Idle, is selected and
/// acquired once collection ends, and its screen is painted directly below
/// the top row from the monitor's own model: plain, ANSI-coloured,
/// bold-on-blue, wide and combining text, each cell as written, and the
/// cursor where the application left it. Later output is painted live. A
/// session created later never moves the selection, and the monitor
/// controls the session.
#[test]
fn the_initial_selection_is_acquired_and_painted_live() {
    let package = Package::new();
    without_introduction(&package);
    let script = r#"printf '\033]2;Paint\007plain \033[31mred\033[0m \033[1;44mbold\033[0m \xe7\x95\x8ce\xcc\x81\r\n\033[3;5H'; until [ -e "$HOME/more" ] || [ "$SECONDS" -ge 120 ]; do sleep 0.05; done; printf 'MORE'"#;
    headless(&package, &working(script), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    let top = session_top_row(
        usize::from(COLS),
        "Paint",
        &[RUNNING, CONTROLLING, SESSIONS_HINT],
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

    headless(&package, &working(&titled_hold("Later")), "finish");
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║{RUNNING} Later"));
    // The selector covers the canvas, but control has not changed: the top
    // row still says Controlling (owner decision, 2026-11-14).
    top_row_is(&mut terminal, &top);
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
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
    headless(&package, &working(script), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    // Initial selection acquires Queries, the only session (#266).
    top_row_is(
        &mut terminal,
        &session_top_row(
            usize::from(COLS),
            "Queries",
            &[RUNNING, CONTROLLING, SESSIONS_HINT],
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
        headless(&package, &working(&corner), "finish");
        let mut terminal =
            support::outer(&package, Path::new("/bin/bash"), "c2 monitor", rows, cols);
        // Initial selection acquires Corner, the only session (#266).
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
            &[RUNNING, CONTROLLING, SESSIONS_HINT],
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
/// selected controller. Selecting a session, initially or explicitly, sizes
/// it to the host less the top row; after another is selected, host resizes
/// reach only that one, although the first stays controlled in the
/// background; selecting the first again sizes it to the host as it is then.
#[test]
fn host_resizes_reach_only_the_selected_controller() {
    let package = Package::new();
    without_introduction(&package);
    // After an empty start First, the first to appear, is the initial
    // selection (#266); Second comes after it. Both report `working`, so
    // their rows stay put: Second, newer, then First.
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    headless(&package, &working(&sized("First")), "finish");
    sized_to(&package, "First", &["24 80", "29 120"]);
    headless(&package, &working(&sized("Second")), "finish");
    terminal.send("\x1ds").unwrap();
    listed_with(&terminal, RUNNING, "Second");
    enter_on(&mut terminal, "Second");
    sized_to(&package, "Second", &["24 80", "29 120"]);
    terminal.resize(40, 100).unwrap();
    sized_to(&package, "Second", &["24 80", "29 120", "39 100"]);
    terminal.resize(35, 90).unwrap();
    sized_to(&package, "Second", &["24 80", "29 120", "39 100", "34 90"]);
    terminal.send("\x1ds").unwrap();
    enter_on(&mut terminal, "First");
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
    headless(&package, &working(&sized("Small")), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    // Initial selection acquires Small, the only session (#266).
    sized_to(&package, "Small", &["24 80", "29 120"]);
    let fields = [RUNNING, CONTROLLING, SESSIONS_HINT];
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
    headless(&package, &working(&titled_hold("Plain")), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    // Initial selection acquires Plain, the only session (#266).
    top_row_is(
        &mut terminal,
        &session_top_row(usize::from(COLS), "Plain", &[RUNNING, CONTROLLING]),
    );
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// Spec #73 Enter ("never exposing cached output") and CTL-05 (revoked):
/// when another client takes control of the selected session, the monitor
/// shows it is no longer controlling and paints only the placeholder
/// (#110), so its now-cached screen never shows. The session's screen then changes
/// while the other client controls it; Enter takes control back, and only
/// the new restoration is painted, never the cache.
#[test]
fn a_revoked_session_shows_no_cached_output_until_enter_takes_control_again() {
    let package = Package::new();
    without_introduction(&package);
    let script = "printf '\\033]2;Taken\\007VISIBLE\\r\\n'; \
         until [ -e \"$HOME/change\" ] || [ \"$SECONDS\" -ge 120 ]; do sleep 0.05; done; \
         printf '\\033[2J\\033[HCHANGED\\r\\n'";
    let id = headless(&package, &working(script), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    // Initial selection acquires Taken, the only session (#266).
    let fields = |control| [RUNNING, control, SESSIONS_HINT];
    let controlling = session_top_row(usize::from(COLS), "Taken", &fields(CONTROLLING));
    top_row_is(&mut terminal, &controlling);
    assert_eq!(terminal.row(1).trim_end(), "VISIBLE");

    let mut attach = package.command("c2");
    attach.args(["attach", &id]);
    let mut attach = Terminal::spawn(attach, ROWS, COLS).unwrap();
    attach.wait_for_text("VISIBLE", TIMEOUT).unwrap();
    // The whole frame, since a host read can split it after its top row:
    // only the placeholder (#110), never the cached screen.
    shows(&mut terminal, &placeholder_screen("Taken", RUNNING));
    assert!(!terminal.cursor().visible);
    fs::write(package.isolation.home().join("change"), b"").unwrap();
    attach.wait_for_text("CHANGED", TIMEOUT).unwrap();

    terminal.send("\x1ds").unwrap();
    enter_on(&mut terminal, "Taken");
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

/// The #94 rule for #108, with COM-06 (Monitor labels, #107): a finished
/// session's view freezes at its Command final. Its last screen stays
/// painted, and the top row says it ended with its exact outcome, which
/// outlives its connection; Enter on its row, known final, neither selects
/// nor acquires anything.
#[test]
fn a_finished_session_keeps_its_last_screen_and_cannot_be_acquired() {
    let package = Package::new();
    without_introduction(&package);
    let script = "printf '\\033]2;Done\\007FINAL OUTPUT'; until [ -e \"$HOME/end\" ] || [ \"$SECONDS\" -ge 120 ]; do sleep 0.05; done; exit 3";
    headless(&package, &working(script), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    // Initial selection acquires Done, the only session (#266).
    top_row_is(
        &mut terminal,
        &session_top_row(
            usize::from(COLS),
            "Done",
            &[RUNNING, CONTROLLING, SESSIONS_HINT],
        ),
    );
    fs::write(package.isolation.home().join("end"), b"").unwrap();
    let ended = session_top_row(
        usize::from(COLS),
        "Done",
        &["Failed (exit 3)", "Ended", SESSIONS_HINT],
    );
    top_row_is(&mut terminal, &ended);
    assert_eq!(terminal.row(1).trim_end(), "FINAL OUTPUT");
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, "║Failed (exit 3) Done");
    terminal.send("\r").unwrap();
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    assert_eq!(terminal.row(0), ended);
    assert_eq!(terminal.row(1).trim_end(), "FINAL OUTPUT");
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// Bash that titles its session `title`, puts its terminal in raw mode
/// without echo, and appends every byte it receives to `$HOME/got`, saying
/// `RAW` once it does; when `$HOME/modes` appears it turns on application
/// cursor keys (DECCKM) and the application keypad (DECKPAM). It holds until
/// `$HOME/finish` exists.
fn recorder(title: &str) -> String {
    recording(title, "-echo")
}

/// A [`recorder`] whose terminal echoes every byte it receives after `RAW`.
/// The daemon answers an Input before it reads the echo, on the same
/// connection, so once the monitor shows a byte's echo, every operation it
/// submitted before that byte has been answered.
fn echoing_recorder(title: &str) -> String {
    recording(title, "echo")
}

/// The [`recorder`] script, with stty's `echo` setting `echo`.
fn recording(title: &str, echo: &str) -> String {
    working(&format!(
        "printf '\\033]2;{title}\\007'; stty raw {echo}; cat </dev/tty >\"$HOME/got\" & printf RAW; \
         until [ -e \"$HOME/finish\" ] || [ \"$SECONDS\" -ge 120 ]; do \
         if [ -e \"$HOME/modes\" ] && [ -z \"$m\" ]; then printf '\\033[?1h\\033='; m=1; fi; \
         sleep 0.05; done; kill $!"
    ))
}

/// Waits until the [`recorder`] session has received exactly `expected`.
fn received(package: &Package, expected: &[u8]) {
    let path = package.isolation.home().join("got");
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let got = fs::read(&path).unwrap_or_default();
        if got == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the session received {:?}, not {:?}",
            got.escape_ascii().to_string(),
            expected.escape_ascii().to_string()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// Waits until initial selection has selected and acquired the
/// [`recorder`] session `title`, the first to appear after an empty start
/// or the only session (#266), and the monitor
/// controls it with its terminal raw.
fn record(terminal: &mut Terminal, title: &str, cols: u16) {
    top_row_is(
        terminal,
        &session_top_row(
            usize::from(cols),
            title,
            &[RUNNING, CONTROLLING, SESSIONS_HINT],
        ),
    );
    whole_frame_shows(terminal, "RAW", |screen| {
        screen
            .lines()
            .nth(1)
            .is_some_and(|row| row.starts_with("RAW"))
    });
}

/// Typed keys, host replies and reports, as the host sends them: NUL, a
/// non-UTF-8 byte, `A`, Ctrl+C and invalid UTF-8; a doubled prefix; the
/// prefix with d, h, r, uppercase J and the take-control key; prefix+j;
/// Up as CSI and Down as SS3; keypad 0 in application form; Alt+x; a device
/// attributes reply, an OSC colour reply, a focus report, an SGR mouse
/// press and release; then `end`.
const TYPED: &[u8] = b"\x00\xff\x41\x03\xc3\x28\x1d\x1d\x1dd\x1dh\x1dr\x1dJ\x1d\x14\x1dj\
\x1b[A\x1bOB\x1bOp\x1bx\x1b[?62;22c\x1b]11;rgb:0/0/0\x07\x1b[I\x1b[<0;5;5M\x1b[<0;5;5mend";

/// What an application in normal cursor and keypad modes receives for
/// [`TYPED`], written by hand: every key exactly, one literal prefix for
/// the doubled one, both bytes of each unknown suffix, nothing for prefix+j,
/// the replies and the reports, the cursor keys as CSI and keypad 0 as `0`.
const RECEIVED: &[u8] =
    b"\x00\xff\x41\x03\xc3\x28\x1d\x1dd\x1dh\x1dr\x1dJ\x1d\x14\x1b[A\x1b[B0\x1bxend";

/// [`TYPED`] cut inside a doubled prefix, after a lone prefix, right after a
/// cursor key's Escape (released to the session at once), inside a keypad
/// key, a reply, its OSC string and a mouse report.
const FRAGMENTS: [&[u8]; 10] = [
    b"\x00\xff\x41\x03\xc3",
    b"\x28\x1d",
    b"\x1d\x1d",
    b"d\x1dh\x1dr\x1dJ\x1d\x14\x1d",
    b"j\x1b",
    b"[A\x1bOB\x1bO",
    b"p\x1bx\x1b[?62;",
    b"22c\x1b]11;rgb",
    b":0/0/0\x07\x1b[I\x1b[<0;5",
    b";5M\x1b[<0;5;5mend",
];

/// INP-01, INP-02 and TRM-02 (Monitor), COM-04 (Monitor, typed cases) and
/// EXT-03 (Monitor, Ctrl+C): bytes typed at the host reach the selected,
/// controlling session's application exactly: binary and invalid UTF-8
/// bytes and Ctrl+C as they are, a doubled prefix as one literal prefix,
/// the prefix with d, h, r, J or the take-control key as both bytes, and
/// prefix+j, host replies, focus and mouse reports never. The same bytes
/// cut into ten reads, each read alone, arrive identically. Once the
/// application turns on application cursor keys and keypad, the host keypad
/// follows it, and cursor keys typed in either form arrive as SS3, keypad
/// keys as sent and modified cursor keys unchanged. A lone Escape reaches
/// the session at once, before the rest of its key arrives, and that rest
/// still follows the application's cursor mode. Keys read with prefix+x
/// before it reach the session before the monitor shuts down (owner
/// decision on #109), and none after it.
#[test]
fn typed_bytes_reach_the_selected_session_exactly_however_reads_split() {
    let package = Package::new();
    without_introduction(&package);
    headless(&package, &recorder("Keys"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    record(&mut terminal, "Keys", COLS);
    terminal.send(TYPED).unwrap();
    received(&package, RECEIVED);
    assert_eq!(FRAGMENTS.concat(), TYPED);
    let monitor = monitor_process(&package).0;
    for fragment in FRAGMENTS {
        read_alone(&mut terminal, monitor, fragment);
    }
    let twice = [RECEIVED, RECEIVED].concat();
    received(&package, &twice);
    assert!(!terminal.modes().application_keypad);
    fs::write(package.isolation.home().join("modes"), b"").unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while !terminal.modes().application_keypad {
        assert!(Instant::now() < deadline, "the host keypad never followed");
        thread::sleep(Duration::from_millis(10));
    }
    terminal
        .send(b"\x1b[A\x1bOB\x1b[D\x1bOp\x1bOM\x1b[1;5A")
        .unwrap();
    let moded = [&twice[..], b"\x1bOA\x1bOB\x1bOD\x1bOp\x1bOM\x1b[1;5A"].concat();
    received(&package, &moded);
    read_alone(&mut terminal, monitor, b"\x1b");
    received(&package, &[&moded[..], b"\x1b"].concat());
    read_alone(&mut terminal, monitor, b"[C");
    let moded = [&moded[..], b"\x1bOC"].concat();
    received(&package, &moded);
    read_alone(&mut terminal, monitor, b"z\x1dxlate");
    assert_released(&package, &mut terminal, 0);
    received(&package, &[&moded[..], b"z"].concat());
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// INP-01 (Monitor: live and too-small screens) and spec #73 Routing: from a
/// live session, prefix+? opens the key introduction and prefix+s the
/// selector, and keys typed while either is open reach neither the session
/// nor anything later. On a host too small for any session, typed input is
/// discarded, never kept for later, while prefix+s still opens the selector,
/// shown once the host has room again, and prefix+x still exits.
#[test]
fn global_actions_work_from_live_and_too_small_screens_and_input_is_never_buffered() {
    let package = Package::new();
    without_introduction(&package);
    headless(&package, &recorder("Keys"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    record(&mut terminal, "Keys", COLS);
    terminal.send("\x1d?").unwrap();
    shows_row(&mut terminal, "Exit the monitor");
    terminal.send("a\r").unwrap();
    whole_frame_shows(&terminal, "no key introduction", |screen| {
        !screen.contains("Exit the monitor")
    });
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║{RUNNING} Keys"));
    terminal.send("b").unwrap();
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    terminal.send("c").unwrap();
    received(&package, b"c");
    terminal.resize(3, COLS).unwrap();
    top_row_is(&mut terminal, &format!("{:<120}", "Terminal too small"));
    let monitor = monitor_process(&package).0;
    read_alone(&mut terminal, monitor, b"lost\x1ds");
    terminal.resize(ROWS, COLS).unwrap();
    shows_row(&mut terminal, &format!("║{RUNNING} Keys"));
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    terminal.send("d").unwrap();
    received(&package, b"cd");
    terminal.resize(3, COLS).unwrap();
    top_row_is(&mut terminal, &format!("{:<120}", "Terminal too small"));
    terminal.send("\x1dx").unwrap();
    terminal.wait_for_bytes(b"OUTER-DONE=0", TIMEOUT).unwrap();
    // The three-row host scrolled the status away; growing it brings it back.
    terminal.resize(ROWS, COLS).unwrap();
    assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// INP-02 (Monitor) and HLP-02 (Monitor, Escape), by owner decisions on
/// #106, #105 and #109: Escape handling does not depend on read
/// fragmentation and uses no timer. Down split right after its Escape and
/// Up split after its introducer, each read alone, move the selector's
/// highlight and never close it. A lone Escape leaves the selector open
/// until the next key arrives, then closes it, and that key acts as itself:
/// prefix+? opens the key introduction. The key introduction behaves alike
/// with split Up and Down, and Escape and prefix+s in one read close it
/// (the prefix, Ctrl+], does not consume Escape) and then open the
/// selector. Escape and a key in one read close the selector or the key
/// introduction and the key reaches the session (owner decision on #109:
/// the uncovered session is painted before the rest of the read is
/// routed), as does a key read after a lone Escape. No other dialog key
/// reaches the session, and prefix+x exits from the selector (INP-01).
#[test]
fn split_escape_sequences_never_close_dialogs_and_escape_then_a_key_is_both() {
    let package = Package::new();
    without_introduction(&package);
    // After an empty start Keys, the first to appear, is the initial
    // selection; Other comes after it.
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    headless(&package, &recorder("Keys"), "finish");
    record(&mut terminal, "Keys", COLS);
    headless(&package, &working(&titled_hold("Other")), "finish");
    let monitor = monitor_process(&package).0;
    // Once Other's row has its status, reopen the selector on the settled
    // list: Other, the newer, then Keys, the selected row it opens on (#113).
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║{RUNNING} Other"));
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║{RUNNING} Other"));
    assert_eq!(highlighted_rows(&terminal), [6]);
    for (reads, row) in [([&b"\x1b"[..], b"[A"], 5), ([b"\x1b[", b"B"], 6)] {
        for read in reads {
            read_alone(&mut terminal, monitor, read);
        }
        highlight_at(&terminal, row);
    }
    read_alone(&mut terminal, monitor, b"\x1b");
    whole_frame_shows(&terminal, "the selector", |screen| screen.contains('║'));
    read_alone(&mut terminal, monitor, b"\x1d?");
    whole_frame_shows(&terminal, "the key introduction alone", |screen| {
        !screen.contains('║') && screen.contains("Exit the monitor")
    });
    for read in [&b"\x1b"[..], b"[A", b"\x1b[", b"B", b"\x1b\x1ds"] {
        read_alone(&mut terminal, monitor, read);
    }
    whole_frame_shows(&terminal, "the selector alone", |screen| {
        screen.contains('║') && !screen.contains("Exit the monitor")
    });
    read_alone(&mut terminal, monitor, b"\x1bq");
    received(&package, b"q");
    selector_closed(&terminal);
    terminal.send("\x1d?").unwrap();
    shows_row(&mut terminal, "Exit the monitor");
    read_alone(&mut terminal, monitor, b"\x1bz");
    received(&package, b"qz");
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║{RUNNING} Other"));
    read_alone(&mut terminal, monitor, b"\x1b");
    read_alone(&mut terminal, monitor, b"y");
    received(&package, b"qzy");
    selector_closed(&terminal);
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║{RUNNING} Other"));
    terminal.send("\x1dx").unwrap();
    // Its many frames outgrow the oracle's recent bytes, so only the
    // title itself is checked.
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// In the open selector, once a whole frame lists a row containing `text`,
/// moves the highlight from where it is to that row and presses Enter. The
/// rows must not reorder meanwhile.
fn enter_on(terminal: &mut Terminal, text: &str) {
    highlight_on(terminal, text);
    terminal.send("\r").unwrap();
}

/// In the open selector, once a whole frame lists a row containing `text`,
/// moves the highlight from where it is to that row, and waits until it is
/// there. The rows must not reorder meanwhile.
fn highlight_on(terminal: &mut Terminal, text: &str) {
    let listed = |line: &str| line.starts_with(" ║") && line.contains(text);
    whole_frame_shows(terminal, text, |screen| screen.lines().any(listed));
    let target = terminal.screen().lines().position(listed).unwrap();
    let from = highlighted_rows(terminal)[0];
    let keys = if target < from {
        "\x1b[A".repeat(from - target)
    } else {
        "\x1b[B".repeat(target - from)
    };
    terminal.send(keys).unwrap();
    highlight_at(terminal, target);
}

/// Waits until the selector highlights host row `row` alone.
fn highlight_at(terminal: &Terminal, row: usize) {
    whole_frame_shows(terminal, "the highlight", |_| {
        highlighted_rows(terminal) == [row]
    });
}

/// COM-03 (Monitor, queue-full case) and COM-04 (Monitor, chunking): with
/// two-byte chunks and room for three outstanding operations, once every
/// earlier operation is answered (a typed z is echoed) and while the
/// session's daemon is stopped, nine typed bytes are submitted as ab, cd
/// and ef, and the fourth chunk does not fit: the top row says that its
/// three bytes and those after them (ghi) were not sent and that earlier
/// input was accepted. Once the daemon resumes, the session has received exactly
/// zabcdef, nothing is replayed, and later typing reaches it.
#[test]
fn a_full_input_queue_is_shown_and_only_accepted_input_arrives() {
    const WIDE: u16 = 250;
    let package = Package::new();
    without_introduction(&package);
    // ab, cd and ef stay unanswered while the daemon is stopped, and an
    // operation past `deadlines.operation_ms` fails the connection: the
    // deadline must outlast the test's steps until it resumes the daemon,
    // even on a loaded host. The operations a newly ready controller
    // submits by itself, such as Focus, leave room for the z typed to wait
    // for their answers.
    limits_config(
        &package,
        serde_json::json!({
            "transfer": {"chunk_bytes": 2},
            "queues": {"operations": 3},
            "deadlines": {"operation_ms": 120_000},
        }),
    );
    headless(&package, &echoing_recorder("Keys"), "finish");
    let daemon = package.metadata().daemon_pid;
    let mut terminal = outer_sized(&package, "c2 monitor", WIDE);
    record(&mut terminal, "Keys", WIDE);
    terminal.send("z").unwrap();
    whole_frame_shows(&terminal, "the echoed z", |screen| {
        screen
            .lines()
            .nth(1)
            .is_some_and(|row| row.starts_with("RAWz"))
    });
    let monitor = monitor_process(&package).0;
    stop(daemon as i32);
    read_alone(&mut terminal, monitor, b"abcdefghi");
    shows_row(
        &mut terminal,
        "Input not sent (3 bytes; earlier input was accepted): queue_full: the request does not fit",
    );
    kill(Pid::from_raw(daemon as i32), Signal::SIGCONT).unwrap();
    received(&package, b"zabcdef");
    terminal.send("j").unwrap();
    received(&package, b"zabcdefj");
    terminal.send("\x1dx").unwrap();
    // Its wide frames can outgrow the oracle's recent bytes, so only the
    // title itself is checked.
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// Spec #73 exit (owner decision on #109): input accepted before prefix+x
/// holds shutdown only until it is answered or `deadlines.operation_ms`
/// passes. With the session's daemon stopped, a key read with prefix+x is
/// never answered, and the monitor still exits with status 0 within the
/// one-second operation deadline plus the two-second shutdown deadline
/// (and a few seconds' margin for the host's release).
#[test]
fn a_stuck_session_cannot_hold_exit_beyond_the_operation_deadline() {
    let package = Package::new();
    without_introduction(&package);
    limits_config(
        &package,
        serde_json::json!({"deadlines": {"operation_ms": 1000, "shutdown_ms": 2000}}),
    );
    headless(&package, &recorder("Keys"), "finish");
    let daemon = package.metadata().daemon_pid as i32;
    let mut terminal = outer(&package, "c2 monitor");
    record(&mut terminal, "Keys", COLS);
    let monitor = monitor_process(&package).0;
    stop(daemon);
    let started = Instant::now();
    read_alone(&mut terminal, monitor, b"k\x1dx");
    terminal.wait_for_bytes(DONE, TIMEOUT).unwrap();
    let took = started.elapsed();
    kill(Pid::from_raw(daemon), Signal::SIGCONT).unwrap();
    assert!(took < Duration::from_secs(6), "{took:?}");
    assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// The non-controller placeholder (#110), as the client executable words
/// it, and its takeover hint with the default take-control key.
const VIEWED_ELSEWHERE: &str = "This session is being viewed elsewhere.";
const TAKEOVER_HINT: &str = "Press Ctrl+T to take control";
const NO_CONTENT: &str = "No terminal content is available.";
const NOT_CONTROLLING: &str = "Not controlling";

/// Bash for a session the #110 scenarios watch. It reports `working`
/// ([`working`]), titles its session
/// `title`, puts its terminal in raw mode without echo, appends every byte
/// it receives to `$HOME/<title>.got` and says `RAW`. Until `$HOME/finish`
/// exists it appends each new `stty size` to `$HOME/<title>.sizes` (with
/// [`sized`]'s probe) and writes it on its second row as `SIZE <rows>
/// <cols>`, leaving the cursor where it was; it retitles itself whenever
/// `$HOME/<title>.title` holds a new title, reports the root OSC 7501 state
/// `$HOME/<title>.state` holds whenever it changes, says ` SAID` once
/// `$HOME/<title>.say` exists, and writes, then removes, whatever
/// `$HOME/<title>.mode` holds ([`set_modes`]).
fn watched(title: &str) -> String {
    working(&format!(
        "printf '\\033]2;{title}\\007'; stty raw -echo; cat </dev/tty >\"$HOME/{title}.got\" & printf RAW; \
         p=; t=; o=; w=; \
         until [ -e \"$HOME/finish\" ] || [ \"$SECONDS\" -ge 120 ]; do \
         q=; [ -e \"$HOME/{title}.probe\" ] && q=1; \
         s=$(stty size); if [ \"$s\" != \"$p\" ]; then echo \"$s\" >> \"$HOME/{title}.sizes\"; p=$s; \
         printf '\\x1b7\\033[2;1HSIZE %s\\033[K\\x1b8' \"$s\"; fi; \
         n=$(cat \"$HOME/{title}.title\" 2>/dev/null); \
         if [ \"$n\" != \"$t\" ]; then printf '\\033]2;%s\\007' \"$n\"; t=$n; fi; \
         r=$(cat \"$HOME/{title}.state\" 2>/dev/null); \
         if [ -n \"$r\" ] && [ \"$r\" != \"$w\" ]; then printf '\\033]7501;state=%s\\033\\\\' \"$r\"; w=$r; fi; \
         if [ -e \"$HOME/{title}.say\" ] && [ -z \"$o\" ]; then printf ' SAID'; o=1; fi; \
         if [ -e \"$HOME/{title}.mode\" ]; then cat \"$HOME/{title}.mode\"; rm \"$HOME/{title}.mode\"; fi; \
         [ -n \"$q\" ] && mv \"$HOME/{title}.probe\" \"$HOME/{title}.probed\"; \
         sleep 0.05; done; kill $!"
    ))
}

/// Has the [`watched`] session `title` retitle itself `new`.
fn retitle(package: &Package, title: &str, new: &str) {
    let home = package.isolation.home();
    let staged = home.join(format!("{title}.title.new"));
    fs::write(&staged, new).unwrap();
    fs::rename(staged, home.join(format!("{title}.title"))).unwrap();
}

/// Has the [`watched`] session `title` say ` SAID`.
/// Makes the [`watched`] session `title` report the root OSC 7501 `state`.
fn report_state(package: &Package, title: &str, state: &str) {
    let home = package.isolation.home();
    let staged = home.join(format!("{title}.state.new"));
    fs::write(&staged, state).unwrap();
    fs::rename(staged, home.join(format!("{title}.state"))).unwrap();
}

/// Waits until the open selector lists `label` with `status`, whatever the
/// status column's width.
fn listed_with(terminal: &Terminal, status: &str, label: &str) {
    whole_frame_shows(terminal, &format!("{status} {label}"), |screen| {
        screen.lines().any(|line| {
            line.strip_prefix(" ║")
                .and_then(|row| row.strip_prefix(status))
                .is_some_and(|rest| rest.trim_start().starts_with(label))
        })
    });
}

fn say(package: &Package, title: &str) {
    fs::write(package.isolation.home().join(format!("{title}.say")), b"").unwrap();
}

/// Waits until the [`watched`] session `title` has received exactly
/// `expected`.
fn received_by(package: &Package, title: &str, expected: &[u8]) {
    let path = package.isolation.home().join(format!("{title}.got"));
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let got = fs::read(&path).unwrap_or_default();
        if got == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{title} received {:?}, not {:?}",
            got.escape_ascii().to_string(),
            expected.escape_ascii().to_string()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// Waits until a whole frame shows the [`watched`] session `label`
/// controlled with input open at `size`: its top row says Controlling and
/// its canvas shows the `SIZE` line the session wrote once resized to
/// `size`. That line follows the restoration the resize brought, which the
/// canvas shows only once applied, and Controlling means its Ready was
/// accepted, so the frame shows the restoration and keys typed now reach
/// the session.
fn ready_at(terminal: &mut Terminal, label: &str, status: &str, size: &str) {
    ready_on(terminal, COLS, (label, status), size);
}

/// [`ready_at`] on a host `cols` wide.
fn ready_on(terminal: &mut Terminal, cols: u16, (label, status): (&str, &str), size: &str) {
    let top = session_top_row(
        usize::from(cols),
        label,
        &[status, CONTROLLING, SESSIONS_HINT],
    );
    let line = format!("SIZE {size}");
    whole_frame_shows(terminal, &line, |screen| {
        let mut rows = screen.lines();
        rows.next() == Some(top.as_str()) && rows.nth(1).is_some_and(|row| row.starts_with(&line))
    });
}

/// Waits until a whole frame's second row starts with `text`: the
/// selected session's first row.
fn first_row_starts(terminal: &Terminal, text: &str) {
    whole_frame_shows(terminal, text, |screen| {
        screen
            .lines()
            .nth(1)
            .is_some_and(|row| row.starts_with(text))
    });
}

/// The `c2 list` state of the session whose arguments contain `text`, once
/// it is `expected`.
fn listed_as(package: &Package, text: &str, expected: &Value) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let state = listed(package, text)["state"].clone();
        if state == *expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{text} is {state}, not {expected}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// The daemon process of session `id`, from its metadata.
fn daemon_of(package: &Package, id: &str) -> i32 {
    let path = package
        .isolation
        .climon2_home()
        .join("sessions")
        .join(format!("{id}.pty"));
    let metadata: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    i32::try_from(metadata["daemon_pid"].as_i64().unwrap()).unwrap()
}

/// `c2 attach` to the [`watched`] session `id` on its own host, once it is
/// ready: keys typed then reach the session. Its grant resizes the session
/// to the host, and the session writes that size only after the
/// restoration's snapshot, so the line arrives as live output, which the
/// client shows only after its restoration ended.
fn attach(package: &Package, id: &str) -> Terminal {
    let mut attach = package.command("c2");
    attach.args(["attach", id]);
    let mut attach = Terminal::spawn(attach, ROWS, COLS).unwrap();
    attached_ready(&mut attach);
    attach
}

/// Waits until the attached client `attach`, granted control on its own
/// host, shows the [`watched`] session's `SIZE` line for that host: live
/// output, as the session was another size before the grant.
fn attached_ready(attach: &mut Terminal) {
    let line = format!("SIZE {ROWS} {COLS}");
    attach
        .wait_for_screen(TIMEOUT, |screen| {
            let mut rows = screen.lines();
            rows.next().is_some_and(|row| row.starts_with("RAW"))
                && rows.next().is_some_and(|row| row.starts_with(&line))
        })
        .unwrap();
}

/// Waits until the attached client `attach`, after its placeholder, shows
/// the [`watched`] session again: a grant's restoration, painted.
fn restored(attach: &mut Terminal) {
    attach
        .wait_for_screen(TIMEOUT, |screen| {
            screen.starts_with("RAW") && !screen.contains(VIEWED_ELSEWHERE)
        })
        .unwrap();
}

/// Detaches the attached client `attach`, which exits 0.
fn detach(mut attach: Terminal) {
    attach.send("\x1dd").unwrap();
    assert_eq!(attach.wait_exit(TIMEOUT).unwrap(), 0);
}

/// The whole host showing `label`'s session revoked: its top row not
/// controlling, and only the placeholder, never its cached output.
fn placeholder_screen(label: &str, status: &str) -> String {
    let top = session_top_row(
        usize::from(COLS),
        label,
        &[status, NOT_CONTROLLING, SESSIONS_HINT],
    );
    centred_below(&top, ROWS, COLS, &[VIEWED_ELSEWHERE, TAKEOVER_HINT])
}

/// CTL-02 (Monitor), CTL-03 (Monitor), CTL-05 (Monitor: background and
/// revoked), DSC-05 (Monitor: revocation) and INP-01 (Monitor: takeover):
/// Alpha, the first to appear after an empty start, is the initial
/// selection (#266); explicitly selecting
/// Beta then leaves Alpha controlled by the monitor in the background, and
/// typed keys reach only the selected session. Alpha's title updates in the
/// background, under the selector, and switching back shows Alpha at once. When another client attaches,
/// the last acquisition wins: the selection stays on Alpha, which shows
/// only the placeholder with its takeover hint, never its cached output;
/// typed keys and a host resize reach no session, the take-control key does
/// nothing in the selector or the key introduction, Alpha's title still
/// updates, and the monitor never takes control back by itself. The
/// take-control key then acquires Alpha and is consumed; once the monitor
/// controls Alpha it is ordinary input. Beta, background throughout, then
/// receives only what is typed once it is selected again.
#[test]
fn a_revoked_selection_shows_only_the_placeholder_until_the_take_control_key() {
    let package = Package::new();
    without_introduction(&package);
    // After an empty start Alpha, the first to appear, is the initial
    // selection; Beta comes after it.
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    let alpha = headless(&package, &watched("Alpha"), "finish");
    top_row_is(&mut terminal, &controlled("Alpha", RUNNING));
    headless(&package, &watched("Beta"), "finish");
    first_row_starts(&terminal, "RAW");
    terminal.send("a").unwrap();
    received_by(&package, "Alpha", b"a");
    // Rows, both Running: Beta, newer, then Alpha.
    terminal.send("\x1ds").unwrap();
    listed_with(&terminal, RUNNING, "Beta");
    enter_on(&mut terminal, "Beta");
    top_row_is(&mut terminal, &controlled("Beta", RUNNING));
    first_row_starts(&terminal, "RAW");
    terminal.send("b").unwrap();
    received_by(&package, "Beta", b"b");
    let attached =
        serde_json::json!({"lifecycle": "attached", "attachments": 1, "controlled": true});
    listed_as(&package, "Alpha", &attached);

    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║{RUNNING} Alpha"));
    retitle(&package, "Alpha", "Alpha background");
    shows_row(&mut terminal, &format!("║{RUNNING} Alpha background"));
    enter_on(&mut terminal, "Alpha background");
    top_row_is(&mut terminal, &controlled("Alpha background", RUNNING));
    first_row_starts(&terminal, "RAW");
    terminal.send("c").unwrap();
    received_by(&package, "Alpha", b"ac");

    let mut other = attach(&package, &alpha);
    shows(
        &mut terminal,
        &placeholder_screen("Alpha background", RUNNING),
    );
    assert!(!terminal.cursor().visible);
    terminal.send("x").unwrap();
    terminal.resize(40, 100).unwrap();
    let wide = session_top_row(
        100,
        "Alpha background",
        &[RUNNING, NOT_CONTROLLING, SESSIONS_HINT],
    );
    shows(
        &mut terminal,
        &centred_below(&wide, 40, 100, &[VIEWED_ELSEWHERE, TAKEOVER_HINT]),
    );
    terminal.resize(ROWS, COLS).unwrap();
    shows(
        &mut terminal,
        &placeholder_screen("Alpha background", RUNNING),
    );
    // Alpha was sized for the monitor, then for the attached client; Beta
    // only for the monitor.
    assert_eq!(
        settled_sizes(&package, "Alpha"),
        ["24 80", "29 120", "30 120"]
    );
    assert_eq!(settled_sizes(&package, "Beta"), ["24 80", "29 120"]);

    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║{RUNNING} Alpha background"));
    terminal.send("\x14").unwrap();
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    terminal.send("\x1d?").unwrap();
    terminal.wait_for_text("Continue (Enter)", TIMEOUT).unwrap();
    terminal.send("\x14\r").unwrap();
    retitle(&package, "Alpha", "Alpha revoked");
    shows(&mut terminal, &placeholder_screen("Alpha revoked", RUNNING));
    assert!(
        !other.screen().contains(VIEWED_ELSEWHERE),
        "{}",
        other.screen()
    );

    terminal.send("\x14").unwrap();
    other.wait_for_text(VIEWED_ELSEWHERE, TIMEOUT).unwrap();
    // The attached client sized Alpha 30 by 120; the grant's resize brings
    // it back, and its restoration.
    ready_at(&mut terminal, "Alpha revoked", RUNNING, "29 120");
    terminal.send("\x14d").unwrap();
    received_by(&package, "Alpha", b"ac\x14d");
    // Beta, in the background all along, received nothing more: once
    // selected, what it receives next follows anything sent before.
    terminal.send("\x1ds").unwrap();
    enter_on(&mut terminal, "Beta");
    top_row_is(&mut terminal, &controlled("Beta", RUNNING));
    first_row_starts(&terminal, "RAW");
    terminal.send("e").unwrap();
    received_by(&package, "Beta", b"be");
    terminal.send("\x1dx").unwrap();
    // Its many frames can outgrow the oracle's recent bytes, so only the
    // title itself is checked.
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    // The monitor's exit handed Alpha back to the attached client, whose
    // grant resized it back to that client's host.
    attached_ready(&mut other);
    detach(other);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// CTL-03 (Monitor, status-value case, moved from #107 to #266 by the
/// owner): a session's effective status keeps updating in the monitor
/// whatever its role, on any clock: each session reports `working` at
/// start, then what the test makes it report. Alpha, the initial selection
/// after an empty start, keeps control in the background once Beta is
/// selected, and its row turns Needs attention when it reports `idle`;
/// Beta's row, its canvas covered by the open selector, turns Needs
/// attention when Beta reports `idle`, as does its top row once the
/// selector closes. After another client's attach takes Alpha over and
/// acknowledges its attention, its revoked row turns Acknowledged, then
/// Needs attention again when Alpha reports `done`.
#[test]
fn status_keeps_updating_for_background_covered_and_revoked_rows() {
    let package = Package::new();
    without_introduction(&package);
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    let alpha = headless(&package, &watched("Alpha"), "finish");
    ready_at(&mut terminal, "Alpha", RUNNING, "29 120");
    headless(&package, &watched("Beta"), "finish");
    terminal.send("\x1ds").unwrap();
    listed_with(&terminal, RUNNING, "Beta");
    enter_on(&mut terminal, "Beta");
    ready_at(&mut terminal, "Beta", RUNNING, "29 120");

    report_state(&package, "Alpha", "idle");
    terminal.send("\x1ds").unwrap();
    listed_with(&terminal, "Needs attention", "Alpha");
    report_state(&package, "Beta", "idle");
    listed_with(&terminal, "Needs attention", "Beta");
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    ready_at(&mut terminal, "Beta", "Needs attention", "29 120");

    // Ready on its own host once Alpha, resized for it, says so live; its
    // attach acknowledged Alpha's pending attention (#102).
    let other = attach(&package, &alpha);
    terminal.send("\x1ds").unwrap();
    listed_with(&terminal, "Acknowledged", "Alpha");
    report_state(&package, "Alpha", "done");
    listed_with(&terminal, "Needs attention", "Alpha");
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    detach(other);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// DSC-01 (Monitor, bounded collection; uncertainty explicit), the owner's
/// decisions of 2026-11-16 and spec #73 initial selection, with fake
/// daemons and one start slot: Held never completes its client's Open, so
/// Slow waits for the slot. While both start the body shows the startup
/// progress, `Connecting to 2 sessions (0 ready)`; at `deadlines.start_ms`
/// collection ends without them, selecting nothing, and the body says
/// discovery is incomplete while a late session still starts, also in the
/// top row's error slot once a row is selected (owner decision on #266,
/// 2026-11-16). Once Slow's
/// Initial metadata reports its status, it is the first eligible row and
/// becomes the one-time initial selection, acquired. Had the user selected
/// a row meanwhile, even read-only, Slow would not be selected: here the
/// user selects Slow itself while it still starts, and it stays not
/// controlled once eligible. Slow's Open stays unanswered until the test
/// releases it, so Slow is the late session whatever the host's speed;
/// Held's client fails at its own start deadline soon after collection
/// ends, and its row then shows that failure instead.
#[test]
fn a_late_session_ends_collection_at_its_deadline_and_is_selected_once_eligible() {
    // Wide enough for the late note beside the held row's label.
    const WIDE: u16 = 200;
    let held = fake_id(0xd1);
    for explicit in [false, true] {
        let package = Package::new();
        without_introduction(&package);
        // Slow's client starts only once Held's fails at its own start
        // deadline, about when collection ends, and then has one more
        // deadline to receive Slow's Initial metadata: the test's steps
        // between seeing the late note and releasing Slow must fit in it,
        // even on a loaded host.
        limits_config(
            &package,
            serde_json::json!({
                "scheduling": {"starts": 1},
                "deadlines": {"start_ms": 10_000, "operation_ms": 120_000},
            }),
        );
        let go = package.isolation.home().join("slow.go");
        let ear = Ear::default();
        let slow_id = fake_id(0xd2);
        let slow = serving(vec![
            Act::Await(go.clone()),
            opened_with(true),
            initial_metadata("Slow", Some(wire_status("idle", &utc(1))), None),
        ]);
        let daemons = [
            FakeDaemon::start(
                &package,
                held,
                "2026-01-01T00:00:02.000Z",
                Answer::Held(held),
                &ear,
            ),
            FakeDaemon::start(&package, slow_id, "2026-01-01T00:00:01.000Z", slow, &ear),
        ];
        let mut terminal = outer_sized(&package, "c2 monitor", WIDE);
        let hint = "Press Ctrl+] then x to exit.";
        shows(
            &mut terminal,
            &empty_monitor(ROWS, WIDE, &["Connecting to 2 sessions (0 ready)", hint]),
        );
        let late = "Discovery incomplete: some sessions are still starting";
        shows(
            &mut terminal,
            &empty_monitor(ROWS, WIDE, &["No session selected.", hint, late]),
        );
        if explicit {
            terminal.send("\x1ds").unwrap();
            // Once Held's client has failed the rows keep their order.
            listed_with(&terminal, "Unavailable", &format!("fake {held}"));
            enter_on(&mut terminal, slow_id);
            // With a row selected, the top row's error slot says it.
            let noted = format!("fake {slow_id} | {late} ");
            whole_frame_shows(&terminal, &noted, |screen| screen.starts_with(&noted));
        }
        fs::write(&go, b"").unwrap();
        if explicit {
            terminal.send("\x1ds").unwrap();
            listed_with(&terminal, STATUS, "Slow");
            terminal.send(ESCAPE).unwrap();
            selector_closed(&terminal);
            // Eligible now, but selected by the user: never acquired.
            let top = session_top_row(
                usize::from(WIDE),
                "Slow",
                &[STATUS, "Not controlling", SESSIONS_HINT],
            );
            top_row_is(&mut terminal, &top);
        } else {
            let top = session_top_row(
                usize::from(WIDE),
                "Slow",
                &[STATUS, "Acquiring", SESSIONS_HINT],
            );
            shows(&mut terminal, &centred_below(&top, ROWS, WIDE, &[]));
        }
        terminal.send("\x1dx").unwrap();
        // Its wide frames can outgrow the oracle's recent bytes, so only the
        // title itself is checked.
        support::assert_released(&package, &mut terminal, 0);
        assert_eq!(terminal.title().as_deref(), Some(TITLE));
        drop(daemons);
        package.assert_clean();
    }
}

/// Waits until a whole frame shows `lines` centred below a top row whose
/// slots read `status` and `control`.
fn read_only_shows(terminal: &Terminal, status: &str, control: &str, lines: &[&str]) {
    let expected = centred_below("", ROWS, COLS, lines);
    let slots = format!(" | {status:<18} | {control:<15} | {SESSIONS_HINT}");
    whole_frame_shows(terminal, &expected, |screen| {
        let (top, below) = screen.split_once('\n').unwrap();
        top.ends_with(&slots) && below == &expected[1..]
    });
}

/// Spec #73 Enter on unconnected rows (derived scenarios) and CTL-05
/// (Monitor: unavailable): with one start slot held by a fake daemon that
/// never completes its client's Open, Enter on an Unavailable row with no
/// client (its daemon stopped) and then on a Starting row selects each
/// read-only: no terminal content, its condition, nothing acquired, and
/// typed keys reach no session, not even the one the monitor controls in
/// the background. Once the Starting row's client connects, it shows the
/// non-controller placeholder until the take-control key acquires it.
#[test]
fn enter_selects_starting_and_unavailable_rows_read_only_until_the_user_acquires() {
    const HELD: &str = "0d5c0000-0000-4000-8000-0000000000c1";
    let package = Package::new();
    without_introduction(&package);
    limits_config(
        &package,
        serde_json::json!({"scheduling": {"starts": 1}, "deadlines": {"start_ms": 120_000}}),
    );
    headless(&package, &watched("Alpha"), "finish");
    let paused = headless(&package, &watched("Paused"), "finish");
    let paused = daemon_of(&package, &paused);
    stop(paused);
    let mut terminal = outer(&package, "c2 monitor");
    terminal.send("\x1ds").unwrap();
    // A stopped daemon never answers the probe's hello: no client.
    whole_frame_shows(&terminal, "Paused unavailable", |screen| {
        screen
            .lines()
            .any(|row| row.starts_with(" ║Unavailable /bin/bash") && row.contains("Paused"))
    });
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    // Initial selection acquires Alpha, the one connected row (#266).
    top_row_is(&mut terminal, &controlled("Alpha", RUNNING));
    terminal.send("a").unwrap();
    received_by(&package, "Alpha", b"a");

    let ear = Ear::default();
    let held = FakeDaemon::start(
        &package,
        HELD,
        "2099-01-01T00:00:00.000Z",
        Answer::Held(HELD),
        &ear,
    );
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║Starting    fake {HELD}"));
    // Reopened, the selector highlights the selected session, Alpha.
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    headless(&package, &watched("Slow"), "finish");
    terminal.send("\x1ds").unwrap();
    whole_frame_shows(&terminal, "Slow starting", |screen| {
        screen
            .lines()
            .any(|row| row.starts_with(" ║Starting    /bin/bash") && row.contains("Slow"))
    });
    // Rows: Alpha, then the not-connected rows by entry, newest first:
    // Slow, the held fake, Paused.
    enter_on(&mut terminal, "Paused");
    read_only_shows(
        &terminal,
        "Unavailable",
        NOT_CONTROLLING,
        &[NO_CONTENT, "Unavailable"],
    );
    terminal.send("k").unwrap();
    terminal.send("\x1ds").unwrap();
    enter_on(&mut terminal, "Slow");
    read_only_shows(
        &terminal,
        "Starting",
        NOT_CONTROLLING,
        &[NO_CONTENT, "Starting"],
    );
    terminal.send("k\x14").unwrap();

    drop(held);
    shows(&mut terminal, &placeholder_screen("Slow", RUNNING));
    listed_as(
        &package,
        "Slow",
        &serde_json::json!({"lifecycle": "detached", "attachments": 0, "controlled": false}),
    );
    terminal.send("\x14").unwrap();
    top_row_is(&mut terminal, &controlled("Slow", RUNNING));
    ready_at(&mut terminal, "Slow", RUNNING, "29 120");
    // The take-control key and keys typed before reached no session.
    terminal.send("s").unwrap();
    received_by(&package, "Slow", b"s");

    kill(Pid::from_raw(paused), Signal::SIGCONT).unwrap();
    terminal.send("\x1ds").unwrap();
    // Every session reported `working`; once Paused's resumed daemon is
    // reached too, the rows stay put.
    for label in ["Slow", "Paused", "Alpha"] {
        listed_with(&terminal, RUNNING, label);
    }
    enter_on(&mut terminal, "Alpha");
    first_row_starts(&terminal, "RAW");
    terminal.send("z").unwrap();
    received_by(&package, "Alpha", b"az");
    terminal.send("\x1dx").unwrap();
    // Its many frames can outgrow the oracle's recent bytes, so only the
    // title itself is checked.
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// CTL-04 (Monitor): when the only attached client of a session detaches,
/// the daemon promotes the monitor's passive client. The monitor applies
/// that restoration and keeps the model live in the background without
/// selecting the session, resizing it or sending it input: the selection
/// stays where it was and typed keys still reach only it. Enter then finds
/// the session already controlled, so the daemon starts no new
/// restoration, and the model built in the background is painted at once,
/// output written since included; only now is the session resized.
#[test]
fn promotion_keeps_a_background_model_without_selection_resize_or_input() {
    let package = Package::new();
    without_introduction(&package);
    // After an empty start Other, the first to appear, is the initial
    // selection; Promoted, attached elsewhere, comes after it.
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    headless(&package, &watched("Other"), "finish");
    top_row_is(&mut terminal, &controlled("Other", RUNNING));
    let promoted = headless(&package, &watched("Promoted"), "finish");
    let other = attach(&package, &promoted);
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║{RUNNING} Promoted"));
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);

    detach(other);
    let attached =
        serde_json::json!({"lifecycle": "attached", "attachments": 1, "controlled": true});
    listed_as(&package, "Promoted", &attached);
    say(&package, "Promoted");
    terminal.send("o").unwrap();
    received_by(&package, "Other", b"o");
    top_row_is(&mut terminal, &controlled("Other", RUNNING));
    assert_eq!(settled_sizes(&package, "Promoted"), ["24 80", "30 120"]);

    // Rows, both Running: Promoted, newer, then Other.
    terminal.send("\x1ds").unwrap();
    enter_on(&mut terminal, "Promoted");
    top_row_is(&mut terminal, &controlled("Promoted", RUNNING));
    first_row_starts(&terminal, "RAW SAID");
    sized_to(&package, "Promoted", &["24 80", "30 120", "29 120"]);
    // The resize brings a restoration, which closes input until it is ready.
    ready_at(&mut terminal, "Promoted", RUNNING, "29 120");
    terminal.send("p").unwrap();
    received_by(&package, "Promoted", b"p");
    received_by(&package, "Other", b"o");
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// The top row's start once Taken's Acquire timed out: its label and the
/// library's timeout.
const TIMED_OUT: &str = "Taken | the session daemon did not answer in time";

/// DSC-05 (Monitor, Fault: failed acquisition and loss): with a session's
/// daemon stopped, Enter selects it and its Acquire times out, which also
/// ends that client. The selection stays on it, showing the failure, and
/// typed keys reach no session (not the one the monitor still controls in
/// the background). The resumed daemon still serves that Acquire, so the
/// client that controls the session loses control and gets it back.
#[test]
fn a_failed_acquisition_keeps_the_selection_and_redirects_no_input() {
    let package = Package::new();
    without_introduction(&package);
    // After an empty start Alpha, the first to appear, is the initial
    // selection; Taken, attached elsewhere, comes after it.
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    headless(&package, &watched("Alpha"), "finish");
    top_row_is(&mut terminal, &controlled("Alpha", RUNNING));
    let taken = headless(&package, &watched("Taken"), "finish");
    let mut other = attach(&package, &taken);
    first_row_starts(&terminal, "RAW");
    terminal.send("a").unwrap();
    received_by(&package, "Alpha", b"a");
    // Rows, both Running: Taken, newer, then Alpha.
    terminal.send("\x1ds").unwrap();
    listed_with(&terminal, RUNNING, "Taken");
    highlight_on(&mut terminal, "Taken");
    let daemon = daemon_of(&package, &taken);
    stop(daemon);
    terminal.send("\r").unwrap();
    whole_frame_shows(&terminal, "the failed acquisition", |screen| {
        let top = screen.lines().next().unwrap_or_default();
        top.starts_with(TIMED_OUT)
            && top.contains(&format!(
                "| {:<18} | {NOT_CONTROLLING:<15} |",
                "Disconnected"
            ))
    });
    terminal.send("k").unwrap();
    kill(Pid::from_raw(daemon), Signal::SIGCONT).unwrap();
    // The timeout left the Acquire's outcome indeterminate, not failed: the
    // resumed daemon serves it before it sees that connection closed. The
    // other client loses control to it, then gets control back with a new
    // restoration, which discards keys typed before it ends.
    other
        .wait_for_bytes(VIEWED_ELSEWHERE.as_bytes(), TIMEOUT)
        .unwrap();
    restored(&mut other);
    // Live output follows the restoration, so the other client is ready.
    say(&package, "Taken");
    other.wait_for_text("RAW SAID", TIMEOUT).unwrap();
    assert!(terminal.row(0).starts_with(TIMED_OUT));
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║{RUNNING:<12} Alpha"));
    enter_on(&mut terminal, "Alpha");
    first_row_starts(&terminal, "RAW");
    terminal.send("z").unwrap();
    received_by(&package, "Alpha", b"az");
    // What Taken receives from its controller follows anything sent before.
    other.send("t").unwrap();
    received_by(&package, "Taken", b"t");
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    detach(other);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// INP-01 (Monitor: takeover on a too-small host; owner decision on #110,
/// 2026-11-14): the take-control key acquires a revoked selection even on
/// a host under four rows or five columns, where the placeholder cannot
/// show. Nothing is resized while the host is too small; once it has room
/// again the session is resized, painted, and takes input. When the other
/// client takes control back, prefix+x exits from the placeholder (INP-01,
/// the placeholder screen).
#[test]
fn the_take_control_key_acquires_on_a_too_small_host() {
    let package = Package::new();
    without_introduction(&package);
    let alpha = headless(&package, &watched("Alpha"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    // Initial selection acquires Alpha, the only session (#266).
    ready_at(&mut terminal, "Alpha", RUNNING, "29 120");
    let mut other = attach(&package, &alpha);
    shows(&mut terminal, &placeholder_screen("Alpha", RUNNING));
    terminal.resize(3, 4).unwrap();
    shows_too_small(&mut terminal, 3, 4);
    terminal.send("\x14").unwrap();
    other.wait_for_text(VIEWED_ELSEWHERE, TIMEOUT).unwrap();
    assert_eq!(
        settled_sizes(&package, "Alpha"),
        ["24 80", "29 120", "30 120"]
    );
    terminal.resize(ROWS, COLS).unwrap();
    ready_at(&mut terminal, "Alpha", RUNNING, "29 120");
    terminal.send("z").unwrap();
    received_by(&package, "Alpha", b"z");
    // The other client takes control back with its own take-control key,
    // and prefix+x exits from the placeholder.
    other.send("\x14").unwrap();
    shows(&mut terminal, &placeholder_screen("Alpha", RUNNING));
    // Its grant resized Alpha back to its host, after its restoration.
    attached_ready(&mut other);
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    detach(other);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// A headless [`watched`] session `title` whose daemon has the
/// `CLIMON2_DAEMON_HOLD` test hook at `hold`: its full session ID.
fn held_headless(package: &Package, title: &str, hold: &Path) -> String {
    let script = watched(title);
    let mut command = package.command("c2");
    command.args(["exec", "--headless", "/bin/bash", "-c", &script]);
    command.env("CLIMON2_DAEMON_HOLD", hold);
    command.env("CLIMON2_DAEMON_CLOCK", clock(package, None));
    // As small as `headless` creates its sessions.
    let mut created = Terminal::spawn(command, 24, 80).unwrap();
    assert_eq!(created.wait_exit(TIMEOUT).unwrap(), 0);
    created.screen().trim().to_owned()
}

/// Holds every frame the session daemon would read, through its
/// `CLIMON2_DAEMON_HOLD` hook at `hold`, once it has said it holds them.
fn hold(hold: &Path) {
    fs::write(hold, b"").unwrap();
    let held = hold.with_extension("held");
    let deadline = Instant::now() + TIMEOUT;
    while !held.exists() {
        assert!(Instant::now() < deadline, "the daemon never held its input");
        thread::sleep(Duration::from_millis(5));
    }
}

/// CTL-05 (Monitor: restoring): while a restoration is in progress (the
/// hand-back's restoration applied, its Ready not yet accepted, because
/// the daemon holds every frame it would read), the top row says
/// Restoring, typed keys reach no session and are never kept for later,
/// and a host resize reaches no session; the background controller gets
/// nothing either. The monitor's stop makes the order deterministic: the
/// hand-back's records wait in its socket, flushed in the daemon pass that
/// answers the departing client, before the hold begins. Once the daemon
/// reads again, the restoration becomes ready, the session is sized to the
/// host as it is then, and keys reach it.
/// It holds the daemon with the `CLIMON2_DAEMON_HOLD` test hook, which
/// release archives lack, so it returns early in archive mode.
#[test]
fn keys_and_resizes_during_a_restoration_reach_no_session() {
    if archive() {
        return;
    }
    let package = Package::new();
    without_introduction(&package);
    // Ready is held for as long as the test holds the daemon.
    limits_config(
        &package,
        serde_json::json!({"deadlines": {"operation_ms": 120_000}}),
    );
    let gate = package.isolation.home().join("hold");
    // After an empty start Alpha, the first to appear, is the initial
    // selection; Held comes after it.
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    headless(&package, &watched("Alpha"), "finish");
    ready_at(&mut terminal, "Alpha", RUNNING, "29 120");
    let restoring = held_headless(&package, "Held", &gate);
    terminal.send("a").unwrap();
    received_by(&package, "Alpha", b"a");
    // Rows, both Running: Held, newer, then Alpha.
    terminal.send("\x1ds").unwrap();
    listed_with(&terminal, RUNNING, "Held");
    enter_on(&mut terminal, "Held");
    ready_at(&mut terminal, "Held", RUNNING, "29 120");
    let other = attach(&package, &restoring);
    shows(&mut terminal, &placeholder_screen("Held", RUNNING));

    let monitor = monitor_process(&package).0;
    stop(monitor);
    detach(other);
    hold(&gate);
    kill(Pid::from_raw(monitor), Signal::SIGCONT).unwrap();
    let held = session_top_row(
        usize::from(COLS),
        "Held",
        &[RUNNING, "Restoring", SESSIONS_HINT],
    );
    whole_frame_shows(&terminal, "Restoring", |screen| {
        let mut rows = screen.lines();
        rows.next() == Some(held.as_str())
            && rows
                .nth(1)
                .is_some_and(|row| row.starts_with("SIZE 30 120"))
    });
    terminal.send("k").unwrap();
    terminal.resize(40, 100).unwrap();
    let wide = session_top_row(100, "Held", &[RUNNING, "Restoring", SESSIONS_HINT]);
    top_row_is(&mut terminal, &wide);
    assert_eq!(
        settled_sizes(&package, "Held"),
        ["24 80", "29 120", "30 120"]
    );
    assert_eq!(settled_sizes(&package, "Alpha"), ["24 80", "29 120"]);

    fs::remove_file(&gate).unwrap();
    ready_on(&mut terminal, 100, ("Held", RUNNING), "39 100");
    terminal.send("z").unwrap();
    received_by(&package, "Held", b"z");
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║{RUNNING} Held"));
    enter_on(&mut terminal, "Alpha");
    // Alpha, background throughout, is sized only now that it is selected.
    sized_to(&package, "Alpha", &["24 80", "29 120", "39 100"]);
    ready_on(&mut terminal, 100, ("Alpha", RUNNING), "39 100");
    terminal.send("y").unwrap();
    received_by(&package, "Alpha", b"ay");
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// Bash that titles its session `name`, then runs each of `steps` once
/// `$HOME/<name>.<n>` exists, `n` counting from 1, giving up after two
/// minutes.
fn stepped(name: &str, steps: &[&str]) -> String {
    let mut script = format!("printf '\\033]2;{name}\\007'");
    for (step, action) in (1..).zip(steps) {
        script.push_str(&format!(
            "\nuntil [ -e \"$HOME/{name}.{step}\" ] || [ \"$SECONDS\" -ge 120 ]; do sleep 0.05; done; {action}"
        ));
    }
    script
}

/// Lets the session [`stepped`] named `name` take its step `step`.
fn step(package: &Package, name: &str, step: u32) {
    fs::write(package.isolation.home().join(format!("{name}.{step}")), b"").unwrap();
}

/// A root OSC 7501 program status report of `state`, as bash prints it.
fn report(state: &str) -> String {
    format!("printf '\\033]7501;state={state}\\033\\\\'")
}

/// Waits until the rows of the open selector are exactly `rows`, over the
/// screen `beneath` it.
fn lists(terminal: &mut Terminal, beneath: &str, rows: &[(&str, &str)]) {
    let expected = with_selector(beneath, rows);
    terminal
        .wait_for_screen(TIMEOUT, |screen| screen == expected)
        .unwrap_or_else(|_| panic!("expected:\n{expected}\nshown:\n{}", terminal.screen()));
}

/// A host showing the top row `top` above a blank canvas.
fn beneath(top: &str) -> String {
    centred_below(top, ROWS, COLS, &[])
}

/// Waits until the open selector highlights exactly the host row `row`.
fn highlights(terminal: &Terminal, row: usize) {
    let deadline = Instant::now() + TIMEOUT;
    while highlighted_rows(terminal) != [row] {
        assert!(
            Instant::now() < deadline,
            "highlighted {:?}, not {row}",
            highlighted_rows(terminal)
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// DSC-01 (Monitor, status order), COM-06 (Monitor labels), STA-07
/// (Monitor) and DSC-05 (Monitor, reorder case), with real sessions whose
/// daemons' status clocks stand at known times: rows show the effective
/// status the daemon reports, Needs attention, Running and Idle, and sort by
/// group, then by the daemon's change time, newest first. A command that
/// exits 0 shows Succeeded; one that exits 143 shows Failed (exit 143) and
/// one killed by SIGTERM Failed (signal 15), each after the status rows by
/// its terminal status time, and each stays once its session has ended. A
/// hidden change (working under pending attention) moves nothing, even with
/// its daemon's clock moved past a newer attention row's time. A status
/// change that moves the highlighted row keeps the highlight on it. The
/// daemons' status clock is a test hook, which release archives lack, so
/// this returns early in archive mode.
#[test]
fn real_sessions_show_their_effective_status_and_final_outcomes_in_order() {
    if archive() {
        return;
    }
    let package = Package::new();
    without_introduction(&package);
    let mut clocks = BTreeMap::new();
    let sessions: [(&str, u64, &[&str]); 7] = [
        (
            "attention",
            1000,
            &[
                &report("idle"),
                &format!("{}; printf '\\033]2;attention 2\\007'", report("working")),
            ],
        ),
        ("waiting", 1500, &[&report("idle")]),
        ("running", 2000, &[&report("working"), &report("clear")]),
        ("idle", 3000, &[]),
        ("succeeded", 4000, &["exit 0"]),
        ("exit 143", 5000, &["exit 143"]),
        ("signal", 6000, &["kill -TERM $$"]),
    ];
    for (name, millis, steps) in sessions {
        let at = clock(&package, Some(millis));
        headless_on(&package, &stepped(name, steps), "finish", &at);
        clocks.insert(name, at);
    }
    let mut terminal = outer(&package, "c2 monitor");
    // Initial selection acquires signal, the first row (#266).
    let selected = beneath(&controlled("signal", "Idle"));
    shows(&mut terminal, &selected);
    terminal.send("\x1ds").unwrap();
    lists(
        &mut terminal,
        &selected,
        &[
            ("Idle", "signal"),
            ("Idle", "exit 143"),
            ("Idle", "succeeded"),
            ("Idle", "idle"),
            ("Idle", "running"),
            ("Idle", "waiting"),
            ("Idle", "attention"),
        ],
    );
    for name in ["attention", "waiting", "running"] {
        step(&package, name, 1);
    }
    lists(
        &mut terminal,
        &selected,
        &[
            ("Needs attention", "waiting"),
            ("Needs attention", "attention"),
            ("Running", "running"),
            ("Idle", "signal"),
            ("Idle", "exit 143"),
            ("Idle", "succeeded"),
            ("Idle", "idle"),
        ],
    );
    for name in ["succeeded", "exit 143", "signal"] {
        step(&package, name, 1);
    }
    let finals = [
        ("Failed (signal 15)", "signal"),
        ("Failed (exit 143)", "exit 143"),
        ("Succeeded", "succeeded"),
    ];
    let mut rows = vec![
        ("Needs attention", "waiting"),
        ("Needs attention", "attention"),
        ("Running", "running"),
        ("Idle", "idle"),
    ];
    rows.extend(finals);
    let ended = beneath(&session_top_row(
        usize::from(COLS),
        "signal",
        &["Failed (signal 15)", "Ended", SESSIONS_HINT],
    ));
    lists(&mut terminal, &ended, &rows);
    // Past waiting's attention time, any status the hidden change caused
    // would move the row above it. The hidden change comes before the title
    // change on the same connection, so any such status has arrived once the
    // title shows.
    fs::write(&clocks["attention"], (T0 + 2500).to_string()).unwrap();
    step(&package, "attention", 2);
    rows[1] = ("Needs attention", "attention 2");
    lists(&mut terminal, &ended, &rows);

    // The selector opened on whichever row was first then; Up without
    // wrapping brings the highlight to the top.
    terminal.send("\x1b[A".repeat(rows.len())).unwrap();
    highlights(&terminal, 5);
    terminal.send("\x1b[B".repeat(3)).unwrap();
    highlights(&terminal, 8);
    step(&package, "running", 2);
    let mut reordered = vec![
        ("Needs attention", "waiting"),
        ("Needs attention", "attention 2"),
        ("Idle", "idle"),
        ("Idle", "running"),
    ];
    reordered.extend(finals);
    lists(&mut terminal, &ended, &reordered);
    highlights(&terminal, 7);
    terminal.send("\x1dx").unwrap();
    // Its many frames can outgrow the oracle's recent bytes, so only the
    // title itself is checked.
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// A wire UTC time `seconds` into 2027-01-15T08:00.
fn utc(seconds: u32) -> String {
    format!("2027-01-15T08:00:{seconds:02}.000Z")
}

/// A wire Status of `state` changed at `changed_at`, with the attention
/// episode exactly when the state carries one.
fn wire_status(state: &str, changed_at: &str) -> Value {
    let episode = matches!(state, "needs_attention" | "acknowledged").then_some("1");
    json!({"state": state, "changed_at": changed_at, "episode_id": episode})
}

/// A wire normal exit with `code`.
fn exited(code: u32) -> Value {
    json!({"kind": "exit", "code": code})
}

/// The Open's success, selecting `title-v1` and `focus-v1` and, with
/// `status`, `status-v1`.
fn opened_with(status: bool) -> Act {
    opened_selecting(status, true)
}

/// The Open's success, selecting `title-v1`, with `status` `status-v1` and
/// with `focus` `focus-v1`.
fn opened_selecting(status: bool, focus: bool) -> Act {
    let mut selected = vec!["daemon-wire-v1", "monitor-v1", "title-v1"];
    selected.extend(status.then_some("status-v1"));
    selected.extend(focus.then_some("focus-v1"));
    Act::Send(
        RESPONSE,
        json!({
            "request_id": "$open", "outcome": "ok", "error": null,
            "result": {
                "kind": "opened",
                "connection_id": "00112233-4455-4677-8899-aabbccddeeff",
                "selected_capabilities": selected,
            },
        }),
    )
}

/// Initial metadata titled `title`, with `status` exactly when `status-v1`
/// is selected, and the command's `result` once it is known.
fn initial_metadata(title: &str, status: Option<Value>, result: Option<Value>) -> Act {
    Act::Send(
        INITIAL_METADATA,
        json!({
            "title": {"available": true, "value": title},
            "status": {"available": status.is_some(), "value": status},
            "progress": {"available": false, "value": null},
            "command_result": result,
        }),
    )
}

/// A fake daemon serving `acts` after a client's Open.
fn serving(acts: Vec<Act>) -> Answer {
    Answer::Serve(std::sync::Arc::new(acts))
}

/// A fake daemon whose Open selects `status-v1` and whose Initial metadata
/// reports `title`, `status` and `result`, then `after`.
fn with_status(title: &str, status: Value, result: Option<Value>, after: Vec<Act>) -> Answer {
    let mut acts = vec![
        opened_with(true),
        initial_metadata(title, Some(status), result),
    ];
    acts.extend(after);
    serving(acts)
}

/// `00..ff` as the last byte of a fake session ID.
fn fake_id(suffix: u8) -> &'static str {
    format!("0d5c0000-0000-4000-8000-0000000000{suffix:02x}").leak()
}

/// DSC-01 (Monitor, status order and initial-selection case) and DSC-05
/// (Monitor, reorder case), with fake daemons reporting exact statuses and
/// times: rows order by group, then by the daemon's change time, newest
/// first, then by ascending full session ID on a tie, never by creation
/// (each fake was created the later, the lower it should sort) or by when
/// the monitor heard it. A client whose Initial metadata is delayed shows
/// Starting in the not-connected group until it arrives, and the startup
/// progress, `Connecting to 9 sessions (8 ready)`, shows until then (#266).
/// Initial selection then takes the first eligible row, the oldest created
/// and the last started, and acquires it, never moving the open selector's
/// highlight. The highlighted row keeps the highlight while a delayed row
/// moves in above it and while its own status change moves it.
#[test]
fn rows_order_by_status_group_daemon_change_time_and_full_session_id() {
    let package = Package::new();
    without_introduction(&package);
    limits_config(
        &package,
        serde_json::json!({"deadlines": {"start_ms": 120_000, "operation_ms": 120_000}}),
    );
    let home = package.isolation.home().to_owned();
    let (idle_go, delayed_go) = (home.join("idle.go"), home.join("delayed.go"));
    let fakes = [
        (
            0xa9,
            with_status(
                "attention new",
                wire_status("needs_attention", &utc(2)),
                None,
                vec![],
            ),
        ),
        (
            0xa8,
            with_status(
                "attention old",
                wire_status("needs_attention", &utc(1)),
                None,
                vec![],
            ),
        ),
        (
            0xa7,
            with_status(
                "acknowledged",
                wire_status("acknowledged", &utc(1)),
                None,
                vec![],
            ),
        ),
        (
            0xa5,
            with_status("running low", wire_status("running", &utc(5)), None, vec![]),
        ),
        (
            0xa6,
            with_status(
                "running high",
                wire_status("running", &utc(5)),
                None,
                vec![],
            ),
        ),
        (
            0xa4,
            with_status(
                "idle",
                wire_status("idle", &utc(9)),
                None,
                vec![
                    Act::Await(idle_go.clone()),
                    Act::Send(
                        STATUS_CHANGED,
                        json!({"status": wire_status("running", &utc(10))}),
                    ),
                ],
            ),
        ),
        (
            0xa3,
            with_status(
                "failed",
                wire_status("failed", &utc(3)),
                Some(exited(3)),
                vec![],
            ),
        ),
        (
            0xa2,
            with_status(
                "succeeded",
                wire_status("completed", &utc(8)),
                Some(exited(0)),
                vec![],
            ),
        ),
        (
            0xa1,
            serving(vec![
                opened_with(true),
                Act::Await(delayed_go.clone()),
                initial_metadata("delayed", Some(wire_status("running", &utc(6))), None),
            ]),
        ),
    ];
    let ear = Ear::default();
    let daemons: Vec<_> = (1..)
        .zip(fakes)
        .map(|(second, (suffix, answer))| {
            let created = format!("2026-01-01T00:00:{second:02}.000Z");
            FakeDaemon::start(&package, fake_id(suffix), &created, answer, &ear)
        })
        .collect();
    let mut terminal = outer(&package, "c2 monitor");
    // Collection waits for the delayed client's Initial metadata, showing
    // its progress: every client was started for the first pass.
    let collecting = empty_monitor(
        ROWS,
        COLS,
        &[
            "Connecting to 9 sessions (8 ready)",
            "Press Ctrl+] then x to exit.",
        ],
    );
    shows(&mut terminal, &collecting);
    terminal.send("\x1ds").unwrap();
    let mut rows = vec![
        ("Needs attention", "attention new"),
        ("Needs attention", "attention old"),
        ("Acknowledged", "acknowledged"),
        ("Running", "running low"),
        ("Running", "running high"),
        ("Idle", "idle"),
        ("Failed (exit 3)", "failed"),
        ("Succeeded", "succeeded"),
        // Until its Initial metadata, nothing reported its title either.
        ("Starting", "fake 0d5c0000-0000-4000-8000-0000000000a1"),
    ];
    lists(&mut terminal, &collecting, &rows);
    // The selector opened on whichever row was first then; Up without
    // wrapping brings the highlight to the top.
    terminal.send("\x1b[A".repeat(rows.len())).unwrap();
    highlights(&terminal, 5);
    terminal.send("\x1b[B".repeat(5)).unwrap();
    highlights(&terminal, 10);

    fs::write(&delayed_go, b"").unwrap();
    rows.pop();
    rows.insert(3, ("Running", "delayed"));
    // Collection over, initial selection takes the first eligible row,
    // neither the newest created nor the first connected, and acquires it,
    // which no fake ever grants; the open selector stays open over it.
    let selected = beneath(&session_top_row(
        usize::from(COLS),
        "attention new",
        &["Needs attention", "Acquiring", SESSIONS_HINT],
    ));
    lists(&mut terminal, &selected, &rows);
    highlights(&terminal, 11);

    fs::write(&idle_go, b"").unwrap();
    rows.remove(6);
    rows.insert(3, ("Running", "idle"));
    lists(&mut terminal, &selected, &rows);
    highlights(&terminal, 8);
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    drop(daemons);
    package.assert_clean();
}

/// Not-connected rows (confirmed 2026-11-08), final rows (derived) and
/// STA-07 (Monitor, disconnection case), with fake daemons: rows with no
/// current status and no final outcome (Unavailable without `status-v1` or
/// uncertain, Disconnected) sort after every status and final row, newest
/// condition first; the monitor's own session is never listed (owner
/// decision on #110). A connection lost without final
/// evidence shows Disconnected there, by when it was lost, never its stale
/// Running nor a fabricated outcome. Final rows without a status time (no
/// `status-v1`), from the Initial metadata or a later Command final, sort
/// by when the monitor learned them, between finals whose daemon times are
/// newer and older.
#[test]
fn not_connected_rows_and_untimed_finals_sort_by_when_the_monitor_recorded_them() {
    let package = Package::new();
    without_introduction(&package);
    // The acquisition initial selection starts is never granted, nor timed
    // out while the test runs. Nor is lost's client retried: each retry its
    // fake daemon accepts would lose it again and re-record when, so on a
    // slow host lost could sort as newer than rows the test adds after it.
    limits_config(
        &package,
        serde_json::json!({
            "deadlines": {"operation_ms": 120_000},
            "retry": {"initial_ms": 120_000, "cap_ms": 120_000},
        }),
    );
    let home = package.isolation.home().to_owned();
    let (lost_go, late_go) = (home.join("lost.go"), home.join("late.go"));
    let ear = Ear::default();
    let created = "2026-01-01T00:00:00.000Z";
    let own = fake_id(0xc0);
    let start = |suffix: u8, answer: Answer| {
        FakeDaemon::start(&package, fake_id(suffix), created, answer, &ear)
    };
    let mut daemons = vec![start(0xc0, Answer::Hello(own))];
    let mut terminal = outer(&package, &format!("CLIMON2_SESSION_ID={own} c2 monitor"));
    terminal
        .wait_for_text("No sessions to show.", TIMEOUT)
        .unwrap();
    // After the empty start, the first eligible arrival is the one-time
    // initial selection (#266); no later arrival moves it.
    daemons.push(start(
        0xc1,
        with_status("idle", wire_status("idle", &utc(1)), None, vec![]),
    ));
    let selected = beneath(&session_top_row(
        usize::from(COLS),
        "idle",
        &["Idle", "Acquiring", SESSIONS_HINT],
    ));
    shows(&mut terminal, &selected);

    let without = vec![opened_with(false), initial_metadata("without", None, None)];
    daemons.extend([
        start(
            0xc2,
            with_status(
                "lost",
                wire_status("running", &utc(7)),
                None,
                vec![Act::Await(lost_go.clone()), Act::Close],
            ),
        ),
        start(
            0xc3,
            with_status(
                "fail new",
                wire_status("failed", &utc(4)),
                Some(exited(6)),
                vec![],
            ),
        ),
        start(
            0xc4,
            with_status(
                "fail old",
                wire_status("failed", "2001-01-01T00:00:00.000Z"),
                Some(exited(4)),
                vec![],
            ),
        ),
        start(0xc5, serving(without)),
    ]);
    let mut rows = vec![
        ("Running", "lost"),
        ("Idle", "idle"),
        ("Failed (exit 6)", "fail new"),
        ("Failed (exit 4)", "fail old"),
        ("Unavailable", "without"),
    ];
    terminal.send("\x1ds").unwrap();
    lists(&mut terminal, &selected, &rows);

    daemons.push(start(0xc6, Answer::Silent));
    let silent = format!("fake {}", fake_id(0xc6));
    rows.insert(4, ("Unavailable", &silent));
    lists(&mut terminal, &selected, &rows);

    fs::write(&lost_go, b"").unwrap();
    rows.remove(0);
    rows.insert(3, ("Disconnected", "lost"));
    lists(&mut terminal, &selected, &rows);

    let late = vec![
        opened_with(false),
        initial_metadata("late metadata", None, Some(exited(5))),
    ];
    daemons.push(start(0xc7, serving(late)));
    rows.insert(2, ("Failed (exit 5)", "late metadata"));
    lists(&mut terminal, &selected, &rows);

    let command = vec![
        opened_with(false),
        initial_metadata("late command", None, None),
        Act::Await(late_go.clone()),
        Act::Send(
            COMMAND_FINAL,
            json!({"command_result": {"kind": "signal", "number": 9, "name": "SIGKILL"}, "status": null}),
        ),
    ];
    daemons.push(start(0xc8, serving(command)));
    rows.insert(4, ("Unavailable", "late command"));
    lists(&mut terminal, &selected, &rows);
    fs::write(&late_go, b"").unwrap();
    rows.remove(4);
    rows.insert(2, ("Failed (signal 9)", "late command"));
    lists(&mut terminal, &selected, &rows);
    terminal.send("\x1dx").unwrap();
    assert_released(&package, &mut terminal, 0);
    drop(daemons);
    package.assert_clean();
}

/// Has the selected [`watched`] session `title` write `modes`, then
/// `marker`, and waits until a whole frame shows `marker` on its first row:
/// the monitor's model of the session then has those modes.
fn set_modes(package: &Package, terminal: &Terminal, title: &str, modes: &[u8], marker: &str) {
    let home = package.isolation.home();
    let staged = home.join(format!("{title}.mode.new"));
    fs::write(&staged, [modes, marker.as_bytes()].concat()).unwrap();
    fs::rename(&staged, home.join(format!("{title}.mode"))).unwrap();
    whole_frame_shows(terminal, marker, |screen| {
        screen
            .lines()
            .nth(1)
            .is_some_and(|row| row.contains(marker))
    });
}

/// INP-03 (Monitor) and COM-04 (Monitor, paste cases): a paste reaches the
/// selected session exactly, binary bytes, prefix commands, a nested start
/// and a broken end marker included, never acting as shortcuts: as the
/// payload alone while the application has bracketed paste off, between
/// its markers while it has it on, and alone again once it turns it off.
/// A pending prefix followed by a paste reaches the session as one literal
/// prefix before it, and the paste's end leaves no prefix pending, so the
/// x typed after it is ordinary. A paste whose markers the host splits
/// across reads arrives exactly the same. A prefix left pending by the read
/// that opened the selector still completes prefix+x (INP-04: that opening
/// came before the prefix).
#[test]
fn pastes_reach_the_selected_session_exactly_with_markers_only_when_it_asks() {
    let package = Package::new();
    without_introduction(&package);
    headless(&package, &watched("Paste"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    ready_at(&mut terminal, "Paste", RUNNING, "29 120");
    let monitor = monitor_process(&package).0;
    terminal
        .send(b"\x1b[200~\x00\xffp\x1dx\x1d\x1ds\x1b[200~\x1b[201\x1b[A\r\x1b[201~")
        .unwrap();
    let mut expected = b"\x00\xffp\x1dx\x1d\x1ds\x1b[200~\x1b[201\x1b[A\r".to_vec();
    received_by(&package, "Paste", &expected);
    read_alone(&mut terminal, monitor, b"\x1d");
    terminal.send(b"\x1b[200~q\x1b[201~x").unwrap();
    expected.extend(b"\x1dqx");
    received_by(&package, "Paste", &expected);

    set_modes(&package, &terminal, "Paste", b"\x1b[?2004h", "B1");
    for fragment in [&b"\x1b[2"[..], b"00~ab\x1b", b"[201", b"x\x1b[20", b"1~"] {
        read_alone(&mut terminal, monitor, fragment);
    }
    expected.extend(b"\x1b[200~ab\x1b[201x\x1b[201~");
    received_by(&package, "Paste", &expected);
    read_alone(&mut terminal, monitor, b"\x1d");
    terminal.send(b"\x1b[200~\x1dx\x1b[201~y").unwrap();
    expected.extend(b"\x1d\x1b[200~\x1dx\x1b[201~y");
    received_by(&package, "Paste", &expected);

    set_modes(&package, &terminal, "Paste", b"\x1b[?2004l", "B0");
    terminal.send(b"\x1b[200~off\x1b[201~").unwrap();
    expected.extend(b"off");
    received_by(&package, "Paste", &expected);
    // A modal the read itself opens is no target change for the prefix
    // that read leaves pending after it: prefix+x still exits.
    read_alone(&mut terminal, monitor, b"\x1ds\x1d");
    terminal.send("x").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// INP-03 (Monitor, Fault; owner decision on #111) and COM-04 (Monitor,
/// paste chunks): with eight-byte chunks and three outstanding operations,
/// while the session's daemon is stopped a bracketed paste does not fit.
/// The top row says it was truncated; the rest of the paste, in a later
/// read, is discarded; and once the daemon resumes the application has
/// received the start marker, whole chunks of the paste and then the end
/// marker, so the x typed next is outside the paste.
#[test]
fn a_paste_refused_partway_is_truncated_and_its_application_gets_the_end_marker() {
    const WIDE: u16 = 250;
    let package = Package::new();
    without_introduction(&package);
    limits_config(
        &package,
        serde_json::json!({"transfer": {"chunk_bytes": 8}, "queues": {"operations": 3}}),
    );
    headless(&package, &watched("Cut"), "finish");
    let daemon = package.metadata().daemon_pid as i32;
    let mut terminal = outer_sized(&package, "c2 monitor", WIDE);
    ready_on(&mut terminal, WIDE, ("Cut", RUNNING), "29 250");
    set_modes(&package, &terminal, "Cut", b"\x1b[?2004h", "B1");
    // Typed input is answered, and turns the session Running, before the
    // daemon stops: nothing earlier holds an operation.
    terminal.send("k").unwrap();
    received_by(&package, "Cut", b"k");
    ready_on(&mut terminal, WIDE, ("Cut", RUNNING), "29 250");
    let monitor = monitor_process(&package).0;
    stop(daemon);
    read_alone(&mut terminal, monitor, b"\x1b[200~abcdefghijklmnop");
    read_alone(&mut terminal, monitor, b"qrstuvwx\x1b[201~");
    shows_row(&mut terminal, "Cut | Paste truncated");
    kill(Pid::from_raw(daemon), Signal::SIGCONT).unwrap();
    let path = package.isolation.home().join("Cut.got");
    let deadline = Instant::now() + TIMEOUT;
    let got = loop {
        let got = fs::read(&path).unwrap_or_default();
        if got.ends_with(b"\x1b[201~") {
            break got;
        }
        assert!(
            Instant::now() < deadline,
            "{:?}",
            got.escape_ascii().to_string()
        );
        thread::sleep(Duration::from_millis(20));
    };
    let pasted = got
        .strip_prefix(b"k\x1b[200~")
        .and_then(|got| got.strip_suffix(b"\x1b[201~"))
        .unwrap_or_else(|| panic!("{:?}", got.escape_ascii().to_string()));
    let whole = b"abcdefghijklmnopqrstuvwx";
    assert!(
        pasted.len() % 8 == 0 && pasted.len() < whole.len() && whole.starts_with(pasted),
        "{:?}",
        pasted.escape_ascii().to_string()
    );
    terminal.send("z").unwrap();
    received_by(&package, "Cut", &[&got[..], b"z"].concat());
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// INP-04 (Monitor, Fault): a target change in the middle of a paste or a
/// pending prefix. Another client's takeover revokes the monitor in the
/// middle of a bracketed paste: the daemon ends the paste for the
/// application, and the rest of it is discarded, even after the session
/// hands control back and is ready again; only what is typed after the
/// paste's end arrives. A prefix pending when the monitor is revoked is
/// discarded, so the take-control key after it takes control rather than
/// following the prefix. A host resize brings a new restoration in the
/// middle of a paste, which keeps admission: the rest of that paste is
/// discarded too, and once the new restoration is ready the application
/// receives the end marker under it, before anything typed later (owner
/// decision on #111). A host too small for the session in the middle of a
/// paste also keeps admission: the application receives the end marker at
/// once, under the same restoration, and the rest of the paste is
/// discarded even once the host has room again.
#[test]
fn a_target_change_discards_the_rest_of_a_paste_and_a_pending_prefix() {
    let package = Package::new();
    without_introduction(&package);
    let id = headless(&package, &watched("Moved"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    ready_at(&mut terminal, "Moved", RUNNING, "29 120");
    set_modes(&package, &terminal, "Moved", b"\x1b[?2004h", "B1");
    let monitor = monitor_process(&package).0;
    read_alone(&mut terminal, monitor, b"\x1b[200~abc");
    received_by(&package, "Moved", b"\x1b[200~abc");
    let other = attach(&package, &id);
    received_by(&package, "Moved", b"\x1b[200~abc\x1b[201~");
    shows(&mut terminal, &placeholder_screen("Moved", RUNNING));
    read_alone(&mut terminal, monitor, b"def");
    detach(other);
    ready_at(&mut terminal, "Moved", RUNNING, "29 120");
    read_alone(&mut terminal, monitor, b"ghi\x1b[201~z");
    let mut expected = b"\x1b[200~abc\x1b[201~z".to_vec();
    received_by(&package, "Moved", &expected);

    read_alone(&mut terminal, monitor, b"\x1d");
    let mut other = attach(&package, &id);
    shows(&mut terminal, &placeholder_screen("Moved", RUNNING));
    terminal.send("\x14").unwrap();
    other.wait_for_text(VIEWED_ELSEWHERE, TIMEOUT).unwrap();
    ready_at(&mut terminal, "Moved", RUNNING, "29 120");
    terminal.send("y").unwrap();
    expected.extend(b"y");
    received_by(&package, "Moved", &expected);

    read_alone(&mut terminal, monitor, b"\x1b[200~r1");
    expected.extend(b"\x1b[200~r1");
    received_by(&package, "Moved", &expected);
    terminal.resize(ROWS, 100).unwrap();
    ready_on(&mut terminal, 100, ("Moved", RUNNING), "29 100");
    read_alone(&mut terminal, monitor, b"r2\x1b[201~x");
    expected.extend(b"\x1b[201~x");
    received_by(&package, "Moved", &expected);

    read_alone(&mut terminal, monitor, b"\x1b[200~s1");
    expected.extend(b"\x1b[200~s1");
    received_by(&package, "Moved", &expected);
    terminal.resize(3, 100).unwrap();
    shows_too_small(&mut terminal, 3, 100);
    expected.extend(b"\x1b[201~");
    received_by(&package, "Moved", &expected);
    terminal.resize(ROWS, 100).unwrap();
    ready_on(&mut terminal, 100, ("Moved", RUNNING), "29 100");
    read_alone(&mut terminal, monitor, b"s2\x1b[201~y");
    expected.extend(b"y");
    received_by(&package, "Moved", &expected);
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    attached_ready(&mut other);
    detach(other);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// INP-04 (Monitor, Fault: disconnect): the selected session's daemon dies
/// while a prefix is pending. The lost connection is a target change, so
/// the prefix is discarded: the x read after it is no exit, the monitor
/// still opens the selector, and the x never reaches the other session,
/// which receives only what is typed once it is selected and ready.
#[test]
fn a_disconnect_discards_a_pending_prefix() {
    let package = Package::new();
    without_introduction(&package);
    // After an empty start Gone, the first to appear, is the initial
    // selection (#266); Other comes after it.
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    let gone = headless(&package, &watched("Gone"), "finish");
    ready_at(&mut terminal, "Gone", RUNNING, "29 120");
    headless(&package, &watched("Other"), "finish");
    let monitor = monitor_process(&package).0;
    read_alone(&mut terminal, monitor, b"\x1d");
    kill(Pid::from_raw(daemon_of(&package, &gone)), Signal::SIGKILL).unwrap();
    whole_frame_shows(&terminal, "the lost connection", |screen| {
        let top = screen.lines().next().unwrap_or_default();
        top.starts_with("Gone") && top.contains("Disconnected")
    });
    read_alone(&mut terminal, monitor, b"x");
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║{RUNNING:<12} Other"));
    enter_on(&mut terminal, "Other");
    ready_at(&mut terminal, "Other", RUNNING, "29 120");
    terminal.send("y").unwrap();
    received_by(&package, "Other", b"y");
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    // A real listing reclaims the killed daemon's stale metadata.
    assert_eq!(package.list().len(), 1);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// Over a lost session's retained history, while a passive retry is
/// scheduled or running (#116).
const RECONNECTING: &str = "Reconnecting...";
/// A lost session's daemon proven gone (owner decision on #116).
const GONE: &str = "Session no longer exists";

/// The requests each client of fake `id` sent after its turn's acts, by
/// turn, as `ear` recorded them.
fn requests_by_turn(ear: &Ear, id: &str) -> Vec<Vec<u8>> {
    let mut turns: Vec<Vec<u8>> = Vec::new();
    for (_, heard_by, heard) in ear.lock().unwrap().iter() {
        match heard {
            Heard::Served if *heard_by == id => turns.push(Vec::new()),
            Heard::Request(code) if *heard_by == id => turns.last_mut().unwrap().push(*code),
            _ => {}
        }
    }
    turns
}

/// How many client instances the monitor `pid` made for session `id`: its
/// library logs, apart from any other process's, such as `climon2 kill`'s.
fn instances(package: &Package, id: &str, pid: i32) -> usize {
    let infix = format!("-{pid}-");
    let logs = library_logs(package, id);
    logs.iter()
        .filter(|log| log.to_string_lossy().contains(&infix))
        .count()
}

/// Waits until a whole frame's top row ends with the slots `status` and
/// `control` (and the sessions hint) and its rows satisfy `rows`.
fn read_only_frame(
    terminal: &Terminal,
    (status, control): (&str, &str),
    what: &str,
    rows: impl Fn(&[&str]) -> bool,
) {
    let slots = format!(" | {status:<18} | {control:<15} | {SESSIONS_HINT}");
    whole_frame_shows(terminal, what, |screen| {
        let lines: Vec<_> = screen.lines().collect();
        lines.len() == usize::from(ROWS) && lines[0].ends_with(&slots) && rows(&lines)
    });
}

/// The middle canvas row of a 30-row host holds only `overlay`, centred.
fn overlaid_by(lines: &[&str], overlay: &str) -> bool {
    let middle = lines.get(15).copied().unwrap_or_default();
    let start = (usize::from(COLS) - overlay.len() - 2) / 2 + 1;
    middle
        .get(start..)
        .is_some_and(|rest| rest.trim_end() == overlay)
}

/// REC-01 (Monitor, Fault: transport loss), REC-02 (Monitor, Fault: full
/// history, reconnecting while scheduled and running, placeholder after the
/// reconnect), COM-03 and COM-05 (Monitor cases: indeterminate work never
/// replayed, a fresh attempt sees no old delivery), DSC-05 (Monitor, Fault:
/// loss keeps the selection) and Enter on disconnected and unconnected
/// Starting rows (derived scenarios), against a fake daemon: its first
/// client is granted control by succession in the background and restored,
/// then selected, so the monitor's Acquire is left unanswered, and the
/// connection is lost. The row stays selected, Disconnected, its retained
/// history read-only under exactly `Reconnecting...` while the retry is
/// scheduled, and still while the retry runs and the row is Starting.
/// Typed keys reach no session. Once the new client opens, the overlay and
/// the cache give way to the non-controller placeholder: the second client
/// was never sent the Acquire, Ready or any input, only its Leave at exit.
#[test]
fn a_lost_connection_keeps_its_history_read_only_until_a_passive_reconnect() {
    const ID: &str = "0d5c0000-0000-4000-8000-0000000000d1";
    let package = Package::new();
    without_introduction(&package);
    // The fake never answers a request: operations wait as long as the
    // test may.
    limits_config(
        &package,
        json!({
            "retry": {"initial_ms": 300, "cap_ms": 300, "jitter_percent": 0},
            "deadlines": {"operation_ms": 600_000},
        }),
    );
    let home = package.isolation.home();
    let authority = json!({"period": "1", "restoration_id": "1"});
    let geometry = json!({"cols": 120, "rows": 29});
    let mut display = [0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1].to_vec();
    display.extend(b"CACHED-LINE\r\nsecond");
    let first = vec![
        opened_selecting(false, false),
        initial_metadata("Kept", None, None),
        Act::Send(
            0x46,
            json!({"period": "1", "restoration_id": "1", "geometry": geometry, "reason": "successor"}),
        ),
        Act::Send(
            0x48,
            json!({"period": "1", "restoration_id": "1", "geometry": geometry}),
        ),
        Act::Raw(0x49, display),
        Act::Send(0x4a, authority),
        Act::Await(home.join("drop")),
        Act::Close,
    ];
    let later = vec![
        Act::Await(home.join("answer")),
        opened_selecting(false, false),
        initial_metadata("Kept", None, None),
    ];
    let ear = Ear::default();
    let fake = FakeDaemon::start(
        &package,
        ID,
        "2099-01-01T00:00:00.000Z",
        Turns::answer(vec![first, later]),
        &ear,
    );
    let mut terminal = outer(&package, "c2 monitor");
    terminal.send("\x1ds").unwrap();
    listed_with(&terminal, "Unavailable", "Kept");
    // Reopened over its row, the selector highlights it.
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    terminal.send("\x1ds").unwrap();
    enter_on(&mut terminal, "Kept");
    let cached =
        |lines: &[&str]| lines[1].starts_with("CACHED-LINE") && lines[2].trim_end() == "second";
    read_only_frame(
        &terminal,
        ("Unavailable", "Restoring"),
        "the background restoration",
        cached,
    );
    fs::write(home.join("drop"), b"").unwrap();
    read_only_frame(
        &terminal,
        ("Disconnected", NOT_CONTROLLING),
        "the retained history, reconnecting",
        |lines| cached(lines) && overlaid_by(lines, RECONNECTING),
    );
    // The retry's client is held at its Open, still Starting.
    let deadline = Instant::now() + TIMEOUT;
    while heard(&ear, Heard::Served).len() < 2 {
        assert!(
            Instant::now() < deadline,
            "the retry never reached the daemon"
        );
        thread::sleep(Duration::from_millis(10));
    }
    read_only_frame(
        &terminal,
        ("Starting", NOT_CONTROLLING),
        "the retained history while the retry is held",
        |lines| cached(lines) && overlaid_by(lines, RECONNECTING),
    );
    terminal.send("typed\r").unwrap();
    fs::write(home.join("answer"), b"").unwrap();
    read_only_frame(
        &terminal,
        ("Unavailable", NOT_CONTROLLING),
        "the placeholder after the reconnect",
        |lines| {
            lines.iter().any(|line| line.trim() == VIEWED_ELSEWHERE)
                && !lines
                    .iter()
                    .any(|line| line.contains("CACHED") || line.contains(RECONNECTING))
        },
    );
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    let turns = requests_by_turn(&ear, ID);
    assert_eq!(turns.len(), 2, "{turns:?}");
    // The first client was held in its acts until it was lost; the second
    // was sent nothing but its Leave at exit: no Acquire, Ready or input.
    assert_eq!(turns[1], [LEAVE]);
    drop(fake);
    package.assert_clean();
}

/// REC-02 (Monitor: full history, cache loss to a gone daemon), REC-01
/// (Monitor: a definitive outcome and a gone daemon stop retry), HDL-01
/// (Monitor case), COM-06 (Monitor: a known outcome survives, the loss is
/// never completion), DSC-05 (Monitor: loss and completion keep the
/// selection; with every row final or gone the monitor stays open and
/// keeps discovering) and Enter on final rows (derived scenario): after an
/// empty start, Lost is selected and acquired, writes forty lines and its
/// daemon is killed. Lost stays selected with its retained history
/// read-only under exactly `Reconnecting...` while retries run, then, once
/// discovery proves its daemon gone, under `Session no longer exists`,
/// which its top row's error slot also says, and retries stop. A wheel
/// notch scrolls the read-only history. `climon2 kill` from another
/// terminal ends Ended, which stays listed Failed with its signal; Enter
/// selects it read-only with no terminal content, the monitor never having
/// controlled it, and it is never retried. A session created then still
/// appears.
#[test]
fn killed_and_ended_sessions_stay_read_only_and_are_never_retried() {
    let package = Package::new();
    without_introduction(&package);
    limits_config(
        &package,
        json!({"retry": {"initial_ms": 100, "cap_ms": 100}}),
    );
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    let lost = headless(&package, &watched("Lost"), "finish");
    ready_at(&mut terminal, "Lost", RUNNING, "29 120");
    emit(&package, "Lost", &numbered(1..=40));
    frame_rows(&terminal, &controlled("Lost", RUNNING), &[(29, "line40")]);
    let ended = headless(&package, &watched("Ended"), "finish");
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, &format!("║{RUNNING} Ended"));
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    kill(Pid::from_raw(daemon_of(&package, &lost)), Signal::SIGKILL).unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while !contains(&terminal.received(), RECONNECTING.as_bytes()) {
        assert!(Instant::now() < deadline, "never reconnecting");
        thread::sleep(Duration::from_millis(10));
    }
    let history = |lines: &[&str], first: &str| {
        lines[1].starts_with(first) && lines[29].starts_with("line40")
    };
    let gone = |status: &str| {
        let label = format!("Lost | {GONE}");
        session_top_row(
            usize::from(COLS),
            &label,
            &[status, NOT_CONTROLLING, SESSIONS_HINT],
        )
    };
    whole_frame_shows(&terminal, "the gone session's history", |screen| {
        let lines: Vec<_> = screen.lines().collect();
        if lines.len() != usize::from(ROWS) {
            return false;
        }
        let top = [gone("Disconnected"), gone("Unavailable")];
        top.iter().any(|top| lines[0] == top)
            && history(&lines, "line12")
            && overlaid_by(&lines, GONE)
    });
    let monitor = monitor_process(&package).0;
    let logs = instances(&package, &lost, monitor);
    terminal.send(WHEEL_UP).unwrap();
    whole_frame_shows(&terminal, "the scrolled history", |screen| {
        let lines: Vec<_> = screen.lines().collect();
        if lines.len() != usize::from(ROWS) {
            return false;
        }
        lines[0].contains(" | History ")
            && lines[1].starts_with("line11")
            && overlaid_by(&lines, GONE)
    });

    support::kill_session(&package, &ended);
    terminal.send("\x1ds").unwrap();
    // The final row lists first, and the selector opens on the selected
    // row, Lost, below it (#113).
    shows_row(&mut terminal, "║Failed (signal 1) Ended");
    terminal.send("\x1b[A\r").unwrap();
    whole_frame_shows(&terminal, "Ended read-only", |screen| {
        let lines: Vec<_> = screen.lines().collect();
        if lines.is_empty() {
            return false;
        }
        let top = session_top_row(
            usize::from(COLS),
            "Ended",
            &["Failed (signal 1)", "Ended", SESSIONS_HINT],
        );
        lines[0] == top
            && lines.iter().any(|line| line.trim() == NO_CONTENT)
            && lines.iter().any(|line| line.trim() == "Failed (signal 1)")
    });
    headless(&package, &watched("Later"), "finish");
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, "Later");
    // Retrying would make another library log at least every 100 ms.
    thread::sleep(Duration::from_secs(1));
    assert_eq!(instances(&package, &lost, monitor), logs);
    assert_eq!(instances(&package, &ended, monitor), 1);
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    // A real listing reclaims the killed daemon's stale metadata.
    package.list();
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// The delays the monitor's general log says it scheduled for `id`'s
/// retries, in order.
fn retry_delays(package: &Package, id: &str) -> Vec<u64> {
    let session = format!("session={id}");
    monitor_logs(package)
        .lines()
        .filter(|line| line.contains("client retry scheduled") && line.contains(&session))
        .map(|line| {
            let delay = line.split("delay_ms=").nth(1).unwrap();
            delay.split_whitespace().next().unwrap().parse().unwrap()
        })
        .collect()
}

/// REC-01 (Monitor, Fault: missing library, repaired installation,
/// permanently rejected protocol; visible exponential retry with jitter and
/// cap), LIB-02 (Monitor: a library error stays on its rows) and DSC-05
/// (Monitor, Fault: every row unavailable): with its version's client
/// library missing, every row is Unavailable and the monitor stays open,
/// still discovering a newly appearing session; selected, that row shows no
/// terminal content, its condition and exactly `Reconnecting...`, the
/// loader's diagnostic in its top row's error slot. Once the installation
/// is repaired, the real session connects by itself and is listed Running,
/// not selected, as an explicit selection came first. The other row's
/// daemon lacks
/// `monitor-v1`, a permanent rejection: it says it is not reconnecting and
/// is never retried, while a session whose daemon keeps hanging up before
/// answering its Open is retried with delays doubling from
/// `retry.initial_ms`, each within its jitter, held at `retry.cap_ms`.
#[test]
fn failed_clients_retry_with_backoff_until_repaired_or_rejected_for_good() {
    const REJECTING: &str = "0d5c0000-0000-4000-8000-0000000000d2";
    const FLAKY: &str = "0d5c0000-0000-4000-8000-0000000000d3";
    let package = Package::private();
    without_introduction(&package);
    limits_config(
        &package,
        json!({"retry": {"initial_ms": 200, "cap_ms": 800, "jitter_percent": 10}}),
    );
    headless(&package, &watched("Fixed"), "finish");
    let library = package
        .executable_home()
        .join(VERSION)
        .join("libclimon2_client.so");
    let moved = library.with_extension("so.moved");
    fs::rename(&library, &moved).unwrap();
    let mut terminal = outer(&package, "c2 monitor");
    terminal.send("\x1ds").unwrap();
    whole_frame_shows(&terminal, "Fixed unavailable", |screen| {
        screen
            .lines()
            .any(|row| row.starts_with(" ║Unavailable /bin/bash") && row.contains("Fixed"))
    });
    let ear = Ear::default();
    let rejecting = FakeDaemon::start(
        &package,
        REJECTING,
        "2099-01-01T00:00:00.000Z",
        Answer::Bare(REJECTING),
        &ear,
    );
    listed_with(&terminal, "Unavailable", &format!("fake {REJECTING}"));
    // Opened before any row existed, the selector highlights none until
    // it is opened again.
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    terminal.send("\x1ds").unwrap();
    enter_on(&mut terminal, REJECTING);
    let missing = |lines: &[&str]| {
        lines[0].starts_with(&format!("fake {REJECTING} | "))
            && lines.iter().any(|line| line.trim() == NO_CONTENT)
            && lines.iter().any(|line| line.trim() == RECONNECTING)
    };
    read_only_frame(
        &terminal,
        ("Unavailable", NOT_CONTROLLING),
        "the missing library",
        missing,
    );
    fs::rename(&moved, &library).unwrap();
    read_only_frame(
        &terminal,
        ("Unavailable", NOT_CONTROLLING),
        "the permanent rejection",
        |lines| {
            lines
                .iter()
                .any(|line| line.trim().starts_with("Not reconnecting: "))
                && !lines.iter().any(|line| line.contains(RECONNECTING))
        },
    );
    terminal.send("\x1ds").unwrap();
    listed_with(&terminal, RUNNING, "Fixed");
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    let monitor = monitor_process(&package).0;
    let rejected = instances(&package, REJECTING, monitor);
    let flaky = FakeDaemon::start(
        &package,
        FLAKY,
        "2099-01-01T00:00:01.000Z",
        Turns::answer(vec![vec![Act::Close]]),
        &ear,
    );
    let deadline = Instant::now() + TIMEOUT;
    while retry_delays(&package, FLAKY).len() < 5 {
        assert!(
            Instant::now() < deadline,
            "the flaky session was not retried"
        );
        thread::sleep(Duration::from_millis(20));
    }
    let delays = retry_delays(&package, FLAKY);
    let ranges = [180..=220, 360..=440, 720..=800, 720..=800, 720..=800];
    for (delay, range) in delays.iter().zip(ranges) {
        assert!(range.contains(delay), "{delays:?}");
    }
    assert_eq!(instances(&package, REJECTING, monitor), rejected);
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    drop((rejecting, flaky));
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// REC-01 (Monitor, Fault: a contained panic) and LIB-05 (Monitor: a
/// panicking instance stays on its own row): with the hooked library
/// panicking once in the monitor's process as a connection handles its
/// first frame, that session's first client fails, contained, while the
/// other session connects; the failed one is retried passively, connects,
/// and the first row to become eligible is selected and acquired. Fault
/// hooks are absent from release archives, so this returns early there.
#[test]
fn a_contained_panic_is_retried_on_its_own_row() {
    if archive() {
        return;
    }
    let package = Package::new();
    without_introduction(&package);
    let first = headless(&package, &watched("First"), "finish");
    let second = headless(&package, &watched("Second"), "finish");
    let mut terminal = outer(&package, "CLIMON2_LIBRARY_FAULT=panic:receive c2 monitor");
    terminal.send("\x1ds").unwrap();
    for label in ["First", "Second"] {
        listed_with(&terminal, RUNNING, label);
    }
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    let retried: Vec<_> = [&first, &second]
        .into_iter()
        .filter(|id| !retry_delays(&package, id).is_empty())
        .collect();
    assert_eq!(retried.len(), 1, "{retried:?}");
    let monitor = monitor_process(&package).0;
    assert_eq!(instances(&package, retried[0], monitor), 2);
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// Waits until the host's mouse tracking is the monitor's own button-event
/// tracking (1002), or with `level` 1000 click-only or 1003 any-event
/// tracking, as the bytes the monitor wrote set it.
fn host_tracks(terminal: &Terminal, level: u16) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let modes = terminal.modes();
        let tracking = (modes.mouse, modes.mouse_drag, modes.mouse_motion);
        let expected = (true, level == 1002, level == 1003);
        if tracking == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "host mouse tracking {tracking:?}, not {level}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// INP-05 (Monitor; owner decision on #111): the host mirrors the mouse
/// tracking level of the application taking input, so an application
/// asking for any-event tracking (1003) gets motion with no button held,
/// and one asking for clicks only (1000) gets no drags from the host. The
/// host returns to the monitor's own button-event tracking (1002) while
/// the application reports nothing, while the selector covers it, and
/// while another client's takeover has it, and mirrors it again once it
/// takes input again; leaving restores the baseline.
#[test]
fn the_host_mirrors_the_application_mouse_tracking_level() {
    let package = Package::new();
    without_introduction(&package);
    let id = headless(&package, &watched("Track"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    ready_at(&mut terminal, "Track", RUNNING, "29 120");
    host_tracks(&terminal, 1002);
    set_modes(
        &package,
        &terminal,
        "Track",
        b"\x1b[?1003h\x1b[?1006h",
        "T3",
    );
    host_tracks(&terminal, 1003);
    terminal.send("\x1b[<35;6;4M").unwrap();
    received_by(&package, "Track", b"\x1b[<35;6;3M");
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, "║Running Track");
    host_tracks(&terminal, 1002);
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    host_tracks(&terminal, 1003);
    set_modes(
        &package,
        &terminal,
        "Track",
        b"\x1b[?1003l\x1b[?1000h",
        "T1",
    );
    host_tracks(&terminal, 1000);
    set_modes(&package, &terminal, "Track", b"\x1b[?1000l", "T0");
    host_tracks(&terminal, 1002);
    set_modes(&package, &terminal, "Track", b"\x1b[?1003h", "T4");
    host_tracks(&terminal, 1003);
    let other = attach(&package, &id);
    shows(&mut terminal, &placeholder_screen("Track", RUNNING));
    host_tracks(&terminal, 1002);
    detach(other);
    ready_at(&mut terminal, "Track", RUNNING, "29 120");
    host_tracks(&terminal, 1003);
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// INP-05 (Monitor: application, chrome and selector cases) and INP-04
/// (Monitor: a modal in the middle of a gesture): with button-event
/// tracking in SGR form, a click below the top row reaches the application
/// one row up, a drag that strays onto the top row is reported at the
/// application's first row, and the wheel reaches it at the bottom row. A
/// gesture that begins on the top row stays the monitor's over the
/// application. When the selector opens in the middle of a drag, the rest
/// of the drag is discarded through its release, even once the selector
/// has closed, and clicks in the selector reach no application. In the
/// single-byte encoding a click and its release arrive as button 0 and 3,
/// and with reporting off nothing arrives.
#[test]
fn mouse_events_reach_the_application_translated_and_keep_their_gesture() {
    let package = Package::new();
    without_introduction(&package);
    headless(&package, &watched("Mouse"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    ready_at(&mut terminal, "Mouse", RUNNING, "29 120");
    set_modes(
        &package,
        &terminal,
        "Mouse",
        b"\x1b[?1002h\x1b[?1006h",
        "M1",
    );
    terminal.send("\x1b[<0;5;3M\x1b[<0;5;3m").unwrap();
    let mut expected = b"\x1b[<0;5;2M\x1b[<0;5;2m".to_vec();
    received_by(&package, "Mouse", &expected);
    terminal
        .send("\x1b[<0;7;4M\x1b[<32;8;1M\x1b[<0;8;1m\x1b[<65;2;30M")
        .unwrap();
    expected.extend(b"\x1b[<0;7;3M\x1b[<32;8;1M\x1b[<0;8;1m\x1b[<65;2;29M");
    received_by(&package, "Mouse", &expected);
    terminal
        .send("\x1b[<0;3;1M\x1b[<32;3;5M\x1b[<0;3;5ma")
        .unwrap();
    expected.extend(b"a");
    received_by(&package, "Mouse", &expected);

    terminal.send("\x1b[<0;4;4M").unwrap();
    expected.extend(b"\x1b[<0;4;3M");
    received_by(&package, "Mouse", &expected);
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, "║Running Mouse");
    terminal.send("\x1b[<32;5;5M").unwrap();
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    terminal
        .send("\x1b[<32;6;6M\x1b[<0;6;6m\x1b[<0;9;9M\x1b[<0;9;9m")
        .unwrap();
    expected.extend(b"\x1b[<0;9;8M\x1b[<0;9;8m");
    received_by(&package, "Mouse", &expected);
    terminal.send("\x1ds").unwrap();
    shows_row(&mut terminal, "║Running Mouse");
    terminal.send("\x1b[<0;10;3M\x1b[<0;10;3m").unwrap();
    terminal.send(ESCAPE).unwrap();
    selector_closed(&terminal);
    terminal.send("c").unwrap();
    expected.extend(b"c");
    received_by(&package, "Mouse", &expected);

    set_modes(&package, &terminal, "Mouse", b"\x1b[?1006l", "M2");
    terminal.send("\x1b[<0;5;3M\x1b[<0;5;3m").unwrap();
    expected.extend(b"\x1b[M\x20\x25\x22\x1b[M\x23\x25\x22");
    received_by(&package, "Mouse", &expected);
    set_modes(&package, &terminal, "Mouse", b"\x1b[?1002l", "M0");
    terminal.send("\x1b[<0;5;3M\x1b[<0;5;3mb").unwrap();
    expected.extend(b"b");
    received_by(&package, "Mouse", &expected);
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// FOC-01 (Monitor: application focus cases): application focus follows
/// selection, menus and host focus, each change once. Alpha, selected, ready
/// and assumed host-focused, enables focus reporting and receives focus in
/// once. Opening the selector sends focus out, closing it focus in; the
/// host's focus out sends focus out, and a second focus out or a menu
/// opened while the host is unfocused sends nothing more, until the host's
/// focus in. Switching to Beta sends Alpha focus out when the selector
/// opens and Beta its focus once ready; switching back focuses Alpha and
/// unfocuses Beta, once each.
#[test]
fn application_focus_follows_selection_menus_and_host_focus_once_each() {
    let package = Package::new();
    without_introduction(&package);
    // After an empty start Alpha, the first to appear, is the initial
    // selection (#266); Beta comes after it.
    let mut terminal = outer(&package, "c2 monitor");
    opened(&mut terminal);
    headless(&package, &watched("Alpha"), "finish");
    ready_at(&mut terminal, "Alpha", RUNNING, "29 120");
    headless(&package, &watched("Beta"), "finish");
    set_modes(&package, &terminal, "Alpha", b"\x1b[?1004h", "F1");
    let mut alpha = b"\x1b[I".to_vec();
    received_by(&package, "Alpha", &alpha);
    terminal.send("\x1ds").unwrap();
    alpha.extend(b"\x1b[O");
    received_by(&package, "Alpha", &alpha);
    terminal.send(ESCAPE).unwrap();
    alpha.extend(b"\x1b[I");
    received_by(&package, "Alpha", &alpha);
    terminal.send("\x1b[O").unwrap();
    alpha.extend(b"\x1b[O");
    received_by(&package, "Alpha", &alpha);
    terminal.send("\x1b[O\x1ds").unwrap();
    shows_row(&mut terminal, "║Running Alpha");
    // Escape closes the selector; the host then reports focus in.
    terminal.send(ESCAPE).unwrap();
    alpha.extend(b"\x1b[I");
    received_by(&package, "Alpha", &alpha);

    terminal.send("\x1ds").unwrap();
    alpha.extend(b"\x1b[O");
    received_by(&package, "Alpha", &alpha);
    listed_with(&terminal, RUNNING, "Beta");
    enter_on(&mut terminal, "Beta");
    ready_at(&mut terminal, "Beta", RUNNING, "29 120");
    set_modes(&package, &terminal, "Beta", b"\x1b[?1004h", "F2");
    let mut beta = b"\x1b[I".to_vec();
    received_by(&package, "Beta", &beta);
    terminal.send("\x1ds").unwrap();
    beta.extend(b"\x1b[O");
    received_by(&package, "Beta", &beta);
    enter_on(&mut terminal, "Alpha");
    alpha.extend(b"\x1b[I");
    received_by(&package, "Alpha", &alpha);
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    received_by(&package, "Alpha", &alpha);
    received_by(&package, "Beta", &beta);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// FOC-02 (Monitor; owner decisions on #111): missing focus support is a
/// documented limitation only, never shown. A host that never reports
/// focus (this test host) leaves sessions usable with focus assumed: Real,
/// served by a daemon with `focus-v1`, takes typed input. A session whose
/// daemon lacks `focus-v1` shows nothing about it in its top row.
#[test]
fn without_focus_support_sessions_stay_usable_and_nothing_is_shown() {
    let package = Package::new();
    without_introduction(&package);
    limits_config(
        &package,
        serde_json::json!({"deadlines": {"operation_ms": 600_000}}),
    );
    headless(&package, &watched("Real"), "finish");
    let ear = Ear::default();
    let unfocused = Answer::Unfocused(std::sync::Arc::new(vec![
        opened_selecting(true, false),
        initial_metadata("plain", Some(wire_status("idle", &utc(1))), None),
    ]));
    let fake = FakeDaemon::start(
        &package,
        fake_id(0xd1),
        "2026-01-01T00:00:01.000Z",
        unfocused,
        &ear,
    );
    let mut terminal = outer(&package, "c2 monitor");
    // Initial selection picks one of the two; both are listed by then.
    whole_frame_shows(&terminal, "a selection", |screen| {
        !screen.starts_with(TOP_ROW)
    });
    terminal.send("\x1ds").unwrap();
    enter_on(&mut terminal, "Real");
    ready_at(&mut terminal, "Real", RUNNING, "29 120");
    terminal.send("u").unwrap();
    received_by(&package, "Real", b"u");
    terminal.send("\x1ds").unwrap();
    enter_on(&mut terminal, "plain");
    let fields = ["Idle", "Acquiring", SESSIONS_HINT];
    let top = session_top_row(usize::from(COLS), "plain", &fields);
    top_row_is(&mut terminal, &top);
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    drop(fake);
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// Has the [`watched`] session `title` write `bytes`, as [`set_modes`]
/// does, without waiting for them to show; first it waits until the
/// session has written, and removed, what it was given before, so the
/// removal never takes these.
fn emit(package: &Package, title: &str, bytes: &[u8]) {
    let home = package.isolation.home();
    let pending = home.join(format!("{title}.mode"));
    let deadline = Instant::now() + TIMEOUT;
    while pending.exists() {
        assert!(Instant::now() < deadline, "{title} never wrote its bytes");
        thread::sleep(Duration::from_millis(20));
    }
    let staged = home.join(format!("{title}.mode.new"));
    fs::write(&staged, bytes).unwrap();
    fs::rename(&staged, home.join(format!("{title}.mode"))).unwrap();
}

/// `line<n>` for each `n` in `numbers`, two digits at least, each on a new
/// line with the rest of that line erased.
fn numbered(numbers: std::ops::RangeInclusive<u32>) -> Vec<u8> {
    numbers
        .flat_map(|number| format!("\r\nline{number:02}\x1b[K").into_bytes())
        .collect()
}

/// Waits until a whole frame's top row is `top` and each host row in
/// `rows` starts with its text.
fn frame_rows(terminal: &Terminal, top: &str, rows: &[(usize, &str)]) {
    whole_frame_shows(terminal, &format!("{top:?} over {rows:?}"), |screen| {
        let lines: Vec<_> = screen.lines().collect();
        lines.first() == Some(&top)
            && rows
                .iter()
                .all(|(row, text)| lines.get(*row).is_some_and(|line| line.starts_with(text)))
    });
}

/// A wheel notch up (back) or down over the session's canvas.
const WHEEL_UP: &str = "\x1b[<64;10;10M";
const WHEEL_DOWN: &str = "\x1b[<65;10;10M";
/// The glossary's truncation notice.
const NOTICE: &str = "[climon2: older session scrollback was discarded]";

/// TRM-05 (Monitor; owner decisions on #112) and FOC-01 (Monitor, history
/// case): the wheel scrolls the selected session's normal history one row
/// a notch while the application does not report the mouse, `History`
/// shows in the control slot and the cursor, scrolled out of view, hides.
/// Output keeps the view anchored on the rows it shows. Typing returns it
/// live and reaches the application, which saw focus in once and no focus
/// change from scrolling, and so does a paste. Scrolled to the oldest row (`RAW`, history not
/// truncated, so no notice), 210 more rows with 200 history rows kept
/// evict it: the view moves to the truncation notice above the oldest row
/// kept, and one notch down leaves the notice. On the alternate screen the
/// wheel shows no history: it sends nothing, then cursor keys once the
/// application turns alternate scroll on, in its cursor mode. Nothing
/// reaches the host's native scrollback.
#[test]
fn the_wheel_scrolls_anchored_history_and_typing_returns_live() {
    let package = Package::new();
    without_introduction(&package);
    limits_config(&package, json!({"history": {"max_rows": 200}}));
    headless(&package, &watched("Hist"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    ready_at(&mut terminal, "Hist", RUNNING, "29 120");
    let native = terminal.history();
    let live = session_top_row(
        usize::from(COLS),
        "Hist",
        &[RUNNING, CONTROLLING, SESSIONS_HINT],
    );
    let history = session_top_row(
        usize::from(COLS),
        "Hist",
        &[RUNNING, "History", SESSIONS_HINT],
    );

    emit(
        &package,
        "Hist",
        &[&b"\x1b[?1004h"[..], &numbered(1..=60)].concat(),
    );
    received_by(&package, "Hist", b"\x1b[I");
    frame_rows(&terminal, &live, &[(1, "line32"), (29, "line60")]);
    terminal.send(WHEEL_UP.repeat(3)).unwrap();
    frame_rows(
        &terminal,
        &history,
        &[(1, "line29"), (3, "line31"), (4, "line32"), (29, "line57")],
    );
    assert!(!terminal.cursor().visible, "scrolled out of view");

    emit(
        &package,
        "Hist",
        &[&numbered(61..=65)[..], b"\x1b[1;1HMARKER\x1b[29;7H"].concat(),
    );
    frame_rows(
        &terminal,
        &history,
        &[(1, "line29"), (8, "line36"), (9, "MARKER"), (29, "line57")],
    );
    terminal.send("k").unwrap();
    received_by(&package, "Hist", b"\x1b[Ik");
    frame_rows(&terminal, &live, &[(1, "MARKER"), (29, "line65")]);
    cursor_at(&terminal, 29, 6);

    terminal.send(WHEEL_UP.repeat(60)).unwrap();
    frame_rows(&terminal, &history, &[(1, "RAW"), (2, "line01")]);
    emit(&package, "Hist", &numbered(66..=275));
    frame_rows(
        &terminal,
        &history,
        &[(1, NOTICE), (2, "line47"), (29, "line74")],
    );
    terminal.send(WHEEL_DOWN).unwrap();
    frame_rows(&terminal, &history, &[(1, "line47"), (29, "line75")]);

    // A paste returns the view live too; the application, without
    // bracketed paste, receives its payload alone.
    terminal.send("\x1b[200~j\x1b[201~").unwrap();
    received_by(&package, "Hist", b"\x1b[Ikj");
    frame_rows(&terminal, &live, &[(1, "line247"), (29, "line275")]);
    emit(&package, "Hist", b"\x1b[?1049h\x1b[HALT");
    frame_rows(&terminal, &live, &[(1, "ALT"), (2, "")]);
    terminal.send(format!("{WHEEL_UP}{WHEEL_DOWN}a")).unwrap();
    received_by(&package, "Hist", b"\x1b[Ikja");
    frame_rows(&terminal, &live, &[(1, "ALT")]);
    emit(&package, "Hist", b"\x1b[?1007h\x1b[2;1HSCROLL");
    frame_rows(&terminal, &live, &[(2, "SCROLL")]);
    terminal.send(format!("{WHEEL_UP}{WHEEL_DOWN}b")).unwrap();
    received_by(&package, "Hist", b"\x1b[Ikja\x1b[A\x1b[Bb");
    emit(&package, "Hist", b"\x1b[?1h\x1b[3;1HCURSOR");
    frame_rows(&terminal, &live, &[(3, "CURSOR")]);
    terminal.send(format!("{WHEEL_UP}c")).unwrap();
    received_by(&package, "Hist", b"\x1b[Ikja\x1b[A\x1b[Bb\x1bOAc");
    emit(&package, "Hist", b"\x1b[?1049l");
    frame_rows(&terminal, &live, &[(29, "line275")]);

    assert_eq!(terminal.history(), native, "no native scrollback");
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// The top row of the selected, controlled session whose label the host
/// shows as `label` over `cells` cells (its row text has a wide
/// character's covered cell as a blank, and no combining mark), in the 55
/// cells a 120-cell top row leaves it, with `status`.
fn labelled(label: &str, cells: usize, status: &str) -> String {
    let slots = session_top_row(65, "", &[status, CONTROLLING, SESSIONS_HINT]);
    format!("{label}{}{slots}", " ".repeat(55 - cells))
}

/// TRM-06 (Monitor; owner decision on #112: nothing marquees): the label
/// is the latest nonempty title, literal. One that fits its 55 cells
/// exactly is whole; one a cell longer is truncated statically to end
/// with `…`. A wide character that would cross the limit leaves a blank
/// cell instead, and combining marks stay with their base. An empty title
/// keeps the latest nonempty one, and a status change moves neither the
/// truncated label nor the slots.
#[test]
fn overflowing_titles_are_truncated_statically_with_an_ellipsis() {
    let package = Package::new();
    without_introduction(&package);
    headless(&package, &watched("Title"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    ready_at(&mut terminal, "Title", RUNNING, "29 120");

    let fits = "a".repeat(55);
    retitle(&package, "Title", &fits);
    top_row_is(&mut terminal, &labelled(&fits, 55, RUNNING));
    retitle(&package, "Title", &"b".repeat(56));
    let cut = format!("{}…", "b".repeat(54));
    top_row_is(&mut terminal, &labelled(&cut, 55, RUNNING));
    retitle(&package, "Title", &format!("c{}", "日".repeat(28)));
    let wide = format!("c{}…", "日 ".repeat(26));
    top_row_is(&mut terminal, &labelled(&wide, 54, RUNNING));
    retitle(&package, "Title", &"e\u{301}".repeat(56));
    let combining = format!("{}…", "e".repeat(54));
    top_row_is(&mut terminal, &labelled(&combining, 55, RUNNING));
    let marks = terminal.cells(0);
    assert_eq!(marks[0].text, "e\u{301}");
    assert_eq!(marks[53].text, "e\u{301}");
    assert_eq!(marks[54].text, "…");
    retitle(&package, "Title", "");
    report_state(&package, "Title", "idle");
    top_row_is(&mut terminal, &labelled(&combining, 55, "Needs attention"));
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// TRM-01 (Monitor; #108's note on #112): what an application draws
/// reaches the host faithfully. Each underline style keeps its style;
/// the cursor keeps its DECSCUSR shape and blink, and hides when the
/// application hides it; the 1049 alternate screen replaces the normal
/// one and leaving it restores the normal screen and cursor; a
/// synchronized update that spans two writes shows whole once it ends.
#[test]
fn underline_styles_cursor_styles_screens_and_synchronized_updates_reach_the_host() {
    let package = Package::new();
    without_introduction(&package);
    headless(&package, &watched("Fid"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    ready_at(&mut terminal, "Fid", RUNNING, "29 120");
    let live = session_top_row(
        usize::from(COLS),
        "Fid",
        &[RUNNING, CONTROLLING, SESSIONS_HINT],
    );

    emit(
        &package,
        "Fid",
        b"\x1b[3;1H\x1b[4mS\x1b[4:2mD\x1b[4:3mC\x1b[4:4mO\x1b[4:5mA\x1b[0mN\x1b[5 q",
    );
    frame_rows(&terminal, &live, &[(3, "SDCOAN")]);
    let styles: Vec<_> = terminal.cells(3)[..6]
        .iter()
        .map(|cell| cell.rendition.underline)
        .collect();
    assert_eq!(
        styles,
        [
            Some(Underline::Single),
            Some(Underline::Double),
            Some(Underline::Curly),
            Some(Underline::Dotted),
            Some(Underline::Dashed),
            None,
        ]
    );
    cursor_at(&terminal, 3, 6);
    let cursor = terminal.cursor();
    assert_eq!((cursor.shape, cursor.blinking), (CursorShape::Bar, true));
    emit(&package, "Fid", b"\x1b[4 q\x1b[3;8HU");
    frame_rows(&terminal, &live, &[(3, "SDCOAN U")]);
    cursor_at(&terminal, 3, 8);
    let cursor = terminal.cursor();
    assert_eq!(
        (cursor.shape, cursor.blinking),
        (CursorShape::Underline, false)
    );
    emit(&package, "Fid", b"\x1b[?25l\x1b[3;10HH");
    frame_rows(&terminal, &live, &[(3, "SDCOAN U H")]);
    assert!(!terminal.cursor().visible);

    emit(&package, "Fid", b"\x1b[?25h\x1b[?1049h\x1b[2;2HALT");
    frame_rows(&terminal, &live, &[(1, ""), (2, " ALT"), (3, "")]);
    cursor_at(&terminal, 2, 4);
    emit(&package, "Fid", b"\x1b[?1049l");
    frame_rows(&terminal, &live, &[(3, "SDCOAN U H")]);
    cursor_at(&terminal, 3, 10);

    emit(&package, "Fid", b"\x1b[?2026h\x1b[5;1HHALF");
    emit(&package, "Fid", b"DONE\x1b[?2026l");
    frame_rows(&terminal, &live, &[(5, "HALFDONE")]);
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    finish_sessions(&package, "finish");
    package.assert_clean();
}

/// Bash that titles its session `title`, then, until `$HOME/finish`
/// exists, prints `tick <n>` lines every 20 ms and appends each new `stty
/// size` of its terminal to `$HOME/<title>.sizes`.
fn ticking(title: &str) -> String {
    working(&format!(
        "printf '\\033]2;{title}\\007'; p=; i=0; \
         until [ -e \"$HOME/finish\" ] || [ \"$SECONDS\" -ge 120 ]; do \
         s=$(stty size); if [ \"$s\" != \"$p\" ]; then echo \"$s\" >> \"$HOME/{title}.sizes\"; p=$s; fi; \
         i=$((i+1)); printf 'tick %d\\r\\n' \"$i\"; sleep 0.02; done"
    ))
}

/// Resizes the host to `rows` by `cols`, then waits until a whole frame
/// meets `done`, checking that every whole frame the monitor paints for
/// the new size is coherent: `Terminal too small` alone, or the `Tick`
/// session's top row over a canvas showing its output, never a blank or
/// partial one. Until the monitor has painted one frame laid out for the
/// new size (its top row too small, or the session's top row at the new
/// width), the host shows its own reflow of earlier frames and whatever a
/// frame painted for the old size makes of it, which is not judged.
fn resized_coherently(
    terminal: &mut Terminal,
    (rows, cols): (u16, u16),
    what: &str,
    done: impl Fn(&str) -> bool,
) {
    let laid_out: Vec<_> = ["Acquiring", "Restoring", CONTROLLING]
        .into_iter()
        .map(|control| {
            session_top_row(
                usize::from(cols),
                "Tick",
                &[RUNNING, control, SESSIONS_HINT],
            )
        })
        .collect();
    let started = std::cell::Cell::new(false);
    terminal.resize(rows, cols).unwrap();
    whole_frame_shows(terminal, what, |screen| {
        let mut lines = screen.lines();
        let top = lines.next().unwrap_or_default();
        let canvas: Vec<_> = lines.collect();
        let small = top.starts_with("Terminal too small");
        started.set(started.get() || small || laid_out.iter().any(|row| row == top));
        if !started.get() {
            return false;
        }
        let blank = canvas.iter().all(|row| row.trim().is_empty());
        let painted = top.starts_with("Tick") && canvas.iter().any(|row| row.starts_with("tick "));
        assert!(
            (small && blank) || painted,
            "an incoherent frame:\n{screen}"
        );
        done(screen)
    });
}

/// TRM-04 (Monitor): while the session prints continuously, host resizes
/// to a narrower host, to one too small and back each show only coherent
/// frames: the last frame clipped or padded until the restoration at the
/// new size is applied, `Terminal too small` alone while too small, and
/// the session at its new size after; never a blank or partial canvas. The
/// session is resized to each size the monitor sent, and never to one
/// while too small.
#[test]
fn resizes_under_continuous_output_show_only_coherent_frames() {
    let package = Package::new();
    without_introduction(&package);
    headless(&package, &ticking("Tick"), "finish");
    let mut terminal = outer(&package, "c2 monitor");
    let wide = controlled("Tick", RUNNING);
    whole_frame_shows(&terminal, "the first ticks", |screen| {
        screen.lines().next() == Some(wide.as_str()) && screen.contains("\ntick ")
    });
    let narrow = session_top_row(100, "Tick", &[RUNNING, CONTROLLING, SESSIONS_HINT]);
    resized_coherently(&mut terminal, (ROWS, 100), "the narrower host", |screen| {
        screen.lines().next() == Some(narrow.as_str())
    });
    sized_to(&package, "Tick", &["24 80", "29 120", "29 100"]);
    resized_coherently(&mut terminal, (3, 100), "a host too small", |screen| {
        screen.starts_with("Terminal too small")
    });
    resized_coherently(
        &mut terminal,
        (ROWS, COLS),
        "the recovered host",
        |screen| screen.lines().next() == Some(wide.as_str()),
    );
    sized_to(&package, "Tick", &["24 80", "29 120", "29 100", "29 120"]);
    terminal.send("\x1dx").unwrap();
    support::assert_released(&package, &mut terminal, 0);
    assert_eq!(terminal.title().as_deref(), Some(TITLE));
    finish_sessions(&package, "finish");
    package.assert_clean();
}
