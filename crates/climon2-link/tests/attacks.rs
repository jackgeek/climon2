//! The on-path proxy and the link peer stand-in against a link connection
//! at the Link layer (#494): SEC-02, SEC-03 and the second half of SEC-04.

mod support;
mod wire;

use climon2_link::{End, Ended, HubEnd, Input, Link, Record, RelayClose, RelayEnd, Time};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use support::{Harness, Seeded};
use wire::{
    HANDSHAKE_REPLY, INITIATION, NANOSECOND, OnPath, REPLY, REQUEST, SECOND, StandIn, Way, hub_end,
    hub_key, key, paired, relay_end, relay_key, step, units,
};

/// The two ends of one remembered pairing, joined through an on-path proxy
/// that changes nothing until a test gives it an edit, and linked.
fn linked() -> Harness<HubEnd, OnPath<RelayEnd>> {
    let (hub, relay) = paired();
    let mut harness = Harness::new(hub, OnPath::new(relay));
    harness.run();
    assert!(matches!(harness.hub.link(), Link::Established(_)));
    harness
}

fn record(stream: u32, tag: u8, body: &[u8]) -> Record {
    Record {
        stream,
        tag,
        body: body.to_vec(),
    }
}

/// What the proxy does to the units of one batch.
type UnitEdit = fn(Vec<Vec<u8>>) -> Vec<Vec<u8>>;

/// The edits of SEC-02, each with the records that may still arrive: only
/// the first of two units, when the proxy repeats it.
fn sec_02_edits() -> Vec<(&'static str, UnitEdit, usize)> {
    vec![
        (
            "flips a bit",
            |mut units| {
                units[0][9] ^= 0x10;
                units
            },
            0,
        ),
        (
            "truncates",
            |mut units| {
                let unit = &mut units[0];
                unit.pop();
                let length = (unit.len() - 2) as u16;
                unit[..2].copy_from_slice(&length.to_be_bytes());
                units
            },
            0,
        ),
        ("drops", |units| units[1..].to_vec(), 0),
        (
            "reorders",
            |units| vec![units[1].clone(), units[0].clone()],
            0,
        ),
        (
            "duplicates",
            |units| vec![units[0].clone(), units[0].clone(), units[1].clone()],
            1,
        ),
        (
            "injects",
            |units| {
                let mut forged = vec![0x00, 0x15];
                forged.extend([0x5a; 21]);
                [vec![forged], units].concat()
            },
            0,
        ),
    ]
}

/// SEC-02: the proxy alters the units of an established link, each way. No
/// altered, repeated or out-of-order unit is delivered as a record: the
/// receiving end ends the link as lost and closes, and so does the other
/// end when the byte stream closes.
#[test]
fn sec_02_an_altered_unit_ends_the_link_and_delivers_nothing() {
    for way in [Way::ToRelay, Way::ToHub] {
        for (name, edit, delivered) in sec_02_edits() {
            let mut harness = linked();
            harness.relay.edit = Box::new(move |seen, bytes| match seen == way {
                true => edit(units(&bytes)).concat(),
                false => bytes,
            });
            let expected = match way {
                Way::ToRelay => {
                    harness.hub.send(1, 0x41, b"first").expect("sent");
                    harness.hub.send(1, 0x42, b"second").expect("sent");
                    [record(1, 0x41, b"first")]
                }
                Way::ToHub => {
                    harness.relay.end.send(1, 0x81, b"first").expect("sent");
                    harness.relay.end.send(1, 0x82, b"second").expect("sent");
                    [record(1, 0x81, b"first")]
                }
            };
            harness.run();

            let received = match way {
                Way::ToRelay => harness.relay.end.records(),
                Way::ToHub => harness.hub.records(),
            };
            assert_eq!(received, expected[..delivered], "{way:?} {name}");
            assert_eq!(
                harness.hub.link(),
                &Link::Ended(Ended::Lost),
                "{way:?} {name}"
            );
            let relay = harness.relay.end.link();
            assert_eq!(relay, &Link::Ended(Ended::Lost), "{way:?} {name}");
            assert!(
                harness.hub_closed() && harness.relay_closed(),
                "{way:?} {name}"
            );
        }
    }
}

/// SEC-02: a byte stream that closes part-way through a unit delivers none
/// of it and ends the link as lost.
#[test]
fn sec_02_a_unit_cut_off_by_a_close_delivers_nothing() {
    let mut harness = linked();
    harness.relay.edit = Box::new(|way, mut bytes| {
        if way == Way::ToRelay {
            bytes.truncate(bytes.len() - 1);
        }
        bytes
    });
    harness.hub.send(1, 0x41, b"cut").expect("sent");
    harness.run();
    let now = harness.now();
    let output = harness.relay.step(now, Input::Closed, &mut Seeded::new(0));

    assert!(output.close && output.bytes.is_empty());
    assert!(harness.relay.end.records().is_empty());
    assert_eq!(harness.relay.end.link(), &Link::Ended(Ended::Lost));
}

/// Every batch of bytes the hub monitor sent, in order, through a proxy
/// that recorded them.
type Recorded = Rc<RefCell<Vec<Vec<u8>>>>;

