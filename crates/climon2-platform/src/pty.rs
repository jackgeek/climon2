//! Darwin PTY allocation serialization. Concurrent `posix_openpt` calls can
//! fail with the kernel-internal EREDRIVEOPEN (-6), so every climon2 process
//! (and test harness) holds this per-user lock only while allocating a pair.
#[cfg(not(target_os = "macos"))]
compile_error!("PTY allocation seam requires macOS");

#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "macos")]
mod macos {
    use std::{
        fs::{File, OpenOptions},
        io,
        os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
        path::PathBuf,
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
        crate::runtime::private_directory(&directory)?;
        Ok(directory.join("allocation.lock"))
    }

    fn lock(file: &File) -> io::Result<()> {
        loop {
            // flock only reads the descriptor owned by `file`.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }

    pub fn allocation() -> io::Result<Allocation> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(lock_path()?)?;
        lock(&file)?;
        Ok(Allocation(file))
    }
}
