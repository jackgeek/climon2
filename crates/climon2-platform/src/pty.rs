//! PTY allocation serialization and master I/O. Concurrent `posix_openpt` calls on Darwin can
//! fail with the kernel-internal EREDRIVEOPEN (-6), so every climon2 process
//! (and test harness) holds this per-user lock only while allocating a pair.
//! On Windows the session's terminal is a pseudoconsole of the bundled
//! ConPTY instead (#317; ADR 0003, "Windows I/O and transport").
pub use native::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use unix as native;

#[cfg(windows)]
#[path = "pty/windows.rs"]
mod native;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use std::{
        fs::{File, OpenOptions},
        io,
        os::{
            fd::{AsRawFd, BorrowedFd},
            unix::fs::OpenOptionsExt,
        },
        path::{Path, PathBuf},
    };

    /// Held only while a PTY pair is being allocated; dropping releases the lock.
    pub struct Allocation(File);

    impl Drop for Allocation {
        fn drop(&mut self) {
            // Unlocking the owned descriptor cannot affect any other file; close would also release it.
            unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
        }
    }

    fn lock_path() -> io::Result<PathBuf> {
        let directory = PathBuf::from(format!("/tmp/climon2-{}-pty", crate::peer::effective_uid()));
        crate::private::private_directory(&directory)?;
        Ok(directory.join("allocation.lock"))
    }

    fn lock(file: &File) -> io::Result<()> {
        // flock only reads the descriptor owned by `file`.
        while unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            crate::syscall::retry_if_interrupted(io::Error::last_os_error())?;
        }
        Ok(())
    }

    pub fn allocation() -> io::Result<Allocation> {
        locked(&lock_path()?)
    }

    /// The first allocation after boot creates the lock file; nothing is ever written to it.
    fn locked(path: &Path) -> io::Result<Allocation> {
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        lock(&file)?;
        Ok(Allocation(file))
    }

    /// Makes a PTY master's reads and writes nonblocking, keeping its other status flags.
    pub fn set_nonblocking(master: BorrowedFd<'_>) -> io::Result<()> {
        let fd = master.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// A master read failing because the command's side of the terminal is
    /// gone (EIO): the end of its output.
    pub fn output_ended(error: &io::Error) -> bool {
        error.raw_os_error() == Some(libc::EIO)
    }

    #[cfg(test)]
    mod tests {
        use std::os::fd::{AsFd, AsRawFd};

        #[test]
        fn the_first_allocation_creates_its_lock_file() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("allocation.lock");
            drop(super::locked(&path).unwrap());
            assert!(path.is_file());
        }

        /// A read-only descriptor's status flags read as zero, which is not a failure.
        #[test]
        fn a_read_only_descriptor_becomes_nonblocking() {
            let (reader, _writer) = std::io::pipe().unwrap();
            let fd = reader.as_raw_fd();
            assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFL) }, libc::O_RDONLY);
            super::set_nonblocking(reader.as_fd()).unwrap();
            assert_eq!(
                unsafe { libc::fcntl(fd, libc::F_GETFL) },
                libc::O_RDONLY | libc::O_NONBLOCK
            );
        }

        #[test]
        fn only_a_lost_terminal_ends_the_output() {
            assert!(super::output_ended(&std::io::Error::from_raw_os_error(
                libc::EIO
            )));
            assert!(!super::output_ended(&std::io::ErrorKind::WouldBlock.into()));
            assert!(!super::output_ended(&std::io::Error::other("bad")));
        }
    }
}
