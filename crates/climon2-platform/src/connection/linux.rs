//! Linux's signal-free socket sends.

use std::{
    io,
    os::fd::{AsRawFd, BorrowedFd},
};

/// Sends what the socket takes now. A closed peer fails the send with
/// `EPIPE` instead of raising `SIGPIPE` in the embedder's process, whose
/// signal handling the library never changes.
pub fn send(socket: BorrowedFd<'_>, bytes: &[u8]) -> io::Result<usize> {
    // SAFETY: the buffer covers its length.
    let sent = unsafe {
        libc::send(
            socket.as_raw_fd(),
            bytes.as_ptr().cast(),
            bytes.len(),
            libc::MSG_NOSIGNAL,
        )
    };
    usize::try_from(sent).map_err(|_| io::Error::last_os_error())
}

/// Linux sends with `MSG_NOSIGNAL`, so a socket needs no setup.
pub fn quiet(_socket: BorrowedFd<'_>) -> io::Result<()> {
    Ok(())
}
