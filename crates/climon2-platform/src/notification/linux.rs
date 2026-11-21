//! Linux notifiers: one `eventfd` each.

use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
};

/// A pollable descriptor the library raises and clears: one `eventfd`,
/// close-on-exec and nonblocking, readable while its counter is nonzero.
/// The notification descriptor and an instance's waker are both one.
pub struct Notifier(OwnedFd);

impl Notifier {
    /// The descriptors one notifier holds: the `eventfd` counts once.
    pub const DESCRIPTORS: u32 = 1;

    pub fn new() -> io::Result<Self> {
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
    pub fn waker() -> io::Result<Self> {
        Self::new()
    }

    pub fn descriptor(&self) -> RawFd {
        self.0.as_raw_fd()
    }

    /// Raises the descriptor, or clears it.
    pub fn set(&self, readable: bool) {
        if readable {
            self.raise();
        } else {
            self.clear();
        }
    }

    /// Adds one to the counter, making the descriptor readable.
    pub fn raise(&self) {
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
    pub fn clear(&self) {
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
