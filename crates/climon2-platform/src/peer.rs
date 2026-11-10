//! Authenticate a connection before reading any hello bytes.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("peer credential seam requires macOS or Linux");

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
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

    #[cfg(target_os = "macos")]
    pub fn uid(stream: &SocketStream) -> io::Result<u32> {
        let mut uid = 0;
        let mut gid = 0;
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(uid)
    }

    #[cfg(target_os = "linux")]
    pub fn uid(stream: &SocketStream) -> io::Result<u32> {
        let mut credentials = std::mem::MaybeUninit::<libc::ucred>::uninit();
        let mut length = size_of::<libc::ucred>() as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                credentials.as_mut_ptr().cast(),
                &mut length,
            )
        } == -1
        {
            return Err(io::Error::last_os_error());
        }
        if length as usize != size_of::<libc::ucred>() {
            return Err(io::Error::other("incomplete peer credentials"));
        }
        Ok(unsafe { credentials.assume_init() }.uid)
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
