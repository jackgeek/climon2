//! #321: the client's console on Windows (docs/for-humans/windows/lifecycle.md,
//! "Client console"), at the terminal seam. Each test hosts this test binary
//! as a console program in a pseudoconsole of the pinned ConPTY, under the
//! harness's Windows Terminal model, and the program plays one role: it uses
//! the seam as the client does and reports what its console did to a file.
#![cfg(windows)]
use climon2_core::session::Input;
use climon2_platform::{
    terminal::{self, Bridged, Prompt, RawModeGuard, Signals},
    waiting,
};
use climon2_test_term::{Key, Modifiers, Terminal};
use portable_pty::CommandBuilder;
use std::{
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, GenerateConsoleCtrlEvent, GetConsoleCP, GetConsoleMode,
    GetConsoleOutputCP, GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleCP,
    SetConsoleOutputCP,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};

const TIMEOUT: Duration = Duration::from_secs(20);
const ROLE: &str = "CLIMON2_CONSOLE_ROLE";
const REPORT: &str = "CLIMON2_CONSOLE_REPORT";
/// A code page that is not UTF-8, so restoring it shows.
const OEM_US: u32 = 437;

/// The role this binary plays when a test hosts it; as an ordinary test it
/// does nothing.
#[test]
fn climon2_console_role() {
    let Some(role) = env::var_os(ROLE) else {
        return;
    };
    let report = PathBuf::from(env::var_os(REPORT).expect("a report path"));
    let status = match role.to_str().expect("a role") {
        "raw" => raw(&report),
        "panic" => panicked(&report),
        "keys" => keys(&report),
        "resize" => resize(&report),
        "output" => output(&report),
        "ttys" => ttys(&report),
        "tty" => tty(),
        "signals" => signals(&report),
        "close" => close(&report),
        "prompt" => prompt(&report),
        other => panic!("unknown role {other}"),
    };
    std::process::exit(status);
}

/// This binary hosted in a pseudoconsole of 30 rows and 100 columns,
/// playing `role`, and the file it reports to.
fn hosted(role: &str) -> (Terminal, Report) {
    let report = Report::new();
    let mut command = CommandBuilder::new(env::current_exe().unwrap());
    command.args([
        "--exact",
        "climon2_console_role",
        "--nocapture",
        "--test-threads=1",
    ]);
    command.env(ROLE, role);
    command.env(REPORT, report.path());
    (Terminal::spawn(command, 30, 100).unwrap(), report)
}

struct Report(tempfile::TempDir);

