//! The socket seam's contract suite (ONE-03): every platform's sockets pass
//! these by themselves, over loopback, with no link code and no second
//! machine. A listener honours its deadlines: for a connection to arrive,
//! for a dial to connect and for bytes to be read.
//!
//! Unix only for now: the seam suites on Windows wait until monitor links
//! on Windows are specified, and a full accept queue refuses there.
#![cfg(unix)]

use climon2_platform::link_socket::{self, LinkListener, LinkStream};
use std::{
    io,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

/// Long enough that only a hang reaches it.
const GENEROUS: Duration = Duration::from_secs(10);
/// The deadline every expiry test sets.
const SHORT: Duration = Duration::from_millis(200);

fn loopback() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
}

fn listener() -> (LinkListener, SocketAddr) {
    let listener = link_socket::listen(loopback()).expect("loopback listens");
    let address = listener
        .local_address()
        .expect("a bound listener has an address");
    (listener, address)
}

/// A dialled stream and the listener's end of it.
fn pair() -> (LinkStream, LinkStream) {
    let (listener, address) = listener();
    let dialled = link_socket::dial(address, GENEROUS).expect("the dial connects");
    let accepted = listener
        .accept(GENEROUS)
        .expect("the connection is accepted");
    (dialled, accepted)
}

/// Runs `wait`, which must give up on `SHORT`: with a timed-out error, no
/// sooner than the deadline and long before a hang.
fn expires<T: std::fmt::Debug>(wait: impl FnOnce() -> io::Result<T>) {
    let started = Instant::now();
    let error = wait().expect_err("nothing arrived, so the wait expires");
    let waited = started.elapsed();

    assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
    assert!(waited >= SHORT, "gave up after only {waited:?}");
    assert!(waited < GENEROUS / 2, "gave up only after {waited:?}");
}

fn read_exactly(stream: &mut LinkStream, length: usize) -> Vec<u8> {
    let mut bytes = vec![0; length];
    let mut filled = 0;
    while filled < length {
        let read = stream
            .read(&mut bytes[filled..], GENEROUS)
            .expect("bytes arrive");
        assert_ne!(read, 0, "the stream ended after {filled} bytes");
        filled += read;
    }
    bytes
}

#[test]
fn a_listener_on_port_zero_reports_the_port_it_was_given() {
    let (_listener, address) = listener();

    assert_eq!(address.ip(), Ipv4Addr::LOCALHOST);
    assert_ne!(address.port(), 0);
}

#[test]
fn bytes_cross_in_both_directions() {
    let (mut dialled, mut accepted) = pair();

    dialled
        .write_all(b"opening request")
        .expect("the dialler sends");
    assert_eq!(read_exactly(&mut accepted, 15), b"opening request");
    accepted
        .write_all(b"opening reply")
        .expect("the listener sends");
    assert_eq!(read_exactly(&mut dialled, 13), b"opening reply");
}

#[test]
fn a_megabyte_crosses_whole_and_in_order() {
    let (mut dialled, mut accepted) = pair();
    let sent: Vec<u8> = (0..1_048_576u32).map(|place| (place % 251) as u8).collect();

    let received = thread::scope(|scope| {
        let reader = scope.spawn(|| read_exactly(&mut accepted, 1_048_576));
        dialled.write_all(&sent).expect("the megabyte is sent");
        reader.join().expect("the megabyte is read")
    });

    assert!(received == sent);
}

#[test]
fn a_read_takes_what_has_arrived_without_waiting_for_a_full_buffer() {
    let (mut dialled, mut accepted) = pair();
    dialled.write_all(b"abc").expect("three bytes are sent");

    let mut buffer = [0; 64];
    let read = accepted.read(&mut buffer, GENEROUS).expect("bytes arrive");

    assert!((1..=3).contains(&read));
    assert_eq!(&buffer[..read], &b"abc"[..read]);
}

#[test]
fn each_end_knows_the_others_address() {
    let (listener, address) = listener();
    let dialled = link_socket::dial(address, GENEROUS).expect("the dial connects");
    let accepted = listener
        .accept(GENEROUS)
        .expect("the connection is accepted");

    let dialler = accepted
        .peer_address()
        .expect("the listener sees the dialler");

    assert_eq!(
        dialled
            .peer_address()
            .expect("the dialler sees the listener"),
        address
    );
    assert_eq!(dialler.ip(), Ipv4Addr::LOCALHOST);
    assert_ne!(dialler.port(), address.port());
}

#[test]
fn a_listener_accepts_connections_in_the_order_they_were_made() {
    let (listener, address) = listener();
    let mut first = link_socket::dial(address, GENEROUS).expect("the first dial connects");
    first.write_all(b"1").expect("the first sends");
    let mut first_accepted = listener.accept(GENEROUS).expect("the first is accepted");
    let mut second = link_socket::dial(address, GENEROUS).expect("the second dial connects");
    second.write_all(b"2").expect("the second sends");
    let mut second_accepted = listener.accept(GENEROUS).expect("the second is accepted");

    assert_eq!(read_exactly(&mut first_accepted, 1), b"1");
    assert_eq!(read_exactly(&mut second_accepted, 1), b"2");
}

#[test]
fn a_closed_peer_reads_as_the_end_of_the_stream() {
    let (dialled, mut accepted) = pair();
    drop(dialled);

    let mut buffer = [0; 8];
    assert_eq!(
        accepted
            .read(&mut buffer, GENEROUS)
            .expect("the end is not an error"),
        0
    );
}

