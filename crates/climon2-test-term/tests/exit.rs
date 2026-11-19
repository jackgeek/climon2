#![cfg(any(target_os = "macos", target_os = "linux"))]

use climon2_test_term::exit_within;
use nix::{errno::Errno, sys::signal, unistd::Pid};
use std::{
    process::Command,
    time::{Duration, Instant},
};

/// Whether `pid` is gone: neither running nor a zombie waiting to be reaped.
fn reaped(pid: u32) -> bool {
    signal::kill(Pid::from_raw(pid as i32), None) == Err(Errno::ESRCH)
}

/// A child that exits within the bound is reaped with its own status.
#[test]
fn a_child_that_exits_in_time_is_reaped_with_its_own_status() {
    let child = Command::new("/bin/sh")
        .args(["-c", "exit 7"])
        .spawn()
        .unwrap();
    let pid = child.id();
    let status = exit_within(child, Duration::from_secs(30));
    assert_eq!(status.and_then(|status| status.code()), Some(7));
    assert!(reaped(pid));
}

/// A child still running at the deadline is killed and reaped, so a wedged child fails its
/// test instead of hanging it or outliving it. A session daemon is known only by its PID.
#[test]
fn a_child_still_running_at_the_deadline_is_killed_and_reaped() {
    let child = Command::new("/bin/sleep").arg("60").spawn().unwrap();
    let pid = child.id();
    // Released so that only its PID identifies it, like a daemon's.
    drop(child);
    let started = Instant::now();
    assert_eq!(exit_within(pid, Duration::from_millis(100)), None);
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(30), "{elapsed:?}");
    assert!(reaped(pid));
}
