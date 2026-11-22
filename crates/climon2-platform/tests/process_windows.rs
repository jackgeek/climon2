//! #316: daemon identity and liveness on Windows (docs/windows-lifecycle.md,
//! "Identity and liveness"). A daemon is its PID plus its creation time, and
//! only "no such process" or an exited process proves it gone; a process the
//! caller may not open is never taken for a dead one.
#![cfg(windows)]
use climon2_core::metadata::SessionMetadata;
use climon2_platform::process;
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::Path,
    process::{Child, Command, Stdio},
    ptr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FALSE, LUID},
    Security::{
        AdjustTokenPrivileges,
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        DuplicateTokenEx, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, RevertToSelf, SE_DEBUG_NAME,
        SE_PRIVILEGE_REMOVED, SECURITY_ATTRIBUTES, SecurityImpersonation, TOKEN_ALL_ACCESS,
        TOKEN_DUPLICATE, TOKEN_PRIVILEGES, TokenImpersonation,
    },
    System::Threading::{
        CREATE_NO_WINDOW, CreateProcessW, GetCurrentProcess, OpenProcessToken, PROCESS_INFORMATION,
        STARTUPINFOW, SetThreadToken, TerminateProcess,
    },
};

fn micros(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH).unwrap().as_micros() as u64
}

/// A command that runs until it is killed, so its PID stays its own.
fn waiting_child() -> Child {
    Command::new("cmd")
        .args(["/d", "/c", "pause"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap()
}

fn metadata(pid: u32, start: u64) -> SessionMetadata {
    let json = serde_json::json!({
        "format_version": 1,
        "session_id": "12345678-1234-4234-8234-123456789abc",
        "socket_path": "C:\\c2\\12345678-1234-4234-8234-123456789abc.sock",
        "daemon_pid": pid,
        "daemon_start_time": start,
        "command": "cmd",
        "args": [],
        "cwd": "C:\\",
        "depth": 1,
        "created_at": "2026-11-22T00:00:00.000Z",
        "climon2_version": "0.1.0",
    });
    SessionMetadata::from_json(
        Path::new("12345678-1234-4234-8234-123456789abc.pty"),
        json.to_string().as_bytes(),
    )
    .unwrap()
}

/// Creation time is read from the system, in epoch microseconds, within the
/// clock tick (at most 15.625 ms) of when the process was started.
#[test]
fn a_live_process_has_its_creation_time_as_epoch_microseconds() {
    let tick = Duration::from_micros(15_625);
    let before = SystemTime::now() - tick;
    let mut child = waiting_child();
    let after = SystemTime::now() + tick;

    let start = process::start_time(child.id()).unwrap().unwrap();

    assert!(
        (micros(before)..=micros(after)).contains(&start),
        "{start} not within {before:?}..{after:?}"
    );
    assert_eq!(process::start_time(child.id()).unwrap(), Some(start));
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn a_daemon_is_the_same_only_with_its_pid_and_its_creation_time() {
    let mut child = waiting_child();
    let start = process::start_time(child.id()).unwrap().unwrap();

    assert!(process::is_same_process(child.id(), start).unwrap());
    assert!(process::is_same_daemon(&metadata(child.id(), start)).unwrap());
    assert!(!process::is_same_process(child.id(), start + 1).unwrap());
    assert!(!process::is_same_daemon(&metadata(child.id(), start - 1)).unwrap());
    child.kill().unwrap();
    child.wait().unwrap();
}

/// The PID names no process once the last handle to it is closed.
#[test]
fn a_gone_process_is_not_the_same_daemon() {
    let mut child = waiting_child();
    let (pid, start) = (
        child.id(),
        process::start_time(child.id()).unwrap().unwrap(),
    );
    child.kill().unwrap();
    child.wait().unwrap();
    drop(child);

    assert_eq!(process::start_time(pid).unwrap(), None);
    assert!(!process::is_same_daemon(&metadata(pid, start)).unwrap());
}

/// Someone still holding a handle keeps the process object, and its PID, but
/// the process has exited, as a Unix zombie has.
#[test]
fn an_exited_process_whose_handle_is_held_is_not_the_same_daemon() {
    let mut child = waiting_child();
    let (pid, start) = (
        child.id(),
        process::start_time(child.id()).unwrap().unwrap(),
    );
    child.kill().unwrap();
    child.wait().unwrap();

    assert_eq!(process::start_time(pid).unwrap(), None);
    assert!(!process::is_same_process(pid, start).unwrap());
    drop(child);
}

/// A process whose DACL grants nothing, as one belonging to another user may
/// be: refusing to open it is not evidence that it is gone.
#[test]
fn a_process_the_caller_may_not_open_is_never_taken_for_a_gone_one() {
    let (pid, process) = unopenable_process();

    let (error, same) = without_debug_privilege(|| {
        (
            process::start_time(pid).unwrap_err(),
            process::is_same_process(pid, 1),
        )
    });

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(same.is_err());
    // The creator's handle keeps full access.
    unsafe { TerminateProcess(process.as_raw_handle(), 1) };
}

/// Runs `work` on this thread as this process's user without
/// `SeDebugPrivilege`, which an elevated process may hold enabled and which
/// opens any process whatever its DACL.
fn without_debug_privilege<T>(work: impl FnOnce() -> T) -> T {
    let mut token = ptr::null_mut();
    assert_ne!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE, &mut token) },
        FALSE
    );
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut copy = ptr::null_mut();
    assert_ne!(
        unsafe {
            DuplicateTokenEx(
                token.as_raw_handle(),
                TOKEN_ALL_ACCESS,
                ptr::null(),
                SecurityImpersonation,
                TokenImpersonation,
                &mut copy,
            )
        },
        FALSE
    );
    let copy = unsafe { OwnedHandle::from_raw_handle(copy) };
    let mut privileges = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: LUID::default(),
            Attributes: SE_PRIVILEGE_REMOVED,
        }],
    };
    assert_ne!(
        unsafe {
            LookupPrivilegeValueW(
                ptr::null(),
                SE_DEBUG_NAME,
                &mut privileges.Privileges[0].Luid,
            )
        },
        FALSE
    );
    assert_ne!(
        unsafe {
            AdjustTokenPrivileges(
                copy.as_raw_handle(),
                FALSE,
                &privileges,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        },
        FALSE
    );
    assert_ne!(
        unsafe { SetThreadToken(ptr::null(), copy.as_raw_handle()) },
        FALSE
    );
    let result = work();
    assert_ne!(unsafe { RevertToSelf() }, FALSE);
    result
}

/// `cmd /d /c pause` with an empty, protected DACL on its process object.
fn unopenable_process() -> (u32, OwnedHandle) {
    let sddl: Vec<u16> = "D:P".encode_utf16().chain([0]).collect();
    let mut descriptor = ptr::null_mut();
    assert_ne!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        },
        FALSE
    );
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: FALSE,
    };
    let mut command: Vec<u16> = "cmd.exe /d /c pause".encode_utf16().chain([0]).collect();
    let startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    let mut information: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    assert_ne!(
        unsafe {
            CreateProcessW(
                ptr::null(),
                command.as_mut_ptr(),
                &attributes,
                ptr::null(),
                FALSE,
                CREATE_NO_WINDOW,
                ptr::null(),
                ptr::null(),
                &startup,
                &mut information,
            )
        },
        FALSE,
        "{}",
        std::io::Error::last_os_error()
    );
    unsafe { CloseHandle(information.hThread) };
    (information.dwProcessId, unsafe {
        OwnedHandle::from_raw_handle(information.hProcess)
    })
}
