//! Authenticate a connection before reading any hello bytes.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("peer credential seam requires macOS or Linux");

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::*;

#[cfg(target_os = "linux")]
#[path = "peer/linux.rs"]
mod linux;

#[cfg(target_os = "macos")]
#[path = "peer/macos.rs"]
mod macos;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use crate::runtime::{SocketListener, SocketStream};
    use std::io;

    /// Return only authenticated streams; rejected streams close before hello.
    pub fn accept(listener: &SocketListener) -> io::Result<SocketStream> {
        let (stream, _) = listener.accept()?;
        authenticate(&stream)?;
        Ok(stream)
    }

    pub fn effective_uid() -> u32 {
        unsafe { libc::geteuid() }
    }

    #[cfg(target_os = "macos")]
    pub use super::macos::uid;

    #[cfg(target_os = "linux")]
    pub use super::linux::uid;

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
