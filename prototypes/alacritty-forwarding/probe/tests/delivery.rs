//! THROWAWAY #48: bounded per-attachment delivery with explicit clock.

use std::time::{Duration, Instant};

use alacritty_embedding_probe::delivery::{
    Attachments, DeliveryBudget, Disconnect, Outbound, SessionEvent,
};

const BUDGET: DeliveryBudget = DeliveryBudget {
    chunk_bytes: 4,
    queue_bytes: 10,
    no_progress: Duration::from_millis(100),
    restoration_deadline: Duration::from_millis(1_000),
};

fn drain(outbound: &mut Outbound, now: Instant) -> Vec<u8> {
    let mut received = Vec::new();
    while let Some(chunk) = outbound.next_chunk() {
        assert!(chunk.len() <= BUDGET.chunk_bytes);
        let chunk = chunk.to_vec();
        outbound.wrote(chunk.len(), now);
        received.extend(chunk);
    }
    received
}

#[test]
fn restoration_precedes_live_output_in_order_and_in_bounded_chunks() {
    let start = Instant::now();
    let mut outbound = Outbound::restoring(BUDGET, b"RESTORED-SCREEN".to_vec(), start);
    outbound.publish(b"live1", start).unwrap();
    outbound.publish(b"live2", start).unwrap();
    assert_eq!(drain(&mut outbound, start), b"RESTORED-SCREENlive1live2");
    assert!(!outbound.restoring_now());
    assert_eq!(outbound.poll(start + Duration::from_secs(60)), None);
}

#[test]
fn live_output_beyond_the_queue_bound_disconnects_instead_of_dropping() {
    let start = Instant::now();
    let mut outbound = Outbound::live(BUDGET, start);
    outbound.publish(b"0123456789", start).unwrap();
    assert_eq!(
        outbound.publish(b"X", start),
        Err(Disconnect::QueueOverflow)
    );
}

#[test]
fn restoration_bytes_do_not_count_against_the_live_queue_bound() {
    let start = Instant::now();
    let mut outbound = Outbound::restoring(BUDGET, vec![b'r'; 1_000], start);
    outbound.publish(b"0123456789", start).unwrap();
    let received = drain(&mut outbound, start);
    assert_eq!(received.len(), 1_010);
    assert_eq!(&received[1_000..], b"0123456789");
}

#[test]
fn stalled_delivery_disconnects_after_the_no_progress_timeout() {
    let start = Instant::now();
    let mut outbound = Outbound::live(BUDGET, start);
    assert_eq!(
        outbound.poll(start + Duration::from_secs(5)),
        None,
        "idle is not stalled"
    );
    let queued_at = start + Duration::from_secs(5);
    outbound.publish(b"abc", queued_at).unwrap();
    assert_eq!(
        outbound.poll(queued_at + BUDGET.no_progress - Duration::from_millis(1)),
        None
    );
    assert_eq!(
        outbound.poll(queued_at + BUDGET.no_progress),
        Some(Disconnect::NoProgress)
    );
}

#[test]
fn slow_but_progressing_restoration_is_bounded_by_the_overall_deadline() {
    let start = Instant::now();
    let mut outbound = Outbound::restoring(BUDGET, vec![b'r'; 1_000], start);
    let mut now = start;
    let mut outcome = None;
    while outcome.is_none() {
        now += Duration::from_millis(50);
        if let Some(chunk) = outbound.next_chunk() {
            let len = chunk.len();
            outbound.wrote(len, now);
        }
        outcome = outbound.poll(now);
    }
    assert_eq!(outcome, Some(Disconnect::RestorationDeadline));
    assert_eq!(now, start + BUDGET.restoration_deadline);
}

#[test]
fn losing_one_attachment_isolates_it_and_losing_the_final_one_terminates() {
    let start = Instant::now();
    let mut attachments = Attachments::default();
    let slow = attachments.attach(Outbound::live(BUDGET, start));
    let fast = attachments.attach(Outbound::live(BUDGET, start));
    assert_eq!(attachments.publish(b"012345", start), vec![]);
    attachments
        .get_mut(fast)
        .map(|outbound| drain(outbound, start));
    assert_eq!(
        attachments.publish(b"6789X", start),
        vec![SessionEvent::Disconnected(slow, Disconnect::QueueOverflow)]
    );
    assert_eq!(drain(attachments.get_mut(fast).unwrap(), start), b"6789X");
    let later = start + BUDGET.no_progress;
    assert_eq!(attachments.publish(b"Y", start), vec![]);
    assert_eq!(
        attachments.poll(later),
        vec![
            SessionEvent::Disconnected(fast, Disconnect::NoProgress),
            SessionEvent::FinalAttachmentLost,
        ]
    );
}
