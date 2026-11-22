//! Connections on Windows (#317; ADR 0003, "Windows I/O and transport").
//! Windows AF_UNIX reports no peer credentials, so nothing is checked per
//! connection: the runtime directory's private descriptor keeps other users
//! out, and an administrator can bypass it, as accepted there.
use crate::runtime::{SocketListener, SocketStream};
use std::io;

/// The next connection; every stream is accepted as authenticated.
pub fn accept(listener: &SocketListener) -> io::Result<SocketStream> {
    listener.accept().map(|(stream, _)| stream)
}

/// There is nothing to compare: the directory's DACL already admitted it.
pub fn authenticate(_stream: &SocketStream) -> io::Result<()> {
    Ok(())
}
