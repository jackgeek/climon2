//! PID identity, exact wait status and terminal-session leave policy.
#[cfg(not(target_os = "macos"))]
compile_error!("process identity, hangup and exit seams require macOS");

#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "macos")]
mod macos {
    use climon2_core::{exit::ExitStatus, protocol::HANGUP_GRACE};
    use std::{
        io,
        mem::MaybeUninit,
        num::NonZeroU32,
        os::unix::fs::{FileTypeExt, MetadataExt},
        path::Path,
        time::Duration,
    };

    /// `libproc.h` listing kinds: processes of one process group, or of one controlling
    /// terminal. Session kills never enumerate all processes.
    const PROC_PGRP_ONLY: u32 = 2;
    const PROC_TTY_ONLY: u32 = 3;
    /// Room for processes started between sizing and filling the PID list.
    const PID_SLACK: usize = 64;
    /// SIGKILL rounds (each followed by `KILL_SETTLE`) before members count as survivors.
    const KILL_ROUNDS: u32 = 100;
    const KILL_SETTLE: Duration = Duration::from_millis(10);

    fn pid(value: u32) -> io::Result<i32> {
        let value = i32::try_from(value).map_err(io::Error::other)?;
        if value == 0 {
            return Err(io::Error::other("PID must be positive"));
        }
        Ok(value)
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
        searchable(path)
    }

