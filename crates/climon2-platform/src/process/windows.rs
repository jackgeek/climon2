//! Daemon identity and liveness on Windows (#316; docs/for-humans/windows/lifecycle.md,
//! "Identity and liveness"). A daemon is its PID plus its creation time from
//! `GetProcessTimes`, in epoch microseconds as the frozen `daemon_start_time`
//! (ADR 0005). Only "no such process" or an exited process proves it gone;
//! any other failure to look is an error, never evidence of death.
use climon2_core::metadata::{LinuxProcessIdentity, SessionMetadata, SessionOrigin};
use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::{
    Win32::{
        Foundation::{
            ERROR_INVALID_PARAMETER, ERROR_SUCCESS, ERROR_TOO_MANY_OPEN_FILES, FALSE, FILETIME,
            WAIT_OBJECT_0,
        },
        Networking::WinSock::WSAEMFILE,
        System::{
            Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW},
            Threading::{
                GetCurrentProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
                PROCESS_SYNCHRONIZE, TerminateProcess, WaitForSingleObject,
            },
        },
    },
    w,
};

/// Room for `MachineGuid`: a 36-character GUID and its terminating NUL.
const MACHINE_GUID_UNITS: usize = 64;

/// The most handles one process can hold: the kernel's per-process handle
/// table, 2^24 entries. Windows has no soft limit below it.
const HANDLE_LIMIT: u64 = 1 << 24;

/// 100-nanosecond intervals from 1601-01-01, where FILETIME counts from, to
/// the Unix epoch.
const UNIX_EPOCH_INTERVALS: u64 = 116_444_736_000_000_000;

/// Windows has no Linux boot and start-tick identity: creation time alone,
/// in `daemon_start_time`, identifies a daemon.
pub fn linux_identity(_pid: u32) -> io::Result<Option<LinuxProcessIdentity>> {
    Ok(None)
}

pub fn is_same_daemon(metadata: &SessionMetadata) -> io::Result<bool> {
    is_same_process(metadata.daemon_pid, metadata.daemon_start_time)
}

/// Where this process runs (#133): the machine, by the `MachineGuid` Windows
/// keeps under `HKLM\SOFTWARE\Microsoft\Cryptography` across reboots. A
/// climon2 home on a network share can be read from another machine, whose
/// PIDs say nothing about this one's daemons. Windows has no PID namespaces.
pub fn origin() -> io::Result<SessionOrigin> {
    let mut units = [0u16; MACHINE_GUID_UNITS];
    let mut bytes = u32::try_from(size_of_val(&units)).map_err(io::Error::other)?;
    // SAFETY: both names are NUL-terminated literals, and `bytes` is the size
    // of the writable `units`.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Cryptography"),
            w!("MachineGuid"),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            units.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let text = String::from_utf16(&units).map_err(io::Error::other)?;
    let machine = text.trim_end_matches('\0').parse();
    Ok(SessionOrigin {
        machine_id: Some(machine.map_err(io::Error::other)?),
        pid_namespace: None,
    })
}

/// Errors, such as a process the caller may not open, are not evidence of death.
pub fn is_same_process(pid: u32, expected_start: u64) -> io::Result<bool> {
    Ok(start_time(pid)? == Some(expected_start))
}

/// The creation time of the running process `pid`, in microseconds since the
/// Unix epoch; `None` when no process has that PID or it has exited.
pub fn start_time(pid: u32) -> io::Result<Option<u64>> {
    let Some(process) = open(pid)? else {
        return Ok(None);
    };
    if exited(&process) {
        return Ok(None);
    }
    created(&process).map(Some)
}

/// Only "no such process" (`ERROR_INVALID_PARAMETER`) means there is none.
fn open(pid: u32) -> io::Result<Option<OwnedHandle>> {
    // Opening a process by PID has no memory-safety preconditions.
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            FALSE,
            pid,
        )
    };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            return Ok(None);
        }
        return Err(error);
    }
    // OpenProcess returned a new handle this call owns.
    Ok(Some(unsafe { OwnedHandle::from_raw_handle(handle) }))
}

/// A process object outlives its process while anyone holds a handle to it.
fn exited(process: &OwnedHandle) -> bool {
    // The handle is open with SYNCHRONIZE; a zero timeout only looks. Spelled
    // `0u32`: a 1 ms timeout gives the same answer, an equivalent mutant.
    unsafe { WaitForSingleObject(process.as_raw_handle(), 0u32) == WAIT_OBJECT_0 }
}

fn created(process: &OwnedHandle) -> io::Result<u64> {
    let mut times = [FILETIME::default(); 4];
    let [creation, exit, kernel, user] = &mut times;
    // The handle is open with PROCESS_QUERY_LIMITED_INFORMATION and each
    // FILETIME is writable.
    if unsafe { GetProcessTimes(process.as_raw_handle(), creation, exit, kernel, user) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    epoch_micros(times[0])
}

fn epoch_micros(time: FILETIME) -> io::Result<u64> {
    let intervals = u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime);
    intervals
        .checked_sub(UNIX_EPOCH_INTERVALS)
        .map(|since_epoch| since_epoch / 10)
        .ok_or_else(|| io::Error::other("process created before the Unix epoch"))
}

/// Ends this process at once with all 32 bits of `status` (#323;
/// docs/for-humans/windows/lifecycle.md, "Client console"), without flushing
/// std's standard output. Not `std::process::exit`, which sets the status
/// only after unloading the process's libraries: a close event arriving
/// meanwhile replaces it with `STATUS_CONTROL_C_EXIT`.
pub fn exit(status: u32) -> ! {
    // SAFETY: this process's own pseudo-handle.
    unsafe { TerminateProcess(GetCurrentProcess(), status) };
    std::process::exit(status as i32)
}

/// The process ran out of sockets (Winsock's `WSAEMFILE`) or of file
/// handles (`ERROR_TOO_MANY_OPEN_FILES`).
pub fn out_of_descriptors(error: &io::Error) -> bool {
    const TOO_MANY_OPEN_FILES: i32 = ERROR_TOO_MANY_OPEN_FILES as i32;
    matches!(error.raw_os_error(), Some(WSAEMFILE | TOO_MANY_OPEN_FILES))
}

/// The limit the monitor's admission counts descriptors against: Windows
/// has no per-process soft limit to read, only the handle table's size.
pub fn descriptor_limit() -> u64 {
    HANDLE_LIMIT
}

/// Ends the process with `code` at once, all 32 bits of it: no exit
/// handlers, no flushing and no waiting for other threads, so whatever must
/// be seen is flushed first.
pub fn exit_immediately(code: i32) -> ! {
    // SAFETY: this process's own pseudo-handle; the call does not return.
    unsafe { TerminateProcess(GetCurrentProcess(), code as u32) };
    unreachable!("TerminateProcess on this process returned")
}
