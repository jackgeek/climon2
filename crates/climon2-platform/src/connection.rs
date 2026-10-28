use crate::runtime::SocketStream;
use std::{io, path::Path, time::Duration};

#[cfg(not(target_os = "macos"))]
compile_error!("climon2 socket connections currently support macOS only");

#[cfg(target_os = "macos")]
pub fn connect(path: &Path, timeout: Duration) -> io::Result<SocketStream> {
    let address = socket2::SockAddr::unix(path)?;
    let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
    socket.connect_timeout(&address, timeout)?;
    let descriptor: std::os::fd::OwnedFd = socket.into();
    Ok(descriptor.into())
}

#[derive(Clone, Copy)]
pub enum Interest {
    Read,
    Write,
}

#[cfg(target_os = "macos")]
impl Interest {
    fn events(self) -> libc::c_short {
        match self {
            Self::Read => libc::POLLIN,
            Self::Write => libc::POLLOUT,
        }
    }
}

#[cfg(target_os = "macos")]
pub fn wait(stream: &SocketStream, interest: Interest, timeout: Duration) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let mut descriptor = libc::pollfd {
        fd: stream.as_raw_fd(),
        events: interest.events(),
        revents: 0,
    };
    let millis = i32::try_from(timeout.as_millis().saturating_add(1)).unwrap_or(i32::MAX);
    // The single pollfd remains valid for the duration of this call.
    poll_outcome(unsafe { libc::poll(&mut descriptor, 1, millis) })
}

#[cfg(target_os = "macos")]
fn poll_outcome(ready: libc::c_int) -> io::Result<()> {
    match ready {
        -1 => Err(io::Error::last_os_error()),
        0 => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "socket deadline expired",
        )),
        _ => Ok(()),
    }
}
