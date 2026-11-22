pub use crate::waiting::Interest;
use crate::{runtime::SocketStream, waiting::Watch};
use std::{io, path::Path, time::Duration};

#[cfg(target_os = "linux")]
#[path = "connection/linux.rs"]
mod native;
#[cfg(target_os = "macos")]
#[path = "connection/macos.rs"]
mod native;
#[cfg(windows)]
#[path = "connection/windows.rs"]
mod native;

/// The client library's sends, which never raise `SIGPIPE` in its embedder.
pub use native::{quiet, send};

pub fn connect(path: &Path, timeout: Duration) -> io::Result<SocketStream> {
    let address = socket2::SockAddr::unix(path)?;
    let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
    socket.connect_timeout(&address, timeout)?;
    Ok(crate::runtime::into_stream(socket))
}

pub fn wait(stream: &SocketStream, interest: Interest, timeout: Duration) -> io::Result<()> {
    let millis = timeout.as_millis().saturating_add(1);
    let timeout = Duration::from_millis(u64::try_from(millis).unwrap_or(u64::MAX));
    let mut watches = [Watch::stream(stream, interest)];
    poll_outcome(crate::waiting::wait(&mut watches, timeout)?)
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
/// missing or refuses. Only these can begin to prove a session stale. std
/// reads `ENOENT` and `ECONNREFUSED` this way on Unix, and Windows's
/// not-found codes and `WSAECONNREFUSED`.
pub fn nobody_listening(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}
