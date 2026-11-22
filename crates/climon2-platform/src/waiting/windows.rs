//! Waiting for readiness on Windows (#316; ADR 0003, "Multiplexing"):
//! `WSAPoll`, the `poll` of sockets, for sockets, and
//! `WaitForMultipleObjects` for events. One wait watches sockets or events,
//! never both: mixing them would need `WSAEventSelect`, which ADR 0003
//! rejected. ConPTY and console handles get threads of their own, which
//! signal events (#321).
use super::Interest;
use crate::runtime::SocketStream;
use std::{
    io,
    marker::PhantomData,
    os::windows::io::{AsRawHandle, AsRawSocket, AsSocket, BorrowedHandle, BorrowedSocket},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HANDLE, WAIT_FAILED, WAIT_OBJECT_0},
    Networking::WinSock::{
        POLLRDNORM, POLLWRNORM, SOCKET_ERROR, WSAGetLastError, WSAPOLLFD, WSAPoll,
    },
    System::Threading::{INFINITE, WaitForMultipleObjects, WaitForSingleObject},
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

/// What one slot watches, and what the last wait reported for it.
enum Slot {
    Socket(WSAPOLLFD),
    /// An event, and whether the last wait found it signalled.
    Event(HANDLE, bool),
    Idle,
}

/// One slot of a wait: a borrowed socket and what is wanted from it, a
/// borrowed event, or an idle slot that keeps its place and is never ready.
pub struct Watch<'a> {
    slot: Slot,
    handle: PhantomData<BorrowedSocket<'a>>,
}

impl<'a> Watch<'a> {
    pub fn new(handle: BorrowedSocket<'a>, interest: Interest) -> Self {
        Self::slot(Slot::Socket(WSAPOLLFD {
            fd: handle.as_raw_socket() as usize,
            events: interest.events(),
            // Output-only.
            revents: i16::default(),
        }))
    }

    /// A connected socket, as every platform names it.
    pub fn stream(stream: &'a SocketStream, interest: Interest) -> Self {
        Self::new(stream.as_socket(), interest)
    }

    /// A manual-reset event, ready while it is signalled. Waiting never
    /// resets it.
    pub fn event(handle: BorrowedHandle<'a>) -> Self {
        Self::slot(Slot::Event(handle.as_raw_handle(), false))
    }

    /// A slot the wait ignores.
    pub fn idle() -> Self {
        Self::slot(Slot::Idle)
    }

    fn slot(slot: Slot) -> Self {
        Self {
            slot,
            handle: PhantomData,
        }
    }

    /// The last wait reported anything for this slot: what was wanted, a
    /// hang-up or an error, or a signalled event.
    pub fn ready(&self) -> bool {
        match &self.slot {
            Slot::Socket(entry) => entry.revents != 0,
            Slot::Event(_, signalled) => *signalled,
            Slot::Idle => false,
        }
    }

    /// The last wait reported this socket readable, or this event signalled.
    pub fn readable(&self) -> bool {
        match &self.slot {
            Slot::Socket(entry) => entry.revents & POLLRDNORM != 0,
            Slot::Event(_, signalled) => *signalled,
            Slot::Idle => false,
        }
    }

    fn entry(&self) -> Option<WSAPOLLFD> {
        match &self.slot {
            Slot::Socket(entry) => Some(WSAPOLLFD { ..*entry }),
            _ => None,
        }
    }

    fn handle(&self) -> Option<HANDLE> {
        match &self.slot {
            Slot::Event(handle, _) => Some(*handle),
            _ => None,
        }
    }

    /// Records whether this slot's event is signalled now; `None` for any
    /// other slot.
    fn look(&mut self) -> Option<bool> {
        match &mut self.slot {
            Slot::Event(handle, signalled) => {
                // SAFETY: an open event the caller borrowed; a zero timeout
                // only looks, and a manual-reset event stays as it is.
                *signalled = unsafe { WaitForSingleObject(*handle, 0u32) } == WAIT_OBJECT_0;
                Some(*signalled)
            }
            _ => None,
        }
    }
}

