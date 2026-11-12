use crate::runtime::SocketStream;
use std::{io, os::fd::AsRawFd};

pub fn uid(stream: &SocketStream) -> io::Result<u32> {
    let mut credentials = std::mem::MaybeUninit::<libc::ucred>::uninit();
    let mut length = size_of::<libc::ucred>() as libc::socklen_t;
    crate::syscall::checked(unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            credentials.as_mut_ptr().cast(),
            &mut length,
        )
    })?;
    if length as usize != size_of::<libc::ucred>() {
        return Err(io::Error::other("incomplete peer credentials"));
    }
    Ok(unsafe { credentials.assume_init() }.uid)
}
