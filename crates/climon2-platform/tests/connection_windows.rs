//! #316: AF_UNIX stream sockets on Windows through socket2, with `WSAPoll`
//! readiness (ADR 0003, "Windows I/O and transport"), and the connect, bind
//! and delete error codes stale-socket cleanup rests on
//! (docs/for-humans/windows/quality.md, "Facts only CI can show").
#![cfg(windows)]
use climon2_platform::{
    connection::{self, Interest},
    runtime::{self, SocketListener, SocketStream},
    waiting::{self, Watch},
};
use std::{
    fs, io,
    io::{ErrorKind, Read, Write},
    os::windows::io::AsSocket,
    path::Path,
    time::{Duration, Instant},
};

const WSAENETDOWN: i32 = 10050;
const WSAEADDRINUSE: i32 = 10048;
const WSAECONNREFUSED: i32 = 10061;

/// A listener at `path`, a client connected to it, and the accepted peer.
fn connected(path: &Path) -> (SocketListener, SocketStream, SocketStream) {
    let listener = runtime::bind(path).unwrap();
    let client = connection::connect(path, Duration::from_secs(1)).unwrap();
    let (peer, _) = listener.accept().unwrap();
    (listener, client, peer)
}

fn refused(path: &Path) -> io::Error {
    connection::connect(path, Duration::from_millis(100)).unwrap_err()
}

#[test]
fn a_bound_socket_accepts_a_connection_that_carries_bytes_both_ways() {
    let directory = tempfile::tempdir().unwrap();
    let (_listener, mut client, mut peer) = connected(&directory.path().join("daemon.sock"));

    client.write_all(b"hello").unwrap();
    peer.write_all(b"world").unwrap();

    let mut received = [0; 5];
    peer.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"hello");
    client.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"world");
}

/// As #19 on Unix: probes wait for readiness only until their deadline.
#[test]
fn a_connection_waits_for_real_readiness_until_the_deadline() {
    let directory = tempfile::tempdir().unwrap();
    let (_listener, client, mut peer) = connected(&directory.path().join("daemon.sock"));
    connection::wait(&client, Interest::Write, Duration::from_secs(1)).unwrap();

    let started = Instant::now();
    let error = connection::wait(&client, Interest::Read, Duration::from_millis(50)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::TimedOut);
    assert!(started.elapsed() >= Duration::from_millis(50));

    peer.write_all(b"x").unwrap();
    connection::wait(&client, Interest::Read, Duration::from_secs(1)).unwrap();
}

/// As #310 on Unix: one wait reports which sockets are ready, and with
/// nothing ready it returns once the timeout passes.
#[test]
fn one_wait_reports_each_ready_socket_until_the_timeout() {
    let directory = tempfile::tempdir().unwrap();
    let (_quiet_listener, quiet, mut quiet_peer) = connected(&directory.path().join("q.sock"));
    let (_loud_listener, loud, mut loud_peer) = connected(&directory.path().join("l.sock"));

    let started = Instant::now();
    let mut watches = [Watch::stream(&quiet, Interest::Read)];
    assert_eq!(
        waiting::wait(&mut watches, Duration::from_millis(50)).unwrap(),
        0
    );
    assert!(started.elapsed() >= Duration::from_millis(50));
    assert!(!watches[0].ready());

    loud_peer.write_all(b"x").unwrap();
    let started = Instant::now();
    let mut watches = [
        Watch::stream(&quiet, Interest::Read),
        Watch::stream(&loud, Interest::Read),
        Watch::new(quiet.as_socket(), Interest::Write),
    ];
    assert_eq!(
        waiting::wait(&mut watches, Duration::from_secs(5)).unwrap(),
        2
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "a ready wait returns at once"
    );
    let ready: Vec<_> = watches.iter().map(Watch::ready).collect();
    assert_eq!(ready, [false, true, true]);
    assert!(watches[1].readable());
    assert!(!watches[2].readable());

    quiet_peer.write_all(b"y").unwrap();
    let mut watches = [Watch::stream(&quiet, Interest::ReadWrite)];
    // No deadline to keep: the wait ends only when something is ready.
    waiting::wait(&mut watches, Duration::MAX).unwrap();
    assert!(watches[0].readable());
}

