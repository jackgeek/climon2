//! Linux-only library code.
//!
//! The Linux client library links the GCC unwinder statically. Unwinding never
//! leaves the library, because no unwinding crosses its C ABI, so this private
//! copy lets `libclimon2_client.so` need no shared library beyond glibc's,
//! where Rust's standard library otherwise needs the shared GCC runtime
//! `libgcc_s.so.1` (library embedding).
//!
//! This is the library target's own native library, so the linker meets it
//! before the standard library's `-lgcc_s` and never needs the shared runtime.
#[link(name = "gcc_eh", kind = "static", modifiers = "-bundle")]
unsafe extern "C" {}

use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
};

/// A pollable descriptor the library raises and clears: one `eventfd`,
/// close-on-exec and nonblocking, readable while its counter is nonzero.
/// The notification descriptor and an instance's waker are both one.
pub(super) struct Notifier(OwnedFd);

impl Notifier {
    /// The descriptors one notifier holds: the `eventfd` counts once.
    pub(super) const DESCRIPTORS: u32 = 1;

    pub(super) fn new() -> io::Result<Self> {
        // SAFETY: eventfd takes no pointers.
        let descriptor = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if descriptor == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a successful eventfd returns a new descriptor nothing else owns.
        Ok(Self(unsafe { OwnedFd::from_raw_fd(descriptor) }))
    }

    /// A waker: raised by any thread, as often as it likes, and cleared by
    /// its one waiter. An `eventfd` never blocks either way.
    pub(super) fn waker() -> io::Result<Self> {
        Self::new()
    }

    pub(super) fn descriptor(&self) -> RawFd {
        self.0.as_raw_fd()
    }

    /// Raises the descriptor, or clears it.
    pub(super) fn set(&self, readable: bool) {
        if readable {
            self.raise();
        } else {
            self.clear();
        }
    }

    /// Adds one to the counter, making the descriptor readable.
    pub(super) fn raise(&self) {
        let counter: u64 = 1;
        // SAFETY: the buffer holds the eight bytes an eventfd transfers; a
        // full counter only fails the write, it never blocks.
        unsafe {
            libc::write(
                self.descriptor(),
                (&raw const counter).cast::<libc::c_void>(),
                size_of::<u64>(),
            )
        };
    }

    /// Reads the counter back to zero, clearing the descriptor.
    pub(super) fn clear(&self) {
        let mut counter = u64::default();
        // SAFETY: as above; a zero counter only fails the read.
        unsafe {
            libc::read(
                self.descriptor(),
                (&raw mut counter).cast::<libc::c_void>(),
                size_of::<u64>(),
            )
        };
    }
}

/// Sends what the socket takes now. A closed peer fails the send with
/// `EPIPE` instead of raising `SIGPIPE` in the embedder's process, whose
/// signal handling the library never changes.
pub(super) fn send(socket: RawFd, bytes: &[u8]) -> io::Result<usize> {
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
pub(super) fn quiet(_socket: RawFd) -> io::Result<()> {
    Ok(())
}