impl Report {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }

    fn path(&self) -> PathBuf {
        self.0.path().join("report")
    }

    /// The report, once the program has written it.
    fn read(&self, terminal: &Terminal) -> String {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Ok(text) = fs::read_to_string(self.path()) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "no report; screen:\n{}",
                terminal.screen()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Whether the program has said it is ready, in a file beside the report.
    fn wait_ready(&self, terminal: &Terminal) {
        let ready = self.path().with_extension("ready");
        let deadline = Instant::now() + TIMEOUT;
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "never ready; screen:\n{}",
                terminal.screen()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

fn write_report(report: &Path, text: &str) {
    let written = report.with_extension("written");
    fs::write(&written, text).unwrap();
    fs::rename(written, report).unwrap();
}

fn say_ready(report: &Path) {
    fs::write(report.with_extension("ready"), b"").unwrap();
}

/// The console's input mode, output mode and both code pages.
fn console_state() -> [u32; 4] {
    let (mut input, mut output) = (0, 0);
    // SAFETY: the standard handles; each call writes one u32.
    unsafe {
        GetConsoleMode(GetStdHandle(STD_INPUT_HANDLE), &mut input);
        GetConsoleMode(GetStdHandle(STD_OUTPUT_HANDLE), &mut output);
        [input, output, GetConsoleCP(), GetConsoleOutputCP()]
    }
}

/// Starts from code pages that are not UTF-8.
fn oem_code_pages() {
    // SAFETY: code pages only.
    unsafe {
        SetConsoleCP(OEM_US);
        SetConsoleOutputCP(OEM_US);
    }
}

fn raw(report: &Path) -> i32 {
    oem_code_pages();
    let before = console_state();
    let input = terminal::input().unwrap();
    let guard = RawModeGuard::new(&input).unwrap();
    let during = console_state();
    let second = RawModeGuard::new(&input)
        .err()
        .map(|error| error.to_string());
    drop(guard);
    let after = console_state();
    write_report(
        report,
        &format!("{before:?}|{during:?}|{second:?}|{after:?}"),
    );
    0
}

fn panicked(report: &Path) -> i32 {
    oem_code_pages();
    let before = console_state();
    let input = terminal::input().unwrap();
    let caught = std::panic::catch_unwind(|| {
        let _guard = RawModeGuard::new(&input).unwrap();
        // The guard is never dropped: only the panic hook restores.
        std::mem::forget(_guard);
        panic!("raw-mode panic");
    });
    let after = console_state();
    write_report(report, &format!("{}|{before:?}|{after:?}", caught.is_err()));
    0
}

/// Raw input through the bridged console, until a `.` arrives.
fn keys(report: &Path) -> i32 {
    let input = terminal::input().unwrap();
    let _raw = RawModeGuard::new(&input).unwrap();
    let mut console = Bridged::new(input).unwrap();
    say_ready(report);
    // Input waits in several chunks before the first read, so each must be
    // signalled.
    thread::sleep(Duration::from_secs(1));
    let mut typed = Vec::new();
    while !typed.ends_with(b".") {
        let read = read_ready(&mut console);
        assert_ne!(read, [0u8; 0], "the console hung up");
        typed.extend(read);
    }
    write_report(report, &format!("{typed:?}"));
    0
}

/// One read, once the input's wait slot is ready; nothing means a hang-up.
/// Input that waits unsignalled fails.
fn read_ready(console: &mut Bridged) -> Vec<u8> {
    loop {
        let ready = waiting::wait(&mut [console.watch_input()], TIMEOUT).unwrap();
        assert_eq!(ready, 1, "input was never signalled");
        let mut bytes = [0u8; 64];
        match console.read(&mut bytes) {
            Ok(count) => return bytes[..count].to_vec(),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("{error}"),
        }
    }
}

/// The starting size, then the size after the first resize.
fn resize(report: &Path) -> i32 {
    let input = terminal::input().unwrap();
    let initial = terminal::size(&input).unwrap();
    let signals = Signals::client().unwrap();
    let _raw = RawModeGuard::new(&input).unwrap();
    let console = Bridged::new(input).unwrap();
    assert!(!signals.take_resize());
    say_ready(report);
    while !signals.take_resize() {
        thread::sleep(Duration::from_millis(10));
    }
    let resized = console.size().unwrap();
    write_report(report, &format!("{initial:?}|{resized:?}"));
    0
}

/// Much output, ending with a character split between two writes, then the
/// end of the bridge and of the program at once.
fn output(report: &Path) -> i32 {
    let input = terminal::input().unwrap();
    let _raw = RawModeGuard::new(&input).unwrap();
    let mut console = Bridged::new(input).unwrap();
    let mut text = Vec::new();
    for line in 0..4000 {
        text.extend(format!("line {line:04} {}\r\n", "x".repeat(60)).as_bytes());
    }
    write_all(&mut console, &text);
    write_all(&mut console, b"END-\xc3");
    write_all(&mut console, b"\xa9!\r\n");
    write_report(report, "written");
    drop(console);
    // Ends at once, as the CLI ends itself: no output thread outlives this.
    // SAFETY: this process's own pseudo-handle.
    unsafe { TerminateProcess(GetCurrentProcess(), 0) };
    0
}

/// Writes all of `bytes`, each time once the output's wait slot is ready.
/// Room that is never signalled fails.
fn write_all(console: &mut Bridged, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let ready = waiting::wait(&mut [console.watch_output()], TIMEOUT).unwrap();
        assert_eq!(ready, 1, "room was never signalled");
        match console.write(bytes) {
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("{error}"),
        }
    }
}

/// Whether a terminal is required and found with stdout redirected, with
/// stdin redirected and with neither, each in a child of its own.
fn ttys(report: &Path) -> i32 {
    let child = |stdin: Stdio, stdout: Stdio| {
        let status = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "climon2_console_role",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(ROLE, "tty")
            .stdin(stdin)
            .stdout(stdout)
            .status()
            .unwrap();
        status.code().unwrap()
    };
    let found = [
        child(Stdio::inherit(), Stdio::piped()),
        child(Stdio::piped(), Stdio::inherit()),
        child(Stdio::inherit(), Stdio::inherit()),
    ];
    write_report(report, &format!("{found:?}"));
    0
}

/// 1 when a terminal is required and found, plus 2 when stdin is a console
/// input.
fn tty() -> i32 {
    i32::from(terminal::require_ttys().is_ok()) + 2 * i32::from(terminal::input().is_ok())
}

/// Ctrl+Break is a break, taken once, and no termination (#326); Ctrl+C is
/// SIGINT and no break.
fn signals(report: &Path) -> i32 {
    let signals = Signals::client().unwrap();
    let before = signals.take_break();
    // SAFETY: events for every process on this console, which is this one.
    unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0) };
    let deadline = Instant::now() + TIMEOUT;
    let mut broke = false;
    while !broke && Instant::now() < deadline {
        broke = signals.take_break();
        thread::sleep(Duration::from_millis(10));
    }
    let after_break = signals.termination_signal();
    let again = signals.take_break();
    unsafe { GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) };
    let deadline = Instant::now() + TIMEOUT;
    while signals.termination_signal().is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let after_c = signals.termination_signal();
    let c_broke = signals.take_break();
    write_report(
        report,
        &format!("{before}|{broke}|{again}|{after_break:?}|{after_c:?}|{c_broke}"),
    );
    130
}