    fn searchable(path: &Path) -> io::Result<()> {
        let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| io::Error::other("invalid command cwd"))?;
        if unsafe { libc::access(path.as_ptr(), libc::X_OK) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    unsafe extern "C" {
        /// libSystem's libnotify; any call first waits for its globals to be initialised.
        fn notify_is_valid_token(token: libc::c_int) -> bool;
    }

    /// Call on the forking thread just before every fork. macOS runs libnotify's fork handler
    /// in the child, which traps (SIGILL) or hangs if another thread was initialising libnotify
    /// at the fork; touching libnotify here first waits any such initialisation out.
    pub fn before_fork() {
        unsafe { notify_is_valid_token(0) };
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
            let result = unsafe { libc::waitpid(pid, status, options) };
            if result != -1 {
                return Ok(result);
            }
            crate::syscall::retry_if_interrupted(io::Error::last_os_error())?;
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
    #[repr(i32)]
    pub enum Signal {
        Hangup = libc::SIGHUP,
        Interrupt = libc::SIGINT,
        Terminate = libc::SIGTERM,
        Kill = libc::SIGKILL,
    }

    impl Signal {
        fn raw(self) -> i32 {
            self as i32
        }
    }

    /// Group 1 is launchd's and 0 means the caller's own group; refusing both also keeps
    /// `kill(-1)` (every permitted process) unreachable from here.
    pub fn signal_group(group: u32, signal: Signal) -> io::Result<()> {
        let group = i32::try_from(group).map_err(io::Error::other)?;
        if group <= 1 {
            return Err(io::Error::other("invalid process group"));
        }
        deliver(-group, signal)
    }

    fn deliver(target: i32, signal: Signal) -> io::Result<()> {
        vanished_is_ok(unsafe { libc::kill(target, signal.raw()) })
    }

    /// A `kill(2)` result; a vanished target is not an error.
    fn vanished_is_ok(result: i32) -> io::Result<()> {
        if result == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }

    /// A terminal device (e.g. a PTY slave), identified by its device number.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct Terminal(u32);

    impl Terminal {
        /// Only a character device names a terminal; anything else is refused.
        pub fn from_path(path: &Path) -> io::Result<Self> {
            let metadata = std::fs::metadata(path)?;
            if !metadata.file_type().is_char_device() {
                return Err(io::Error::other("not a terminal device"));
            }
            u32::try_from(metadata.rdev())
                .map(Self)
                .map_err(io::Error::other)
        }
    }

    /// The terminal session a command leads (its session ID is the leader's PID) on its PTY,
    /// and the job-control process groups seen in it. macOS detaches the terminal from the
    /// session when the leader exits, so `observe` must record job groups while it lives.
    #[derive(Debug)]
    pub struct TerminalSession {
        id: i32,
        terminal: Terminal,
        groups: Vec<NonZeroU32>,
    }

    impl TerminalSession {
        pub fn leader(&self) -> u32 {
            self.id.unsigned_abs()
        }

        /// Looks at the terminal at once, so the leader's own group is recorded while the
        /// leader is still on it: members left in that group are killed even if the leader
        /// exits before any later look.
        pub fn new(leader: u32, terminal: Terminal) -> io::Result<Self> {
            let mut session = Self {
                id: pid(leader)?,
                terminal,
                groups: Vec::new(),
            };
            session.observe()?;
            Ok(session)
        }

        /// Records the process groups of members currently on the terminal.
        pub fn observe(&mut self) -> io::Result<()> {
            for member in self.members(listed_pids(PROC_TTY_ONLY, self.terminal.0)?) {
                if let Some(group) = group_of(member)
                    && !self.groups.contains(&group)
                {
                    self.groups.push(group);
                }
            }
            Ok(())
        }

        /// SIGKILLs, in rounds until none remain, every process of this session that is on its
        /// terminal or in a recorded job group. Candidates come only from those kernel lists and
        /// must also be in this session, so processes that started their own session, other
        /// terminals and the calling process are never signalled. Works after the leader was
        /// reaped; only the caller's own children (e.g. the leader) are left to reap.
        pub fn kill(&mut self) -> io::Result<()> {
            self.observe()?;
            self.kill_rounds()
        }

        fn kill_rounds(&self) -> io::Result<()> {
            for _ in 0..KILL_ROUNDS {
                if !self.kill_members()? {
                    return Ok(());
                }
                std::thread::sleep(KILL_SETTLE);
            }
            Err(io::Error::other("session processes survived SIGKILL"))
        }

        /// Returns whether any member was found (and signalled).
        fn kill_members(&self) -> io::Result<bool> {
            let members = self.members(self.candidates()?);
            for member in &members {
                kill_process(*member)?;
            }
            Ok(!members.is_empty())
        }

        fn candidates(&self) -> io::Result<Vec<i32>> {
            let mut candidates = listed_pids(PROC_TTY_ONLY, self.terminal.0)?;
            for group in &self.groups {
                candidates.extend(listed_pids(PROC_PGRP_ONLY, group.get())?);
            }
            Ok(candidates)
        }

        fn members(&self, candidates: Vec<i32>) -> Vec<NonZeroU32> {
            let caller = std::process::id();
            candidates
                .into_iter()
                .filter_map(|pid| u32::try_from(pid).ok().and_then(NonZeroU32::new))
                .filter(|pid| pid.get() != caller && in_session(*pid, self.id))
                .collect()
        }
    }

    /// `getsid` fails for zombies, which are already dead and only await their parent's reap.
    fn in_session(pid: NonZeroU32, session: i32) -> bool {
        unsafe { libc::getsid(pid.get() as i32) == session }
    }

    fn group_of(member: NonZeroU32) -> Option<NonZeroU32> {
        let group = unsafe { libc::getpgid(member.get() as i32) };
        u32::try_from(group).ok().and_then(NonZeroU32::new)
    }

    /// Takes a non-zero PID, so neither `kill(0)` (the caller's group) nor a negative target
    /// (a group, or `-1`: every permitted process) is expressible here.
    fn kill_process(member: NonZeroU32) -> io::Result<()> {
        let member = i32::try_from(member.get()).map_err(io::Error::other)?;
        vanished_is_ok(unsafe { libc::kill(member, libc::SIGKILL) })
    }

    /// The kernel's list of processes of one terminal or one process group.
    fn listed_pids(kind: u32, which: u32) -> io::Result<Vec<i32>> {
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

    /// Caller closes every PTY-master handle here. Even if the leader exits, every
    /// process still in its terminal session gets the full grace period, then SIGKILL.
    pub fn hangup(
        session: &mut TerminalSession,
        close_master: impl FnOnce(),
    ) -> io::Result<ExitStatus> {
        session.observe()?;
        close_master();
        std::thread::sleep(HANGUP_GRACE);
        let leader = session.leader();
        let exited = try_wait(leader)?;
        session.kill()?;
        leader_status(leader, exited)
    }

    fn leader_status(leader: u32, exited: Option<ExitStatus>) -> io::Result<ExitStatus> {
        match exited {
            Some(status) => Ok(status),
            None => wait(leader),
        }
    }
}