#[test]
fn a_hung_up_peer_is_ready() {
    let directory = tempfile::tempdir().unwrap();
    let (_listener, client, peer) = connected(&directory.path().join("daemon.sock"));
    drop(peer);

    let mut watches = [Watch::stream(&client, Interest::Read)];
    assert_eq!(
        waiting::wait(&mut watches, Duration::from_secs(1)).unwrap(),
        1
    );
    assert!(watches[0].ready());
}

#[test]
fn a_listener_is_readable_once_a_connection_is_pending() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("daemon.sock");
    let listener = runtime::bind(&path).unwrap();
    let mut watches = [Watch::new(listener.as_socket(), Interest::Read)];
    assert_eq!(
        waiting::wait(&mut watches, Duration::from_millis(20)).unwrap(),
        0
    );

    let _client = connection::connect(&path, Duration::from_secs(1)).unwrap();

    assert_eq!(
        waiting::wait(&mut watches, Duration::from_secs(1)).unwrap(),
        1
    );
    assert!(watches[0].readable());
}

/// Windows refuses a connect to a missing path and to a socket file nobody
/// listens on alike, with `WSAECONNREFUSED`; both say nobody listens. A
/// missing directory fails with `WSAENETDOWN`, which proves nothing.
#[test]
fn a_missing_or_stale_socket_is_refused_and_says_nobody_listens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("daemon.sock");

    let missing = refused(&path);
    drop(runtime::bind(&path).unwrap());
    assert!(path.exists(), "a closed listener leaves its socket file");
    let stale = refused(&path);
    let no_directory = refused(&directory.path().join("gone").join("daemon.sock"));

    for error in [&missing, &stale] {
        assert_eq!(error.raw_os_error(), Some(WSAECONNREFUSED), "{error:?}");
        assert!(connection::nobody_listening(error), "{error:?}");
    }
    assert_eq!(no_directory.raw_os_error(), Some(WSAENETDOWN));
    assert!(!connection::nobody_listening(&no_directory));
    let timed_out = io::Error::from(ErrorKind::TimedOut);
    assert!(!connection::nobody_listening(&timed_out));
    let denied = io::Error::from_raw_os_error(5); // ERROR_ACCESS_DENIED
    assert!(!connection::nobody_listening(&denied));
}

/// Unlike `unlink` on Unix, nothing documents this; the listener keeps its
/// open connections, and new ones are refused.
#[test]
fn a_live_socket_file_can_be_deleted_and_then_refuses_new_connections() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("daemon.sock");
    let (_listener, mut client, mut peer) = connected(&path);

    fs::remove_file(&path).unwrap();

    assert!(!path.exists());
    assert_eq!(refused(&path).raw_os_error(), Some(WSAECONNREFUSED));
    client.write_all(b"x").unwrap();
    let mut received = [0];
    peer.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"x");
}

/// Windows never removes a socket file, so a stale one is deleted before
/// binding its path again.
#[test]
fn binding_over_a_stale_socket_file_fails_until_it_is_deleted() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("daemon.sock");
    drop(runtime::bind(&path).unwrap());

    let error = runtime::bind(&path).err().unwrap();
    assert_eq!(error.raw_os_error(), Some(WSAEADDRINUSE));

    fs::remove_file(&path).unwrap();
    drop(runtime::bind(&path).unwrap());
}

/// #318: the client library's sends take what the socket takes now; a send
/// to a closed peer fails rather than raising anything.
#[test]
fn a_send_carries_bytes_and_one_to_a_closed_peer_fails() {
    let directory = tempfile::tempdir().unwrap();
    let (_listener, client, mut peer) = connected(&directory.path().join("send.sock"));
    connection::quiet(&client).unwrap();
    assert_eq!(connection::send(&client, b"ab").unwrap(), 2);
    let mut received = [0; 2];
    peer.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"ab");
    drop(peer);
    let deadline = Instant::now() + Duration::from_secs(5);
    while connection::send(&client, b"c").is_ok() {
        assert!(
            Instant::now() < deadline,
            "sends to a closed peer kept succeeding"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
