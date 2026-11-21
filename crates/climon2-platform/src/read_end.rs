//! Read errors that mean the other end is gone (ADR 0003, Windows I/O and
//! transport): a lost terminal ends input like end of file.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("read-end seam requires macOS or Linux");

use std::io;

/// A lost terminal reads as EIO on Unix.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn hung_up(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::EIO)
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::hung_up;
    use std::io;

    #[test]
    fn only_a_lost_terminal_reads_as_hung_up() {
        assert!(hung_up(&io::Error::from_raw_os_error(libc::EIO)));
        assert!(!hung_up(&io::Error::from_raw_os_error(libc::EAGAIN)));
        assert!(!hung_up(&io::Error::other("not an OS error")));
    }
}
