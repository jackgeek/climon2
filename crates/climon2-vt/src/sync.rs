//! The first-begin latch for synchronized output (`CSI ? 2026 h/l`).

use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Latch {
    deadline: Option<Instant>,
}

impl Latch {
    /// A begin opens an update that ends 150 ms later; begins while it is
    /// open never extend it. An end closes it.
    pub(crate) fn observe(&mut self, begin: Option<bool>, now: Instant) {
        match begin {
            Some(true) if !self.pending(now) => self.deadline = Some(now + DEADLINE),
            Some(false) => self.deadline = None,
            _ => {}
        }
    }

    pub(crate) fn pending(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now < deadline)
    }

    /// Close an expired update; true when this call closed it.
    pub(crate) fn tick(&mut self, now: Instant) -> bool {
        let expired = self.deadline.is_some() && !self.pending(now);
        if expired {
            self.deadline = None;
        }
        expired
    }
}
