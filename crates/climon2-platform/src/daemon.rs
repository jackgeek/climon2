//! Detached daemon startup with a bounded inherited readiness pipe: on Unix
//! by exec, on Windows by one `CreateProcessW` (#317,
//! docs/windows-lifecycle.md, "Starting the daemon"). The readiness reply is
//! the same bytes on both.
use std::{
    fs::File,
    io::{self, Write},
};

pub use native::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use unix as native;

#[cfg(windows)]
#[path = "daemon/windows.rs"]
mod native;

const MAX_MESSAGE: usize = 4096;

#[derive(Debug, Eq, PartialEq)]
pub enum Readiness {
    Ready,
    Failed { code: u32, message: String },
}

pub struct ReadinessWriter(File);

impl ReadinessWriter {
    pub fn send(&mut self, readiness: &Readiness) -> io::Result<()> {
        let bytes = encode(readiness)?;
        self.0.write_all(&bytes)
    }
}

fn encode(readiness: &Readiness) -> io::Result<Vec<u8>> {
    let Readiness::Failed { code, message } = readiness else {
        return Ok(vec![b'R']);
    };
    valid_code(*code)?;
    if message.len() > MAX_MESSAGE {
        return Err(io::Error::other("readiness failure message too long"));
    }
    let mut bytes = vec![b'F'];
    bytes.extend(code.to_be_bytes());
    bytes.extend(
        u32::try_from(message.len())
            .map_err(io::Error::other)?
            .to_be_bytes(),
    );
    bytes.extend(message.as_bytes());
    Ok(bytes)
}

fn valid_code(code: u32) -> io::Result<()> {
    if !matches!(code, 125..=127) {
        return Err(io::Error::other("invalid readiness failure code"));
    }
    Ok(())
}

/// One reply, read through `read`, which fills its whole buffer or fails.
fn decode(read: &mut impl FnMut(&mut [u8]) -> io::Result<()>) -> io::Result<Readiness> {
    let mut tag = [u8::default()];
    read(&mut tag)?;
    match tag[0] {
        b'R' => Ok(Readiness::Ready),
        b'F' => decode_failure(read),
        _ => Err(io::Error::other("invalid daemon readiness reply")),
    }
}

fn decode_failure(read: &mut impl FnMut(&mut [u8]) -> io::Result<()>) -> io::Result<Readiness> {
    let mut header = [u8::default(); 8];
    read(&mut header)?;
    let [c0, c1, c2, c3, l0, l1, l2, l3] = header;
    let code = u32::from_be_bytes([c0, c1, c2, c3]);
    valid_code(code)?;
    let length = usize::try_from(u32::from_be_bytes([l0, l1, l2, l3])).map_err(io::Error::other)?;
    decode_message(read, code, length)
}