/// Ends as the client does once the console has closed.
fn close(report: &Path) -> i32 {
    let signals = Signals::client().unwrap();
    say_ready(report);
    loop {
        if let Some(signal) = signals.termination_signal() {
            write_report(report, &signal.to_string());
            return 128 + signal as i32;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Parses `[a, b, c, d]`.
fn state(text: &str) -> [u32; 4] {
    let numbers: Vec<u32> = text
        .trim_matches(['[', ']'])
        .split(", ")
        .map(|number| number.parse().unwrap())
        .collect();
    numbers.try_into().unwrap()
}

const PROCESSED_INPUT: u32 = 0x1;
const LINE_INPUT: u32 = 0x2;
const ECHO_INPUT: u32 = 0x4;
const WINDOW_INPUT: u32 = 0x8;
const QUICK_EDIT: u32 = 0x40;
const EXTENDED_FLAGS: u32 = 0x80;
const VT_INPUT: u32 = 0x200;
const VT_PROCESSING: u32 = 0x4;
const NO_AUTO_RETURN: u32 = 0x8;

#[test]
fn raw_mode_sets_the_agreed_modes_and_utf8_and_restores_them() {
    let (mut terminal, report) = hosted("raw");
    let text = report.read(&terminal);
    let [before, during, second, after]: [&str; 4] =
        text.split('|').collect::<Vec<_>>().try_into().unwrap();
    let (before, during, after) = (state(before), state(during), state(after));
    assert_eq!(before[2..], [OEM_US, OEM_US], "{text}");
    let cooked = PROCESSED_INPUT | LINE_INPUT | ECHO_INPUT | QUICK_EDIT;
    assert_eq!(
        during[0],
        (before[0] & !cooked) | VT_INPUT | WINDOW_INPUT | EXTENDED_FLAGS,
        "{text}"
    );
    assert_eq!(
        during[1],
        before[1] | VT_PROCESSING | NO_AUTO_RETURN,
        "{text}"
    );
    assert_eq!(during[2..], [65001, 65001], "{text}");
    assert_eq!(second, "Some(\"raw terminal already owned\")");
    assert_eq!(after, before, "{text}");
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
}

#[test]
fn a_panic_in_raw_mode_restores_the_console() {
    let (mut terminal, report) = hosted("panic");
    let text = report.read(&terminal);
    let [caught, before, after]: [&str; 3] =
        text.split('|').collect::<Vec<_>>().try_into().unwrap();
    assert_eq!(caught, "true");
    assert_eq!(state(after), state(before), "{text}");
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
}

/// Keys arrive once each, as typed: key-up records and modifier keys type
/// nothing, Ctrl+C is byte 0x03, and characters beyond ASCII, a surrogate
/// pair among them, arrive as UTF-8.
#[test]
fn only_key_down_characters_arrive_as_utf8() {
    let (mut terminal, report) = hosted("keys");
    report.wait_ready(&terminal);
    // More records than one read takes.
    let many = "x".repeat(150);
    terminal.send(&many).unwrap();
    terminal.type_text("aB").unwrap();
    terminal.press(Key::Char('c'), Modifiers::CTRL).unwrap();
    terminal.press(Key::Enter, Modifiers::NONE).unwrap();
    terminal.send("é😀".as_bytes()).unwrap();
    terminal.type_text(".").unwrap();
    let mut expected = many.into_bytes();
    expected.extend(b"aB\x03\r");
    expected.extend("é😀.".as_bytes());
    assert_eq!(report.read(&terminal), format!("{expected:?}"));
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
}

#[test]
fn a_resize_is_seen_with_the_new_window_size() {
    let (mut terminal, report) = hosted("resize");
    report.wait_ready(&terminal);
    terminal.resize(20, 70).unwrap();
    assert_eq!(
        report.read(&terminal),
        "TerminalSize { cols: 100, rows: 30 }|TerminalSize { cols: 70, rows: 20 }"
    );
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
}

/// Output queued before the bridge ends is all written, and a character
/// split between two writes is shown whole.
#[test]
fn queued_output_is_written_whole_before_the_bridge_ends() {
    let (mut terminal, report) = hosted("output");
    assert_eq!(report.read(&terminal), "written");
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
    let screen = terminal.screen();
    assert!(screen.contains("line 3999"), "{screen}");
    assert!(screen.contains("END-é!"), "{screen}");
}

#[test]
fn a_terminal_needs_console_input_and_output() {
    let (mut terminal, report) = hosted("ttys");
    // Stdout redirected: no terminal, though stdin is a console; stdin
    // redirected: neither; both consoles: both.
    assert_eq!(report.read(&terminal), "[2, 0, 3]");
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
}

#[test]
fn ctrl_break_is_a_break_and_ctrl_c_is_an_interrupt() {
    let (mut terminal, report) = hosted("signals");
    assert_eq!(
        report.read(&terminal),
        "false|true|false|None|Some(2)|false"
    );
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 130);
}

/// The handler keeps the program alive after its console closes until it
/// has ended itself with 129.
#[test]
fn a_closed_console_is_a_hang_up_the_program_outlives() {
    let (mut terminal, report) = hosted("close");
    report.wait_ready(&terminal);
    terminal.close();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 129);
    assert_eq!(report.read(&terminal), "1");
}

/// #139: keys typed before a nested-start prompt are discarded; keys typed
/// once it shows come one at a time; Ctrl+Break ends its wait with
/// `STATUS_CONTROL_C_EXIT` instead of ending the program, and dropping it
/// puts the console's modes and code pages back and stops handling console
/// events, so a later Ctrl+C ends the program.
#[test]
fn a_prompt_discards_type_ahead_takes_keys_singly_and_ends_restored_on_ctrl_break() {
    let (mut terminal, report) = hosted("prompt");
    report.wait_ready(&terminal);
    terminal.type_text("early").unwrap();
    terminal.press(Key::Enter, Modifiers::NONE).unwrap();
    // The typed records reach the console before the prompt starts.
    thread::sleep(Duration::from_millis(500));
    fs::write(report.path().with_extension("go"), b"").unwrap();
    terminal.wait_for_text("PROMPT-SHOWN", TIMEOUT).unwrap();
    terminal.type_text("yz").unwrap();
    terminal.press(Key::Enter, Modifiers::NONE).unwrap();
    thread::sleep(Duration::from_millis(500));
    terminal.press(Key::Break, Modifiers::CTRL).unwrap();
    assert_eq!(
        report.read(&terminal),
        "[Key(121), Key(122), Key(13), Ended(3221225786)]|true"
    );
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0xC000_013A);
}

