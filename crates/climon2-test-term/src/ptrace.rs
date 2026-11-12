//! Stops a test's child process at chosen system calls (Linux ptrace), so a
//! test can act between two of its system calls deterministically.
use nix::libc;
use std::{
    fs::File,
    os::unix::fs::FileExt,
    path::PathBuf,
    time::{Duration, Instant},
};

/// A system call the tracee is about to make.
#[derive(Clone, Copy, Debug)]
pub struct Syscall {
    pub number: u64,
    pub arguments: [u64; 6],
}

pub struct Tracer {
    pid: i32,
    /// Bounds the whole trace: a tracee that never makes the awaited call fails.
    deadline: Instant,
}

impl Tracer {
    /// Attaches to a child of the test and stops it wherever it is.
    pub fn seize(pid: u32, bound: Duration) -> Self {
        let tracer = Self {
            pid: i32::try_from(pid).unwrap(),
            deadline: Instant::now() + bound,
        };
        tracer.request(libc::PTRACE_SEIZE, 0, libc::PTRACE_O_TRACESYSGOOD as usize);
        tracer.request(libc::PTRACE_INTERRUPT, 0, 0);
        tracer.wait();
        tracer
    }

    /// Resumes until the tracee enters its next system call.
    pub fn next_entry(&self) -> Syscall {
        loop {
            let info = self.next_syscall_stop();
            if info.op == libc::PTRACE_SYSCALL_INFO_ENTRY {
                let entry = unsafe { info.u.entry };
                return Syscall {
                    number: entry.nr,
                    arguments: entry.args,
                };
            }
        }
    }

    /// Runs the system call the tracee entered to its exit and returns its result.
    pub fn finish_syscall(&self) -> i64 {
        loop {
            let info = self.next_syscall_stop();
            if info.op == libc::PTRACE_SYSCALL_INFO_EXIT {
                return unsafe { info.u.exit }.sval;
            }
        }
    }

    /// What one of the tracee's descriptors refers to, if it is open.
    pub fn descriptor(&self, fd: u64) -> Option<PathBuf> {
        std::fs::read_link(format!("/proc/{}/fd/{fd}", self.pid)).ok()
    }

    /// Bytes of the tracee's memory, such as a write's buffer.
    pub fn memory(&self, address: u64, length: u64) -> Vec<u8> {
        let mut bytes = vec![0; usize::try_from(length).unwrap()];
        File::open(format!("/proc/{}/mem", self.pid))
            .unwrap()
            .read_exact_at(&mut bytes, address)
            .unwrap();
        bytes
    }

    /// Lets the tracee continue untraced, including any stopped system call.
    pub fn detach(self) {
        self.request(libc::PTRACE_DETACH, 0, 0);
    }

    fn request(&self, request: libc::c_uint, address: usize, data: usize) -> libc::c_long {
        let result = unsafe { libc::ptrace(request, self.pid, address, data) };
        assert!(result >= 0, "{}", std::io::Error::last_os_error());
        result
    }

    /// Resumes to the next system-call stop, passing real signals on.
    fn next_syscall_stop(&self) -> libc::ptrace_syscall_info {
        let mut signal = 0;
        loop {
            self.request(libc::PTRACE_SYSCALL, 0, signal);
            let status = self.wait();
            signal = 0;
            if libc::WSTOPSIG(status) == libc::SIGTRAP | 0x80 {
                let mut info: libc::ptrace_syscall_info = unsafe { std::mem::zeroed() };
                self.request(
                    libc::PTRACE_GET_SYSCALL_INFO,
                    std::mem::size_of_val(&info),
                    (&raw mut info) as usize,
                );
                return info;
            }
            // Event stops (status >> 16) carry no signal to deliver.
            if status >> 16 == 0 {
                signal = libc::WSTOPSIG(status) as usize;
            }
        }
    }

    fn wait(&self) -> i32 {
        loop {
            assert!(
                Instant::now() < self.deadline,
                "tracee never made the traced call"
            );
            let mut status = 0;
            let waited =
                unsafe { libc::waitpid(self.pid, &mut status, libc::__WALL | libc::WNOHANG) };
            if waited == self.pid {
                assert!(libc::WIFSTOPPED(status), "tracee ended while traced");
                return status;
            }
            assert_eq!(waited, 0, "{}", std::io::Error::last_os_error());
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
