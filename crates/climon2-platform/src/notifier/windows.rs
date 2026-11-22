//! Windows's notifier (#318). The notification descriptor is one unnamed,
//! non-inheritable manual-reset event, carried in the frozen `int32_t`
//! output (ADR 0008). The waker must wake a `WSAPoll` that also watches the
//! instance's sockets, and only a socket can, so it is a loopback UDP socket
//! connected to itself: it accepts only its own datagrams.
use crate::waiting::{Interest, Watch};
use std::{
    io,
    net::{Ipv4Addr, UdpSocket},
    os::windows::io::{AsRawHandle, AsSocket, FromRawHandle, OwnedHandle},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{HANDLE, WAIT_FAILED},
    System::Threading::{CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForSingleObject},
};

/// The notification descriptor: a manual-reset event, signalled while there
/// is something to drain.
pub struct Notifier(OwnedHandle);

impl Notifier {
    /// The handles one notifier holds: the event.
    pub const DESCRIPTORS: u32 = 1;

    pub fn new() -> io::Result<Self> {
        // SAFETY: no security attributes, so the handle is not inherited, and
        // no name; manual reset (TRUE), initially clear (FALSE).
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a new event handle nothing else owns.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(event) }))
    }

    /// The event's handle, truncated to the `int32_t` output: 64-bit
    /// Windows handles have 32 significant bits, so the caller sign-extends
    /// it back (ADR 0008).
    pub fn descriptor(&self) -> i32 {
        self.0.as_raw_handle() as isize as i32
    }

    /// Signals the event, or clears it.
    pub fn set(&self, readable: bool) {
        // SAFETY (both): an event handle this notifier owns. Neither call
        // fails on a valid event.
        if readable {
            unsafe { SetEvent(self.0.as_raw_handle()) };
        } else {
            unsafe { ResetEvent(self.0.as_raw_handle()) };
        }
    }
}

/// Waits until the notification descriptor `descriptor` is signalled or
/// about `timeout` passes: a Windows wait can end up to a clock tick early,
/// which only makes the caller look again.
///
/// # Safety
///
/// `descriptor` is a notification descriptor that stays open for the call.
pub unsafe fn wait_raised(descriptor: i32, timeout: Duration) -> io::Result<()> {
    let event = descriptor as isize as HANDLE;
    // Whole milliseconds; a timeout too long to count waits forever.
    let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(INFINITE);
    // SAFETY: guaranteed by the caller: an open event handle.
    if unsafe { WaitForSingleObject(event, milliseconds) } == WAIT_FAILED {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// An instance's waker: raised by any thread, as often as it likes, and
/// cleared by its one waiter. The socket is nonblocking, so a raise into a
/// full buffer (already readable) only fails, and so does clearing an empty
/// one.
pub struct Waker(UdpSocket);

impl Waker {
    /// The handles one waker holds: the socket.
    pub const DESCRIPTORS: u32 = 1;

    /// The slot of a wait that ends when the waker is raised.
    pub fn watch(&self) -> Watch<'_> {
        Watch::new(self.0.as_socket(), Interest::Read)
    }

    /// Sends the socket one byte, making it readable.
    pub fn raise(&self) {
        let _ = self.0.send(&[u8::MAX]);
    }

    /// Receives every byte sent, clearing the socket. Any failure, the empty
    /// socket's `WouldBlock` among them, ends the clearing.
    pub fn clear(&self) {
        let mut byte = [u8::MAX];
        while self.0.recv(&mut byte).is_ok() {}
    }
}

impl Notifier {
    /// A waker: a loopback UDP socket connected to its own address.
    pub fn waker() -> io::Result<Waker> {
        // Port 0 asks for any free port. It is spelled with its type, which the
        // mutator leaves alone: port 1 would usually bind too.
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0u16))?;
        socket.connect(socket.local_addr()?)?;
        socket.set_nonblocking(true)?;
        Ok(Waker(socket))
    }
}
