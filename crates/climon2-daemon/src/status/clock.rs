//! The detector's clock and random source in the daemon. Released daemons
//! use the real clock; test builds can substitute a controllable one.
use super::{Now, Random};
use climon2_core::wire::{Counter, Utc};
use std::time::{Instant, SystemTime};

/// The clock a daemon turn reads once, at its start.
pub trait Clock {
    fn now(&mut self) -> Now;

    /// The turn that read the clock ended: its effects are queued.
    fn settled(&mut self) {}
}

/// Monotonic time since the daemon's initialization, and the system's UTC.
pub struct RealClock {
    origin: Instant,
}

impl Clock for RealClock {
    fn now(&mut self) -> Now {
        let since_epoch = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let utc = u64::try_from(since_epoch.as_millis())
            .ok()
            .and_then(Utc::from_unix_millis)
            .expect("the system clock is before year 10000");
        Now {
            monotonic: self.origin.elapsed(),
            utc,
        }
    }
}

/// Random draws from the operating system's generator.
pub struct SystemRandom;

impl Random for SystemRandom {
    fn next(&mut self) -> u64 {
        uuid::Uuid::new_v4().as_u64_pair().0
    }
}

/// The daemon's clock: the real one, or with `test-hooks` the controllable
/// one `CLIMON2_DAEMON_CLOCK` names.
pub fn daemon_clock() -> Box<dyn Clock> {
    #[cfg(feature = "test-hooks")]
    if let Some(path) = std::env::var_os("CLIMON2_DAEMON_CLOCK") {
        return Box::new(hooked::HookedClock::new(path.into()));
    }
    Box::new(RealClock {
        origin: Instant::now(),
    })
}

/// The first attention episode: 1, or with `test-hooks` the counter
/// `CLIMON2_DAEMON_FIRST_EPISODE` gives, to reach exhaustion.
pub fn first_episode() -> Counter {
    #[cfg(feature = "test-hooks")]
    if let Some(first) = std::env::var("CLIMON2_DAEMON_FIRST_EPISODE")
        .ok()
        .and_then(|text| text.parse().ok())
    {
        return first;
    }
    Counter::new(1).expect("1 is a counter")
}

#[cfg(feature = "test-hooks")]
mod hooked {
    use super::{Clock, Now};
    use climon2_core::wire::Utc;
    use std::{ffi::OsString, fs, path::PathBuf, time::Duration};

    /// A clock the test sets: the file holds milliseconds, both the
    /// monotonic reading and the UTC time since the Unix epoch. Each turn
    /// reads it; an unreadable value keeps the previous reading. Once a
    /// turn that read a new value ends, the value is written to the file's
    /// `.ack` sibling, so the test knows that turn's effects are queued.
    pub struct HookedClock {
        path: PathBuf,
        acknowledged: PathBuf,
        millis: u64,
        settled: Option<u64>,
    }

    impl HookedClock {
        pub fn new(path: PathBuf) -> Self {
            let mut acknowledged = OsString::from(&path);
            acknowledged.push(".ack");
            Self {
                path,
                acknowledged: acknowledged.into(),
                millis: u64::default(),
                settled: None,
            }
        }

        fn read(&self) -> Option<u64> {
            let millis = fs::read_to_string(&self.path).ok()?.trim().parse().ok()?;
            Utc::from_unix_millis(millis).map(|_| millis)
        }
    }

    impl Clock for HookedClock {
        fn now(&mut self) -> Now {
            self.millis = self.read().unwrap_or(self.millis);
            Now {
                monotonic: Duration::from_millis(self.millis),
                utc: Utc::from_unix_millis(self.millis).expect("a value read as a UTC time"),
            }
        }

        fn settled(&mut self) {
            if self.settled != Some(self.millis) {
                let _ = fs::write(&self.acknowledged, self.millis.to_string());
                self.settled = Some(self.millis);
            }
        }
    }
}
