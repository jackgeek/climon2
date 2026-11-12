use super::unix::{ProcessList, pid};
use climon2_core::metadata::{LinuxProcessIdentity, SessionMetadata};
use std::{io, mem::MaybeUninit, num::NonZeroU32};

/// Room for processes started between sizing and filling the PID list.
const PID_SLACK: usize = 64;
/// `PROC_PIDTBSDINFO` takes no argument.
const NO_ARGUMENT: u64 = 0;
/// Without a buffer, `proc_listpids` returns the number of bytes it would fill.
const SIZE_QUERY: libc::c_int = 0;
/// Any token: the call only waits for libnotify's initialisation.
const ANY_TOKEN: libc::c_int = 0;

pub fn linux_identity(_pid: u32) -> io::Result<Option<LinuxProcessIdentity>> {
    Ok(None)
}

pub fn is_same_daemon(metadata: &SessionMetadata) -> io::Result<bool> {
    super::unix::is_same_process(metadata.daemon_pid, metadata.daemon_start_time)
}

pub fn start_time(value: u32) -> io::Result<Option<u64>> {
    let Some(info) = bsd_info(pid(value)?)? else {
        return Ok(None);
    };
    started_micros(&info).map(Some)
}

fn bsd_info(pid: i32) -> io::Result<Option<libc::proc_bsdinfo>> {
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = i32::try_from(size_of::<libc::proc_bsdinfo>()).map_err(io::Error::other)?;
    let result = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            NO_ARGUMENT,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if !result.is_positive() {
        return missing_process(io::Error::last_os_error());
    }
    if result != size {
        return Err(io::Error::other("incomplete process identity"));
    }
    Ok(Some(unsafe { info.assume_init() }))
}

fn started_micros(info: &libc::proc_bsdinfo) -> io::Result<u64> {
    info.pbi_start_tvsec
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
        .ok_or_else(|| io::Error::other("process start time overflow"))
}

fn missing_process<T>(error: io::Error) -> io::Result<Option<T>> {
    match error.raw_os_error() {
        Some(libc::ESRCH | libc::ENOENT) => Ok(None),
        _ => Err(error),
    }
}

unsafe extern "C" {
    fn notify_is_valid_token(token: libc::c_int) -> bool;
}

/// Wait out libnotify's global initialization before its macOS fork handler runs.
pub fn before_fork() {
    unsafe { notify_is_valid_token(ANY_TOKEN) };
}

/// The kernel's list of processes of one terminal or one process group. Slots it did not
/// fill stay 0, which is no process.
pub(super) fn listed_pids(kind: ProcessList, which: u32) -> io::Result<Vec<NonZeroU32>> {
    let kind = kind as u32;
    let needed =
        listed(unsafe { libc::proc_listpids(kind, which, std::ptr::null_mut(), SIZE_QUERY) })?;
    let mut pids = vec![i32::default(); needed / size_of::<i32>() + PID_SLACK];
    let size = i32::try_from(size_of_val(pids.as_slice())).map_err(io::Error::other)?;
    listed(unsafe { libc::proc_listpids(kind, which, pids.as_mut_ptr().cast(), size) })?;
    Ok(pids
        .into_iter()
        .filter_map(|pid| u32::try_from(pid).ok().and_then(NonZeroU32::new))
        .collect())
}

fn listed(bytes: i32) -> io::Result<usize> {
    usize::try_from(bytes).map_err(|_| io::Error::last_os_error())
}