/// Waits until a watched socket or event is ready or `timeout` passes, and
/// returns how many are ready. A Windows wait counts whole milliseconds and
/// can return up to a clock tick early with nothing ready, so it waits again,
/// with what is left, until the timeout has truly passed; the last fraction
/// of a millisecond is polled rather than slept. A wait with nothing to
/// watch only lets the timeout pass.
pub fn wait(watches: &mut [Watch<'_>], timeout: Duration) -> io::Result<usize> {
    let deadline = Instant::now().checked_add(timeout);
    loop {
        match once(watches, remaining(deadline))? {
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

/// One wait, on the events if there are any, else on the sockets.
fn once(watches: &mut [Watch<'_>], timeout: Duration) -> io::Result<usize> {
    let events = watches.iter().any(|watch| watch.handle().is_some());
    let sockets = watches.iter().any(|watch| watch.entry().is_some());
    match (events, sockets) {
        (true, true) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "one wait cannot watch both sockets and events",
        )),
        (true, false) => wait_events(watches, timeout),
        (false, true) => poll(watches, timeout),
        (false, false) => {
            std::thread::sleep(timeout);
            Ok(0)
        }
    }
}

/// One `WSAPoll` over the socket slots, its timeout cut to whole
/// milliseconds and saturating.
fn poll(watches: &mut [Watch<'_>], timeout: Duration) -> io::Result<usize> {
    let mut entries: Vec<WSAPOLLFD> = watches.iter().filter_map(Watch::entry).collect();
    let milliseconds = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: `entries` holds `entries.len()` initialized WSAPOLLFDs, which
    // WSAPoll reads and updates for the call's duration.
    let ready = unsafe { WSAPoll(entries.as_mut_ptr(), entries.len() as u32, milliseconds) };
    if ready == SOCKET_ERROR {
        // SAFETY: reads only this thread's last Winsock error.
        return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
    }
    let mut reported = entries.into_iter();
    for watch in watches.iter_mut() {
        if let Slot::Socket(entry) = &mut watch.slot {
            *entry = reported.next().expect("one entry per socket slot");
        }
    }
    Ok(ready as usize)
}

/// One `WaitForMultipleObjects` over the event slots, its timeout cut to
/// whole milliseconds and saturating, then which of them are signalled.
fn wait_events(watches: &mut [Watch<'_>], timeout: Duration) -> io::Result<usize> {
    let handles: Vec<HANDLE> = watches.iter().filter_map(Watch::handle).collect();
    let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(INFINITE);
    // SAFETY: `handles` holds `handles.len()` open events, which the caller
    // borrowed for the wait; FALSE waits for any one of them.
    let waited =
        unsafe { WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, milliseconds) };
    if waited == WAIT_FAILED {
        return Err(io::Error::last_os_error());
    }
    Ok(watches
        .iter_mut()
        .filter_map(Watch::look)
        .filter(|&signalled| signalled)
        .count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::{AsHandle, FromRawHandle, OwnedHandle, RawSocket};
    use windows_sys::Win32::{
        Foundation::ERROR_INVALID_HANDLE,
        Networking::WinSock::WSAENOTSOCK,
        System::Threading::{CreateEventW, SetEvent},
    };

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

    /// A socket with something to accept is ready and readable; an idle
    /// slot beside it keeps its place and is never ready.
    #[test]
    fn a_ready_socket_is_reported_beside_an_idle_slot() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ready.sock");
        let listener = crate::runtime::bind(&path).unwrap();
        let client =
            socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
        client
            .connect(&socket2::SockAddr::unix(&path).unwrap())
            .unwrap();

        let mut watches = [
            Watch::idle(),
            Watch::new(listener.as_socket(), Interest::Read),
        ];
        let ready = wait(&mut watches, Duration::from_secs(5)).unwrap();

        assert_eq!(ready, 1);
        assert!(!watches[0].ready() && !watches[0].readable());
        assert!(watches[1].ready() && watches[1].readable());
    }

    /// A new manual-reset event, initially clear.
    fn event() -> OwnedHandle {
        // SAFETY: no security attributes and no name; manual reset, clear.
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        assert!(!event.is_null());
        // SAFETY: a new handle nothing else owns.
        unsafe { OwnedHandle::from_raw_handle(event) }
    }

    fn set(event: &OwnedHandle) {
        // SAFETY: an open event.
        assert_ne!(unsafe { SetEvent(event.as_raw_handle()) }, 0);
    }

    #[test]
    fn a_clear_event_waits_out_the_timeout() {
        let clear = event();
        let timeout = Duration::from_micros(20_700);

        let started = Instant::now();
        let mut watches = [Watch::event(clear.as_handle()), Watch::idle()];
        let ready = wait(&mut watches, timeout).unwrap();

        assert_eq!(ready, 0);
        assert!(started.elapsed() >= timeout, "{:?}", started.elapsed());
        assert!(!watches[0].ready() && !watches[0].readable());
    }

    #[test]
    fn only_the_signalled_events_are_ready_and_stay_signalled() {
        let (clear, signalled) = (event(), event());
        set(&signalled);

        let mut watches = [
            Watch::event(clear.as_handle()),
            Watch::idle(),
            Watch::event(signalled.as_handle()),
        ];
        let ready = wait(&mut watches, Duration::MAX).unwrap();

        assert_eq!(ready, 1);
        assert!(!watches[0].ready() && !watches[1].ready());
        assert!(watches[2].ready() && watches[2].readable());
        let mut again = [Watch::event(signalled.as_handle())];
        assert_eq!(wait(&mut again, Duration::ZERO).unwrap(), 1);
    }

    /// An event signalled during the wait ends it.
    #[test]
    fn an_event_signalled_later_ends_the_wait() {
        let later = std::sync::Arc::new(event());
        let setter = later.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            set(&setter);
        });

        let mut watches = [Watch::event(later.as_handle())];
        assert_eq!(wait(&mut watches, Duration::from_secs(30)).unwrap(), 1);
        thread.join().unwrap();
    }

    #[test]
    fn a_wait_on_no_event_fails_with_the_windows_error() {
        // SAFETY: a handle value no process is given; only the wait, which
        // rejects it, uses it.
        let none = unsafe { BorrowedHandle::borrow_raw(0x7fff_fff1 as _) };
        let mut watches = [Watch::event(none)];

        let error = wait(&mut watches, Duration::from_secs(5)).unwrap_err();

        assert_eq!(error.raw_os_error(), Some(ERROR_INVALID_HANDLE as i32));
    }

    #[test]
    fn one_wait_cannot_watch_sockets_and_events() {
        let directory = tempfile::tempdir().unwrap();
        let listener = crate::runtime::bind(&directory.path().join("mixed.sock")).unwrap();
        let signalled = event();
        set(&signalled);

        let mut watches = [
            Watch::new(listener.as_socket(), Interest::Read),
            Watch::event(signalled.as_handle()),
        ];
        let error = wait(&mut watches, Duration::from_secs(5)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_wait_with_nothing_to_watch_lets_the_timeout_pass() {
        let timeout = Duration::from_millis(30);

        let started = Instant::now();
        let ready = wait(&mut [Watch::idle()], timeout).unwrap();

        assert_eq!(ready, 0);
        assert!(started.elapsed() >= timeout, "{:?}", started.elapsed());
    }
}
