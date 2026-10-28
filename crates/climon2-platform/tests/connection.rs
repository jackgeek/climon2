#![cfg(target_os = "macos")]

use climon2_platform::connection::{self, Interest};
use std::{
    io::{ErrorKind, Write},
    os::unix::net::UnixListener,
    time::{Duration, Instant},
};

/// #19: probes reach a daemon socket and wait for readiness only until their deadline.
#[test]
fn issue19_socket_connection_waits_for_real_readiness_until_the_deadline() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("daemon.sock");
    assert!(connection::connect(&path, Duration::from_millis(100)).is_err());
    let listener = UnixListener::bind(&path).unwrap();
    let stream = connection::connect(&path, Duration::from_secs(1)).unwrap();
    let (mut peer, _) = listener.accept().unwrap();
    connection::wait(&stream, Interest::Write, Duration::from_secs(1)).unwrap();
    let started = Instant::now();
    let error = connection::wait(&stream, Interest::Read, Duration::from_millis(50)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::TimedOut);
    assert!(started.elapsed() >= Duration::from_millis(50));
    peer.write_all(b"x").unwrap();
    connection::wait(&stream, Interest::Read, Duration::from_secs(1)).unwrap();
}
