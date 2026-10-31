//! THROWAWAY #48: the pump keeps running while a real Unix-socket peer stalls.

use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::{Duration, Instant};

use alacritty_embedding_probe::delivery::{
    Attachments, DeliveryBudget, Disconnect, Outbound, SessionEvent,
};

const BUDGET: DeliveryBudget = DeliveryBudget {
    chunk_bytes: 16 * 1024,
    queue_bytes: 256 * 1024,
    no_progress: Duration::from_millis(300),
    restoration_deadline: Duration::from_secs(5),
};

/// One pump iteration: publish PTY output, then write without blocking.
fn pump(
    attachments: &mut Attachments,
    sockets: &mut [(usize, UnixStream)],
    output: &[u8],
) -> Vec<SessionEvent> {
    let now = Instant::now();
    let mut events = attachments.publish(output, now);
    for (id, socket) in sockets.iter_mut() {
        while let Some(outbound) = attachments.get_mut(*id) {
            let Some(chunk) = outbound.next_chunk() else {
                break;
            };
            match socket.write(chunk) {
                Ok(written) => outbound.wrote(written, Instant::now()),
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => panic!("{error}"),
            }
        }
    }
    events.extend(attachments.poll(Instant::now()));
    events
}

#[test]
fn stalled_peer_is_disconnected_while_the_pump_never_blocks() {
    let (daemon, _peer_never_reads) = UnixStream::pair().unwrap();
    daemon.set_nonblocking(true).unwrap();
    let mut attachments = Attachments::default();
    let id = attachments.attach(Outbound::live(BUDGET, Instant::now()));
    let mut sockets = vec![(id, daemon)];
    let started = Instant::now();
    let mut slowest = Duration::ZERO;
    let events = loop {
        let iteration = Instant::now();
        let events = pump(&mut attachments, &mut sockets, &[b'x'; 512]);
        slowest = slowest.max(iteration.elapsed());
        if !events.is_empty() {
            break events;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "never disconnected"
        );
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        events,
        vec![
            SessionEvent::Disconnected(id, Disconnect::NoProgress),
            SessionEvent::FinalAttachmentLost,
        ]
    );
    assert!(started.elapsed() >= BUDGET.no_progress);
    assert!(
        slowest < Duration::from_millis(50),
        "pump blocked for {slowest:?}"
    );
}

#[test]
fn slow_progressing_reader_receives_restoration_then_live_output_intact() {
    let (daemon, mut peer) = UnixStream::pair().unwrap();
    daemon.set_nonblocking(true).unwrap();
    let restoration: Vec<u8> = (0..512 * 1024).map(|i| (i % 251) as u8).collect();
    let reader = thread::spawn(move || {
        let mut received = Vec::new();
        let mut buffer = [0; 8 * 1024];
        loop {
            match peer.read(&mut buffer) {
                Ok(0) => break received,
                Ok(read) => received.extend_from_slice(&buffer[..read]),
                Err(error) => panic!("{error}"),
            }
            thread::sleep(Duration::from_millis(1));
        }
    });
    let mut attachments = Attachments::default();
    let id = attachments.attach(Outbound::restoring(
        BUDGET,
        restoration.clone(),
        Instant::now(),
    ));
    let mut sockets = vec![(id, daemon)];
    let mut live = Vec::new();
    for round in 0..2_000u32 {
        let output = format!("<{round}>");
        live.extend_from_slice(output.as_bytes());
        assert_eq!(
            pump(&mut attachments, &mut sockets, output.as_bytes()),
            vec![]
        );
        thread::sleep(Duration::from_micros(200));
    }
    while attachments.get_mut(id).unwrap().next_chunk().is_some() {
        assert_eq!(pump(&mut attachments, &mut sockets, b""), vec![]);
        thread::sleep(Duration::from_millis(1));
    }
    drop(sockets);
    let received = reader.join().unwrap();
    assert_eq!(received.len(), restoration.len() + live.len());
    assert_eq!(&received[..restoration.len()], restoration);
    assert_eq!(&received[restoration.len()..], live);
}
