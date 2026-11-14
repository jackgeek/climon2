//! macOS-only library code.

use std::{
    io,
    os::fd::{AsRawFd, OwnedFd, RawFd},
};

/// The pollable notification descriptor: the read end of a close-on-exec
/// pipe, readable while it holds the library's one byte. The library writes
/// and reads that byte only when its tracked readiness changes, so neither
/// end ever blocks.
pub(super) struct Notifier {
    reader: OwnedFd,
    writer: OwnedFd,
}

impl Notifier {
    /// The descriptors one notifier holds: both ends of the pipe.
    pub(super) const DESCRIPTORS: u32 = 2;

    pub(super) fn new() -> io::Result<Self> {
        let (reader, writer) = io::pipe()?;
        Ok(Self {
            reader: reader.into(),
            writer: writer.into(),
        })
    }

    pub(super) fn descriptor(&self) -> RawFd {
        self.reader.as_raw_fd()
    }

    /// Writes the byte, making the descriptor readable, or reads it back.
    pub(super) fn set(&self, readable: bool) {
        let mut byte = u8::from(readable);
        let buffer = (&raw mut byte).cast::<libc::c_void>();
        // SAFETY: the buffer holds the one byte transferred.
        unsafe {
            if readable {
                libc::write(self.writer.as_raw_fd(), buffer, 1);
            } else {
                libc::read(self.reader.as_raw_fd(), buffer, 1);
            }
        }
    }
}
