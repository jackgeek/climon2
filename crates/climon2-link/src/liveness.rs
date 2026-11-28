//! Keepalive and dead-link detection (#477): an established end shows it
//! is alive when it has sent nothing for a while, and declares the link
//! lost when it has heard nothing for longer. Both timings are fixed by
//! link revision 1; there is no setting and no negotiation.

use crate::Time;
use std::time::Duration;

/// How long an established end sends nothing before it sends a keepalive.
const KEEPALIVE_AFTER: Duration = Duration::from_secs(5);
/// How long an established end receives no unit before it declares the
/// link lost.
const SILENCE_MOST: Duration = Duration::from_secs(15);

/// When one end of a link last sent a unit and last received one that
/// decrypted.
pub(crate) struct Liveness {
    sent: Time,
    heard: Time,
}

impl Liveness {
    pub(crate) fn new() -> Self {
        let start = Time::since_start(Duration::ZERO);
        Self {
            sent: start,
            heard: start,
        }
    }

    /// This end sent at least one unit at `now`.
    pub(crate) fn sent(&mut self, now: Time) {
        self.sent = now;
    }

    /// At least one unit that decrypted arrived at `now`.
    pub(crate) fn heard(&mut self, now: Time) {
        self.heard = now;
    }

    /// Whether, at `now`, this end has sent nothing for 5 seconds.
    pub(crate) fn keepalive_due(&self, now: Time) -> bool {
        now >= self.sent.after(KEEPALIVE_AFTER)
    }

    /// Whether, at `now`, this end has heard nothing for 15 seconds.
    pub(crate) fn silent(&self, now: Time) -> bool {
        now >= self.heard.after(SILENCE_MOST)
    }

    /// The next moment either timing runs out.
    pub(crate) fn deadline(&self) -> Time {
        let keepalive = self.sent.after(KEEPALIVE_AFTER);
        keepalive.min(self.heard.after(SILENCE_MOST))
    }
}
