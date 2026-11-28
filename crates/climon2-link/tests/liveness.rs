//! Keepalive and dead-link detection at the Link layer (#497): LOSS-01 of
//! the monitor-link acceptance criteria, with the time supplied. A link
//! peer stand-in or the on-path proxy makes one end fall silent.

mod samples;
mod support;
mod wire;

use climon2_link::{Ended, HubStream, Input, Link, Time};
use samples::{LATER, from_hub, linked_hub, linked_relay, relay_streaming};
use std::time::Duration;
use support::Harness;
use wire::{NANOSECOND, OnPath, SECOND, Way, paired, step};

/// A keepalive's plaintext: stream 0, no flags and an empty fragment.
const KEEPALIVE: [u8; 5] = [0; 5];

fn at(elapsed: Duration) -> Option<Time> {
    Some(Time::since_start(elapsed))
}

/// The relay sent its Hello at 1 second. It sends nothing more until it
/// has sent nothing for 5 seconds, then one keepalive of 23 bytes, and
/// names that moment as its deadline.
#[test]
fn loss_01_an_idle_relay_sends_a_keepalive_after_5_seconds() {
    let (mut relay, mut stand_in) = linked_relay();
    let early = step(&mut relay, 6 * SECOND - NANOSECOND, Input::Tick);
    assert!(early.bytes.is_empty(), "nothing before 5 seconds");
    assert_eq!(early.deadline, at(6 * SECOND));
    let due = step(&mut relay, 6 * SECOND, Input::Tick);
    assert_eq!(due.bytes.len(), 23, "the length 00 15 and 21 bytes");
    assert_eq!(&due.bytes[..2], [0x00, 0x15]);
    assert_eq!(stand_in.open(&due.bytes), [KEEPALIVE]);
    assert!(!due.close);
}

/// The hub monitor sent its Hello at the handshake, at time zero, so its
/// first keepalive is due at 5 seconds, and the next 5 seconds after it.
#[test]
fn loss_01_an_idle_hub_monitor_sends_a_keepalive_every_5_seconds() {
    let (mut hub, mut stand_in) = linked_hub();
    assert_eq!(step(&mut hub, SECOND, Input::Tick).deadline, at(5 * SECOND));
    let first = step(&mut hub, 5 * SECOND, Input::Tick);
    assert_eq!(stand_in.open(&first.bytes), [KEEPALIVE]);
    assert_eq!(first.deadline, at(10 * SECOND));
    let again = step(&mut hub, 10 * SECOND - NANOSECOND, Input::Tick);
    assert!(again.bytes.is_empty(), "no keepalive before 5 more seconds");
    let second = step(&mut hub, 10 * SECOND, Input::Tick);
    assert_eq!(stand_in.open(&second.bytes), [KEEPALIVE]);
    assert!(matches!(hub.link(), Link::Established(_)));
}

/// An end that has received no unit for 15 seconds declares the link
/// lost: it closes the byte stream and sends nothing, not even a Close.
/// The relay heard the stand-in's Hello at 1 second.
#[test]
fn loss_01_15_seconds_without_a_unit_loses_the_link() {
    let (mut relay, mut stand_in) = linked_relay();
    let before = step(&mut relay, 16 * SECOND - NANOSECOND, Input::Tick);
    assert!(!before.close, "still linked just before 15 seconds");
    stand_in.open(&before.bytes);
    assert!(matches!(relay.link(), Link::Established(_)));
    let lost = step(&mut relay, 16 * SECOND, Input::Tick);
    assert!(
        lost.close && lost.bytes.is_empty(),
        "closed with nothing sent"
    );
    assert_eq!(*relay.link(), Link::Ended(Ended::Lost));
}

/// Each unit that decrypts resets the silence timer, not only a whole
/// record: a record whose three units arrive 10 seconds apart keeps the
/// link for 30 seconds and arrives whole.
#[test]
fn loss_01_each_unit_of_a_slow_record_keeps_the_link() {
    let (mut relay, mut stand_in) = relay_streaming();
    let (request, record) = samples::requests().remove(1);
    let thirds: Vec<_> = record.chunks(record.len().div_ceil(3)).collect();
    assert_eq!(thirds.len(), 3);
    for (at, third) in (1..).zip(&thirds) {
        let unit = stand_in.unit(1, u8::from(at < 3), third);
        let output = step(&mut relay, LATER + at * 10 * SECOND, Input::Bytes(&unit));
        assert!(!output.close, "still linked at unit {at}");
    }
    assert!(matches!(relay.link(), Link::Established(_)));
    assert_eq!(relay.records(), [from_hub(1, HubStream::Request(request))]);
}

/// Bytes that have not formed a whole unit prove nothing: a silent link
/// fed unauthenticated bytes, one a second, is lost 15 seconds after the
/// last unit that decrypted, the stand-in's Hello at 1 second.
#[test]
fn loss_01_unauthenticated_bytes_do_not_keep_the_link() {
    let (mut relay, _) = linked_relay();
    let unauthenticated = [&[0x00, 0x15][..], &[0x5a; 20]].concat();
    for (at, byte) in (2..16).zip(&unauthenticated) {
        let output = step(&mut relay, at * SECOND, Input::Bytes(&[*byte]));
        assert!(!output.close, "still linked at {at} seconds");
    }
    let lost = step(&mut relay, 16 * SECOND, Input::Bytes(&[0x5a]));
    assert!(lost.close && lost.bytes.is_empty());
    assert_eq!(*relay.link(), Link::Ended(Ended::Lost));
}

/// The on-path proxy stops carrying the relay's bytes once the link is
/// established at time zero. The hub monitor hears nothing more and
/// declares the link lost at 15 seconds; the relay, which went on hearing
/// the hub monitor's keepalives, learns of it when the byte stream closes.
#[test]
fn loss_01_an_end_that_hears_nothing_for_15_seconds_loses_the_link() {
    let (hub, relay) = paired();
    let mut harness = Harness::new(hub, OnPath::new(relay));
    harness.run();
    assert!(matches!(harness.hub.link(), Link::Established(_)));
    harness.relay.edit = Box::new(|way, bytes| match way {
        Way::ToHub => Vec::new(),
        Way::ToRelay => bytes,
    });

    harness.advance(15 * SECOND - NANOSECOND);
    assert!(matches!(harness.hub.link(), Link::Established(_)));
    assert!(matches!(harness.relay.end.link(), Link::Established(_)));
    harness.advance(NANOSECOND);
    assert_eq!(*harness.hub.link(), Link::Ended(Ended::Lost));
    assert!(harness.hub_closed());
    assert_eq!(*harness.relay.end.link(), Link::Ended(Ended::Lost));
}

/// Two ends that hear each other's keepalives stay linked however long
/// nothing else crosses.
#[test]
fn loss_01_keepalives_keep_an_idle_link() {
    let (hub, relay) = paired();
    let mut harness = Harness::new(hub, relay);
    harness.run();
    harness.advance(600 * SECOND);
    assert!(matches!(harness.hub.link(), Link::Established(_)));
    assert!(matches!(harness.relay.link(), Link::Established(_)));
    assert!(!harness.hub_closed() && !harness.relay_closed());
}
