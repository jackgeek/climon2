//! The clock seam (ADR 0003): the time monitor links run on. The link code
//! never reads a clock; its caller reads this one and passes the time in.
//!
//! - [`monotonic`] measures deadlines and intervals.
//! - [`wall`] dates what is remembered, such as when a pairing was made.

use std::{
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// The time since this process first asked. It never goes back and no change
/// to the system's date moves it, so differences between two readings are
/// real elapsed time. Readings from two processes do not compare.
pub fn monotonic() -> Duration {
    static FIRST_ASKED: OnceLock<Instant> = OnceLock::new();
    FIRST_ASKED.get_or_init(Instant::now).elapsed()
}

/// The system's date, as the time since the Unix epoch. It moves when the
/// system's date is changed, so it dates records and never measures a
/// deadline. A system set before 1970 reads as the epoch.
pub fn wall() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}
