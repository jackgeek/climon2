//! Daemon identity and liveness on Windows (#316; docs/windows-lifecycle.md,
//! "Identity and liveness"). A daemon is its PID plus its creation time from
//! `GetProcessTimes`, in epoch microseconds as the frozen `daemon_start_time`
//! (ADR 0005). Only "no such process" or an exited process proves it gone;
//! any other failure to look is an error, never evidence of death.
use climon2_core::metadata::{LinuxProcessIdentity, SessionMetadata};
use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
};
use windows_sys::Win32::{
    Foundation::{ERROR_INVALID_PARAMETER, FALSE, FILETIME, WAIT_OBJECT_0},
    System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        WaitForSingleObject,
    },
};

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
