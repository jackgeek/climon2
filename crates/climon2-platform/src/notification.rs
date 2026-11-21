//! Pollable notification descriptors (native revision 1): a client library
//! raises and clears its own through a [`Notifier`], and the monitor waits on
//! every instance's at once with [`wait`].
#[cfg(target_os = "linux")]
#[path = "notification/linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "notification/macos.rs"]
mod platform;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use platform::Notifier;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use crate::readiness::{self, Handle, Interest, Watch};

    /// Waits one tick of `milliseconds` for any of `descriptors` to be
    /// raised, or simply one tick when there are none. The caller looks at
    /// every instance after it whatever woke it, so a failed or interrupted
    /// wait only ends it early.
    pub fn wait(descriptors: &[i32], milliseconds: i32) {
        let watches: Vec<_> = descriptors
            .iter()
            .map(|&descriptor| watch(descriptor))
            .collect();
        let _ = readiness::wait_all(&watches, milliseconds);
    }

    fn watch(descriptor: i32) -> Watch {
        Watch::new(Handle::notification(descriptor), Interest::Read, true)
    }

    #[cfg(test)]
    mod tests {
        use super::{super::Notifier, wait, watch};
        use crate::readiness;
        use std::time::{Duration, Instant};

        const TICK: i32 = 20;
        const FOREVER: i32 = 600_000;

        fn raised(notifier: &Notifier) -> bool {
            readiness::wait([watch(notifier.descriptor())], 0).unwrap()[0].readable
        }

        /// A notifier is readable exactly while raised, however often it was
        /// raised, until it is cleared; `set` raises or clears it.
        #[test]
        fn a_notifier_is_readable_from_its_raise_until_its_clear() {
            for notifier in [Notifier::new().unwrap(), Notifier::waker().unwrap()] {
                assert!(!raised(&notifier));
                notifier.raise();
                notifier.raise();
                assert!(raised(&notifier));
                notifier.clear();
                assert!(!raised(&notifier));
                notifier.set(true);
                assert!(raised(&notifier));
                notifier.set(false);
                assert!(!raised(&notifier));
            }
        }

        /// A raised descriptor ends the wait at once.
        #[test]
        fn a_raised_descriptor_ends_the_wait() {
            let idle = Notifier::new().unwrap();
            let notifier = Notifier::new().unwrap();
            notifier.raise();
            let started = Instant::now();
            wait(&[idle.descriptor(), notifier.descriptor()], FOREVER);
            assert!(started.elapsed() < Duration::from_millis(FOREVER as u64));
        }

        /// With nothing raised, or nothing to wait on, the wait lasts its
        /// tick.
        #[test]
        fn a_wait_with_nothing_raised_lasts_its_tick() {
            let idle = Notifier::new().unwrap();
            for descriptors in [vec![idle.descriptor()], vec![]] {
                let started = Instant::now();
                wait(&descriptors, TICK);
                assert!(started.elapsed() >= Duration::from_millis(TICK as u64));
            }
        }
    }
}
