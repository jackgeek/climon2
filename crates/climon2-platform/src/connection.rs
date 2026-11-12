use crate::runtime::SocketStream;
use std::{io, path::Path, time::Duration};

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("climon2 socket connections require macOS or Linux");

#[cfg(any(target_os = "macos", target_os = "linux"))]
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Interest {
    fn events(self) -> libc::c_short {
        match self {
            Self::Read => libc::POLLIN,
            Self::Write => libc::POLLOUT,
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn wait(stream: &SocketStream, interest: Interest, timeout: Duration) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let mut descriptor = libc::pollfd {
        fd: stream.as_raw_fd(),
        events: interest.events(),
        // Output-only.
        revents: Default::default(),
    };
    let millis = i32::try_from(timeout.as_millis().saturating_add(1)).unwrap_or(i32::MAX);
    // The single pollfd remains valid for the duration of this call.
    poll_outcome(unsafe { libc::poll(&mut descriptor, 1, millis) })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
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
