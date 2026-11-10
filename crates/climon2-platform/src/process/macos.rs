use super::unix::pid;
use std::{io, mem::MaybeUninit};

/// Room for processes started between sizing and filling the PID list.
const PID_SLACK: usize = 64;

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
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if result <= 0 {
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
    unsafe { notify_is_valid_token(0) };
}

/// The kernel's list of processes of one terminal or one process group.
pub(super) fn listed_pids(kind: u32, which: u32) -> io::Result<Vec<i32>> {
    let needed = listed(unsafe { libc::proc_listpids(kind, which, std::ptr::null_mut(), 0) })?;
    let mut pids = vec![0; needed / size_of::<i32>() + PID_SLACK];
    let size = i32::try_from(size_of_val(pids.as_slice())).map_err(io::Error::other)?;
    let bytes =
        listed(unsafe { libc::proc_listpids(kind, which, pids.as_mut_ptr().cast(), size) })?;
    pids.truncate(bytes / size_of::<i32>());
    Ok(pids)
}

fn listed(bytes: i32) -> io::Result<usize> {
    usize::try_from(bytes).map_err(|_| io::Error::last_os_error())
}