fn decode_message(
    read: &mut impl FnMut(&mut [u8]) -> io::Result<()>,
    code: u32,
    length: usize,
) -> io::Result<Readiness> {
    if length > MAX_MESSAGE {
        return Err(io::Error::other("readiness failure message too long"));
    }
    let mut message = vec![u8::default(); length];
    read(&mut message)?;
    let message =
        String::from_utf8(message).map_err(|_| io::Error::other("invalid readiness message"))?;
    Ok(Readiness::Failed { code, message })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use super::{Readiness, ReadinessWriter};
    use crate::waiting::{self, Interest, Watch};
    use climon2_core::protocol::STARTUP_TIMEOUT;
    use std::{
        fs::File,
        io::{self, Read},
        os::{
            fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
            unix::process::CommandExt,
        },
        path::Path,
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };

    impl ReadinessWriter {
        /// # Safety
        /// `fd` must be the inherited readiness pipe, owned exclusively by the
        /// caller and not represented by another Rust I/O owner.
        pub unsafe fn from_inherited(fd: u32) -> io::Result<Self> {
            let fd = i32::try_from(fd).map_err(io::Error::other)?;
            if fd < 3 {
                return Err(io::Error::other("invalid readiness descriptor"));
            }
            close_on_exec(fd, true)?;
            Ok(Self(unsafe { File::from_raw_fd(fd) }))
        }
    }

    /// Configure the daemon's readiness-fd argument in `configure`, never the
    /// eventual PTY command's environment. stderr is preserved for diagnostics.
    pub fn spawn(
        mut command: Command,
        configure: impl FnOnce(&mut Command, u32),
    ) -> io::Result<PendingDaemon> {
        let (reader, writer) = pipe()?;
        let descriptor = writer.as_raw_fd();
        configure(
            &mut command,
            u32::try_from(descriptor).map_err(io::Error::other)?,
        );
        command.stdin(Stdio::null()).stdout(Stdio::null());
        detach_on_exec(&mut command, descriptor);
        let started = Instant::now();
        crate::process::before_fork();
        let child = command.spawn()?;
        drop(writer);
        Ok(PendingDaemon {
            pid: child.id(),
            child: Some(child),
            reader,
            started,
        })
    }

    /// Runs in the forked child: a new session, and the readiness pipe survives exec.
    fn detach_on_exec(command: &mut Command, descriptor: i32) {
        unsafe {
            command.pre_exec(move || {
                crate::syscall::checked(libc::setsid())?;
                close_on_exec(descriptor, false)
            });
        }
    }

    /// Both ends close on exec.
    pub(crate) fn pipe() -> io::Result<(File, File)> {
        let (reader, writer) = io::pipe()?;
        Ok((
            File::from(OwnedFd::from(reader)),
            File::from(OwnedFd::from(writer)),
        ))
    }

    fn close_on_exec(fd: i32, enabled: bool) -> io::Result<()> {
        let flags = if enabled { libc::FD_CLOEXEC } else { 0 };
        crate::syscall::checked(unsafe { libc::fcntl(fd, libc::F_SETFD, flags) }).map(drop)
    }

    pub struct PendingDaemon {
        pid: u32,
        /// Present until the daemon is ready or aborted; dropping kills and reaps it.
        child: Option<Child>,
        reader: File,
        started: Instant,
    }

    impl PendingDaemon {
        pub fn id(&self) -> u32 {
            self.pid
        }

        /// Supply metadata then socket as cleanup paths. Success leaves the
        /// daemon running; every failure kills/reaps it before deleting files.
        pub fn wait_ready(mut self, cleanup: &[&Path]) -> io::Result<Readiness> {
            let result = read_readiness(&mut self.reader, self.started + STARTUP_TIMEOUT);
            if matches!(result, Ok(Readiness::Ready)) {
                // A ready daemon runs on: dropping its handle neither kills nor reaps it.
                self.child.take();
                return result;
            }
            self.abort();
            for path in cleanup {
                let _ = std::fs::remove_file(path);
            }
            result
        }

        fn abort(&mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    impl Drop for PendingDaemon {
        fn drop(&mut self) {
            self.abort();
        }
    }

    fn read_readiness(reader: &mut File, deadline: Instant) -> io::Result<Readiness> {
        super::decode(&mut |bytes| read_deadline(reader, bytes, deadline))
    }

    fn read_deadline(reader: &mut File, mut bytes: &mut [u8], deadline: Instant) -> io::Result<()> {
        while !bytes.is_empty() {
            readable(reader, deadline)?;
            let count = read_once(reader, bytes)?;
            bytes = &mut bytes[count..];
        }
        Ok(())
    }

    /// Reads only once poll has reported the pipe readable, so the read cannot block and
    /// no signal can interrupt it. EOF before the whole reply is a failure.
    fn read_once(reader: &mut File, bytes: &mut [u8]) -> io::Result<usize> {
        match reader.read(bytes)? {
            0 => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "daemon readiness pipe closed",
            )),
            count => Ok(count),
        }
    }

    fn readable(reader: &File, deadline: Instant) -> io::Result<()> {
        let mut watches = [Watch::new(reader.as_fd(), Interest::Read)];
        while !poll_once(&mut watches, remaining(deadline)?)? {}
        Ok(())
    }

    /// The wait rounds this up, so the deadline's last fraction of a
    /// millisecond is waited, not spun.
    fn remaining(deadline: Instant) -> io::Result<Duration> {
        deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "daemon readiness timed out"))
    }

    fn poll_once(watches: &mut [Watch<'_>; 1], timeout: Duration) -> io::Result<bool> {
        match waiting::wait(watches, timeout) {
            Ok(_) => Ok(watches[0].ready()),
            Err(error) => crate::syscall::retry_if_interrupted(error).map(|()| false),
        }
    }
}
