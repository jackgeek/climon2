#![cfg(any(target_os = "macos", target_os = "linux"))]

use climon2_platform::{
    connection,
    notifier::Notifier,
    waiting::{self, Interest, Watch},
};
use std::{
    io::ErrorKind,
    os::{fd::AsFd, unix::net::UnixStream},
    time::Duration,
};

fn readable(notifier: &Notifier) -> bool {
    let mut watches = [Watch::new(notifier.as_fd(), Interest::Read)];
    waiting::wait(&mut watches, Duration::ZERO).unwrap();
    watches[0].readable()
}

/// #312: a notifier is readable from a raise until it is cleared, however
/// often it was raised, and its descriptor is the one waited on.
#[test]
fn issue312_a_notifier_is_readable_from_a_raise_until_cleared() {
    for notifier in [Notifier::new().unwrap(), Notifier::waker().unwrap()] {
        assert!(!readable(&notifier));
        notifier.raise();
        notifier.raise();
        assert!(readable(&notifier));
        notifier.clear();
        assert!(!readable(&notifier));
        notifier.set(true);
        assert!(readable(&notifier));
        notifier.set(false);
        assert!(!readable(&notifier));
        assert!(notifier.descriptor() >= 0);
    }
}

/// #312: a waker never blocks: clearing it unraised returns at once.
#[test]
fn issue312_a_waker_never_blocks() {
    let waker = Notifier::waker().unwrap();
    waker.clear();
    for _ in 0..100_000 {
        waker.raise();
    }
    assert!(readable(&waker));
}

/// #312: a quiet socket's send to a closed peer fails with EPIPE. (Rust
/// test binaries ignore SIGPIPE, so this cannot observe the signal itself.)
#[test]
fn issue312_a_send_to_a_closed_peer_fails_with_epipe() {
    let (ours, peer) = UnixStream::pair().unwrap();
    connection::quiet(ours.as_fd()).unwrap();
    assert_eq!(connection::send(ours.as_fd(), b"ab").unwrap(), 2);
    drop(peer);
    let error = connection::send(ours.as_fd(), b"c").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::BrokenPipe);
}
