//! Windows's socket sends (#318): Windows has no `SIGPIPE`, so a send to a
//! closed peer simply fails.
use crate::runtime::SocketStream;
use std::io;

/// Sends what the socket takes now.
pub fn send(socket: &SocketStream, bytes: &[u8]) -> io::Result<usize> {
    socket.send(bytes)
}

/// Nothing to set up: no send raises a signal.
pub fn quiet(_socket: &SocketStream) -> io::Result<()> {
    Ok(())
}
