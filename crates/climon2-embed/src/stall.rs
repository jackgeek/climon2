//! The delivery stall deadline (#119, LIM-02): how long a client instance
//! may stay congested, its unprocessed deliveries filling its allowance so
//! that it is not drained, before its consumer fails it explicitly.
use climon2_core::SessionId;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

/// Since when each congested client has not been drained.
#[derive(Default)]
pub struct Stalls(BTreeMap<SessionId, Instant>);

impl Stalls {
    /// `id` is congested at `now`: whether that has lasted `deadline`
    /// since it was first found congested and not drained since. Once it
    /// has, its congestion is forgotten: the caller fails it.
    pub fn congested(&mut self, id: &SessionId, now: Instant, deadline: Duration) -> bool {
        let since = *self.0.entry(id.clone()).or_insert(now);
        let stalled = now.checked_duration_since(since + deadline).is_some();
        if stalled {
            self.0.remove(id);
        }
        stalled
    }

    /// `id` is drained again: it is not congested.
    pub fn drained(&mut self, id: &SessionId) {
        self.0.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(index: u8) -> SessionId {
        format!("57a11000-0000-4000-8000-0000000000{index:02x}")
            .parse()
            .unwrap()
    }

    const DEADLINE: Duration = Duration::from_millis(100);

    fn after(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    /// LIM-02 (pure supplement): congestion is timed from when it was
    /// first found, stalls exactly at the deadline and not a millisecond
    /// before, and is timed afresh once it has stalled.
    #[test]
    fn congestion_stalls_at_the_deadline_and_not_before() {
        let (mut stalls, start) = (Stalls::default(), Instant::now());
        assert!(!stalls.congested(&id(1), start, DEADLINE));
        assert!(!stalls.congested(&id(1), after(start, 99), DEADLINE));
        assert!(stalls.congested(&id(1), after(start, 100), DEADLINE));
        assert!(!stalls.congested(&id(1), after(start, 250), DEADLINE));
        assert!(!stalls.congested(&id(1), after(start, 349), DEADLINE));
        assert!(stalls.congested(&id(1), after(start, 350), DEADLINE));
    }

    /// LIM-02 (pure supplement): a drain ends a client's congestion, so
    /// the next is timed from its own start; each client is timed apart.
    #[test]
    fn a_drain_ends_congestion_and_each_client_is_timed_apart() {
        let (mut stalls, start) = (Stalls::default(), Instant::now());
        assert!(!stalls.congested(&id(1), start, DEADLINE));
        assert!(!stalls.congested(&id(2), after(start, 60), DEADLINE));
        stalls.drained(&id(1));
        assert!(!stalls.congested(&id(1), after(start, 100), DEADLINE));
        assert!(!stalls.congested(&id(2), after(start, 159), DEADLINE));
        assert!(stalls.congested(&id(2), after(start, 160), DEADLINE));
        assert!(!stalls.congested(&id(1), after(start, 199), DEADLINE));
        assert!(stalls.congested(&id(1), after(start, 200), DEADLINE));
        stalls.drained(&id(3));
    }

    /// A zero deadline stalls at once.
    #[test]
    fn a_zero_deadline_stalls_at_once() {
        let mut stalls = Stalls::default();
        assert!(stalls.congested(&id(1), Instant::now(), Duration::ZERO));
    }
}
