//! #321: the CLI's spawn-and-wait hand-off on Windows, by the launcher's
//! rules (docs/for-humans/windows/lifecycle.md, "Hand-offs"). Each test hosts this test
//! binary in a pseudoconsole of the pinned ConPTY as the waiter, which hands
//! off to the same binary as its child; the child plays one role.
#![cfg(windows)]
use climon2_platform::handoff;
use climon2_test_term::Terminal;
use portable_pty::CommandBuilder;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU32, Ordering},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, FreeConsole, GenerateConsoleCtrlEvent,
    SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};

const TIMEOUT: Duration = Duration::from_secs(20);
const ROLE: &str = "CLIMON2_HANDOFF_ROLE";
const FOLDER: &str = "CLIMON2_HANDOFF_FOLDER";

/// The waiter's or the child's role when a test hosts this binary; as an
/// ordinary test it does nothing.
#[test]
fn climon2_handoff_role() {
    let Some(role) = env::var(ROLE).ok() else {
        return;
    };
    let folder = PathBuf::from(env::var_os(FOLDER).expect("a folder"));
    let mut words = role.split_whitespace();
    let status = match words.next() {
        Some("waiter") => waiter(&folder, words.collect::<Vec<_>>().join(" ")),
        Some("missing") => missing(&folder),
        Some("status") => words.next().unwrap().parse::<u32>().unwrap(),
        Some("ctrl") => ctrl(&folder),
        Some("linger") => linger(&folder),
        Some("brief") => brief(&folder),
        other => panic!("unknown role {other:?}"),
    };
    std::process::exit(status as i32);
}

fn role(role: &str, folder: &Path) -> Command {
    let mut command = Command::new(env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "climon2_handoff_role",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(ROLE, role)
        .env(FOLDER, folder);
    command
}

/// Hands off to a child playing `child`; returns only if it cannot.
fn waiter(folder: &Path, child: String) -> u32 {
    let error = handoff::replace(role(&child, folder));
    fs::write(folder.join("error"), error.to_string()).unwrap();
    125
}

/// A hand-off to no program returns why.
fn missing(folder: &Path) -> u32 {
    let error = handoff::replace(Command::new(folder.join("no-such-program.exe")));
    fs::write(folder.join("error"), format!("{:?}", error.kind())).unwrap();
    0
}

static SEEN: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn count(event: u32) -> i32 {
    SEEN.fetch_or(1 << event, Ordering::SeqCst);
    1
}

/// Raises Ctrl+C and Ctrl+Break on its console, then outlives the waiter's
/// three-second close wait, so a waiter that took either for a close would
/// exit 129 first, and one that did not handle them would end at once.
/// Exits 42 when both reached it.
fn ctrl(folder: &Path) -> u32 {
    let both = (1 << CTRL_C_EVENT) | (1 << CTRL_BREAK_EVENT);
    // SAFETY: a handler routine that lives as long as the process, and
    // events for every process on this console.
    unsafe {
        SetConsoleCtrlHandler(Some(count), 1);
        GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0);
        GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0);
    }
    thread::sleep(Duration::from_millis(3500));
    let seen = SEEN.load(Ordering::SeqCst);
    fs::write(folder.join("seen"), seen.to_string()).unwrap();
    if seen == both { 42 } else { 43 }
}

/// Leaves its console, so no console event reaches it, and runs until it is
/// ended, after writing its process ID to `ready`.
fn linger(folder: &Path) -> u32 {
    // SAFETY: no arguments.
    unsafe { FreeConsole() };
    fs::write(folder.join("ready"), std::process::id().to_string()).unwrap();
    thread::sleep(Duration::from_secs(60));
    0
}

unsafe extern "system" fn end_soon(event: u32) -> i32 {
    if event == CTRL_CLOSE_EVENT {
        thread::sleep(Duration::from_millis(500));
        std::process::exit(5);
    }
    1
}

/// Ends with status 5 half a second after its console closes.
fn brief(folder: &Path) -> u32 {
    // SAFETY: a handler routine that lives as long as the process.
    unsafe { SetConsoleCtrlHandler(Some(end_soon), 1) };
    fs::write(folder.join("ready"), b"").unwrap();
    thread::sleep(Duration::from_secs(60));
    0
}

/// The waiter, hosted in a pseudoconsole, handing off to a child playing
/// `child`, and the folder the two report in.
fn hosted(waiter_role: &str) -> (Terminal, tempfile::TempDir) {
    let folder = tempfile::tempdir().unwrap();
    let mut command = CommandBuilder::new(env::current_exe().unwrap());
    command.args([
        "--exact",
        "climon2_handoff_role",
        "--nocapture",
        "--test-threads=1",
    ]);
    command.env(ROLE, waiter_role);
    command.env(FOLDER, folder.path());
    (Terminal::spawn(command, 24, 80).unwrap(), folder)
}

/// A process this test ends when it is done with it.
struct Ended(u32);

impl Drop for Ended {
    fn drop(&mut self) {
        // Opening by PID has no preconditions; a new handle, closed here.
        unsafe {
            let process = OpenProcess(PROCESS_TERMINATE, 0, self.0);
            if !process.is_null() {
                TerminateProcess(process, 1);
                CloseHandle(process);
            }
        }
    }
}

fn wait_for(path: &Path, terminal: &Terminal) {
    let deadline = Instant::now() + TIMEOUT;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "no {}; screen:\n{}",
            path.display(),
            terminal.screen()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_waiter_exits_with_the_childs_full_status() {
    for status in [7u32, 259, 0xC000_013A, u32::MAX] {
        let (mut terminal, _folder) = hosted(&format!("waiter status {status}"));
        assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), status);
    }
}

#[test]
fn a_child_that_cannot_start_is_the_reason_returned() {
    let (mut terminal, folder) = hosted("missing");
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
    assert_eq!(
        fs::read_to_string(folder.path().join("error")).unwrap(),
        "NotFound"
    );
}

#[test]
fn ctrl_c_and_ctrl_break_reach_the_child_not_the_waiter() {
    let (mut terminal, folder) = hosted("waiter ctrl");
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 42);
    assert_eq!(
        fs::read_to_string(folder.path().join("seen")).unwrap(),
        "3",
        "the child saw both"
    );
}

#[test]
fn on_close_the_waiter_exits_with_the_childs_status() {
    let (mut terminal, folder) = hosted("waiter brief");
    wait_for(&folder.path().join("ready"), &terminal);
    terminal.close();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 5);
}

#[test]
fn on_close_the_waiter_gives_up_after_three_seconds() {
    let (mut terminal, folder) = hosted("waiter linger");
    wait_for(&folder.path().join("ready"), &terminal);
    let _child = Ended(
        fs::read_to_string(folder.path().join("ready"))
            .unwrap()
            .parse()
            .unwrap(),
    );
    let closed = Instant::now();
    terminal.close();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 129);
    let waited = closed.elapsed();
    assert!(
        waited >= Duration::from_secs(3) && waited < Duration::from_secs(10),
        "{waited:?}"
    );
}
