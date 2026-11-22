//! The client library's notifier: a handle it raises and clears, for its
//! notification descriptor, and each instance's waker, which a wait watches
//! beside the instance's sockets.
#[cfg(target_os = "linux")]
#[path = "notifier/linux.rs"]
mod native;
#[cfg(target_os = "macos")]
#[path = "notifier/macos.rs"]
mod native;
/// An event for the notification descriptor (ADR 0008) and a socket for the
/// waker, which `WSAPoll` must watch beside the instance's sockets.
#[cfg(windows)]
#[path = "notifier/windows.rs"]
mod native;

pub use native::{Notifier, Waker, wait_raised};

#[cfg(unix)]
mod unix {
    use super::Notifier;
    use crate::waiting::{self, Interest, Watch};
    use std::{
        io,
        os::fd::{AsFd, BorrowedFd},
        time::Duration,
    };

    /// A waker is one more notifier: the wait polls its descriptor like a
    /// socket.
    pub type Waker = Notifier;

    impl Notifier {
        /// The slot of a wait that ends when the notifier is raised.
        pub fn watch(&self) -> Watch<'_> {
            Watch::new(self.as_fd(), Interest::Read)
        }
    }

    /// Waits until the notification descriptor `descriptor` is readable or
    /// `timeout` passes. A signal ends the wait early as `Interrupted`.
    ///
    /// # Safety
    ///
    /// `descriptor` is a notification descriptor that stays open for the
    /// call.
    pub unsafe fn wait_raised(descriptor: i32, timeout: Duration) -> io::Result<()> {
        // SAFETY: guaranteed by the caller.
        let descriptor = unsafe { BorrowedFd::borrow_raw(descriptor) };
        waiting::wait(&mut [Watch::new(descriptor, Interest::Read)], timeout).map(drop)
    }
}
