//! macOS-only library code.

use std::{
    io,
    os::fd::{AsRawFd, OwnedFd, RawFd},
};

/// A pollable descriptor the library raises and clears: the read end of a
/// close-on-exec pipe, readable while it holds a byte.
pub(super) struct Notifier {
    reader: OwnedFd,
    writer: OwnedFd,
}

impl Notifier {
    /// The descriptors one notifier holds: both ends of the pipe.
    pub(super) const DESCRIPTORS: u32 = 2;

    /// The notification descriptor: the library writes and reads its one
    /// byte only when its tracked readiness changes, so neither end blocks.
    pub(super) fn new() -> io::Result<Self> {
        let (reader, writer) = io::pipe()?;
        Ok(Self {
            reader: reader.into(),
            writer: writer.into(),
        })
    }

    /// A waker: raised by any thread, as often as it likes, and cleared by
    /// its one waiter. Both ends are nonblocking, so a full pipe (already
    /// readable) only fails a write, and an empty one only fails a read.
    pub(super) fn waker() -> io::Result<Self> {
        let notifier = Self::new()?;
        for descriptor in [notifier.reader.as_raw_fd(), notifier.writer.as_raw_fd()] {
            nonblocking(descriptor)?;
        }
        Ok(notifier)
    }

    pub(super) fn descriptor(&self) -> RawFd {
        self.reader.as_raw_fd()
    }

    /// Raises the descriptor, or clears it.
    pub(super) fn set(&self, readable: bool) {
        if readable {
            self.raise();
        } else {
            self.clear();
        }
    }

    /// Writes a byte, making the descriptor readable.
    pub(super) fn raise(&self) {
        let byte = [u8::MAX];
        // SAFETY: the buffer holds the one byte written.
        unsafe { libc::write(self.writer.as_raw_fd(), byte.as_ptr().cast(), byte.len()) };
    }

    /// Reads back what was written, clearing the descriptor.
    pub(super) fn clear(&self) {
        let mut bytes = [u8::MAX; 64];
        // SAFETY: the buffer holds every byte read.
        unsafe {
            libc::read(
                self.reader.as_raw_fd(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
    }
}

fn nonblocking(descriptor: RawFd) -> io::Result<()> {
    // SAFETY (both): fcntl on an open descriptor takes no pointers.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    status(unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) })
}

/// A call's status: success, or the error it set.
fn status(result: libc::c_int) -> io::Result<()> {
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Makes a closed peer fail a send with `EPIPE` instead of raising
/// `SIGPIPE` in the embedder's process, whose signal handling the library
/// never changes.
pub(super) fn quiet(socket: RawFd) -> io::Result<()> {
    let on: libc::c_int = 1;
    // SAFETY: the option value covers its length.
    status(unsafe {
        libc::setsockopt(
            socket,
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            (&raw const on).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    })
}

/// Sends what the socket takes now; `quiet` made it never raise `SIGPIPE`.
pub(super) fn send(socket: RawFd, bytes: &[u8]) -> io::Result<usize> {
    // SAFETY: the buffer covers its length.
    let sent = unsafe { libc::send(socket, bytes.as_ptr().cast(), bytes.len(), 0) };
    usize::try_from(sent).map_err(|_| io::Error::last_os_error())
}
