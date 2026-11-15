//! One daemon connection's bytes: a nonblocking Unix socket, the frame
//! being read and the bytes waiting to be written. A frame is read whole,
//! one at a time, after its length prefix has been admitted, so no frame is
//! larger than the ceiling it was read under.

use super::{failure::Fault, platform};
use climon2_core::wire::{FatalCode, Frame, FrameLimit};
use std::{
    io::{self, Read},
    mem,
    os::{fd::AsRawFd, unix::net::UnixStream},
    path::Path,
    time::{Duration, Instant},
};

/// The four-byte big-endian length prefix.
const PREFIX: usize = 4;

const UNREACHABLE: Fault = Fault::transport("io_error", "the session daemon cannot be reached");
pub(super) const CLOSED: Fault =
    Fault::transport("io_error", "the session daemon closed the connection");
const BROKEN: Fault = Fault::transport("io_error", "the connection to the session daemon failed");
const TRUNCATED: Fault = Fault::transport(
    "truncated_frame",
    "the session daemon closed the connection inside a frame",
);
pub(super) const TIMEOUT: Fault =
    Fault::library("timeout", "the session daemon did not answer in time");

/// A connection-fatal code the library found in the daemon's traffic.
pub(super) fn fatal(code: FatalCode) -> Fault {
    Fault::transport(
        code.as_str(),
        "the session daemon's traffic breaks daemon wire revision 1",
    )
}

pub(super) struct Link {
    socket: UnixStream,
    incoming: Vec<u8>,
    outgoing: Vec<u8>,
}

impl Link {
    /// Connects to a same-UID peer within `timeout`.
    pub(super) fn connect(path: &Path, timeout: Duration) -> Result<Self, Fault> {
        let socket =
            climon2_platform::connection::connect(path, timeout).map_err(|_| UNREACHABLE)?;
        climon2_platform::peer::authenticate(&socket).map_err(|_| UNREACHABLE)?;
        socket.set_nonblocking(true).map_err(|_| UNREACHABLE)?;
        platform::quiet(socket.as_raw_fd()).map_err(|_| UNREACHABLE)?;
        Ok(Self {
            socket,
            incoming: Vec::new(),
            outgoing: Vec::new(),
        })
    }

    pub(super) fn send(&mut self, frame: &[u8]) {
        self.outgoing.extend_from_slice(frame);
    }

    pub(super) fn writing(&self) -> bool {
        !self.outgoing.is_empty()
    }

    /// Writes what the socket takes now, without waiting, if anything waits.
    pub(super) fn flush(&mut self) -> Result<(), Fault> {
        if self.writing() {
            let sent = platform::send(self.socket.as_raw_fd(), &self.outgoing);
            let written = pending(sent).map_err(|_| BROKEN)?;
            self.outgoing.drain(..written.unwrap_or_default());
        }
        Ok(())
    }

    /// The next whole frame `limit` admits, once all its bytes are here.
    pub(super) fn receive(&mut self, limit: FrameLimit) -> Result<Option<Frame>, Fault> {
        while self.more(limit)? {}
        Ok(self.whole(limit)?.then(|| self.take()))
    }

    /// The first whole frame before `deadline`, writing what waits meanwhile.
    pub(super) fn exchange(
        &mut self,
        limit: FrameLimit,
        deadline: Instant,
    ) -> Result<Frame, Fault> {
        loop {
            if let Some(frame) = self.step(limit, deadline)? {
                return Ok(frame);
            }
        }
    }

    fn step(&mut self, limit: FrameLimit, deadline: Instant) -> Result<Option<Frame>, Fault> {
        self.flush()?;
        remaining(deadline).ok_or(TIMEOUT)?;
        wait(None, Some(self), Some(deadline));
        self.receive(limit)
    }

    /// The length the current frame needs: its prefix, then all of it.
    fn wanted(&self, limit: FrameLimit) -> Result<usize, Fault> {
        match self.incoming.first_chunk::<PREFIX>() {
            None => Ok(PREFIX),
            Some(prefix) => limit
                .frame_length(*prefix)
                .map(|length| PREFIX + length)
                .map_err(fatal),
        }
    }

    fn whole(&self, limit: FrameLimit) -> Result<bool, Fault> {
        Ok(self.incoming.len() == self.wanted(limit)?)
    }

    /// Reads toward the current frame unless it is whole: whether to read on.
    fn more(&mut self, limit: FrameLimit) -> Result<bool, Fault> {
        let wanted = self.wanted(limit)?;
        let start = self.incoming.len();
        if start == wanted {
            return Ok(false);
        }
        self.incoming.resize_with(wanted, u8::default);
        let count = arrived(self.socket.read(&mut self.incoming[start..]), start)?;
        self.incoming.truncate(start + count.unwrap_or_default());
        Ok(count.is_some())
    }

    fn take(&mut self) -> Frame {
        let mut code = mem::take(&mut self.incoming).split_off(PREFIX);
        let body = code.split_off(1);
        Frame::new(code[0], body)
    }

    /// Reading, plus writing while bytes wait.
    fn interest(&self) -> libc::c_short {
        let write = if self.writing() {
            libc::POLLOUT
        } else {
            libc::c_short::default()
        };
        libc::POLLIN | write
    }
}

/// Waits until `waker` or `link`'s socket is ready, or `deadline` passes. A
/// signal or a spurious wake only makes the caller look again.
pub(super) fn wait(waker: Option<i32>, link: Option<&Link>, deadline: Option<Instant>) {
    let mut descriptors: Vec<_> = waker
        .map(|waker| pollfd(waker, libc::POLLIN))
        .into_iter()
        .collect();
    descriptors.extend(link.map(|link| pollfd(link.socket.as_raw_fd(), link.interest())));
    let timeout = deadline.map_or(i32::MAX, milliseconds);
    // SAFETY: the entries are valid for the call's duration.
    unsafe {
        libc::poll(
            descriptors.as_mut_ptr(),
            descriptors.len() as libc::nfds_t,
            timeout,
        )
    };
}

fn pollfd(fd: i32, events: libc::c_short) -> libc::pollfd {
    libc::pollfd {
        fd,
        events,
        revents: libc::c_short::default(),
    }
}

/// The time left before `deadline`, if any.
pub(super) fn remaining(deadline: Instant) -> Option<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
}

/// Milliseconds until `deadline`, rounded up, so a wait never ends early.
fn milliseconds(deadline: Instant) -> i32 {
    let left = remaining(deadline).unwrap_or_default();
    i32::try_from(left.as_micros().div_ceil(1000)).unwrap_or(i32::MAX)
}

/// Bytes a nonblocking call moved; none when it would block or was
/// interrupted.
fn pending(result: io::Result<usize>) -> io::Result<Option<usize>> {
    match result {
        Ok(count) => Ok(Some(count)),
        Err(error) if retryable(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn retryable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}

/// What a read toward the current frame, `start` bytes of it here,
/// returned: the end of the stream is closure between frames and truncation
/// inside one.
fn arrived(read: io::Result<usize>, start: usize) -> Result<Option<usize>, Fault> {
    match pending(read).map_err(|_| BROKEN)? {
        Some(0) => Err(ended(start)),
        count => Ok(count),
    }
}

fn ended(start: usize) -> Fault {
    if start == 0 { CLOSED } else { TRUNCATED }
}
