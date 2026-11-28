//! Advertisement at the Link layer (#496): the Session, Session withdrawn
//! and Snapshot complete records on stream 0, the Link-layer parts of
//! ADV-02 and ADV-05, and the rules of what a relay may advertise.

mod samples;
mod support;
mod wire;

use climon2_link::{
    Advertisement, Ended, HubClose, HubEnd, Input, Link, Peer, Refused, RelayRecord, Session,
};
use samples::{LATER, advertise, linked_hub, linked_relay, on, session, session_record};
use support::Harness;
use wire::{hex, hub_end, hub_key, key, relay_end, relay_key, step};

/// The plaintext of a Close unit that gives "protocol violation".
const VIOLATION: [u8; 8] = [0, 0, 0, 0, 0, 0x02, 0x00, 0x08];

fn session_of(id: u8) -> RelayRecord {
    advertise(Advertisement::Session(samples::session_with([id; 16])))
}

fn withdrawn(id: u8) -> RelayRecord {
    advertise(Advertisement::Withdrawn([id; 16]))
}

fn snapshot() -> RelayRecord {
    advertise(Advertisement::SnapshotComplete)
}

/// The record of an advertisement, tag included, written by hand.
fn bytes_of(record: &RelayRecord) -> Vec<u8> {
    let session = |id: u8| {
        let fields = "00 00 00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00";
        let created = "00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00";
        hex(&format!(
            "10 {} {fields} {created} 00 00 00 01 32",
            format!("{id:02x} ").repeat(16)
        ))
    };
    match record {
        RelayRecord::Advertisement(Advertisement::Session(held)) => session(held.id[0]),
        RelayRecord::Advertisement(Advertisement::Withdrawn(id)) => [&[0x11][..], &id[..]].concat(),
        _ => vec![0x12],
    }
}

/// What a hub monitor does when the stand-in advertises `records` in
/// order: the hub monitor, what it sends back and whether it closes.
fn hub_hears(records: &[RelayRecord]) -> (HubEnd, Vec<Vec<u8>>, bool) {
    let (mut hub, mut stand_in) = linked_hub();
    let bytes: Vec<u8> = records
        .iter()
        .flat_map(|record| stand_in.unit(0, 0, &bytes_of(record)))
        .collect();
    let output = step(&mut hub, LATER, Input::Bytes(&bytes));
    (hub, stand_in.open(&output.bytes), output.close)
}

/// Whether a hub monitor that hears `records` ends the link with
/// "protocol violation", and takes nothing after the broken one.
fn hub_violated(records: &[RelayRecord]) -> bool {
    let (mut hub, sent, closed) = hub_hears(records);
    let left = Link::Ended(Ended::Left(HubClose::ProtocolViolation));
    let taken = hub.records().len();
    sent == [VIOLATION] && closed && hub.link() == &left && taken == records.len() - 1
}

/// ADV-02: a Session record carries the session's ID, command, arguments,
/// working directory, depth, creation time and release, in the Session
/// page's layout, and nothing more: the page's example crosses byte for
/// byte, both ways. Session withdrawn and Snapshot complete have their
/// pages' layouts too.
#[test]
fn adv_02_an_advertisement_has_the_published_layout() {
    let withdrawn_record = hex("11 01 23 45 67 89 ab cd ef 01 23 45 67 89 ab cd ef");
    let examples = [
        (Advertisement::Session(session()), session_record()),
        (
            Advertisement::Withdrawn(samples::SESSION_ID),
            withdrawn_record,
        ),
        (Advertisement::SnapshotComplete, vec![0x12]),
    ];
    let (mut relay, mut stand_in) = linked_relay();
    let (mut hub, mut relay_stand_in) = linked_hub();
    for (advertisement, bytes) in examples {
        let record = advertise(advertisement);
        relay.send(&record).expect("sent");
        let output = step(&mut relay, LATER, Input::Tick);
        assert_eq!(stand_in.open(&output.bytes), [on(0, &bytes)], "{record:?}");

        let unit = relay_stand_in.unit(0, 0, &bytes);
        let output = step(&mut hub, LATER, Input::Bytes(&unit));
        assert!(output.bytes.is_empty() && !output.close, "{record:?}");
        assert_eq!(hub.records(), [record]);
    }
}

/// ADV-02: the working directory, command and arguments cross as the bytes
/// the relay's discovery read, whatever they hold: they are display text
/// for the hub monitor, never parsed by the link code.
#[test]
fn adv_02_session_fields_cross_as_bytes() {
    let held = Session {
        command: b"\x1b]0;title\x07".to_vec(),
        arguments: vec![vec![0xff, 0xfe], Vec::new()],
        working_directory: b"/tmp/\xc3(".to_vec(),
        ..session()
    };
    let record = advertise(Advertisement::Session(held));
    let (hub, relay) = wire::paired();
    let mut harness = Harness::new(hub, relay);
    harness.run();
    harness.relay.send(&record).expect("sent");
    harness.run();
    assert_eq!(harness.hub.records(), [record]);
}