/// Waits for the `.go` file, so keys typed before it wait in the console,
/// then takes a prompt's input until its wait ends, and reports it and
/// whether the console came back as it was.
fn prompt(report: &Path) -> i32 {
    oem_code_pages();
    say_ready(report);
    let go = report.with_extension("go");
    while !go.exists() {
        thread::sleep(Duration::from_millis(10));
    }
    let before = console_state();
    let mut prompt = Prompt::new(terminal::input().unwrap(), terminal::output().unwrap()).unwrap();
    prompt.show("PROMPT-SHOWN\r\n").unwrap();
    let mut inputs = Vec::new();
    loop {
        let input = prompt.read_input().unwrap();
        inputs.push(input);
        if matches!(input, Input::Ended(_) | Input::Gone) {
            break;
        }
    }
    drop(prompt);
    let after = console_state();
    write_report(report, &format!("{inputs:?}|{}", before == after));
    // The prompt's handler routine went with it, so Ctrl+C ends the program
    // as it would have before.
    // SAFETY: events for every process on this console, which is this one.
    unsafe { GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) };
    thread::sleep(TIMEOUT);
    0
}

/// #139: a hang-up opening the prompt's console (a broken pipe or an
/// invalid handle) answers the prompt as a console gone, as a lost console
/// does while it waits; any other failure is returned.
#[test]
fn a_hang_up_opening_a_prompt_answers_gone() {
    use climon2_core::session::Answer;
    // ERROR_BROKEN_PIPE and ERROR_INVALID_HANDLE.
    for code in [109, 6] {
        let failed = || Err(io::Error::from_raw_os_error(code));
        assert_eq!(
            terminal::ask(failed(), failed(), "text").unwrap(),
            Answer::Gone
        );
    }
    // ERROR_ACCESS_DENIED is no hang-up, so it is returned.
    let denied = terminal::ask(
        Err(io::Error::from_raw_os_error(5)),
        Err(io::Error::from_raw_os_error(5)),
        "text",
    )
    .unwrap_err();
    assert_eq!(denied.raw_os_error(), Some(5));
}
