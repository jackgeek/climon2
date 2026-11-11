use crate::runtime::SocketStream;
use std::{io, os::fd::AsRawFd};

pub fn uid(stream: &SocketStream) -> io::Result<u32> {
    let mut uid = libc::uid_t::default();
    let mut gid = libc::gid_t::default();
    crate::syscall::checked(unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) })?;
    Ok(uid)
}
