//! Linux sends with `MSG_NOSIGNAL`.

use std::{io, os::fd::RawFd};

/// Sends what the socket takes now. A closed peer fails the send with
/// `EPIPE` instead of raising `SIGPIPE`.
pub fn send(socket: RawFd, bytes: &[u8]) -> io::Result<usize> {
    // SAFETY: the buffer covers its length.
    let sent = unsafe {
        libc::send(
            socket,
            bytes.as_ptr().cast(),
            bytes.len(),
            libc::MSG_NOSIGNAL,
        )
    };
    usize::try_from(sent).map_err(|_| io::Error::last_os_error())
}

/// Linux sends with `MSG_NOSIGNAL`, so a socket needs no setup.
pub fn prepare(_socket: RawFd) -> io::Result<()> {
    Ok(())
}