#[test]
fn the_listeners_end_honours_a_read_deadline() {
    let (_dialled, mut accepted) = pair();

    expires(|| accepted.read(&mut [0; 8], SHORT));
}

#[test]
fn the_diallers_end_honours_a_read_deadline() {
    let (mut dialled, _accepted) = pair();

    expires(|| dialled.read(&mut [0; 8], SHORT));
}

#[test]
fn a_stream_still_reads_after_a_deadline_expired() {
    let (mut dialled, mut accepted) = pair();
    expires(|| accepted.read(&mut [0; 8], SHORT));

    dialled.write_all(b"late").expect("the dialler sends");

    assert_eq!(read_exactly(&mut accepted, 4), b"late");
}

#[test]
fn a_read_deadline_is_measured_from_each_read() {
    let (mut dialled, mut accepted) = pair();
    thread::scope(|scope| {
        scope.spawn(|| {
            thread::sleep(SHORT * 2);
            dialled.write_all(b"x").expect("the dialler sends");
        });
        // Bytes that arrive after one short deadline are inside a long one.
        expires(|| accepted.read(&mut [0; 8], SHORT));
        assert_eq!(read_exactly(&mut accepted, 1), b"x");
    });
}

#[test]
fn a_listener_honours_an_accept_deadline_and_still_accepts_afterwards() {
    let (listener, address) = listener();

    expires(|| listener.accept(SHORT));

    let _dialled = link_socket::dial(address, GENEROUS).expect("the dial connects");
    listener
        .accept(GENEROUS)
        .expect("the connection is accepted");
}

#[test]
fn an_accept_returns_as_soon_as_a_connection_arrives() {
    let (listener, address) = listener();
    thread::scope(|scope| {
        scope.spawn(|| {
            thread::sleep(SHORT);
            TcpStream::connect(address).expect("a late dial connects")
        });
        let started = Instant::now();
        listener
            .accept(GENEROUS)
            .expect("the late connection is accepted");
        assert!(started.elapsed() < GENEROUS / 2);
    });
}

/// One connection to a listener several callers wait on wakes them all, but
/// only one can take it. Every other caller then finds nothing to accept,
/// as a caller does whose connection was reset between becoming ready and
/// being accepted. It must give up by its deadline, not wait for a later
/// connection. Whether a woken caller loses that race is up to the
/// scheduler, so the test runs several rounds.
#[test]
fn an_accept_that_finds_its_connection_gone_still_gives_up_by_its_deadline() {
    const CALLERS: usize = 4;
    let (listener, address) = listener();
    let listener = Arc::new(listener);
    for round in 0..4 {
        let (report, reports) = mpsc::channel();
        for _ in 0..CALLERS {
            let (listener, report) = (Arc::clone(&listener), report.clone());
            thread::spawn(move || {
                let accepted = listener.accept(SHORT * 5).map(drop);
                let _ = report.send(accepted.map_err(|error| error.kind()));
            });
        }
        // Every caller is waiting before the connection arrives.
        thread::sleep(SHORT / 2);
        let _dialled = TcpStream::connect(address).expect("the dial connects");

        let outcomes: Vec<_> = (0..CALLERS)
            .map(|_| {
                reports.recv_timeout(GENEROUS).unwrap_or_else(|_| {
                    panic!("round {round}: a caller waited long past its deadline")
                })
            })
            .collect();
        let accepted = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
        let expired = outcomes
            .iter()
            .filter(|outcome| **outcome == Err(io::ErrorKind::TimedOut));
        assert_eq!(
            (accepted, expired.count()),
            (1, CALLERS - 1),
            "round {round}: {outcomes:?}"
        );
    }
}

#[test]
fn a_dial_to_a_port_nobody_listens_on_is_refused() {
    let (listener, address) = listener();
    drop(listener);

    let error = link_socket::dial(address, GENEROUS).expect_err("nobody listens");

    assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
}

/// A listener whose queue of unaccepted connections is full answers no
/// further dial, on Linux and macOS alike, so a dial to it can only expire.
#[test]
fn a_dial_honours_a_connect_deadline() {
    let silent =
        socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).expect("a socket");
    silent.bind(&loopback().into()).expect("loopback binds");
    silent
        .listen(0)
        .expect("it listens with the shortest queue");
    let address = silent
        .local_addr()
        .expect("a bound socket has an address")
        .as_socket()
        .expect("an internet address");
    let fillers: Vec<_> = (0..8)
        .map(|_| {
            let filler = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None)
                .expect("a socket");
            filler
                .set_nonblocking(true)
                .expect("a dial that does not wait");
            let _ = filler.connect(&address.into());
            filler
        })
        .collect();
    thread::sleep(Duration::from_millis(100));

    expires(|| link_socket::dial(address, SHORT));
    drop(fillers);
}

#[test]
fn the_seam_carries_ipv6_when_the_host_has_it() {
    let Ok(listener) = link_socket::listen(SocketAddr::from((Ipv6Addr::LOCALHOST, 0))) else {
        // No IPv6 loopback on this host: nothing to carry.
        return;
    };
    let address = listener
        .local_address()
        .expect("a bound listener has an address");
    let mut dialled = link_socket::dial(address, GENEROUS).expect("the dial connects");
    let mut accepted = listener
        .accept(GENEROUS)
        .expect("the connection is accepted");

    dialled.write_all(b"six").expect("the dialler sends");

    assert_eq!(read_exactly(&mut accepted, 3), b"six");
}

#[test]
fn two_listeners_cannot_share_an_address() {
    let (_listener, address) = listener();

    let error = link_socket::listen(address).expect_err("the address is taken");

    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
}
