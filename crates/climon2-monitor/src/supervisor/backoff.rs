//! Passive retry backoff (spec #73, Client supervisor, "Recovery"; the
//! operational limits' "Passive retry"): pure. The delay starts at
//! `retry.initial_ms` and doubles with saturation to `retry.cap_ms`; each
//! delay gets fresh uniform jitter of up to `retry.jitter_percent` either way,
//! clamped to the cap. There is no attempt-count cutoff.
use climon2_core::config::Backoff;
use std::{
    hash::{BuildHasher, Hasher},
    time::Duration,
};

/// The delay before retry `attempt` (0 for the first after a failure),
/// jittered by `random`, a uniformly random value, which picks one
/// millisecond of the spread.
pub(super) fn delay(backoff: Backoff, attempt: u32, random: u64) -> Duration {
    let cap = u64::from(backoff.cap_ms);
    let doubled = 2u64.saturating_pow(attempt);
    let base = u64::from(backoff.initial_ms)
        .saturating_mul(doubled)
        .min(cap);
    let spread = base * u64::from(backoff.jitter_percent) / 100;
    let jittered = base - spread + random % (2 * spread + 1);
    Duration::from_millis(jittered.min(cap))
}

/// A fresh random value: the standard library's per-hasher random keys.
pub(super) fn random() -> u64 {
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BACKOFF: Backoff = Backoff {
        initial_ms: 1000,
        cap_ms: 30_000,
        jitter_percent: 20,
    };

    fn millis(backoff: Backoff, attempt: u32, random: u64) -> u128 {
        delay(backoff, attempt, random).as_millis()
    }

    /// REC-01 (pure supplement): without jitter the delay starts at
    /// `retry.initial_ms` and doubles each attempt until the cap, the
    /// doubling that reaches it and the one past it both held there, and
    /// it never overflows however many attempts fail.
    #[test]
    fn the_delay_doubles_from_its_start_to_its_cap() {
        let steady = Backoff {
            jitter_percent: 0,
            ..BACKOFF
        };
        let delays: Vec<_> = (0..8).map(|attempt| millis(steady, attempt, 0)).collect();
        assert_eq!(
            delays,
            [1000, 2000, 4000, 8000, 16_000, 30_000, 30_000, 30_000]
        );
        let exact = Backoff {
            cap_ms: 16_000,
            ..steady
        };
        assert_eq!(millis(exact, 4, 0), 16_000);
        assert_eq!(millis(exact, 5, 0), 16_000);
        assert_eq!(millis(steady, u32::MAX, 0), 30_000);
        let huge = Backoff {
            initial_ms: u32::MAX,
            cap_ms: u32::MAX,
            jitter_percent: 0,
        };
        assert_eq!(millis(huge, 63, 0), u128::from(u32::MAX));
    }

    /// REC-01 (pure supplement): each delay takes its jitter from a fresh
    /// random value over the whole spread, `retry.jitter_percent` either way
    /// of the base (the lowest and highest values, and one past the highest
    /// wrapping to the lowest), and the jittered delay never exceeds the cap.
    #[test]
    fn jitter_spreads_each_delay_either_way_within_the_cap() {
        assert_eq!(millis(BACKOFF, 1, 0), 1600);
        assert_eq!(millis(BACKOFF, 1, 400), 2000);
        assert_eq!(millis(BACKOFF, 1, 800), 2400);
        assert_eq!(millis(BACKOFF, 1, 801), 1600);
        assert_eq!(millis(BACKOFF, 1, 1601), 2400);
        assert_eq!(millis(BACKOFF, 9, 0), 24_000);
        assert_eq!(millis(BACKOFF, 9, 12_000), 30_000);
        let widest = Backoff {
            jitter_percent: 50,
            ..BACKOFF
        };
        assert_eq!(millis(widest, 0, 0), 500);
        assert_eq!(millis(widest, 0, 1000), 1500);
    }

    /// REC-01 (pure supplement): each delay's random value is fresh.
    #[test]
    fn each_random_value_is_fresh() {
        let values: std::collections::BTreeSet<_> = (0..8).map(|_| random()).collect();
        assert_eq!(values.len(), 8);
    }
}
