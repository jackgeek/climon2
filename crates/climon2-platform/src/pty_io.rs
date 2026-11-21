//! The daemon's I/O on a command terminal's master: nonblocking reads and
//! writes, waiting for output, and recognising the end of output.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("PTY I/O seam requires macOS or Linux");

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use std::{fs::File, io, os::fd::AsRawFd, time::Duration};

    /// Reads and writes on `master` return `WouldBlock` instead of waiting.
    pub fn set_nonblocking(master: &File) -> io::Result<()> {
        let fd = master.as_raw_fd();
        // fcntl only reads and updates the status flags of the descriptor `master` owns.
        let flags = crate::syscall::checked(unsafe { libc::fcntl(fd, libc::F_GETFL) })?;
        crate::syscall::checked(unsafe {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK)
        })?;
        Ok(())
    }

    /// Waits at most `timeout` for output (or hang-up) without reading it. An
    /// interrupted or failed wait only ends early; the next read reports errors.
    pub fn wait_readable(master: &File, timeout: Duration) {
        let mut descriptor = libc::pollfd {
            fd: master.as_raw_fd(),
            events: libc::POLLIN,
            // Output-only.
            revents: Default::default(),
        };
        let milliseconds = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
        // The single pollfd remains valid for the duration of this call.
        unsafe { libc::poll(&mut descriptor, 1, milliseconds) };
    }

    /// Whether a failed read means every slave handle closed: `EIO`.
    pub fn is_end_of_output(error: &io::Error) -> bool {
        error.raw_os_error() == Some(libc::EIO)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{io::Write, time::Instant};

        fn status_flags(file: &impl AsRawFd) -> libc::c_int {
            unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) }
        }

        /// A read-only descriptor's status flags read as zero, which is not a failure.
        #[test]
        fn a_read_only_descriptor_becomes_nonblocking() {
            let (reader, _writer) = io::pipe().unwrap();
            let reader = File::from(std::os::fd::OwnedFd::from(reader));
            assert_eq!(status_flags(&reader), libc::O_RDONLY);
            set_nonblocking(&reader).unwrap();
            assert_eq!(status_flags(&reader), libc::O_RDONLY | libc::O_NONBLOCK);
        }

        /// Asking again keeps it nonblocking and adds nothing else.
        #[test]
        fn a_nonblocking_descriptor_stays_nonblocking() {
            let (_reader, writer) = io::pipe().unwrap();
            let writer = File::from(std::os::fd::OwnedFd::from(writer));
            set_nonblocking(&writer).unwrap();
            set_nonblocking(&writer).unwrap();
            assert_eq!(status_flags(&writer), libc::O_WRONLY | libc::O_NONBLOCK);
        }

        #[test]
        fn an_invalid_descriptor_cannot_become_nonblocking() {
            use std::os::fd::FromRawFd;
            // No process can hold this descriptor; ManuallyDrop never closes it.
            let invalid =
                std::mem::ManuallyDrop::new(unsafe { File::from_raw_fd(libc::c_int::MAX) });
            assert_eq!(
                set_nonblocking(&invalid).unwrap_err().raw_os_error(),
                Some(libc::EBADF)
            );
        }

        #[test]
        fn waiting_without_output_lasts_the_timeout() {
            let (reader, _writer) = io::pipe().unwrap();
            let reader = File::from(std::os::fd::OwnedFd::from(reader));
            let started = Instant::now();
            wait_readable(&reader, Duration::from_millis(50));
            assert!(started.elapsed() >= Duration::from_millis(50));
        }

        #[test]
        fn waiting_ends_once_output_is_ready() {
            let (reader, mut writer) = io::pipe().unwrap();
            let reader = File::from(std::os::fd::OwnedFd::from(reader));
            writer.write_all(b"x").unwrap();
            let started = Instant::now();
            wait_readable(&reader, Duration::from_secs(60));
            assert!(started.elapsed() < Duration::from_secs(60));
        }

        #[test]
        fn only_eio_ends_output() {
            assert!(is_end_of_output(&io::Error::from_raw_os_error(libc::EIO)));
            assert!(!is_end_of_output(&io::Error::from_raw_os_error(
                libc::EAGAIN
            )));
            assert!(!is_end_of_output(&io::Error::other("no OS error")));
        }
    }
}
