//! Nonblocking terminal I/O for the length of a bridge (ADR 0003, Windows
//! I/O and transport): the client's shared tick loop never blocks on its
//! terminal input or output.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("nonblocking terminal seam requires macOS or Linux");

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use crate::syscall::checked;
    use std::{
        fs::File,
        io,
        os::fd::{AsFd, AsRawFd, OwnedFd},
    };

    /// The single owner of the terminal's file-status flags while bridging. Both originals
    /// are captured before either changes, because stdin and stdout may share one open-file
    /// description, and are restored through owned duplicates, so closing the bridge's
    /// descriptors can never skip or misdirect restoration.
    pub struct Nonblocking {
        saved: [(OwnedFd, i32); 2],
    }

    impl Nonblocking {
        pub fn new(input: &File, output: &File) -> io::Result<Self> {
            let guard = Self {
                saved: [save(input)?, save(output)?],
            };
            guard.apply()?;
            Ok(guard)
        }

        fn apply(&self) -> io::Result<()> {
            for (fd, flags) in &self.saved {
                set_flags(fd, flags | libc::O_NONBLOCK)?;
            }
            Ok(())
        }
    }

    impl Drop for Nonblocking {
        fn drop(&mut self) {
            for (fd, flags) in self.saved.iter().rev() {
                let _ = set_flags(fd, *flags);
            }
        }
    }

    fn save(file: &File) -> io::Result<(OwnedFd, i32)> {
        let flags = checked(unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) })?;
        Ok((file.as_fd().try_clone_to_owned()?, flags))
    }

    fn set_flags(fd: &OwnedFd, flags: i32) -> io::Result<()> {
        checked(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags) }).map(drop)
    }
}
