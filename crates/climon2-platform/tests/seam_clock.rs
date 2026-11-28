//! The clock seam's contract suite (ONE-03): every platform's clock passes
//! these by itself, with no link code and no second machine.

use climon2_platform::clock;
use std::{thread, time::Duration};

#[test]
fn monotonic_time_advances_by_at_least_the_time_slept() {
    let before = clock::monotonic();
    thread::sleep(Duration::from_millis(50));
    let after = clock::monotonic();

    let passed = after
        .checked_sub(before)
        .expect("monotonic time never goes back");
    assert!(
        passed >= Duration::from_millis(50),
        "only {passed:?} passed"
    );
    assert!(passed < Duration::from_secs(30), "{passed:?} passed");
}

#[test]
fn monotonic_time_never_goes_back_across_threads() {
    let readers: Vec<_> = (0..4)
        .map(|_| {
            thread::spawn(|| {
                let mut last = clock::monotonic();
                for _ in 0..10_000 {
                    let now = clock::monotonic();
                    assert!(now >= last, "{now:?} follows {last:?}");
                    last = now;
                }
                last
            })
        })
        .collect();
    let latest = readers
        .into_iter()
        .map(|reader| reader.join().expect("a reader finishes"))
        .max()
        .expect("there are readers");

    assert!(clock::monotonic() >= latest);
}

#[test]
fn wall_time_counts_from_the_unix_epoch() {
    // 2026-11-01T00:00:00Z and 2100-01-01T00:00:00Z: this suite was written
    // between them, and a clock that does not count from 1970 is outside them.
    const WRITTEN_AFTER: Duration = Duration::from_secs(1_793_491_200);
    const FAR_FUTURE: Duration = Duration::from_secs(4_102_444_800);

    let now = clock::wall();

    assert!(now > WRITTEN_AFTER, "{now:?}");
    assert!(now < FAR_FUTURE, "{now:?}");
}

#[test]
fn wall_time_keeps_pace_with_monotonic_time() {
    let (wall, monotonic) = (clock::wall(), clock::monotonic());
    thread::sleep(Duration::from_millis(50));
    let wall = clock::wall().saturating_sub(wall);
    let monotonic = clock::monotonic() - monotonic;

    assert!(wall >= Duration::from_millis(40), "wall moved {wall:?}");
    assert!(wall.abs_diff(monotonic) < Duration::from_secs(5));
}
