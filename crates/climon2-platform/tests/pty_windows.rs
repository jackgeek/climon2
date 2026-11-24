//! #317: the session's terminal on Windows. A command runs in a pseudoconsole
//! of the pinned ConPTY and in the session job; output ends with a broken
//! pipe once the released pseudoconsole has no process left; the leader's
//! handle reports its full exit status even while a background process keeps
//! the console; and hang-up closes the pseudoconsole and ends the job.
#![cfg(windows)]
use climon2_platform::{
    command::{self, Launch},
    pty::{self, ConPty, Hosted},
};
use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::Path,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{TRUE, WAIT_OBJECT_0},
    System::{
        Console::SetConsoleCtrlHandler,
        Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    },
};

/// `0xC000013A`, `STATUS_CONTROL_C_EXIT`: how a console process ends on
/// Ctrl+C or a close event.
const CONTROL_C_EXIT: u32 = 0xC000_013A;

fn conpty() -> ConPty {
    let folder = climon2_test_term::ConPty::pinned().unwrap().folder();
    ConPty::load(folder).unwrap()
}

fn start(argv: &[&str]) -> Hosted {
    start_with(argv, &[])
}

fn start_with(argv: &[&str], environment: &[(&str, &str)]) -> Hosted {
    let cwd = std::env::temp_dir();
    pty::open(conpty(), 24, 80)
        .unwrap()
        .start(&launch(argv), &cwd, environment)
        .unwrap()
}

/// `argv` resolved as the daemon resolves a session's command.
fn launch(argv: &[&str]) -> Launch {
    let arguments: Vec<OsString> = argv[1..].iter().map(OsString::from).collect();
    command::launch(OsStr::new(argv[0]), &arguments, &std::env::temp_dir()).unwrap()
}

/// Output read until `done` holds for it or ten seconds pass; the read that
/// stopped it is returned too.
fn read_until(hosted: &Hosted, done: impl Fn(&str) -> bool) -> (String, io::Result<usize>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut text = Vec::new();
    loop {
        hosted.wait_readable(Duration::from_millis(50));
        let mut buffer = [0u8; 7];
        let read = hosted.read(&mut buffer);
        if let Ok(count) = read {
            text.extend_from_slice(&buffer[..count]);
        }
        let shown = String::from_utf8_lossy(&text).into_owned();
        let failed = matches!(&read, Err(error) if error.kind() != io::ErrorKind::WouldBlock);
        if done(&shown) || failed || Instant::now() > deadline {
            return (shown, read);
        }
    }
}

fn exit_status(hosted: &Hosted) -> Option<u32> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(status) = hosted.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

#[test]
fn output_ends_with_a_broken_pipe_and_the_leader_reports_its_full_status() {
    let hosted = start(&["cmd", "/d", "/c", "echo hello& exit -1073741510"]);
    assert_ne!(hosted.pid(), 0);
    let (text, end) = read_until(&hosted, |_| false);
    assert!(text.contains("hello"), "{text:?}");
    let error = end.unwrap_err();
    assert!(pty::output_ended(&error), "{error:?}");
    // The end repeats, as Unix EIO does.
    assert!(pty::output_ended(&hosted.read(&mut [0u8; 4]).unwrap_err()));
    assert_eq!(exit_status(&hosted), Some(CONTROL_C_EXIT));
}

#[test]
fn the_leaders_end_is_known_while_a_background_process_keeps_the_console() {
    let hosted = start(&[
        "cmd",
        "/d",
        "/c",
        "start /b ping -n 6 127.0.0.1 >nul & exit 3",
    ]);
    assert_eq!(exit_status(&hosted), Some(3));
    // `ping` still holds the console, so the output has not ended.
    let (_, read) = read_until(&hosted, |_| true);
    assert!(!matches!(read, Err(error) if pty::output_ended(&error)));
}

#[test]
fn the_session_variables_are_set_before_the_command_starts() {
    let hosted = start_with(
        &[
            "cmd",
            "/d",
            "/c",
            "echo [%CLIMON2_PTY_TEST_A%][%CLIMON2_PTY_TEST_B%]",
        ],
        &[
            ("CLIMON2_PTY_TEST_A", "first"),
            ("CLIMON2_PTY_TEST_B", "second"),
        ],
    );
    let (text, _) = read_until(&hosted, |text| text.contains("second]"));
    assert!(text.contains("[first][second]"), "{text:?}");
}

#[test]
fn input_and_a_resize_reach_the_command() {
    let mut hosted = start(&["cmd", "/d", "/k", "prompt $g"]);
    let _ = read_until(&hosted, |text| text.contains('>'));
    hosted.resize(30, 100).unwrap();
    assert_eq!(hosted.write(b"mode con\r").unwrap(), 9);
    let (text, _) = read_until(&hosted, |text| text.contains("100"));
    assert!(text.contains("100"), "{text:?}");
    hosted.write(b"exit 7\r").unwrap();
    assert_eq!(exit_status(&hosted), Some(7));
    hosted.finish().unwrap();
}

