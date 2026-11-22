//! Waiting for readiness on Windows (#316; ADR 0003, "Multiplexing"):
//! `WSAPoll`, the `poll` of sockets. Sockets are the only handles it waits
//! on; ConPTY and console handles get threads of their own.
use super::Interest;
use crate::runtime::SocketStream;
use std::{
    io,
    marker::PhantomData,
    os::windows::io::{AsRawSocket, AsSocket, BorrowedSocket},
    time::{Duration, Instant},
};
use windows_sys::Win32::Networking::WinSock::{
    POLLRDNORM, POLLWRNORM, SOCKET_ERROR, WSAGetLastError, WSAPOLLFD, WSAPoll,
};

impl Interest {
    /// `POLLIN` and `POLLOUT` without the priority bands `WSAPoll` refuses.
    fn events(self) -> i16 {
        match self {
            Self::Read => POLLRDNORM,
            Self::Write => POLLWRNORM,
            Self::ReadWrite => POLLRDNORM | POLLWRNORM,
        }
    }
}

/// One slot of a wait: a borrowed socket and what is wanted from it. Unix's
/// idle slot has no counterpart yet: `WSAPoll` reports an ignored entry as
/// invalid, and fails when every entry is ignored.
#[repr(transparent)]
pub struct Watch<'a> {
    entry: WSAPOLLFD,
    handle: PhantomData<BorrowedSocket<'a>>,
}

impl<'a> Watch<'a> {
    pub fn new(handle: BorrowedSocket<'a>, interest: Interest) -> Self {
        Self {
            entry: WSAPOLLFD {
                fd: handle.as_raw_socket() as usize,
                events: interest.events(),
                // Output-only.
                revents: i16::default(),
            },
            handle: PhantomData,
        }
    }

    /// A connected socket, as every platform names it.
    pub fn stream(stream: &'a SocketStream, interest: Interest) -> Self {
        Self::new(stream.as_socket(), interest)
    }

    /// The last wait reported anything for this socket: what was wanted, a
    /// hang-up or an error.
    pub fn ready(&self) -> bool {
        self.entry.revents != 0
    }

    /// The last wait reported this socket readable.
    pub fn readable(&self) -> bool {
        self.entry.revents & POLLRDNORM != 0
    }
}

/// Waits until a watched socket is ready or `timeout` passes, and returns how
/// many are ready. `WSAPoll` can return up to a clock tick early with nothing
/// ready, so it is called again until the timeout has truly passed.
pub fn wait(watches: &mut [Watch<'_>], timeout: Duration) -> io::Result<usize> {
    let deadline = Instant::now().checked_add(timeout);
    let mut remaining = timeout;
    loop {
        let ready = poll(watches, remaining)?;
        remaining = deadline.map_or(remaining, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        });
        if ready != 0 || remaining.is_zero() {
            return Ok(ready);
        }
    }
}

/// One `WSAPoll`, its timeout rounded up to whole milliseconds and saturating.
fn poll(watches: &mut [Watch<'_>], timeout: Duration) -> io::Result<usize> {
    let milliseconds = i32::try_from(timeout.as_micros().div_ceil(1000)).unwrap_or(i32::MAX);
    // SAFETY: a Watch is exactly a WSAPOLLFD, and WSAPoll reads and updates
    // exactly `watches`' entries for the call's duration.
    let ready = unsafe {
        WSAPoll(
            watches.as_mut_ptr().cast(),
            watches.len() as u32,
            milliseconds,
        )
    };
    if ready == SOCKET_ERROR {
        // SAFETY: reads only this thread's last Winsock error.
        return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
    }
    Ok(ready as usize)
}
