//! macOS marks each socket `SO_NOSIGPIPE`.

use std::{io, os::fd::RawFd};

/// Makes a closed peer fail a send with `EPIPE` instead of raising
/// `SIGPIPE`.
pub fn prepare(socket: RawFd) -> io::Result<()> {
    let on: libc::c_int = 1;
    // SAFETY: the option value covers its length.
    crate::syscall::checked(unsafe {
        libc::setsockopt(
            socket,
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            (&raw const on).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    })
    .map(|_| ())
}

/// Sends what the socket takes now; `prepare` made it never raise `SIGPIPE`.
pub fn send(socket: RawFd, bytes: &[u8]) -> io::Result<usize> {
    // SAFETY: the buffer covers its length.
    let sent = unsafe { libc::send(socket, bytes.as_ptr().cast(), bytes.len(), 0) };
    usize::try_from(sent).map_err(|_| io::Error::last_os_error())
}