/// A whole connection of one remembered pairing, with every batch the hub
/// monitor sent recorded, up to and including one record after the link.
fn recorded_connection() -> Recorded {
    let recorded = Recorded::default();
    let (hub, relay) = paired();
    let mut on_path = OnPath::new(relay);
    let into = Rc::clone(&recorded);
    on_path.edit = Box::new(move |way, bytes| {
        if way == Way::ToRelay {
            into.borrow_mut().push(bytes.clone());
        }
        bytes
    });
    let mut harness = Harness::new(hub, on_path);
    harness.run();
    harness.hub.send(1, 0x41, b"recorded").expect("sent");
    harness.run();
    assert_eq!(harness.relay.end.records(), [record(1, 0x41, b"recorded")]);
    recorded
}

/// SEC-03: a recorded handshake initiation, replayed alone to the relay,
/// decrypts and gets a handshake reply, because the opening exchange is
/// the same. The relay treats nothing as linked, and when no Hello comes it
/// closes at 30 seconds from accepting, without a byte more.
#[test]
fn sec_03_a_replayed_handshake_initiation_links_nothing() {
    let recorded = recorded_connection();
    let recorded = recorded.borrow();
    let mut relay = relay_end(relay_key(), &[hub_key().public()]);

    step(&mut relay, Duration::ZERO, Input::Tick);
    assert_eq!(recorded[0], REQUEST);
    assert_eq!(
        step(&mut relay, Duration::ZERO, Input::Bytes(&recorded[0])).bytes,
        REPLY
    );
    assert_eq!(recorded[1].len(), INITIATION);
    let output = step(&mut relay, SECOND, Input::Bytes(&recorded[1]));
    assert_eq!(output.bytes.len(), HANDSHAKE_REPLY);
    assert_eq!(output.deadline, Some(Time::since_start(30 * SECOND)));
    assert_eq!(relay.link(), &Link::Pending);

    let output = step(&mut relay, 30 * SECOND - NANOSECOND, Input::Tick);
    assert!(!output.close && output.bytes.is_empty());
    let output = step(&mut relay, 30 * SECOND, Input::Tick);
    assert!(output.close && output.bytes.is_empty());
    assert_eq!(relay.link(), &Link::Ended(Ended::Failed));
    assert!(relay.records().is_empty());
}

/// SEC-03: a whole recorded connection, replayed batch by batch, delivers
/// nothing: the recorded Hello was sealed for the recorded relay's
/// ephemeral key, not this one's. The relay closes without a byte in
/// answer to it, and a link it already holds carries on untouched.
#[test]
fn sec_03_a_replayed_connection_delivers_nothing() {
    let recorded = recorded_connection();
    let mut existing = linked();
    let mut relay = relay_end(relay_key(), &[hub_key().public()]);

    let mut sent = step(&mut relay, Duration::ZERO, Input::Tick).bytes;
    let mut closed = false;
    for batch in recorded.borrow().iter() {
        let output = step(&mut relay, Duration::ZERO, Input::Bytes(batch));
        sent.extend(output.bytes);
        closed = output.close;
        if closed {
            break;
        }
    }

    assert!(closed);
    assert_eq!(
        sent.len(),
        REPLY.len() + HANDSHAKE_REPLY,
        "nothing after the reply"
    );
    assert!(relay.records().is_empty());
    assert_eq!(relay.link(), &Link::Ended(Ended::Failed));
    existing.hub.send(1, 0x41, b"still here").expect("sent");
    existing.run();
    assert_eq!(
        existing.relay.end.records(),
        [record(1, 0x41, b"still here")]
    );
}

/// The plaintext of a Close record that gives `code`, as one unit on
/// stream 0.
fn close_unit(code: u16) -> Vec<u8> {
    let [high, low] = code.to_be_bytes();
    vec![0, 0, 0, 0, 0, 0x02, high, low]
}

/// SEC-04: a stand-in that completes the handshake with a key the relay
/// does not remember gets the handshake reply, then, in answer to its
/// Hello, one Close with "not paired" and nothing else: no Hello, so no
/// name and no release.
#[test]
fn sec_04_an_unknown_key_gets_only_close_not_paired() {
    let mut relay = relay_end(relay_key(), &[key(0x77).public()]);
    let mut stand_in = StandIn::dial(&mut relay, [0x55; 32], relay_key().public());
    assert_eq!(relay.link(), &Link::Pending);

    let hello = stand_in.record(&wire::HELLO);
    let output = step(&mut relay, SECOND, Input::Bytes(&hello));

    assert!(output.close);
    assert_eq!(stand_in.open(&output.bytes), [close_unit(4)]);
    assert_eq!(
        relay.link(),
        &Link::Ended(Ended::Left(RelayClose::NotPaired))
    );
}

