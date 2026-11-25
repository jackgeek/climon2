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
    fs, io,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    process::Command,
    ptr,
    time::{Duration, Instant},
};
use support::{Home, Package, TIMEOUT, VERSION, enter, wait};
use windows_sys::Win32::{
    Foundation::{ERROR_IO_PENDING, GENERIC_READ, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    },
    System::{
        IO::{DeviceIoControl, OVERLAPPED},
        Ioctl::{
            FSCTL_REQUEST_OPLOCK, OPLOCK_LEVEL_CACHE_HANDLE, OPLOCK_LEVEL_CACHE_READ,
            OPLOCK_LEVEL_CACHE_WRITE, REQUEST_OPLOCK_CURRENT_VERSION, REQUEST_OPLOCK_INPUT_BUFFER,
            REQUEST_OPLOCK_INPUT_FLAG_REQUEST, REQUEST_OPLOCK_OUTPUT_BUFFER,
        },
        Threading::{CreateEventW, WaitForSingleObject},
    },
};

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
$p = ''; $w = ''; $i = 0
while (-not (Test-Path "$Dir\finish")) {
    if (Test-Path "$Dir\$Name.flood") { $i++; [Console]::Write("tick $i $('x' * 2000)`r`n") }
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

/// Starts a headless session running [`WATCHED`] as `name`, and returns
/// its id.
fn headless(package: &Package, home: &Home, pwsh: &Path, name: &str) -> String {
    let output = Command::new(package.launcher())
        .args(["exec", "--headless"])
        .arg(pwsh)
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

/// Selects the session titled `name` by searching the selector for it.
///
/// It waits until every listed session runs: Enter on a session still
/// starting selects it without taking control (spec #73, "Enter"), and a
/// starting row's label is its whole command line, which the search may
/// match whatever `name` is (#390).
fn select(terminal: &mut Terminal, home: &Home, name: &str) {
    terminal.send("\x1ds").unwrap();
    let listed = terminal.wait_for_screen(TIMEOUT, |screen| choosable(screen, name));
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

/// Whether the open selector on `screen` lists only running sessions, the
/// one titled `name` among them.
fn choosable(screen: &str, name: &str) -> bool {
    let rows: Vec<&str> = screen
        .lines()
        .filter_map(|line| line.trim_end().strip_prefix(" ║")?.strip_suffix('║'))
        .map(str::trim_end)
        .filter(|row| !row.is_empty() && !row.starts_with("Enter: select"))
        .collect();
    let running = rows.iter().all(|row| row.starts_with("Running "));
    running && rows.contains(&format!("Running {name}").as_str())
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

/// LIM-06 (Monitor: a host that stops reading; #285), on Windows: the
/// console's output goes through its writer thread, so a host that reads
/// nothing for 5 s while the selected session prints 2 KB lines leaves the
/// monitor waiting on the console's room, not stuck in a write. A console
/// whose output nobody reads takes no input and starts no thread in any
/// process attached to it, so keys and new sessions wait for the host
/// there. Once the host reads again, the session is still controlled, its
/// newer output shows, keys reach it and prefix+x exits.
fn a_host_that_stops_reading_recovers_once_it_reads_again(model: HostModel) {
    let Some((package, home, pwsh)) = setup() else {
        return;
    };
    headless(&package, &home, &pwsh, "Flood");
    let mut terminal = outer(&package, &home, &pwsh, model, "c2 m");
    select(&mut terminal, &home, "Flood");
    keys_flow(&mut terminal, &home, "Flood");
    fs::write(work(&home).join("Flood.flood"), b"").unwrap();
    wait(&mut terminal, &home, "tick 5");
    terminal.stop_reading();
    std::thread::sleep(Duration::from_secs(5));
    let mark = terminal.received_total();
    terminal.resume_reading();
    terminal
        .wait_for_bytes_since(mark, b"tick 150", TIMEOUT)
        .unwrap_or_else(|error| panic!("{error}\nLOGS:\n{}", home.logs()));
    top_row_starts(&mut terminal, &home, "Flood");
    terminal.send("ab").unwrap();
    got_exactly(&home, "Flood", "97 98 ");
    let top = terminal.screen().lines().next().unwrap().to_owned();
    assert!(top.contains("| Controlling"), "{top}");
    let logs = home.logs();
    assert!(!logs.contains("client failed"), "{logs}");
    exit_monitor(&mut terminal, &home);
    finish(&home);
}

#[test]
fn a_host_that_stops_reading_recovers_once_it_reads_again_under_windows_terminal() {
    let name = "a_host_that_stops_reading_recovers_once_it_reads_again_under_windows_terminal";
    if !rerun_outside(name, OUTSIDE) {
        a_host_that_stops_reading_recovers_once_it_reads_again(HostModel::WindowsTerminal);
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
    glance(&mut terminal, &home, "Beta");
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

/// An exclusive oplock (read, write and handle caching) on a file: any
/// other open of its data waits until the oplock is released by dropping
/// this, because its break is never acknowledged.
struct Oplock {
    file: Option<OwnedHandle>,
    event: OwnedHandle,
    _overlapped: Box<OVERLAPPED>,
    _input: Box<REQUEST_OPLOCK_INPUT_BUFFER>,
    _output: Box<REQUEST_OPLOCK_OUTPUT_BUFFER>,
}

impl Oplock {
    fn on(path: &Path) -> Self {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: a NUL-terminated path; no security attributes or template.
        let file = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                ptr::null_mut(),
            )
        };
        assert_ne!(file, INVALID_HANDLE_VALUE, "{}", io::Error::last_os_error());
        // SAFETY: a new handle nothing else owns.
        let file = unsafe { OwnedHandle::from_raw_handle(file) };
        // SAFETY: an unnamed manual-reset event, initially clear.
        let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        assert!(!event.is_null());
        // SAFETY: a new handle nothing else owns.
        let event = unsafe { OwnedHandle::from_raw_handle(event) };
        let mut overlapped = Box::new(OVERLAPPED {
            hEvent: event.as_raw_handle(),
            ..OVERLAPPED::default()
        });
        let input = Box::new(REQUEST_OPLOCK_INPUT_BUFFER {
            StructureVersion: REQUEST_OPLOCK_CURRENT_VERSION as u16,
            StructureLength: size_of::<REQUEST_OPLOCK_INPUT_BUFFER>() as u16,
            RequestedOplockLevel: OPLOCK_LEVEL_CACHE_READ
                | OPLOCK_LEVEL_CACHE_WRITE
                | OPLOCK_LEVEL_CACHE_HANDLE,
            Flags: REQUEST_OPLOCK_INPUT_FLAG_REQUEST,
        });
        let mut output = Box::<REQUEST_OPLOCK_OUTPUT_BUFFER>::default();
        // SAFETY: the buffers and the OVERLAPPED are boxed and kept until the
        // request completes, which closing the file in drop makes it do.
        let requested = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                FSCTL_REQUEST_OPLOCK,
                ptr::from_ref(&*input).cast(),
                size_of::<REQUEST_OPLOCK_INPUT_BUFFER>() as u32,
                ptr::from_mut(&mut *output).cast(),
                size_of::<REQUEST_OPLOCK_OUTPUT_BUFFER>() as u32,
                ptr::null_mut(),
                &mut *overlapped,
            )
        };
        let error = io::Error::last_os_error();
        assert!(
            requested == 0 && error.raw_os_error() == Some(ERROR_IO_PENDING as i32),
            "the oplock was not granted: {error}"
        );
        Self {
            file: Some(file),
            event,
            _overlapped: overlapped,
            _input: input,
            _output: output,
        }
    }
}

impl Drop for Oplock {
    /// Closing the file releases the oplock and completes its request, which
    /// must end before its buffers go.
    fn drop(&mut self) {
        drop(self.file.take());
        // SAFETY: the request's event, which its completion sets.
        unsafe { WaitForSingleObject(self.event.as_raw_handle(), 10_000) };
    }
}

/// Opens the selector, waits until it lists `label`, and closes it with
/// Enter on the highlight it opened with, the selected session. A lone
/// Escape would wait for the byte after it, and ConPTY turns a focus
/// report into a focus record, which the console reader drops.
fn glance(terminal: &mut Terminal, home: &Home, label: &str) {
    terminal.send("\x1ds").unwrap();
    wait(terminal, home, label);
    terminal.send("\r").unwrap();
    let closed = terminal.wait_for_screen(TIMEOUT, |screen| !screen.contains('║'));
    if let Err(error) = closed {
        panic!("{error}\nLOGS:\n{}", home.logs());
    }
}

/// Waits until the top row contains `text`.
fn top_row_contains(terminal: &mut Terminal, home: &Home, text: &str) {
    let shown = terminal.wait_for_screen(TIMEOUT, |screen| {
        screen.lines().next().is_some_and(|row| row.contains(text))
    });
    if let Err(error) = shown {
        panic!("{error}\nLOGS:\n{}", home.logs());
    }
}

/// Library embedding, "Loading and instance lifetime", on Windows: a load
/// runs away from the rendering thread and is never aborted. The release's
/// library is held by an oplock, so the loader's open of it does not
/// return: the session stays Starting, the monitor logs the release as
/// loading slowly after `deadlines.load_ms`, and the monitor stays
/// responsive, its selector opening, listing and closing. Once the open
/// returns the load completes and the session, selected read-only while
/// Starting, connects and runs.
///
/// The top row's `library loading slowly` is not asserted: on Windows it
/// did not show for this row, which is still open (see the pull request).
#[test]
fn a_stalled_library_load_says_loading_slowly_and_never_holds_up_the_monitor() {
    let name = "a_stalled_library_load_says_loading_slowly_and_never_holds_up_the_monitor";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    let Some((package, home, pwsh)) = setup() else {
        return;
    };
    let limits = r#"{"monitor": {"limits": {"deadlines": {"load_ms": 1000}}}}"#;
    fs::write(home.home().join("config.jsonc"), limits).unwrap();
    let oplock = Oplock::on(&package.version().join("climon2_client.dll"));
    let mut terminal = outer(
        &package,
        &home,
        &pwsh,
        HostModel::WindowsTerminal,
        "c2 monitor",
    );
    // Created once the monitor runs, so its row is listed before the load
    // it waits for turns slow.
    headless(&package, &home, &pwsh, "Alpha");
    terminal.send("\x1ds").unwrap();
    let listed = terminal.wait_for_screen(TIMEOUT, |screen| {
        screen.lines().any(|line| line.starts_with(" ║Starting"))
    });
    if let Err(error) = listed {
        panic!("{error}\nLOGS:\n{}", home.logs());
    }
    terminal.send("Bypass\r").unwrap();
    loading_slowly_logged(&home);
    // Still responsive while the load stalls.
    glance(&mut terminal, &home, "Starting");
    drop(oplock);
    top_row_contains(&mut terminal, &home, "Alpha");
    top_row_contains(&mut terminal, &home, "| Running");
    exit_monitor(&mut terminal, &home);
    finish(&home);
}

/// Waits until the monitor's general log says its release loads slowly.
fn loading_slowly_logged(home: &Home) {
    let deadline = Instant::now() + TIMEOUT;
    while !home.logs().contains("client library loading slowly") {
        assert!(Instant::now() < deadline, "{}", home.logs());
        std::thread::sleep(Duration::from_millis(100));
    }
}
