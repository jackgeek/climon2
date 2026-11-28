//! The socket seam for monitor links (ADR 0003): one TCP connection per
//! link, which the relay listens for and the hub monitor dials. What crosses
//! it is the link code's business; this seam only moves bytes and keeps the
//! deadlines.
//!
//! - [`listen`] binds a [`LinkListener`], whose `accept` waits no longer
//!   than its deadline for a connection.
//! - [`dial`] connects a [`LinkStream`] within its deadline.
//! - [`LinkStream::read`] waits no longer than its deadline for bytes.
//!
//! Every expired deadline is a `TimedOut` error, and a zero deadline is
//! refused as `InvalidInput` by `dial` and `read`.
//!
//! Unix only: monitor links on Windows are not yet specified, and Windows
//! code belongs in a registered Windows file with tests that run there.

use crate::waiting::{self, Interest, Watch};
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::fd::AsFd,
    time::Duration,
};

/// A bound, listening TCP socket.
#[derive(Debug)]
pub struct LinkListener(TcpListener);

/// One end of a connected TCP stream.
#[derive(Debug)]
pub struct LinkStream(TcpStream);

/// Listens on `address`. Port 0 takes any free port, which
/// [`LinkListener::local_address`] then reports.
pub fn listen(address: SocketAddr) -> io::Result<LinkListener> {
    let listener = TcpListener::bind(address)?;
    // A connection that is reset, or taken by another caller, between
    // becoming ready and being accepted must not leave `accept` waiting
    // past its deadline.
    listener.set_nonblocking(true)?;
    Ok(LinkListener(listener))
}

/// Connects to `address`, giving up with `TimedOut` when no answer comes
/// within `deadline`.
pub fn dial(address: SocketAddr, deadline: Duration) -> io::Result<LinkStream> {
    TcpStream::connect_timeout(&address, deadline).map(LinkStream)
}

impl LinkListener {
    /// The address this listener is bound to.
    pub fn local_address(&self) -> io::Result<SocketAddr> {
        self.0.local_addr()
    }

    /// The next connection made to this listener, in the order they were
    /// made, or `TimedOut` when none arrives within `deadline`.
    pub fn accept(&self, deadline: Duration) -> io::Result<LinkStream> {
        let mut watches = [watch(&self.0)];
        if waiting::wait(&mut watches, deadline)? == 0 {
            return Err(expired(io::ErrorKind::TimedOut.into()));
        }
        let (stream, _) = self.0.accept().map_err(expired)?;
        // Some systems hand an accepted socket its listener's non-blocking
        // mode; a stream waits for its deadlines.
        stream.set_nonblocking(false)?;
        Ok(LinkStream(stream))
    }
}

impl LinkStream {
    /// The address of the other end.
    pub fn peer_address(&self) -> io::Result<SocketAddr> {
        self.0.peer_addr()
    }

    /// Reads what has arrived, up to the buffer's length, waiting no longer
    /// than `deadline` for the first byte. `Ok(0)` is the end of the stream;
    /// `TimedOut` leaves the stream usable.
    pub fn read(&mut self, buffer: &mut [u8], deadline: Duration) -> io::Result<usize> {
        self.0.set_read_timeout(Some(deadline))?;
        self.0.read(buffer).map_err(expired)
    }

    /// Sends all of `bytes`, waiting for the other end to take them.
    pub fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes)
    }
}

/// An expired wait as every platform's caller sees it: systems report one
/// as `WouldBlock` or as `TimedOut`.
fn expired(error: io::Error) -> io::Error {
    match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
            io::Error::new(io::ErrorKind::TimedOut, "socket deadline expired")
        }
        _ => error,
    }
}

fn watch(listener: &TcpListener) -> Watch<'_> {
    Watch::new(listener.as_fd(), Interest::Read)
}