/// The control: the same stand-in with a remembered key gets the relay's
/// Hello.
#[test]
fn a_remembered_key_gets_the_relays_hello() {
    let mut relay = relay_end(relay_key(), &[key(0x55).public()]);
    let mut stand_in = StandIn::dial(&mut relay, [0x55; 32], relay_key().public());

    let hello = stand_in.record(&wire::HELLO);
    let output = step(&mut relay, SECOND, Input::Bytes(&hello));

    assert!(!output.close);
    let relay_hello = [
        &[0, 0, 0, 0, 0][..],
        b"\x01\0\0\0\x052.5.1\0\0\0\x05forge\0\0\0\0\0\0\x10\0",
    ]
    .concat();
    assert_eq!(stand_in.open(&output.bytes), [relay_hello]);
    assert!(matches!(relay.link(), Link::Established(peer) if peer.key == key(0x55).public()));
}

/// SEC-04: a stand-in relay that holds the wrong identity key receives the
/// opening request and the handshake initiation, neither of which holds the
/// hub monitor's static key in the clear, and cannot decrypt the
/// initiation. Whatever it answers is a failed dial: the hub monitor closes
/// and sends no record.
#[test]
fn sec_04_a_wrong_relay_learns_nothing_that_identifies_the_hub() {
    let wrong = [0x66; 32];
    let answers: [(&str, Vec<u8>); 3] = [
        (
            "a reply of its own",
            [&[0x00, 0x30][..], &[0xa5; 48]].concat(),
        ),
        (
            "a reply of another length",
            [&[0x00, 0x31][..], &[0xa5; 49]].concat(),
        ),
        ("a close", Vec::new()),
    ];
    for (answer, bytes) in answers {
        let mut hub = hub_end(hub_key(), relay_key().public());
        let request = step(&mut hub, Duration::ZERO, Input::Tick).bytes;
        let initiation = step(&mut hub, Duration::ZERO, Input::Bytes(&REPLY)).bytes;
        let sent = [request.clone(), initiation.clone()].concat();
        assert_eq!(request, REQUEST, "{answer}");
        assert_eq!(initiation.len(), INITIATION, "{answer}");
        let hub_public = hub_key().public();
        assert!(
            !sent.windows(32).any(|w| w == hub_public.as_bytes()),
            "{answer}"
        );
        assert!(!StandIn::can_read(
            &initiation,
            wrong,
            &[&REQUEST[..], &REPLY].concat()
        ));

        let input = match bytes.is_empty() {
            true => Input::Closed,
            false => Input::Bytes(&bytes),
        };
        let output = step(&mut hub, SECOND, input);

        assert!(output.close && output.bytes.is_empty(), "{answer}");
        assert_eq!(hub.link(), &Link::Ended(Ended::Failed), "{answer}");
    }
}

/// The control: the right relay can read the same initiation.
#[test]
fn the_pinned_relay_can_read_the_initiation() {
    let mut hub = hub_end(hub_key(), relay_key().public());
    step(&mut hub, Duration::ZERO, Input::Tick);
    let initiation = step(&mut hub, Duration::ZERO, Input::Bytes(&REPLY)).bytes;

    assert!(StandIn::can_read(
        &initiation,
        [0x22; 32],
        &[&REQUEST[..], &REPLY].concat()
    ));
}

/// A hub monitor that hears nothing after its handshake initiation fails
/// the dial 30 seconds after dialling, naming that deadline until then.
#[test]
fn a_silent_relay_fails_the_dial_at_30_seconds() {
    let mut hub = hub_end(hub_key(), relay_key().public());
    step(&mut hub, 2 * SECOND, Input::Tick);
    let output = step(&mut hub, 3 * SECOND, Input::Bytes(&REPLY));
    assert_eq!(output.deadline, Some(Time::since_start(32 * SECOND)));

    let output = step(&mut hub, 32 * SECOND - NANOSECOND, Input::Tick);
    assert_eq!(output.deadline, Some(Time::since_start(32 * SECOND)));
    assert!(!output.close);
    let output = step(&mut hub, 32 * SECOND, Input::Tick);
    assert!(output.close && output.bytes.is_empty());
    assert_eq!(hub.link(), &Link::Ended(Ended::Failed));
}

/// A hub monitor whose handshake is done still needs the relay's Hello or
/// Close by the same deadline, and a relay the hub monitor's Hello.
#[test]
fn the_hellos_are_due_30_seconds_after_the_connection_began() {
    let mut relay = relay_end(relay_key(), &[hub_key().public()]);
    let stand_in = StandIn::dial(&mut relay, [0x11; 32], relay_key().public());
    assert_eq!(
        stand_in.first.deadline,
        Some(Time::since_start(30 * SECOND))
    );
    let late = step(&mut relay, 30 * SECOND + NANOSECOND, Input::Tick);
    assert!(late.close && late.bytes.is_empty());

    let mut hub = hub_end(hub_key(), relay_key().public());
    let stand_in = StandIn::answer(&mut hub, [0x22; 32]);
    assert_eq!(
        stand_in.first.deadline,
        Some(Time::since_start(30 * SECOND))
    );
    let output = step(&mut hub, 30 * SECOND, Input::Tick);
    assert!(output.close && output.bytes.is_empty());
    assert_eq!(hub.link(), &Link::Ended(Ended::Failed));
}