/// The key a hub monitor's link proved for its relay.
fn proven_key(hub: &HubEnd) -> climon2_link::PublicKey {
    let Link::Established(Peer { key, .. }) = hub.link() else {
        panic!("linked");
    };
    *key
}

/// ADV-05: the link names a remote session by the relay's pinned key and
/// the session ID. Two relays that advertise the same ID are told apart by
/// their keys, and one relay's session keeps its key and ID across a
/// reconnect.
#[test]
fn adv_05_a_remote_session_is_the_relays_key_and_the_session_id() {
    let named = |relay_private: u8| {
        let relay_key = key(relay_private);
        let hub = hub_end(hub_key(), relay_key.public());
        let relay = relay_end(relay_key, &[hub_key().public()]);
        let mut harness = Harness::new(hub, relay);
        harness.run();
        harness.relay.send(&session_of(7)).expect("sent");
        harness.run();
        let [RelayRecord::Advertisement(Advertisement::Session(held))] = &harness.hub.records()[..]
        else {
            panic!("one Session");
        };
        (proven_key(&harness.hub), held.id)
    };
    let first = named(0x22);
    assert_eq!(first, (relay_key().public(), [7; 16]));
    assert_eq!(named(0x22), first, "the same after a reconnect");
    let other = named(0x23);
    assert_eq!(other.1, first.1, "the same session ID");
    assert_ne!(other.0, first.0, "a different relay");
}

/// A Session for an ID already held replaces it; a Session withdrawn takes
/// it away, once. Snapshot complete comes once, after any number of
/// Sessions or none.
#[test]
fn advertisements_in_turn_are_taken() {
    let in_turn = [
        session_of(1),
        session_of(2),
        session_of(1),
        snapshot(),
        withdrawn(1),
        session_of(1),
        withdrawn(2),
    ];
    let (mut hub, sent, closed) = hub_hears(&in_turn);
    assert!(sent.is_empty() && !closed);
    assert_eq!(hub.records(), in_turn);

    let (mut hub, sent, _) = hub_hears(&[snapshot()]);
    assert!(sent.is_empty());
    assert_eq!(hub.records(), [snapshot()], "a relay with no sessions");
}

/// A Session withdrawn for an ID the relay has not advertised on this
/// connection, and a second Snapshot complete, are protocol violations. A
/// relay asked to send one refuses.
#[test]
fn advertisements_out_of_turn_end_the_link() {
    let out_of_turn = [
        &[withdrawn(1)][..],
        &[session_of(1), withdrawn(2)],
        &[session_of(1), withdrawn(1), withdrawn(1)],
        &[snapshot(), snapshot()],
        &[session_of(1), snapshot(), session_of(2), snapshot()],
    ];
    for records in out_of_turn {
        assert!(hub_violated(records), "{records:?}");

        let (mut relay, _) = linked_relay();
        let (last, before) = records.split_last().expect("records");
        for record in before {
            relay.send(record).expect("in turn");
        }
        assert_eq!(relay.send(last), Err(Refused::OutOfTurn), "{records:?}");
    }
}

/// More than 128 sessions held at once is a protocol violation; a change
/// to one of the 128 held is not. A relay asked to advertise a 129th
/// refuses.
#[test]
fn a_129th_session_ends_the_link() {
    let held: Vec<_> = (0..128).map(session_of).collect();
    let (mut hub, sent, closed) = hub_hears(&[&held[..], &[session_of(5)]].concat());
    assert!(sent.is_empty() && !closed);
    assert_eq!(hub.records().len(), 129);
    assert!(hub_violated(&[&held[..], &[session_of(128)]].concat()));
    let refilled = [&held[..], &[withdrawn(0), session_of(128)]].concat();
    assert!(!hub_violated(&refilled), "room again after a withdrawal");

    let (mut relay, _) = linked_relay();
    for record in &held {
        relay.send(record).expect("in turn");
    }
    assert_eq!(relay.send(&session_of(5)), Ok(()), "a change");
    assert_eq!(relay.send(&session_of(128)), Err(Refused::OutOfTurn));
}

/// A session whose record would be longer than the hub monitor's ceiling
/// is not advertised: the relay refuses to send it.
#[test]
fn a_session_longer_than_the_hub_monitors_ceiling_is_refused() {
    let (mut relay, _) = linked_relay();
    let long = |size| {
        advertise(Advertisement::Session(Session {
            command: vec![b'x'; size],
            ..session()
        }))
    };
    let most = 1_048_576 - session_record().len() + 3;
    assert_eq!(relay.send(&long(most)), Ok(()));
    assert_eq!(relay.send(&long(most + 1)), Err(Refused::TooLong));
}