/// An inherited "ignore Ctrl+C" would reach the command, which would then
/// ignore it; starting clears it.
#[test]
fn ctrl_c_interrupts_the_command_though_this_process_ignored_it() {
    // Ignoring Ctrl+C in this process reads nothing.
    assert_ne!(unsafe { SetConsoleCtrlHandler(None, TRUE) }, 0);
    let hosted = start(&["ping", "-n", "30", "127.0.0.1"]);
    let _ = read_until(&hosted, |text| text.contains("127.0.0.1:"));
    hosted.write(&[0x03]).unwrap();
    assert_eq!(exit_status(&hosted), Some(CONTROL_C_EXIT));
}

#[test]
fn hang_up_closes_the_console_then_ends_the_job_with_the_leaders_status() {
    let mut hosted = start(&["cmd", "/d", "/k"]);
    let _ = read_until(&hosted, |text| text.contains('>'));
    hosted.close();
    // A closed pseudoconsole has nothing to resize, and closing it again
    // does nothing.
    hosted.resize(30, 100).unwrap();
    hosted.close();
    assert_eq!(hosted.end().unwrap(), CONTROL_C_EXIT);
    assert_eq!(hosted.try_wait().unwrap(), Some(CONTROL_C_EXIT));
    let deadline = Instant::now() + Duration::from_secs(10);
    let refused = loop {
        match hosted.write(b"x") {
            Err(error) => break error,
            Ok(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(_) => panic!("input still accepted after hang-up"),
        }
    };
    assert!(pty::output_ended(&refused), "{refused:?}");
}

/// What a PowerShell command prints after `label`, up to the line's end.
fn reported(text: &str, label: &str) -> String {
    let start = text
        .find(label)
        .unwrap_or_else(|| panic!("no {label}: {text:?}"))
        + label.len();
    text[start..]
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect()
}

/// The job lets a process leave it (so a nested `c2 exec` can start its own
/// daemon), and kills a process left in it, one that has its own hidden
/// console and so never sees the close event, when its last handle closes.
#[test]
fn the_session_job_allows_breakaway_and_kills_what_is_left_on_close() {
    let script = std::env::temp_dir().join(format!("climon2-job-{}.ps1", std::process::id()));
    fs::write(&script, JOB_PROBE).unwrap();
    let hosted = start(&[
        "powershell",
        "-NoLogo",
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        script.to_str().unwrap(),
    ]);
    let (text, _) = read_until(&hosted, |text| {
        text.split_once("CHILD=")
            .is_some_and(|(_, pid)| pid.contains('\n'))
    });
    let _ = fs::remove_file(&script);
    assert_eq!(reported(&text, "BREAKAWAY="), "0", "{text:?}");
    let pid: u32 = reported(&text, "CHILD=").parse().unwrap();
    // Opening by PID has no preconditions; the handle keeps the PID ours.
    let child = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!child.is_null());
    let child = unsafe { OwnedHandle::from_raw_handle(child) };
    // SAFETY-free: the handle is open with SYNCHRONIZE.
    assert_ne!(
        unsafe { WaitForSingleObject(child.as_raw_handle(), 0) },
        WAIT_OBJECT_0
    );
    drop(hosted);
    assert_eq!(
        unsafe { WaitForSingleObject(child.as_raw_handle(), 10_000) },
        WAIT_OBJECT_0
    );
}

/// Starts a child with `CREATE_BREAKAWAY_FROM_JOB` and prints the error (0 for
/// none), then a hidden `ping` left in the job and its PID.
const JOB_PROBE: &str = r#"
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class Climon2Breakaway {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct StartupInfo { public int cb; public IntPtr r1, desktop, title; public int x, y, w, h, cx, cy, fill, flags; public short show, r2; public IntPtr r3, input, output, error; }
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInformation { public IntPtr process, thread; public int pid, tid; }
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool CreateProcessW(string app, string line, IntPtr pa, IntPtr ta, bool inherit, uint flags, IntPtr env, string dir, ref StartupInfo si, out ProcessInformation pi);
    public static int Probe() {
        var si = new StartupInfo(); si.cb = Marshal.SizeOf(si);
        ProcessInformation pi;
        // CREATE_BREAKAWAY_FROM_JOB | CREATE_NO_WINDOW
        if (!CreateProcessW(null, "cmd.exe /d /c exit", IntPtr.Zero, IntPtr.Zero, false, 0x09000000, IntPtr.Zero, null, ref si, out pi)) {
            return Marshal.GetLastWin32Error();
        }
        return 0;
    }
    public static int Hidden() {
        var si = new StartupInfo(); si.cb = Marshal.SizeOf(si);
        ProcessInformation pi;
        // CREATE_NO_WINDOW: a console of its own, so no close event of ours
        if (!CreateProcessW(null, "ping.exe -n 120 127.0.0.1", IntPtr.Zero, IntPtr.Zero, false, 0x08000000, IntPtr.Zero, null, ref si, out pi)) {
            return -Marshal.GetLastWin32Error();
        }
        return pi.pid;
    }
}
"@
"BREAKAWAY=" + [Climon2Breakaway]::Probe()
"CHILD=" + [Climon2Breakaway]::Hidden()
Start-Sleep 120
"#;

