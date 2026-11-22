//! macOS's signal-free socket sends.

use std::{
    io,
    os::fd::{AsRawFd, BorrowedFd},
};

/// Makes a closed peer fail a send with `EPIPE` instead of raising
/// `SIGPIPE` in the embedder's process, whose signal handling the library
/// never changes.
pub fn quiet(socket: BorrowedFd<'_>) -> io::Result<()> {
    let on: libc::c_int = 1;
    // SAFETY: the option value covers its length.
    status(unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            (&raw const on).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    })
}

/// A call's status: success, or the error it set.
fn status(result: libc::c_int) -> io::Result<()> {
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Sends what the socket takes now; `quiet` made it never raise `SIGPIPE`.
pub fn send(socket: BorrowedFd<'_>, bytes: &[u8]) -> io::Result<usize> {
    // SAFETY: the buffer covers its length.
    let sent = unsafe { libc::send(socket.as_raw_fd(), bytes.as_ptr().cast(), bytes.len(), 0) };
    usize::try_from(sent).map_err(|_| io::Error::last_os_error())
}
