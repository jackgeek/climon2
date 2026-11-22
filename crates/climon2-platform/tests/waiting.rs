#![cfg(any(target_os = "macos", target_os = "linux"))]

use climon2_platform::waiting::{self, Interest, Watch};
use std::{
    io::Write,
    os::{fd::AsFd, unix::net::UnixStream},
    time::{Duration, Instant},
};

/// One wait covers a set of handles: it reports which are ready, never an
/// idle slot, and with nothing ready it returns once the timeout passes.
#[test]
fn issue310_one_wait_reports_each_ready_handle_until_the_timeout() {
    let (mut quiet_peer, quiet) = UnixStream::pair().unwrap();
    let (mut loud_peer, loud) = UnixStream::pair().unwrap();
    let started = Instant::now();
    let mut watches = [Watch::new(quiet.as_fd(), Interest::Read), Watch::idle()];
    let count = waiting::wait(&mut watches, Duration::from_millis(50)).unwrap();
    assert_eq!(count, 0);
    assert!(started.elapsed() >= Duration::from_millis(50));
    assert!(!watches[0].ready());

    loud_peer.write_all(b"x").unwrap();
    let mut watches = [
        Watch::new(quiet.as_fd(), Interest::Read),
        Watch::idle(),
        Watch::new(loud.as_fd(), Interest::Read),
        Watch::new(quiet.as_fd(), Interest::Write),
    ];
    let count = waiting::wait(&mut watches, Duration::from_secs(1)).unwrap();
    assert_eq!(count, 2);
    let ready: Vec<_> = watches.iter().map(Watch::ready).collect();
    assert_eq!(ready, [false, false, true, true]);
    assert!(watches[2].readable());
    assert!(!watches[3].readable());

    quiet_peer.write_all(b"y").unwrap();
    let mut watches = [Watch::new(quiet.as_fd(), Interest::ReadWrite)];
    waiting::wait(&mut watches, Duration::from_secs(1)).unwrap();
    assert!(watches[0].readable());
}

/// A peer that hangs up makes the handle ready without data to read.
#[test]
fn issue310_a_hung_up_peer_is_ready() {
    let (peer, stream) = UnixStream::pair().unwrap();
    drop(peer);
    let mut watches = [Watch::new(stream.as_fd(), Interest::Read)];
    assert_eq!(
        waiting::wait(&mut watches, Duration::from_secs(1)).unwrap(),
        1
    );
    assert!(watches[0].ready());
}

/// Idle slots are never waited on, so a set of them simply waits the timeout.
#[test]
fn issue310_idle_slots_wait_out_the_timeout() {
    let started = Instant::now();
    let mut watches = [Watch::idle(), Watch::idle()];
    assert_eq!(
        waiting::wait(&mut watches, Duration::from_millis(30)).unwrap(),
        0
    );
    assert!(started.elapsed() >= Duration::from_millis(30));
    assert!(!watches[1].ready());
}
