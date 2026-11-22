//! Waiting for readiness over a set of handles: the one wait every tick loop
//! and deadline shares (ADR 0003, Windows I/O and transport).
use std::{io, os::fd::BorrowedFd, time::Duration};

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("waiting seam requires macOS or Linux");

/// What a wait wants from one handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Interest {
    Read,
    Write,
    ReadWrite,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::{Watch, wait};

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use super::*;
    use std::{marker::PhantomData, os::fd::AsRawFd};

    impl Interest {
        fn events(self) -> libc::c_short {
            match self {
                Self::Read => libc::POLLIN,
                Self::Write => libc::POLLOUT,
                Self::ReadWrite => libc::POLLIN | libc::POLLOUT,
            }
        }
    }

    /// One slot of a wait: a borrowed handle and what is wanted from it, or
    /// an idle slot that keeps its place and is never ready.
    #[repr(transparent)]
    pub struct Watch<'a> {
        entry: libc::pollfd,
        handle: PhantomData<BorrowedFd<'a>>,
    }

    impl<'a> Watch<'a> {
        pub fn new(handle: BorrowedFd<'a>, interest: Interest) -> Self {
            Self::entry(handle.as_raw_fd(), interest.events())
        }

        /// A slot poll ignores.
        pub fn idle() -> Self {
            Self::entry(-1, libc::c_short::default())
        }

        fn entry(fd: libc::c_int, events: libc::c_short) -> Self {
            Self {
                entry: libc::pollfd {
                    fd,
                    events,
                    // Output-only.
                    revents: libc::c_short::default(),
                },
                handle: PhantomData,
            }
        }

        /// The last wait reported anything for this handle: what was wanted,
        /// a hang-up or an error.
        pub fn ready(&self) -> bool {
            self.entry.revents != 0
        }

        /// The last wait reported this handle readable.
        pub fn readable(&self) -> bool {
            self.entry.revents & libc::POLLIN != 0
        }
    }

    /// Waits until a watched handle is ready or `timeout` passes, rounded up
    /// to whole milliseconds and saturating, and returns how many are ready.
    /// Every failure, an interrupting signal included, is the caller's to
    /// judge.
    pub fn wait(watches: &mut [Watch<'_>], timeout: Duration) -> io::Result<usize> {
        let milliseconds = i32::try_from(timeout.as_micros().div_ceil(1000)).unwrap_or(i32::MAX);
        // SAFETY: a Watch is exactly a pollfd, and poll reads and updates
        // exactly `watches`' entries for the call's duration.
        let ready = unsafe {
            libc::poll(
                watches.as_mut_ptr().cast(),
                watches.len() as libc::nfds_t,
                milliseconds,
            )
        };
        crate::syscall::checked(ready).map(|ready| ready as usize)
    }
}
