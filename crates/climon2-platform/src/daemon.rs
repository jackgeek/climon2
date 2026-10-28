//! Exec-based detached daemon startup with a bounded inherited readiness pipe.
#[cfg(not(target_os = "macos"))]
compile_error!("daemon spawn and readiness seam requires macOS");

#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "macos")]
mod macos {
    use climon2_core::protocol::STARTUP_TIMEOUT;
    use std::{
        fs::File,
        io::{self, Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::process::CommandExt,
        },
        path::Path,
        process::{Child, Command, Stdio},
        time::Instant,
    };

    const MAX_MESSAGE: usize = 4096;

    #[derive(Debug, Eq, PartialEq)]
    pub enum Readiness {
        Ready,
        Failed { code: u32, message: String },
    }

    pub struct ReadinessWriter(File);

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
        let child = command.spawn()?;
        drop(writer);
        Ok(PendingDaemon {
            child,
            reader,
            armed: true,
            started,
        })
    }

    /// Runs in the forked child: a new session, and the readiness pipe survives exec.
    fn detach_on_exec(command: &mut Command, descriptor: i32) {
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                close_on_exec(descriptor, false)
            });
        }
    }

    pub(crate) fn pipe() -> io::Result<(File, File)> {
        let mut fds = [-1; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        let (reader, writer) = unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) };
        close_on_exec(reader.as_raw_fd(), true)?;
        close_on_exec(writer.as_raw_fd(), true)?;
        Ok((reader, writer))
    }

    fn close_on_exec(fd: i32, enabled: bool) -> io::Result<()> {
        let flags = if enabled { libc::FD_CLOEXEC } else { 0 };
        if unsafe { libc::fcntl(fd, libc::F_SETFD, flags) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub struct PendingDaemon {
        child: Child,
        reader: File,
        armed: bool,
        started: Instant,
    }

    impl PendingDaemon {
        pub fn id(&self) -> u32 {
            self.child.id()
        }

        /// Supply metadata then socket as cleanup paths. Success leaves the
        /// daemon running; every failure kills/reaps it before deleting files.
        pub fn wait_ready(mut self, cleanup: &[&Path]) -> io::Result<Readiness> {
            let result = read_readiness(&mut self.reader, self.started + STARTUP_TIMEOUT);
            if matches!(result, Ok(Readiness::Ready)) {
                self.armed = false;
                return result;
            }
            self.abort();
            for path in cleanup {
                let _ = std::fs::remove_file(path);
            }
            result
        }

        fn abort(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.armed = false;
        }
    }

    impl Drop for PendingDaemon {
        fn drop(&mut self) {
            if self.armed {
                self.abort();
            }
        }
    }

    fn read_readiness(reader: &mut File, deadline: Instant) -> io::Result<Readiness> {
        let mut tag = [0];
        read_deadline(reader, &mut tag, deadline)?;
        match tag[0] {
            b'R' => Ok(Readiness::Ready),
            b'F' => read_failure(reader, deadline),
            _ => Err(io::Error::other("invalid daemon readiness reply")),
        }
    }

    fn read_failure(reader: &mut File, deadline: Instant) -> io::Result<Readiness> {
        let mut header = [0; 8];
        read_deadline(reader, &mut header, deadline)?;
        let [c0, c1, c2, c3, l0, l1, l2, l3] = header;
        let code = u32::from_be_bytes([c0, c1, c2, c3]);
        valid_code(code)?;
        let length =
            usize::try_from(u32::from_be_bytes([l0, l1, l2, l3])).map_err(io::Error::other)?;
        read_message(reader, code, length, deadline)
    }

    fn read_message(
        reader: &mut File,
        code: u32,
        length: usize,
        deadline: Instant,
    ) -> io::Result<Readiness> {
        if length > MAX_MESSAGE {
            return Err(io::Error::other("readiness failure message too long"));
        }
        let mut message = vec![0; length];
        read_deadline(reader, &mut message, deadline)?;
        let message = String::from_utf8(message)
            .map_err(|_| io::Error::other("invalid readiness message"))?;
        Ok(Readiness::Failed { code, message })
    }

    fn read_deadline(reader: &mut File, mut bytes: &mut [u8], deadline: Instant) -> io::Result<()> {
        while !bytes.is_empty() {
            readable(reader, deadline)?;
            let count = read_once(reader, bytes)?;
            bytes = &mut bytes[count..];
        }
        Ok(())
    }

    fn read_once(reader: &mut File, bytes: &mut [u8]) -> io::Result<usize> {
        match reader.read(bytes) {
            Ok(0) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "daemon readiness pipe closed",
            )),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(0),
            result => result,
        }
    }

    fn readable(reader: &File, deadline: Instant) -> io::Result<()> {
        let mut fd = libc::pollfd {
            fd: reader.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        while !poll_once(&mut fd, remaining_millis(deadline)?)? {}
        Ok(())
    }

    fn remaining_millis(deadline: Instant) -> io::Result<i32> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "daemon readiness timed out"))?;
        i32::try_from(remaining.as_millis().max(1)).map_err(io::Error::other)
    }

    fn poll_once(fd: &mut libc::pollfd, milliseconds: i32) -> io::Result<bool> {
        if unsafe { libc::poll(fd, 1, milliseconds) } >= 0 {
            return Ok(fd.revents != 0);
        }
        crate::syscall::retry_if_interrupted(io::Error::last_os_error())?;
        Ok(false)
    }
}
