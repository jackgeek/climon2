//! #327: the monitor on Windows (docs/for-humans/windows/qualification.md,
//! "Monitor and client-library scenarios"), reached as `c2 monitor` or
//! `c2 m` through the installed launcher from `pwsh`. The CLI starts the
//! monitor and waits for it by the hand-off rules, and the monitor keeps the
//! client's console rules for raw mode, resize and close.
//!
//! The terminal-facing scenarios (rendering, input routing, resize, the
//! selector, toasts, the bell and attention reports) run under both host
//! models, the rest under the Windows Terminal model. Each session runs
//! [`WATCHED`] in the controlled `pwsh`.
//!
//! cargo runs tests in a job that refuses breakaway, and so does the CI
//! runner, so each test that starts sessions runs again outside every job
//! (`climon2_test_term::rerun_outside`).
#![cfg(windows)]
// Each test binary uses part of it.
#[allow(dead_code)]
mod support;

use climon2_platform::process;
use climon2_test_term::{
    HostModel, HostSettings, HostSignal, JobShape, Options, Terminal, controlled_pwsh,
    rerun_outside,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use support::{Home, Package, TIMEOUT, VERSION, enter, wait};

climon2_test_term::outside_relay!();

/// How long one whole test may take outside every job.
const OUTSIDE: Duration = Duration::from_secs(300);
const ROWS: u16 = 30;
const COLS: u16 = 120;

/// The preference file a release writes once Don't show again is checked.
const SUPPRESSED: &str = "{\"version\":1,\"suppress_key_introduction\":true}\n";

/// A session program: it reports `working`, titles itself `$Name`, says
/// `$Name READY`, shows its window size on row 2, reports whatever
/// `$Dir\$Name.state` says, and keeps each typed character's code in
/// `$Dir\$Name.got`, until `$Dir\finish` exists. Ctrl+C is input to it.
const WATCHED: &str = r#"param($Name, $Dir)
$e = [char]27; $b = [char]7
[Console]::TreatControlCAsInput = $true
[Console]::Write("$e]7501;state=working$e\")
[Console]::Write("$e]2;$Name$b")
[Console]::Write("$Name READY`r`n")
$p = ''; $w = ''
while (-not (Test-Path "$Dir\finish")) {
    $s = "$([Console]::WindowHeight) $([Console]::WindowWidth)"
    if ($s -ne $p) { [Console]::Write("$e" + "7$e[2;1HSIZE $s$e[K$e" + "8"); $p = $s }
    $r = $null; if (Test-Path "$Dir\$Name.state") { $r = [IO.File]::ReadAllText("$Dir\$Name.state") }
    if ($r -and $r -ne $w) { [Console]::Write("$e]7501;state=$r$e\"); $w = $r }
    while ([Console]::KeyAvailable) {
        $k = [Console]::ReadKey($true)
        [IO.File]::AppendAllText("$Dir\$Name.got", "$([int]$k.KeyChar) ")
    }
    Start-Sleep -Milliseconds 50
}
"#;

/// The monitor's root OSC 7501 reports (docs/for-humans/protocol/monitor-host).
const WORKING: &[u8] = b"\x1b]7501;state=working:app=climon2-monitor\x1b\\";
const BLOCKED: &[u8] = b"\x1b]7501;state=blocked:kind=question:app=climon2-monitor\x1b\\";
const CLEAR: &[u8] = b"\x1b]7501;state=clear\x1b\\";

/// The host model's settings, with no job around the shell.
fn hosted(model: HostModel) -> HostSettings {
    HostSettings {
        model,
        job: JobShape::None,
    }
}

/// A product, a home without the key introduction, and the controlled
/// `pwsh`; `None` where `pwsh` is missing locally.
fn setup() -> Option<(Package, Home, PathBuf)> {
    let pwsh = controlled_pwsh()?;
    let (package, home) = (Package::new(), Home::new());
    fs::create_dir_all(home.home()).unwrap();
    fs::write(home.home().join("monitor-preferences.json"), SUPPRESSED).unwrap();
    fs::create_dir_all(work(&home)).unwrap();
    fs::write(work(&home).join("watched.ps1"), WATCHED).unwrap();
    Some((package, home, pwsh))
}

/// Where the session programs keep their files.
fn work(home: &Home) -> PathBuf {
    home.0.path().join("work")
}

/// The shell the sessions run: the controlled `pwsh`, unless
/// `CLIMON2_TEST_SESSION_SHELL` names another. A machine whose only
/// PowerShell 7 is the Store build needs that: Windows refuses to start a
/// Store app inside a session's job.
fn session_shell(pwsh: &Path) -> PathBuf {
    std::env::var_os("CLIMON2_TEST_SESSION_SHELL").map_or_else(|| pwsh.to_owned(), PathBuf::from)
}

/// Starts a headless session running [`WATCHED`] as `name`, and returns
/// its id.
fn headless(package: &Package, home: &Home, pwsh: &Path, name: &str) -> String {
    let output = Command::new(package.launcher())
        .args(["exec", "--headless"])
        .arg(session_shell(pwsh))
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(work(home).join("watched.ps1"))
        .arg(name)
        .arg(work(home))
        .env("CLIMON2_HOME", home.home())
        .env("LOCALAPPDATA", home.local())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}\n{}", home.logs());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Makes `name`'s program report `state` next.
fn report_state(home: &Home, name: &str, state: &str) {
    let staged = work(home).join(format!("{name}.state.new"));
    fs::write(&staged, state).unwrap();
    fs::rename(staged, work(home).join(format!("{name}.state"))).unwrap();
}

/// The codes of the characters `name`'s program has read so far, after the
/// probes [`keys_flow`] typed.
fn got(home: &Home, name: &str) -> String {
    let codes = fs::read_to_string(work(home).join(format!("{name}.got"))).unwrap_or_default();
    let mut rest = codes.as_str();
    while let Some(after) = rest.strip_prefix(PROBE_CODE) {
        rest = after;
    }
    rest.to_owned()
}

/// What [`keys_flow`] types, and its code as the session program keeps it.
const PROBE: &str = "~";
const PROBE_CODE: &str = "126 ";

/// Waits until keys typed into the monitor reach `name`'s program, by
/// typing a probe until it arrives: keys typed while the monitor is still
/// restoring a session's screen reach no session, as the monitor
/// specification says.
fn keys_flow(terminal: &mut Terminal, home: &Home, name: &str) {
    let path = work(home).join(format!("{name}.got"));
    let deadline = Instant::now() + TIMEOUT;
    while !fs::read_to_string(&path).is_ok_and(|codes| codes.starts_with(PROBE_CODE)) {
        assert!(
            Instant::now() < deadline,
            "no key reached {name}\n{}",
            home.logs()
        );
        terminal.send(PROBE).unwrap();
        std::thread::sleep(Duration::from_millis(300));
    }
    // A probe typed last may still be on its way.
    std::thread::sleep(Duration::from_millis(500));
}

/// Waits until `name`'s program has read exactly `codes` after the probes.
fn got_exactly(home: &Home, name: &str, codes: &str) {
    let deadline = Instant::now() + TIMEOUT;
    while got(home, name) != codes && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(got(home, name), codes, "{name}\n{}", home.logs());
}

/// The outer `pwsh` under `model`, which marks where the monitor's output
/// begins and runs `command`, then says what it exited with.
fn outer(package: &Package, home: &Home, pwsh: &Path, model: HostModel, command: &str) -> Terminal {
    let shell = home.pwsh(package, pwsh, "OUTER");
    let mut terminal =
        Terminal::spawn_hosted(shell, ROWS, COLS, Options::default(), hosted(model)).unwrap();
    wait(&mut terminal, home, "OUTER> ");
    enter(
        &mut terminal,
        &format!("'BEFORE-' + 'MONITOR'; {command}; \"OUTER-STATUS=$LASTEXITCODE\""),
    );
    // Keys typed before the monitor owns the console would reach the shell.
    let opened = terminal.wait_for_screen(TIMEOUT, |screen| {
        screen.lines().next().is_some_and(|row| {
            row.starts_with("No session selected") || row.trim_end().ends_with("s: sessions")
        })
    });
    if let Err(error) = opened {
        panic!("{error}\nLOGS:\n{}", home.logs());
    }
    terminal
}

/// Waits until the top row starts with `start`.
fn top_row_starts(terminal: &mut Terminal, home: &Home, start: &str) {
    let shown = terminal.wait_for_screen(TIMEOUT, |screen| {
        screen
            .lines()
            .next()
            .is_some_and(|row| row.starts_with(start))
    });
    if let Err(error) = shown {
        panic!("{error}\nLOGS:\n{}", home.logs());
    }
}

/// Selects the session labelled `name` by searching the selector for it.
fn select(terminal: &mut Terminal, home: &Home, name: &str) {
    terminal.send("\x1ds").unwrap();
    let listed = terminal.wait_for_screen(TIMEOUT, |screen| {
        screen
            .lines()
            .any(|line| line.starts_with(" ║") && line.contains(name))
    });
    if let Err(error) = listed {
        panic!("{error}\nLOGS:\n{}", home.logs());
    }
    terminal.send(format!("{name}\r")).unwrap();
    // Keys reach a session only once the monitor controls it.
    top_row_starts(terminal, home, name);
    let controlling = terminal.wait_for_screen(TIMEOUT, |screen| {
        screen
            .lines()
            .next()
            .is_some_and(|row| row.contains("| Controlling"))
    });
    if let Err(error) = controlling {
        panic!("{error}\nLOGS:\n{}", home.logs());
    }
}

/// Exits the monitor with prefix+x and waits until the outer shell says its
/// status was 0, with its own earlier output on the screen again.
fn exit_monitor(terminal: &mut Terminal, home: &Home) {
    terminal.send("\x1dx").unwrap();
    wait(terminal, home, "OUTER-STATUS=0");
    assert!(
        terminal.screen().contains("BEFORE-MONITOR"),
        "{}",
        terminal.screen()
    );
}

/// Whether the session `id`'s daemon still runs.
fn running(home: &Home, id: &str) -> bool {
    home.sessions().iter().any(|session| {
        session.session_id.as_str() == id && process::is_same_daemon(session).unwrap_or(false)
    })
}

/// Ends every session program; the daemons end with them.
fn finish(home: &Home) {
    fs::write(work(home).join("finish"), b"").unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while !home.sessions().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Spec #73 Entry and startup, on Windows: without console handles for
/// stdin and stdout the monitor refuses to start with the climon2 error
/// code, and a missing monitor executable fails naming its version, with
/// the logs, before anything starts.
#[test]
fn the_monitor_needs_a_console_and_a_missing_one_names_its_version() {
    let (package, home) = (Package::new(), Home::new());
    let monitor = |arguments: &[&str]| {
        Command::new(package.launcher())
            .args(arguments)
            .env("CLIMON2_HOME", home.home())
            .env("LOCALAPPDATA", home.local())
            .output()
            .unwrap()
    };
    for shortcut in ["monitor", "m"] {
        let refused = monitor(&[shortcut]);
        assert_eq!(refused.status.code(), Some(125), "{refused:?}");
        assert!(refused.stdout.is_empty(), "{refused:?}");
        assert_eq!(
            String::from_utf8_lossy(&refused.stderr),
            "climon2: error: c2 monitor needs a terminal\n"
        );
    }
    fs::remove_file(package.version().join("climon2-monitor.exe")).unwrap();
    let missing = monitor(&["monitor"]);
    assert_eq!(missing.status.code(), Some(125), "{missing:?}");
    let diagnostic = String::from_utf8_lossy(&missing.stderr);
    assert!(
        diagnostic.starts_with(&format!(
            "climon2: error: cannot start the climon2 {VERSION} monitor: "
        )),
        "{diagnostic}"
    );
    assert!(diagnostic.contains(" (logs: "), "{diagnostic}");
}

/// WINM-13, WINM-14 and WINM-16's exit (Monitor, terminal-facing): with two
/// sessions running, `c2 m` opens the monitor and lists both; the selector
/// switches between them, and typed keys, Ctrl+C among them as 0x03, reach
/// only the selected session. Exiting with prefix+x gives the shell back,
/// its earlier output restored and its console usable, with status 0, and
/// leaves both sessions running.
fn opens_lists_switches_and_types_only_into_the_selected_session(model: HostModel) {
    let Some((package, home, pwsh)) = setup() else {
        return;
    };
    let alpha = headless(&package, &home, &pwsh, "Alpha");
    let beta = headless(&package, &home, &pwsh, "Beta");
    let mut terminal = outer(&package, &home, &pwsh, model, "c2 m");
    select(&mut terminal, &home, "Alpha");
    keys_flow(&mut terminal, &home, "Alpha");
    terminal.send("ab\x03").unwrap();
    got_exactly(&home, "Alpha", "97 98 3 ");
    select(&mut terminal, &home, "Beta");
    keys_flow(&mut terminal, &home, "Beta");
    terminal.send("cd").unwrap();
    got_exactly(&home, "Beta", "99 100 ");
    assert_eq!(got(&home, "Alpha"), "97 98 3 ");

    exit_monitor(&mut terminal, &home);
    enter(&mut terminal, "'AFTER-' + 'EXIT'");
    wait(&mut terminal, &home, "AFTER-EXIT");
    assert!(running(&home, &alpha) && running(&home, &beta));
    enter(&mut terminal, "c2 list");
    wait(&mut terminal, &home, &beta[..8]);
    wait(&mut terminal, &home, &alpha[..8]);
    finish(&home);
}

#[test]
fn opens_lists_switches_and_types_only_into_the_selected_session_under_windows_terminal() {
    let name =
        "opens_lists_switches_and_types_only_into_the_selected_session_under_windows_terminal";
    if !rerun_outside(name, OUTSIDE) {
        opens_lists_switches_and_types_only_into_the_selected_session(HostModel::WindowsTerminal);
    }
}

#[test]
fn opens_lists_switches_and_types_only_into_the_selected_session_under_vs_code() {
    let name = "opens_lists_switches_and_types_only_into_the_selected_session_under_vs_code";
    if !rerun_outside(name, OUTSIDE) {
        opens_lists_switches_and_types_only_into_the_selected_session(HostModel::VsCode);
    }
}

/// WINM-16's resize (Monitor, terminal-facing): resizing the host redraws
/// the monitor at the new size, and the selected session is resized to the
/// rows below the top row.
fn resizing_redraws_the_monitor_and_resizes_the_selected_session(model: HostModel) {
    let Some((package, home, pwsh)) = setup() else {
        return;
    };
    headless(&package, &home, &pwsh, "Alpha");
    let mut terminal = outer(&package, &home, &pwsh, model, "c2 monitor");
    select(&mut terminal, &home, "Alpha");
    wait(&mut terminal, &home, &format!("SIZE {} {COLS}", ROWS - 1));
    terminal.resize(24, 100).unwrap();
    wait(&mut terminal, &home, "SIZE 23 100");
    terminal
        .wait_for_screen(TIMEOUT, |screen| {
            screen
                .lines()
                .next()
                .is_some_and(|row| row.starts_with("Alpha") && row.trim_end().ends_with("sessions"))
        })
        .unwrap();
    assert!(
        terminal
            .screen()
            .lines()
            .all(|line| line.chars().count() == 100)
    );
    exit_monitor(&mut terminal, &home);
    finish(&home);
}

#[test]
fn resizing_redraws_the_monitor_and_resizes_the_selected_session_under_windows_terminal() {
    let name =
        "resizing_redraws_the_monitor_and_resizes_the_selected_session_under_windows_terminal";
    if !rerun_outside(name, OUTSIDE) {
        resizing_redraws_the_monitor_and_resizes_the_selected_session(HostModel::WindowsTerminal);
    }
}

#[test]
fn resizing_redraws_the_monitor_and_resizes_the_selected_session_under_vs_code() {
    let name = "resizing_redraws_the_monitor_and_resizes_the_selected_session_under_vs_code";
    if !rerun_outside(name, OUTSIDE) {
        resizing_redraws_the_monitor_and_resizes_the_selected_session(HostModel::VsCode);
    }
}

/// WINM-15 and NOT-04 (Monitor, terminal-facing): a background session that
/// needs attention is toasted, with one bell, and the host hears the
/// monitor's own attention report, never the session's; prefix+j jumps to
/// it and acknowledges it. On exit the host hears the root clear last.
fn attention_toasts_ring_the_bell_tell_the_host_and_prefix_j_jumps(model: HostModel) {
    let Some((package, home, pwsh)) = setup() else {
        return;
    };
    headless(&package, &home, &pwsh, "Alpha");
    let mut terminal = outer(&package, &home, &pwsh, model, "c2 monitor");
    select(&mut terminal, &home, "Alpha");
    headless(&package, &home, &pwsh, "Beta");
    terminal.send("\x1ds").unwrap();
    wait(&mut terminal, &home, "Beta");
    terminal.send("\x1b\x1b[I").unwrap();
    report_state(&home, "Beta", "idle");
    let toast = terminal.wait_for_screen(TIMEOUT, |screen| {
        screen
            .lines()
            .any(|line| line.contains('│') && line.contains("Beta"))
    });
    if let Err(error) = toast {
        panic!("{error}\nLOGS:\n{}", home.logs());
    }
    terminal
        .wait_for_host_signals(TIMEOUT, |signals| {
            signals.contains(&HostSignal::Bell)
                && signals.contains(&HostSignal::Osc7501(BLOCKED.to_vec()))
        })
        .unwrap();
    terminal.send("\x1dj").unwrap();
    top_row_starts(&mut terminal, &home, "Beta");
    terminal
        .wait_for_screen(TIMEOUT, |screen| {
            screen
                .lines()
                .next()
                .is_some_and(|row| row.contains("Acknowledged"))
        })
        .unwrap();
    exit_monitor(&mut terminal, &home);
    let mut signals = terminal.host_signals();
    signals.retain(|signal| *signal != HostSignal::Bell);
    assert_eq!(signals.last(), Some(&HostSignal::Osc7501(CLEAR.to_vec())));
    let reports = [WORKING, BLOCKED, CLEAR].map(|report| HostSignal::Osc7501(report.to_vec()));
    assert!(
        signals.iter().all(|signal| reports.contains(signal)),
        "{signals:?}"
    );
    let bells = terminal
        .host_signals()
        .iter()
        .filter(|signal| **signal == HostSignal::Bell)
        .count();
    assert_eq!(bells, 1);
    finish(&home);
}

#[test]
fn attention_toasts_ring_the_bell_tell_the_host_and_prefix_j_jumps_under_windows_terminal() {
    let name =
        "attention_toasts_ring_the_bell_tell_the_host_and_prefix_j_jumps_under_windows_terminal";
    if !rerun_outside(name, OUTSIDE) {
        attention_toasts_ring_the_bell_tell_the_host_and_prefix_j_jumps(HostModel::WindowsTerminal);
    }
}

#[test]
fn attention_toasts_ring_the_bell_tell_the_host_and_prefix_j_jumps_under_vs_code() {
    let name = "attention_toasts_ring_the_bell_tell_the_host_and_prefix_j_jumps_under_vs_code";
    if !rerun_outside(name, OUTSIDE) {
        attention_toasts_ring_the_bell_tell_the_host_and_prefix_j_jumps(HostModel::VsCode);
    }
}

/// Lifecycle "Client console" for the monitor: closing the host's console
/// ends the monitor, and the launcher and CLI waiting for it, and leaves
/// every session running. Close events arrive in different orders under
/// real hosts, so only the outcome is checked.
fn closing_the_console_ends_the_monitor_and_leaves_sessions_running(model: HostModel) {
    let Some((package, home, pwsh)) = setup() else {
        return;
    };
    let alpha = headless(&package, &home, &pwsh, "Alpha");
    let mut terminal = outer(&package, &home, &pwsh, model, "c2 monitor");
    select(&mut terminal, &home, "Alpha");
    terminal.close();
    terminal.wait_exit(TIMEOUT).unwrap();
    let monitors = || {
        Command::new("tasklist")
            .args([
                "/fi",
                "imagename eq climon2-monitor.exe",
                "/fo",
                "csv",
                "/nh",
            ])
            .output()
            .unwrap()
    };
    let deadline = Instant::now() + TIMEOUT;
    let folder = package.version();
    // Windows refuses to delete a running executable.
    while fs::remove_file(folder.join("climon2-monitor.exe")).is_err() && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        !folder.join("climon2-monitor.exe").exists(),
        "{:?}",
        monitors()
    );
    assert!(running(&home, &alpha));
    finish(&home);
}

#[test]
fn closing_the_console_ends_the_monitor_and_leaves_sessions_running_under_windows_terminal() {
    let name =
        "closing_the_console_ends_the_monitor_and_leaves_sessions_running_under_windows_terminal";
    if !rerun_outside(name, OUTSIDE) {
        closing_the_console_ends_the_monitor_and_leaves_sessions_running(
            HostModel::WindowsTerminal,
        );
    }
}

#[test]
fn closing_the_console_ends_the_monitor_and_leaves_sessions_running_under_vs_code() {
    let name = "closing_the_console_ends_the_monitor_and_leaves_sessions_running_under_vs_code";
    if !rerun_outside(name, OUTSIDE) {
        closing_the_console_ends_the_monitor_and_leaves_sessions_running(HostModel::VsCode);
    }
}
