//! PROTOTYPE: macOS kernel facts used as proof. None of them come from the environment.
use std::{ffi::CStr, io, os::fd::RawFd};

pub struct ProcInfo {
    pub ppid: u32,
    pub uid: u32,
    pub start_us: u64,
}

pub fn proc_info(pid: u32) -> Option<ProcInfo> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let read = unsafe {
        libc::proc_pidinfo(pid as i32, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size)
    };
    (read == size).then(|| ProcInfo {
        ppid: info.pbi_ppid,
        uid: info.pbi_uid,
        start_us: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
    })
}

pub fn exe_name(pid: u32) -> Option<String> {
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let length = unsafe { libc::proc_pidpath(pid as i32, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if length <= 0 {
        return None;
    }
    let path = CStr::from_bytes_until_nul(&buffer).ok()?.to_string_lossy().into_owned();
    path.rsplit('/').next().map(str::to_owned)
}

/// The session leader of the terminal behind `fd`, if `fd` is a terminal.
pub fn terminal_leader(fd: RawFd) -> Option<u32> {
    if unsafe { libc::isatty(fd) } != 1 {
        return None;
    }
    let sid = unsafe { libc::tcgetsid(fd) };
    (sid > 0).then_some(sid as u32)
}

pub fn peer_pid(fd: RawFd) -> io::Result<u32> {
    const SOL_LOCAL: i32 = 0;
    const LOCAL_PEERPID: i32 = 2;
    let mut pid: libc::pid_t = 0;
    let mut length = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(fd, SOL_LOCAL, LOCAL_PEERPID, (&raw mut pid).cast(), &mut length)
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(pid as u32)
}

pub fn peer_uid(fd: RawFd) -> io::Result<u32> {
    let (mut uid, mut gid) = (0, 0);
    if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(uid)
}

pub fn euid() -> u32 {
    unsafe { libc::geteuid() }
}
