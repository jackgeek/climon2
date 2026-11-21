use crate::runtime::SocketStream;
pub use crate::waiting::Interest;
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn wait(stream: &SocketStream, interest: Interest, timeout: Duration) -> io::Result<()> {
    use crate::waiting::{self, Watch};
    use std::os::fd::AsFd;
    let millis = timeout.as_millis().saturating_add(1);
    let timeout = Duration::from_millis(u64::try_from(millis).unwrap_or(u64::MAX));
    let mut watches = [Watch::new(stream.as_fd(), interest)];
    poll_outcome(waiting::wait(&mut watches, timeout)?)
}

fn poll_outcome(ready: usize) -> io::Result<()> {
    match ready {
        0 => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "socket deadline expired",
        )),
        _ => Ok(()),
    }
}

/// A connect failure that says no daemon listens at the path: the socket is
/// missing or refuses. Only these can begin to prove a session stale.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn nobody_listening(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENOENT | libc::ECONNREFUSED)
    )
}
