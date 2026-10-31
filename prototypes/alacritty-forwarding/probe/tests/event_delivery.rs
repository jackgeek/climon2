use alacritty_embedding_probe::delivery::{DeliveryBudget, Disconnect, Outbound};
use alacritty_embedding_probe::event_delivery::deliver;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

fn budget() -> DeliveryBudget {
    DeliveryBudget {
        chunk_bytes: 4096,
        queue_bytes: 16 << 20,
        no_progress: Duration::from_millis(50),
        restoration_deadline: Duration::from_secs(5),
    }
}

#[test]
fn readiness_delivery_sends_the_exact_restore_prefix_then_live_suffix() {
    let (mut socket, mut peer) = UnixStream::pair().unwrap();
    socket.set_nonblocking(true).unwrap();
    let expected: Vec<_> = (0..4 << 20).map(|i| (i % 251) as u8).collect();
    let mut outbound = Outbound::restoring(budget(), expected.clone(), Instant::now());
    outbound.publish(b"AFTER", Instant::now()).unwrap();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let report = deliver(&mut socket, &mut outbound).unwrap();
    drop(socket);
    let received = reader.join().unwrap();
    assert_eq!(
        received.len(),
        expected.len() + 5,
        "restore was not delivered"
    );
    assert_eq!(received, [expected, b"AFTER".to_vec()].concat());
    assert_eq!(report.bytes, received.len());
    assert!(report.writes >= received.len().div_ceil(4096));
}

#[test]
fn readiness_wait_observes_no_progress_deadline_for_a_stalled_reader() {
    let (mut socket, _never_reads) = UnixStream::pair().unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut outbound = Outbound::restoring(budget(), vec![b'x'; 4 << 20], Instant::now());
    let started = Instant::now();
    assert_eq!(
        deliver(&mut socket, &mut outbound).unwrap_err(),
        Disconnect::NoProgress
    );
    assert!(started.elapsed() >= Duration::from_millis(50));
    assert!(started.elapsed() < Duration::from_secs(2));
}