#[test]
fn the_bundled_conpty_loads_only_beside_open_console() {
    let pinned = climon2_test_term::ConPty::pinned().unwrap().folder();
    let folder = tempfile::tempdir().unwrap();
    fs::copy(pinned.join("conpty.dll"), folder.path().join("conpty.dll")).unwrap();
    let missing = ConPty::load(folder.path()).err().unwrap();
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    assert!(missing.to_string().contains("OpenConsole.exe"), "{missing}");
    let console_only = tempfile::tempdir().unwrap();
    copy(pinned, console_only.path(), "OpenConsole.exe");
    assert!(ConPty::load(console_only.path()).is_err());
}

fn copy(from: &Path, to: &Path, name: &str) {
    fs::copy(from.join(name), to.join(name)).unwrap();
}

#[test]
fn a_command_that_cannot_start_fails_to_start() {
    let missing = std::env::temp_dir().join("climon2-no-such-directory");
    let error = pty::open(conpty(), 24, 80)
        .unwrap()
        .start(&launch(&["cmd", "/d", "/c", "exit"]), &missing, &[])
        .err()
        .unwrap();
    // ERROR_DIRECTORY
    assert_eq!(error.raw_os_error(), Some(267));
}

/// #325, `WIN-08`: the pseudoconsole's own start-up negotiation (the window
/// position report request, the device attributes request, focus reporting
/// and win32-input-mode) never reaches the session's output; the command's
/// own output, a focus reporting request among it, does.
#[test]
fn the_start_up_negotiation_never_reaches_the_output() {
    let hosted = start(&["cmd", "/d", "/c", "echo first& echo \x1b[?1004hfocus-on"]);
    let (text, _) = read_until(&hosted, |text| text.contains("focus-on"));
    for sequence in ["\x1b[1t", "\x1b[c", "\x1b[?9001"] {
        assert!(!text.contains(sequence), "{sequence:?} in {text:?}");
    }
    let first = text.find("first").expect(&text);
    assert!(!text[..first].contains("\x1b[?1004h"), "{text:?}");
    assert!(text[first..].contains("\x1b[?1004hfocus-on"), "{text:?}");
}

/// A command's own device attributes request, after the pseudoconsole's.
/// It shows `QUERY`, asks, and shows the first input it reads in hex.
const DEVICE_ATTRIBUTES_PROBE: &str = r#"$signatures = '[DllImport("kernel32.dll")] public static extern System.IntPtr GetStdHandle(int n);
[DllImport("kernel32.dll")] public static extern bool GetConsoleMode(System.IntPtr h, out uint m);
[DllImport("kernel32.dll")] public static extern bool SetConsoleMode(System.IntPtr h, uint m);
[DllImport("kernel32.dll")] public static extern bool ReadFile(System.IntPtr h, byte[] b, uint n, out uint r, System.IntPtr o);'
Add-Type -Name Console -Namespace Climon2 -MemberDefinition $signatures
$in = [Climon2.Console]::GetStdHandle(-10)
$out = [Climon2.Console]::GetStdHandle(-11)
[void][Climon2.Console]::SetConsoleMode($in, 0x200)
$mode = 0
[void][Climon2.Console]::GetConsoleMode($out, [ref]$mode)
[void][Climon2.Console]::SetConsoleMode($out, $mode -bor 4)
[Console]::Write("QUERY$([char]27)[c")
$buffer = [byte[]]::new(64)
$read = 0
[void][Climon2.Console]::ReadFile($in, $buffer, 64, [ref]$read, [IntPtr]::Zero)
'GOT:' + (-join ($buffer[0..($read - 1)] | ForEach-Object { $_.ToString('x2') }))
"#;

/// The pseudoconsole asks for the device attributes as it starts and takes
/// the first answer it reads as its host's. The daemon answers that request
/// itself, so the answer to the command's own request reaches the command,
/// as an app such as Microsoft Edit waits for it.
#[test]
fn the_answer_to_a_commands_device_attributes_request_reaches_it() {
    let hosted = start(&[
        "powershell",
        "-NoLogo",
        "-NoProfile",
        "-Command",
        DEVICE_ATTRIBUTES_PROBE,
    ]);
    let (text, _) = read_until(&hosted, |text| {
        text.find("QUERY")
            .is_some_and(|at| text[at..].contains("\x1b[c"))
    });
    assert!(text.contains("QUERY"), "{text:?}");
    hosted.write(b"\x1b[?6c").unwrap();
    let (text, _) = read_until(&hosted, |text| text.contains("GOT:1b5b3f3663"));
    assert!(text.contains("GOT:1b5b3f3663"), "{text:?}");
}
