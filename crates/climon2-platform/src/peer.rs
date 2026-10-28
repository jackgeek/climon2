//! Authenticate a connection before reading any hello bytes.
#[cfg(not(target_os = "macos"))]
compile_error!("peer credential seam requires macOS");

#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "macos")]
mod macos {
    use crate::runtime::{SocketListener, SocketStream};
    use std::{io, os::fd::AsRawFd};

    /// Return only authenticated streams; rejected streams close before hello.
    pub fn accept(listener: &SocketListener) -> io::Result<SocketStream> {
        let (stream, _) = listener.accept()?;
        authenticate(&stream)?;
        Ok(stream)
    }

    pub fn effective_uid() -> u32 {
        unsafe { libc::geteuid() }
    }

    pub fn uid(stream: &SocketStream) -> io::Result<u32> {
        let mut uid = 0;
        let mut gid = 0;
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(uid)
    }

    pub fn authenticate(stream: &SocketStream) -> io::Result<()> {
        authenticate_uid(stream, effective_uid())
    }

    pub fn authenticate_uid(stream: &SocketStream, expected: u32) -> io::Result<()> {
        if uid(stream)? != expected {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "peer UID mismatch",
            ));
        }
        Ok(())
    }
}
