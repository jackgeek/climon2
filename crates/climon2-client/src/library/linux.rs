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

/// The pollable notification descriptor: one `eventfd`, close-on-exec and
/// nonblocking, readable while its counter is nonzero.
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

    pub(super) fn descriptor(&self) -> RawFd {
        self.0.as_raw_fd()
    }

    /// Adds one to the counter, making the descriptor readable, or reads the
    /// counter back to zero, clearing it.
    pub(super) fn set(&self, readable: bool) {
        let mut counter = u64::from(readable);
        let buffer = (&raw mut counter).cast::<libc::c_void>();
        let size = size_of::<u64>();
        // SAFETY: the buffer holds the eight bytes an eventfd transfers. The
        // instance tracks readiness, so neither call can block or overflow.
        unsafe {
            if readable {
                libc::write(self.descriptor(), buffer, size);
            } else {
                libc::read(self.descriptor(), buffer, size);
            }
        }
    }
}
