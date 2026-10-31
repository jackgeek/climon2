//! Reused public-API timing seam from the earlier local feasibility probe.
//! The caller must service this deadline during busy input as well as idle time.

use std::time::{Duration, Instant};

use vte::ansi::{Handler, Processor, Timeout};

#[derive(Default)]
pub struct FirstBeginTimeout {
    deadline: Option<Instant>,
}

impl FirstBeginTimeout {
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }
}

impl Timeout for FirstBeginTimeout {
    fn set_timeout(&mut self, duration: Duration) {
        self.deadline
            .get_or_insert_with(|| Instant::now() + duration.min(Duration::from_millis(150)));
    }

    fn clear_timeout(&mut self) {
        self.deadline = None;
    }

    fn pending_timeout(&self) -> bool {
        // Expiration alone must not release bytes before the observer sees the drain.
        self.deadline.is_some()
    }
}

pub fn flush_if_due<H: Handler>(
    parser: &mut Processor<FirstBeginTimeout>,
    handler: &mut H,
    now: Instant,
) -> bool {
    if parser
        .sync_timeout()
        .deadline()
        .is_some_and(|deadline| now >= deadline)
    {
        parser.stop_sync(handler);
        true
    } else {
        false
    }
}
