//! #325: keys, focus, resize, start-up negotiation and the console preflight
//! under both qualified host models, and the VS Code model's subset
//! (docs/for-humans/windows/qualification.md, "Coverage" and
//! "Windows-only scenarios"): the packaged product, driven from `pwsh`.
//!
//! - `WIN-04`: one key table, sent in each model's encoding, arrives in the
//!   session as the expected bytes.
//! - Host focus reports reach a session that asked for them (`CSI ? 1004 h`),
//!   and only then, under each model.
//! - `WIN-08`: ConPTY's own start-up sequences never reach the host, and a
//!   resize reaches the session, under each model.
//! - `WIN-05`: `exec` and `attach` refuse a redirected stdin or stdout.
//! - The VS Code model's close and hang-up, attach and restoration.
//!
//! `WIN-06` is `exec_windows.rs`'s missing ConPTY files test.
//!
//! The session program that shows what arrives is [`READER`]: it reads its
//! console in VT input mode, as a full-screen program does, and keeps every
//! byte it reads.
//!
//! cargo runs tests in a job that refuses breakaway, and so does the CI
//! runner, so each test runs again outside every job
//! (`climon2_test_term::rerun_outside`).
#![cfg(windows)]
// Each test binary uses part of it.
#[allow(dead_code)]
mod support;

