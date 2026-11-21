//! Waiting for readiness on several handles at once (ADR 0003, Windows I/O
//! and transport): the shared tick loops say what they wait for, and each
//! platform decides how to wait.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("readiness seam requires macOS or Linux");

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::*;

/// What a watch waits for on its handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Interest {
    Read,
    Write,
}

/// What a wait reported for one watch.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Ready {
    /// Anything was reported: readiness, a hang-up or an error.
    pub any: bool,
    /// The handle is readable.
    pub readable: bool,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use super::{Interest, Ready};
    use crate::syscall::{checked, retry_if_interrupted};
    use std::{fs::File, io, os::fd::AsRawFd};

    /// A borrowed handle a wait can watch; it is never closed through this.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct Handle(i32);

    impl Handle {
        /// A client library notification, as its frozen `int32_t` output
        /// carries it (ADR 0008).
        pub fn notification(value: i32) -> Self {
            Self(value)
        }

        pub fn of(file: &File) -> Self {
            Self(file.as_raw_fd())
        }
    }

    /// One handle and what to wait for on it. An inactive watch is skipped
    /// and reports nothing.
    #[derive(Clone, Copy, Debug)]
    #[repr(transparent)]
    pub struct Watch(libc::pollfd);

    impl Watch {
        pub fn new(handle: Handle, interest: Interest, active: bool) -> Self {
            Self(libc::pollfd {
                // poll ignores a negative descriptor.
                fd: if active { handle.0 } else { -1 },
                events: events(interest),
                // Output-only.
                revents: Default::default(),
            })
        }
    }

    fn events(interest: Interest) -> libc::c_short {
        match interest {
            Interest::Read => libc::POLLIN,
            Interest::Write => libc::POLLOUT,
        }
    }

    /// Waits until a watch reports something or `milliseconds` pass. An
    /// interrupted wait returns early, reporting nothing.
    pub fn wait<const N: usize>(
        mut watches: [Watch; N],
        milliseconds: i32,
    ) -> io::Result<[Ready; N]> {
        let descriptors = watches.as_mut_ptr().cast::<libc::pollfd>();
        // SAFETY: Watch is a transparent pollfd, and the array outlives the call.
        checked(unsafe { libc::poll(descriptors, N as libc::nfds_t, milliseconds) })
            .map(drop)
            .or_else(retry_if_interrupted)?;
        Ok(watches.map(|watch| reported(watch.0.revents)))
    }

    pub(super) fn reported(events: libc::c_short) -> Ready {
        Ready {
            any: events != 0,
            readable: events & libc::POLLIN != 0,
        }
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::{Handle, Interest, Ready, Watch, unix::reported, wait};
    use std::{
        fs::File,
        io::{self, Write},
        os::fd::OwnedFd,
    };

    fn pipe() -> (File, File) {
        let (reader, writer) = io::pipe().unwrap();
        (
            File::from(OwnedFd::from(reader)),
            File::from(OwnedFd::from(writer)),
        )
    }

    const NOTHING: Ready = Ready {
        any: false,
        readable: false,
    };

    #[test]
    fn readable_and_writable_handles_report_what_they_are_ready_for() {
        let (reader, mut writer) = pipe();
        writer.write_all(b"x").unwrap();
        let ready = wait(
            [
                Watch::new(Handle::of(&reader), Interest::Read, true),
                Watch::new(Handle::of(&writer), Interest::Write, true),
            ],
            0,
        )
        .unwrap();
        assert_eq!(
            ready,
            [
                Ready {
                    any: true,
                    readable: true
                },
                Ready {
                    any: true,
                    readable: false
                }
            ]
        );
    }

    #[test]
    fn an_idle_handle_reports_nothing_once_the_timeout_passes() {
        let (reader, _writer) = pipe();
        let ready = wait([Watch::new(Handle::of(&reader), Interest::Read, true)], 1).unwrap();
        assert_eq!(ready, [NOTHING]);
    }

    #[test]
    fn an_inactive_watch_reports_nothing_even_when_ready() {
        let (reader, mut writer) = pipe();
        writer.write_all(b"x").unwrap();
        let ready = wait([Watch::new(Handle::of(&reader), Interest::Read, false)], 0).unwrap();
        assert_eq!(ready, [NOTHING]);
    }

    #[test]
    fn a_notification_value_is_watched_as_given() {
        let (reader, mut writer) = pipe();
        writer.write_all(b"x").unwrap();
        let value = std::os::fd::AsRawFd::as_raw_fd(&reader);
        let ready = wait(
            [Watch::new(
                Handle::notification(value),
                Interest::Read,
                true,
            )],
            0,
        )
        .unwrap();
        assert!(ready[0].readable);
    }

    #[test]
    fn a_hang_up_alone_is_reported_but_not_readable() {
        assert_eq!(
            reported(libc::POLLHUP),
            Ready {
                any: true,
                readable: false
            }
        );
        assert_eq!(reported(0), NOTHING);
    }

    #[test]
    fn an_invalid_handle_is_reported_but_not_readable() {
        let ready = wait(
            [Watch::new(
                Handle::notification(i32::MAX),
                Interest::Read,
                true,
            )],
            0,
        );
        // poll reports an invalid descriptor per entry, not as a failure.
        assert_eq!(
            ready.unwrap(),
            [Ready {
                any: true,
                readable: false
            }]
        );
    }
}
