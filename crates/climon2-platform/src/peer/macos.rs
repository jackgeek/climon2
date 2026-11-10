use crate::runtime::SocketStream;
use std::{io, os::fd::AsRawFd};

pub fn uid(stream: &SocketStream) -> io::Result<u32> {
    let mut uid = 0;
    let mut gid = 0;
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(uid)
}
