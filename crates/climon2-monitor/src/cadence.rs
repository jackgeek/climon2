//! The UI's cadence (#119; operational limits, "UI, sampling and
//! responsiveness"): at most `ui.fps` frames painted a second, and client
//! work rotated fairly within `ui.client_quantum_ms` per client and
//! `ui.background_quantum_ms` per UI turn.
use std::time::{Duration, Instant};

/// When the next frame may be painted: the first at once, then each at
/// least one frame interval after the last. A frame not yet due is not
/// painted, never queued: the next one due paints the scene as it is then,
/// so obsolete animation frames are skipped.
pub(crate) struct Frames {
    interval: Duration,
    last: Option<Instant>,
}

impl Frames {
    /// At most `fps` frames a second (1 through 120).
    pub(crate) fn new(fps: u32) -> Self {
        Self {
            interval: Duration::from_secs(1) / fps,
            last: None,
        }
    }

    /// Whether a frame may be painted at `now`, counting it if so.
    pub(crate) fn due(&mut self, now: Instant) -> bool {
        let due = self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= self.interval);
        if due {
            self.last = Some(now);
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LIM-06 (pure supplement, render cadence): the first frame is due at
    /// once; at 30 fps the next only 33,333,333 ns after it, not a
    /// nanosecond sooner; a frame refused is not counted, so the next due
    /// one is measured from the last painted; at 1 and 120 fps the
    /// interval is one second and 8,333,333 ns.
    #[test]
    fn frames_are_due_at_most_once_per_frame_interval() {
        let start = Instant::now();
        let mut frames = Frames::new(30);
        assert!(frames.due(start));
        let interval = Duration::from_nanos(33_333_333);
        assert!(!frames.due(start + interval - Duration::from_nanos(1)));
        assert!(frames.due(start + interval));
        assert!(!frames.due(start + interval));
        assert!(frames.due(start + 2 * interval));
        for (fps, nanos) in [(1, 1_000_000_000), (120, 8_333_333)] {
            let mut frames = Frames::new(fps);
            assert!(frames.due(start));
            let interval = Duration::from_nanos(nanos);
            assert!(!frames.due(start + interval - Duration::from_nanos(1)));
            assert!(frames.due(start + interval));
        }
    }

    /// LIM-06 (pure supplement): a clock that steps backwards never makes a
    /// frame due early.
    #[test]
    fn an_earlier_instant_is_never_due() {
        let start = Instant::now() + Duration::from_secs(1);
        let mut frames = Frames::new(30);
        assert!(frames.due(start));
        assert!(!frames.due(start - Duration::from_millis(500)));
    }
}