use climon2_platform::process;
use climon2_test_term::{
    HostModel, HostSettings, JobShape, Key, Modifiers, Options, Terminal, controlled_pwsh,
    rerun_outside,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use support::{Home, Package, TIMEOUT, enter, pwsh_arguments, quoted, wait};

climon2_test_term::outside_relay!();

/// How long one whole test may take outside every job.
const OUTSIDE: Duration = Duration::from_secs(300);
const ROWS: u16 = 30;
const COLS: u16 = 120;
/// The escape prefix, Ctrl+], then `d`.
const DETACH: &[u8] = b"\x1dd";

/// A session program: it puts its console input in VT input mode alone, so
/// keys arrive as the bytes a terminal sends, says `READING`, and appends
/// each byte it reads to `$Out` as two hex digits and a space. Reading `z`
/// turns focus reporting on, and it says `FOCUSING`.
const READER: &str = r#"param($Out)
$signatures = '[DllImport("kernel32.dll")] public static extern System.IntPtr GetStdHandle(int n);
[DllImport("kernel32.dll")] public static extern bool SetConsoleMode(System.IntPtr h, uint m);
[DllImport("kernel32.dll")] public static extern bool ReadFile(System.IntPtr h, byte[] b, uint n, out uint r, System.IntPtr o);'
Add-Type -Name Console -Namespace Climon2 -MemberDefinition $signatures
$in = [Climon2.Console]::GetStdHandle(-10)
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
# ENABLE_VIRTUAL_TERMINAL_INPUT, and nothing processed, echoed or by line.
[void][Climon2.Console]::SetConsoleMode($in, 0x200)
[Console]::Write("READ" + "ING`r`n")
$buffer = [byte[]]::new(256)
while ($true) {
    $read = 0
    if (-not [Climon2.Console]::ReadFile($in, $buffer, 256, [ref]$read, [IntPtr]::Zero) -or $read -eq 0) { break }
    $bytes = $buffer[0..($read - 1)]
    [IO.File]::AppendAllText($Out, -join ($bytes | ForEach-Object { $_.ToString('x2') + ' ' }))
    if ($bytes -contains 0x7A) { [Console]::Write("$([char]27)[?1004hFOCUS" + "ING`r`n") }
}
"#;

/// The host model's settings, with no job around the shell.
fn hosted(model: HostModel) -> HostSettings {
    HostSettings {
        model,
        job: JobShape::None,
    }
}

/// A product, a home, its work folder holding [`READER`], and the
/// controlled `pwsh`; `None` where `pwsh` is missing locally.
fn setup() -> Option<(Package, Home, PathBuf)> {
    let pwsh = controlled_pwsh()?;
    let (package, home) = (Package::new(), Home::new());
    fs::create_dir_all(work(&home)).unwrap();
    fs::write(work(&home).join("reader.ps1"), READER).unwrap();
    Some((package, home, pwsh))
}

fn work(home: &Home) -> PathBuf {
    home.0.path().join("work")
}

/// The outer `pwsh` at the prompt `OUTER> `, hosted under `model`.
fn outer(package: &Package, home: &Home, pwsh: &Path, model: HostModel) -> Terminal {
    let shell = home.pwsh(package, pwsh, "OUTER");
    let mut terminal =
        Terminal::spawn_hosted(shell, ROWS, COLS, Options::default(), hosted(model)).unwrap();
    wait(&mut terminal, home, "OUTER> ");
    terminal
}

/// `pwsh` with the shell profile's start-up at the prompt `<prompt>> `, as
/// arguments quoted for a `pwsh` command line.
fn inner_pwsh(pwsh: &Path, prompt: &str) -> String {
    let arguments: Vec<String> = pwsh_arguments(prompt)
        .iter()
        .map(|argument| quoted(argument))
        .collect();
    format!(
        "{} {}",
        quoted(&pwsh.to_string_lossy()),
        arguments.join(" ")
    )
}

/// Types `c2 exec` of [`READER`] into the outer shell and waits until it
/// reads; returns the file it keeps the bytes in.
fn exec_reader(terminal: &mut Terminal, home: &Home, pwsh: &Path) -> PathBuf {
    let out = work(home).join("got.txt");
    enter(
        terminal,
        &format!(
            "c2 exec {} -NoLogo -NoProfile -ExecutionPolicy Bypass -File {} {}",
            quoted(&pwsh.to_string_lossy()),
            quoted(&work(home).join("reader.ps1").to_string_lossy()),
            quoted(&out.to_string_lossy())
        ),
    );
    wait(terminal, home, "READING");
    out
}

/// Waits until [`READER`] has read exactly `bytes` in all.
fn read_exactly(home: &Home, out: &Path, bytes: &[u8]) {
    let expected: String = bytes.iter().map(|byte| format!("{byte:02x} ")).collect();
    let deadline = Instant::now() + TIMEOUT;
    let got = || fs::read_to_string(out).unwrap_or_default();
    while got() != expected && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(got(), expected, "\nLOGS:\n{}", home.logs());
}

/// Starts a headless session of `pwsh` at the prompt `INNER> ` and returns
/// its id.
fn headless(package: &Package, home: &Home, pwsh: &Path) -> String {
    let output =
        climon2_test_term::process_outside_any_session(&mut Command::new(package.launcher()))
            .args(["exec", "--headless", "--"])
            .arg(pwsh)
            .args(pwsh_arguments("INNER"))
            .env("CLIMON2_HOME", home.home())
            .env("LOCALAPPDATA", home.local())
            .output()
            .unwrap();
    assert!(output.status.success(), "{output:?}\n{}", home.logs());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Whether the session `id`'s daemon still runs.
fn running(home: &Home, id: &str) -> bool {
    home.sessions().iter().any(|session| {
        session.session_id.as_str() == id && process::is_same_daemon(session).unwrap_or(false)
    })
}

/// Waits until the home lists no session.
fn no_sessions(home: &Home) {
    let deadline = Instant::now() + TIMEOUT;
    while !home.sessions().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(home.sessions().is_empty(), "{}", home.logs());
}

/// The key table, and the bytes xterm sends for each key with the cursor
/// keys in normal mode.
const KEY_TABLE: &[(Key, Modifiers, &[u8])] = &[
    (Key::Up, Modifiers::NONE, b"\x1b[A"),
    (Key::Down, Modifiers::NONE, b"\x1b[B"),
    (Key::Right, Modifiers::NONE, b"\x1b[C"),
    (Key::Left, Modifiers::NONE, b"\x1b[D"),
    (Key::Home, Modifiers::NONE, b"\x1b[H"),
    (Key::End, Modifiers::NONE, b"\x1b[F"),
    (Key::F(1), Modifiers::NONE, b"\x1bOP"),
    (Key::F(4), Modifiers::NONE, b"\x1bOS"),
    (Key::F(5), Modifiers::NONE, b"\x1b[15~"),
    (Key::F(12), Modifiers::NONE, b"\x1b[24~"),
    (Key::Char('x'), Modifiers::ALT, b"\x1bx"),
    (Key::Right, Modifiers::CTRL, b"\x1b[1;5C"),
    (Key::Char('a'), Modifiers::CTRL, b"\x01"),
    (Key::Char('c'), Modifiers::CTRL, b"\x03"),
];

/// `WIN-04`: every key of the table, pressed in the model's encoding,
/// arrives in the session as the bytes xterm sends for it, Ctrl+C as
/// `0x03`.
fn the_key_table_arrives_in_the_session_as_the_expected_bytes(model: HostModel) {
    let Some((package, home, pwsh)) = setup() else {
        return;
    };
    let mut terminal = outer(&package, &home, &pwsh, model);
    let out = exec_reader(&mut terminal, &home, &pwsh);
    let mut expected = Vec::new();
    for &(key, modifiers, bytes) in KEY_TABLE {
        terminal.press(key, modifiers).unwrap();
        expected.extend_from_slice(bytes);
        read_exactly(&home, &out, &expected);
    }
}

#[test]
fn the_key_table_arrives_in_the_session_as_the_expected_bytes_under_windows_terminal() {
    let name = "the_key_table_arrives_in_the_session_as_the_expected_bytes_under_windows_terminal";
    if !rerun_outside(name, OUTSIDE) {
        the_key_table_arrives_in_the_session_as_the_expected_bytes(HostModel::WindowsTerminal);
    }
}

#[test]
fn the_key_table_arrives_in_the_session_as_the_expected_bytes_under_vs_code() {
    let name = "the_key_table_arrives_in_the_session_as_the_expected_bytes_under_vs_code";
    if !rerun_outside(name, OUTSIDE) {
        the_key_table_arrives_in_the_session_as_the_expected_bytes(HostModel::VsCode);
    }
}

/// The host's focus reports reach the session once it turned focus
/// reporting on, as `CSI O` and `CSI I`, and not before: as on Unix, where
/// the host reports focus only while the mode is on.
fn focus_reports_reach_a_session_once_it_asks_for_them(model: HostModel) {
    let Some((package, home, pwsh)) = setup() else {
        return;
    };
    let mut terminal = outer(&package, &home, &pwsh, model);
    let out = exec_reader(&mut terminal, &home, &pwsh);
    terminal.focus(false).unwrap();
    terminal.focus(true).unwrap();
    terminal.type_text("z").unwrap();
    read_exactly(&home, &out, b"z");
    wait(&mut terminal, &home, "FOCUSING");
    terminal.focus(false).unwrap();
    read_exactly(&home, &out, b"z\x1b[O");
    terminal.focus(true).unwrap();
    read_exactly(&home, &out, b"z\x1b[O\x1b[I");
}

#[test]
fn focus_reports_reach_a_session_once_it_asks_for_them_under_windows_terminal() {
    let name = "focus_reports_reach_a_session_once_it_asks_for_them_under_windows_terminal";
    if !rerun_outside(name, OUTSIDE) {
        focus_reports_reach_a_session_once_it_asks_for_them(HostModel::WindowsTerminal);
    }
}

#[test]
fn focus_reports_reach_a_session_once_it_asks_for_them_under_vs_code() {
    let name = "focus_reports_reach_a_session_once_it_asks_for_them_under_vs_code";
    if !rerun_outside(name, OUTSIDE) {
        focus_reports_reach_a_session_once_it_asks_for_them(HostModel::VsCode);
    }
}

/// ConPTY's start-up sequences: the window position report request, the
/// device attributes request, focus reporting and win32-input-mode.
const NEGOTIATION: &[&str] = &["\x1b[1t", "\x1b[c", "\x1b[?1004h", "\x1b[?9001"];

/// `WIN-08`, and resize: from `exec` on, while the session runs, the host
/// receives none of ConPTY's start-up sequences, and `CSI ? 9001` least of
/// all; a resize of the host reaches the session.
fn a_session_negotiates_nothing_with_the_host_and_follows_its_size(model: HostModel) {
    let Some((package, home, pwsh)) = setup() else {
        return;
    };
    let mut terminal = outer(&package, &home, &pwsh, model);
    let mark = terminal.received_total();
    enter(
        &mut terminal,
        &format!("c2 exec {}", inner_pwsh(&pwsh, "INNER")),
    );
    wait(&mut terminal, &home, "INNER> ");
    let size = "\"SIZE=$([Console]::WindowHeight)x$([Console]::WindowWidth)\"";
    enter(&mut terminal, size);
    wait(&mut terminal, &home, &format!("SIZE={ROWS}x{COLS}"));
    terminal.resize(24, 100).unwrap();
    enter(&mut terminal, size);
    wait(&mut terminal, &home, "SIZE=24x100");

    let received = terminal.received();
    let since = usize::try_from(terminal.received_total() - mark).unwrap();
    // The host's own pseudoconsole keeps the focus reports it needs: when
    // the client's attach turns focus reporting off, it passes that on and
    // turns it straight back on. Those are the host's, not the session's.
    let since = String::from_utf8_lossy(&received[received.len() - since..])
        .replace("\x1b[?1004l\x1b[?1004h", "");
    for sequence in NEGOTIATION {
        assert!(
            !since.contains(sequence),
            "{sequence:?} reached the host: {since:?}"
        );
    }
}

#[test]
fn a_session_negotiates_nothing_with_the_host_and_follows_its_size_under_windows_terminal() {
    let name =
        "a_session_negotiates_nothing_with_the_host_and_follows_its_size_under_windows_terminal";
    if !rerun_outside(name, OUTSIDE) {
        a_session_negotiates_nothing_with_the_host_and_follows_its_size(HostModel::WindowsTerminal);
    }
}

#[test]
fn a_session_negotiates_nothing_with_the_host_and_follows_its_size_under_vs_code() {
    let name = "a_session_negotiates_nothing_with_the_host_and_follows_its_size_under_vs_code";
    if !rerun_outside(name, OUTSIDE) {
        a_session_negotiates_nothing_with_the_host_and_follows_its_size(HostModel::VsCode);
    }
}

/// `WIN-05`: in a console, `exec` and `attach` with stdin from a pipe, or
/// with stdout to a file, refuse with 125 and the error a missing terminal
/// gives on Unix, and start no session.
#[test]
fn exec_and_attach_refuse_a_redirected_stdin_or_stdout() {
    let name = "exec_and_attach_refuse_a_redirected_stdin_or_stdout";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    let Some((package, home, pwsh)) = setup() else {
        return;
    };
    let id = headless(&package, &home, &pwsh);
    let mut terminal = outer(&package, &home, &pwsh, HostModel::WindowsTerminal);
    let file = quoted(&work(&home).join("stdout.txt").to_string_lossy());
    let refusals = [
        (
            "'x' | c2 exec cmd /d".to_owned(),
            "c2 exec needs a terminal",
        ),
        (
            format!("c2 exec cmd /d > {file}"),
            "c2 exec needs a terminal",
        ),
        (
            format!("'x' | c2 attach {id}"),
            "c2 attach needs a terminal",
        ),
        (
            format!("c2 attach {id} > {file}"),
            "c2 attach needs a terminal",
        ),
    ];
    for (index, (command, refusal)) in refusals.iter().enumerate() {
        enter(
            &mut terminal,
            &format!("{command}; \"STATUS-{index}=$LASTEXITCODE\""),
        );
        wait(&mut terminal, &home, &format!("STATUS-{index}=125"));
        assert!(
            terminal
                .screen()
                .replace('\n', "")
                .contains(&format!("climon2: error: {refusal}")),
            "{command}: {}",
            terminal.screen()
        );
    }
    let [session] = home.sessions().try_into().unwrap();
    assert_eq!(session.session_id.as_str(), id);
}

/// The VS Code model's close: closing the host while a client of an
/// ordinary session runs in it hangs the session up, as the leave policy
/// says, and nothing of it is left running.
#[test]
fn closing_a_vs_code_host_hangs_up_its_ordinary_session() {
    let name = "closing_a_vs_code_host_hangs_up_its_ordinary_session";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    let Some((package, home, pwsh)) = setup() else {
        return;
    };
    let mut terminal = outer(&package, &home, &pwsh, HostModel::VsCode);
    enter(
        &mut terminal,
        &format!("c2 exec {}", inner_pwsh(&pwsh, "INNER")),
    );
    wait(&mut terminal, &home, "INNER> ");
    let [session] = home.sessions().try_into().unwrap();
    terminal.close();
    no_sessions(&home);
    assert!(!running(&home, session.session_id.as_str()));
}

/// The VS Code model's close, attach and restoration: closing the host of
/// a headless session's client leaves the session running, and from a new
/// VS Code host `c2 attach` restores its screen; `exit 7` in it reaches the
/// shell that attached.
#[test]
fn a_headless_session_outlives_a_closed_vs_code_host_and_reattaches_restored() {
    let name = "a_headless_session_outlives_a_closed_vs_code_host_and_reattaches_restored";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    let Some((package, home, pwsh)) = setup() else {
        return;
    };
    let id = headless(&package, &home, &pwsh);
    let mut first = outer(&package, &home, &pwsh, HostModel::VsCode);
    enter(&mut first, &format!("c2 attach {id}"));
    // Keys typed before the client owns the console reach the outer shell.
    wait(&mut first, &home, "INNER> ");
    enter(&mut first, "\"$(6*7)-ok\"");
    wait(&mut first, &home, "42-ok");
    first.close();
    drop(first);
    // Past the time a leave policy that ended it would take.
    std::thread::sleep(Duration::from_secs(6));
    assert!(running(&home, &id), "{}", home.logs());

    let mut second = outer(&package, &home, &pwsh, HostModel::VsCode);
    enter(
        &mut second,
        &format!("c2 attach {id}; \"ATTACH-STATUS=$LASTEXITCODE\""),
    );
    wait(&mut second, &home, "42-ok");
    wait(&mut second, &home, "INNER> ");
    enter(&mut second, "exit 7");
    wait(&mut second, &home, "ATTACH-STATUS=7");
    no_sessions(&home);
}

/// The VS Code model's hang-up and detach: a client in a VS Code host
/// detaches with the prefix and `d`, leaving its session running; once
/// reattached, `c2 kill` from elsewhere hangs the session up, which ends
/// the client and gives the host's shell its prompt back.
#[test]
fn a_vs_code_hosted_client_detaches_and_ends_when_its_session_is_killed() {
    let name = "a_vs_code_hosted_client_detaches_and_ends_when_its_session_is_killed";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    let Some((package, home, pwsh)) = setup() else {
        return;
    };
    let mut terminal = outer(&package, &home, &pwsh, HostModel::VsCode);
    enter(
        &mut terminal,
        &format!(
            "c2 exec {}; \"EXEC-STATUS=$LASTEXITCODE\"",
            inner_pwsh(&pwsh, "INNER")
        ),
    );
    wait(&mut terminal, &home, "INNER> ");
    let [session] = home.sessions().try_into().unwrap();
    let id = session.session_id.to_string();
    terminal.send(DETACH).unwrap();
    wait(&mut terminal, &home, "EXEC-STATUS=0");
    assert!(running(&home, &id));

    enter(
        &mut terminal,
        &format!("c2 attach {id}; \"ATTACH-STATUS=$LASTEXITCODE\""),
    );
    wait(&mut terminal, &home, "INNER> ");
    let killed =
        climon2_test_term::process_outside_any_session(&mut Command::new(package.launcher()))
            .args(["kill", &id])
            .env("CLIMON2_HOME", home.home())
            .env("LOCALAPPDATA", home.local())
            .output()
            .unwrap();
    assert_eq!(killed.status.code(), Some(0), "{killed:?}\n{}", home.logs());
    wait(&mut terminal, &home, "ATTACH-STATUS=");
    no_sessions(&home);
}
