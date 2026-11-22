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
/// many are ready. `WSAPoll` waits whole milliseconds and can return up to a
/// clock tick early with nothing ready, so it is called again, with what is
/// left, until the timeout has truly passed; the last fraction of a
/// millisecond is polled rather than slept.
pub fn wait(watches: &mut [Watch<'_>], timeout: Duration) -> io::Result<usize> {
    let deadline = Instant::now().checked_add(timeout);
    loop {
        match poll(watches, remaining(deadline))? {
            0 if !remaining(deadline).is_zero() => {}
            ready => return Ok(ready),
        }
    }
}

/// What is left until `deadline`; a timeout too long to have one never ends.
fn remaining(deadline: Option<Instant>) -> Duration {
    deadline.map_or(Duration::MAX, |deadline| {
        deadline.saturating_duration_since(Instant::now())
    })
}

/// One `WSAPoll`, its timeout cut to whole milliseconds and saturating.
fn poll(watches: &mut [Watch<'_>], timeout: Duration) -> io::Result<usize> {
    let milliseconds = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::RawSocket;
    use windows_sys::Win32::Networking::WinSock::WSAENOTSOCK;

    /// A wait on something that is not a socket fails with Winsock's error
    /// at once, rather than waiting out its timeout. A test cannot make a
    /// real socket's wait fail, so this is the one way to reach that error.
    #[test]
    fn a_wait_on_no_socket_fails_with_the_winsock_error() {
        // Winsock must be started, which making any socket does.
        drop(socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap());
        // A handle value no process is given (handles are multiples of 4).
        // SAFETY: nothing but WSAPoll, which rejects it, uses it as a socket.
        let none = unsafe { BorrowedSocket::borrow_raw(0x7fff_fff1 as RawSocket) };
        let mut watches = [Watch::new(none, Interest::Read)];

        let error = wait(&mut watches, Duration::from_secs(5)).unwrap_err();

        assert_eq!(error.raw_os_error(), Some(WSAENOTSOCK));
    }

    /// `WSAPoll` waits whole milliseconds, so a timeout with a fraction is
    /// waited out by polling again until it has passed.
    #[test]
    fn a_timeout_with_a_fraction_of_a_millisecond_is_waited_out() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("quiet.sock");
        let listener = crate::runtime::bind(&path).unwrap();
        let timeout = Duration::from_micros(2_700);

        let started = Instant::now();
        let mut watches = [Watch::new(listener.as_socket(), Interest::Read)];
        let ready = wait(&mut watches, timeout).unwrap();

        assert_eq!(ready, 0);
        assert!(started.elapsed() >= timeout, "{:?}", started.elapsed());
    }
}
