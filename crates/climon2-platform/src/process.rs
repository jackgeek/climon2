//! PID identity, exact wait status and process-group leave policy.
#[cfg(not(target_os = "macos"))]
compile_error!("process identity, hangup and exit seams require macOS");

#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "macos")]
mod macos {
    use climon2_core::{exit::ExitStatus, protocol::HANGUP_GRACE};
    use std::{io, mem::MaybeUninit, num::NonZeroU32, path::Path};

    fn pid(value: u32) -> io::Result<i32> {
        let value = i32::try_from(value).map_err(io::Error::other)?;
        if value == 0 {
            return Err(io::Error::other("PID must be positive"));
        }
        Ok(value)
    }

    pub fn start_time(value: u32) -> io::Result<Option<u64>> {
        let pid = pid(value)?;
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
        let info = unsafe { info.assume_init() };
        let started = info
            .pbi_start_tvsec
            .checked_mul(1_000_000)
            .and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
            .ok_or_else(|| io::Error::other("process start time overflow"))?;
        Ok(Some(started))
    }

    fn missing_process(error: io::Error) -> io::Result<Option<u64>> {
        match error.raw_os_error() {
            Some(libc::ESRCH | libc::ENOENT) => Ok(None),
            _ => Err(error),
        }
    }

    /// Errors (including permission failures) are deliberately not evidence of death.
    pub fn is_same_process(pid: u32, expected_start: u64) -> io::Result<bool> {
        Ok(start_time(pid)? == Some(expected_start))
    }

    pub fn validate_cwd(path: &Path) -> io::Result<()> {
        if !std::fs::metadata(path)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "command cwd is not a directory",
            ));
        }
        let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| io::Error::other("invalid command cwd"))?;
        if unsafe { libc::access(path.as_ptr(), libc::X_OK) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn wait(pid: u32) -> io::Result<ExitStatus> {
        reap(pid, 0)?.ok_or_else(|| io::Error::other("missing child exit status"))
    }

    pub fn try_wait(pid: u32) -> io::Result<Option<ExitStatus>> {
        reap(pid, libc::WNOHANG)
    }

    fn reap(value: u32, options: i32) -> io::Result<Option<ExitStatus>> {
        let pid = pid(value)?;
        let mut status = 0;
        if wait_raw(pid, &mut status, options)? == 0 {
            return Ok(None);
        }
        decode_status(status).map(Some)
    }

    fn wait_raw(pid: i32, status: &mut i32, options: i32) -> io::Result<i32> {
        loop {
            match unsafe { libc::waitpid(pid, status, options) } {
                -1 => {
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::Interrupted {
                        return Err(error);
                    }
                }
                result => return Ok(result),
            }
        }
    }

    fn decode_status(status: i32) -> io::Result<ExitStatus> {
        if libc::WIFEXITED(status) {
            return Ok(ExitStatus::Command(
                u32::try_from(libc::WEXITSTATUS(status)).map_err(io::Error::other)?,
            ));
        }
        let signal = u32::try_from(libc::WTERMSIG(status)).map_err(io::Error::other)?;
        let signal =
            NonZeroU32::new(signal).ok_or_else(|| io::Error::other("invalid wait status"))?;
        Ok(ExitStatus::Signal(signal))
    }

    #[derive(Clone, Copy, Debug)]
    pub enum Signal {
        Hangup,
        Interrupt,
        Terminate,
        Kill,
    }

    impl Signal {
        fn raw(self) -> i32 {
            match self {
                Self::Hangup => libc::SIGHUP,
                Self::Interrupt => libc::SIGINT,
                Self::Terminate => libc::SIGTERM,
                Self::Kill => libc::SIGKILL,
            }
        }
    }

    pub fn signal_group(group: u32, signal: Signal) -> io::Result<()> {
        let group = pid(group)?;
        if unsafe { libc::kill(-group, signal.raw()) } == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }

    /// Caller closes every PTY-master handle here. Even if the leader exits,
    /// descendants in its group get the full grace period, then SIGKILL.
    pub fn hangup(group: u32, close_master: impl FnOnce()) -> io::Result<ExitStatus> {
        pid(group)?;
        close_master();
        std::thread::sleep(HANGUP_GRACE);
        let exited = try_wait(group)?;
        signal_group(group, Signal::Kill)?;
        match exited {
            Some(status) => Ok(status),
            None => wait(group),
        }
    }
}
