//! Bounded waits on child processes, so a wedged child fails its test instead of hanging it.
use nix::{
    libc,
    sys::signal::{self, Signal},
    unistd::Pid,
};
use std::{
    io,
    os::unix::process::ExitStatusExt,
    process::{Child, ExitStatus},
    thread,
    time::{Duration, Instant},
};

/// A child of this process, which only this process can reap: a spawned `Child`, or the PID
/// of a child spawned another way, such as a session daemon.
pub trait ChildProcess {
    fn pid(&self) -> u32;
}

impl ChildProcess for Child {
    fn pid(&self) -> u32 {
        self.id()
    }
}

impl ChildProcess for u32 {
    fn pid(&self) -> u32 {
        *self
    }
}

/// Reaps `child` once it exits, polling until `bound` passes. A child still running then is
/// killed and reaped, and `None` is returned.
pub fn exit_within(child: impl ChildProcess, bound: Duration) -> Option<ExitStatus> {
    let pid = child.pid();
    let deadline = Instant::now() + bound;
    while Instant::now() < deadline {
        if let Some(status) = reap(pid, libc::WNOHANG) {
            return Some(status);
        }
        thread::sleep(Duration::from_millis(5));
    }
    signal::kill(Pid::from_raw(pid as i32), Signal::SIGKILL).expect("kill a wedged child");
    reap(pid, 0);
    None
}

/// The child's status once it has exited, or `None` while `WNOHANG` finds it running.
fn reap(pid: u32, options: libc::c_int) -> Option<ExitStatus> {
    let mut status = 0;
    loop {
        match unsafe { libc::waitpid(pid as libc::pid_t, &mut status, options) } {
            0 => return None,
            -1 => {
                let error = io::Error::last_os_error();
                assert_eq!(
                    error.kind(),
                    io::ErrorKind::Interrupted,
                    "wait for {pid}: {error}"
                );
            }
            _ => return Some(ExitStatus::from_raw(status)),
        }
    }
}
