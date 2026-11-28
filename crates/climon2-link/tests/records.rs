//! Hello, Close and the framing of records into units at the Link layer
//! (#494), as the Hello and Close pages and the shared rules publish them.
//! A link peer stand-in sends the bytes, one deliberate defect at a time.

mod support;
mod wire;

use climon2_link::{
    Ended, Hello, HubClose, HubEnd, Input, Link, Record, Refused, RelayClose, RelayEnd,
};
use support::Harness;
use wire::{SECOND, StandIn, hub_end, hub_key, paired, relay_end, relay_key, step};

/// The Hello the relay stand-in of these tests sends: `2.5.1`, `forge`,
/// 4,096.
const RELAY_HELLO: &[u8] = b"\x01\0\0\0\x052.5.1\0\0\0\x05forge\0\0\0\0\0\0\x10\0";

/// A relay that remembers the stand-in, dialled by it and linked.
fn linked_relay() -> (RelayEnd, StandIn) {
    let mut relay = relay_end(relay_key(), &[hub_key().public()]);
    let mut stand_in = StandIn::dial(&mut relay, [0x11; 32], relay_key().public());
    let hello = stand_in.record(&wire::HELLO);
    let output = step(&mut relay, SECOND, Input::Bytes(&hello));
    assert_eq!(stand_in.open(&output.bytes), [on_control(RELAY_HELLO)]);
    assert_eq!(output.deadline, None, "no deadline once linked");
    assert!(matches!(relay.link(), Link::Established(_)));
    (relay, stand_in)
}

/// A hub monitor answered by the stand-in and linked.
fn linked_hub() -> (HubEnd, StandIn) {
    let mut hub = hub_end(hub_key(), relay_key().public());
    let mut stand_in = StandIn::answer(&mut hub, [0x22; 32]);
    let hello = stand_in.record(RELAY_HELLO);
    let output = step(&mut hub, SECOND, Input::Bytes(&hello));
    assert_eq!(
        output,
        climon2_link::Output::default(),
        "no deadline once linked"
    );
    assert!(matches!(hub.link(), Link::Established(_)));
    (hub, stand_in)
}

/// The plaintext of a one-unit record on stream 0.
fn on_control(record: &[u8]) -> Vec<u8> {
    [&[0, 0, 0, 0, 0][..], record].concat()
}

/// The plaintext of a Close unit with this code.
fn close_unit(code: u16) -> Vec<u8> {
    on_control(&[&[0x02][..], &code.to_be_bytes()].concat())
}

/// What a relay that the stand-in linked with does with `bytes`: the
/// plaintexts it sends back and whether it closes.
fn relay_given(bytes: impl FnOnce(&mut StandIn) -> Vec<u8>) -> (RelayEnd, Vec<Vec<u8>>, bool) {
    let (mut relay, mut stand_in) = linked_relay();
    let bytes = bytes(&mut stand_in);
    let output = step(&mut relay, 2 * SECOND, Input::Bytes(&bytes));
    let sent = stand_in.open(&output.bytes);
    (relay, sent, output.close)
}

/// The same for a hub monitor.
fn hub_given(bytes: impl FnOnce(&mut StandIn) -> Vec<u8>) -> (HubEnd, Vec<Vec<u8>>, bool) {
    let (mut hub, mut stand_in) = linked_hub();
    let bytes = bytes(&mut stand_in);
    let output = step(&mut hub, 2 * SECOND, Input::Bytes(&bytes));
    let sent = stand_in.open(&output.bytes);
    (hub, sent, output.close)
}

/// Asserts that a relay given `bytes` on an established link sends Close
/// with "protocol violation" and closes.
fn relay_refuses(what: &str, bytes: impl FnOnce(&mut StandIn) -> Vec<u8>) {
    let (mut relay, sent, closed) = relay_given(bytes);
    assert_eq!(sent, [close_unit(8)], "{what}");
    assert!(closed, "{what}");
    let left = Link::Ended(Ended::Left(RelayClose::ProtocolViolation));
    assert_eq!(relay.link(), &left, "{what}");
    assert!(relay.records().is_empty(), "{what}");
}

/// Each reason the Close page gives, sent by the end that may send it, has
/// its code on the wire, ends the link for both ends and closes the byte
/// stream once the record is written.
#[test]
fn close_ends_the_link_with_its_reason_code() {
    let relay_reasons = [
        (RelayClose::Cut, 1),
        (RelayClose::Stopping, 2),
        (RelayClose::Restarting, 3),
        (RelayClose::NotPaired, 4),
        (RelayClose::OnHold, 5),
        (RelayClose::Busy, 6),
        (RelayClose::Replaced, 7),
        (RelayClose::ProtocolViolation, 8),
        (RelayClose::Superseded, 9),
    ];
    for (reason, code) in relay_reasons {
        let (mut relay, mut stand_in) = linked_relay();
        relay.close(reason).expect("closed");
        let output = step(&mut relay, 2 * SECOND, Input::Tick);
        assert_eq!(
            stand_in.open(&output.bytes),
            [close_unit(code)],
            "{reason:?}"
        );
        assert!(output.close, "{reason:?}");
        assert_eq!(relay.link(), &Link::Ended(Ended::Left(reason)));

        let (hub, _, closed) = hub_given(|stand_in| stand_in.record(&[2, 0, code as u8]));
        assert_eq!(
            hub.link(),
            &Link::Ended(Ended::Closed(reason)),
            "{reason:?}"
        );
        assert!(closed, "{reason:?}");
    }
    let hub_reasons = [
        (HubClose::ProtocolViolation, 8),
        (HubClose::Disconnected, 32),
        (HubClose::Forgotten, 33),
        (HubClose::Stopped, 34),
    ];
    for (reason, code) in hub_reasons {
        let (mut hub, mut stand_in) = linked_hub();
        hub.close(reason).expect("closed");
        let output = step(&mut hub, 2 * SECOND, Input::Tick);
        assert_eq!(
            stand_in.open(&output.bytes),
            [close_unit(code)],
            "{reason:?}"
        );
        assert!(output.close, "{reason:?}");
        assert_eq!(hub.link(), &Link::Ended(Ended::Left(reason)));

        let (relay, _, closed) = relay_given(|stand_in| stand_in.record(&[2, 0, code as u8]));
        assert_eq!(
            relay.link(),
            &Link::Ended(Ended::Closed(reason)),
            "{reason:?}"
        );
        assert!(closed, "{reason:?}");
    }
}

/// The Close page's example: a relay on hold refuses a link with `02 00
/// 05`, the record whole.
#[test]
fn close_has_the_published_layout() {
    let (hub, _, _) = hub_given(|stand_in| stand_in.record(&[0x02, 0x00, 0x05]));
    assert_eq!(hub.link(), &Link::Ended(Ended::Closed(RelayClose::OnHold)));
}

/// A reason the receiver does not know, or one only the receiver's own
/// side may send, is handled as "protocol violation".
#[test]
fn an_unknown_or_misdirected_close_reason_is_a_protocol_violation() {
    for code in [0_u16, 10, 31, 32, 33, 34, 35, 0x0104, u16::MAX] {
        let (hub, _, closed) =
            hub_given(|stand_in| stand_in.record(&[&[2][..], &code.to_be_bytes()].concat()));
        let violation = Link::Ended(Ended::Closed(RelayClose::ProtocolViolation));
        assert_eq!(hub.link(), &violation, "code {code}");
        assert!(closed);
    }
    for code in [0_u16, 1, 4, 7, 9, 31, 35, 0x0108, u16::MAX] {
        let (relay, _, _) =
            relay_given(|stand_in| stand_in.record(&[&[2][..], &code.to_be_bytes()].concat()));
        let violation = Link::Ended(Ended::Closed(HubClose::ProtocolViolation));
        assert_eq!(relay.link(), &violation, "code {code}");
    }
}

/// A Close whose body is not one `u16` is malformed: a protocol violation.
#[test]
fn a_malformed_close_is_a_protocol_violation() {
    relay_refuses("no reason", |stand_in| stand_in.record(&[0x02]));
    relay_refuses("one byte", |stand_in| stand_in.record(&[0x02, 0x00]));
    relay_refuses("three bytes", |stand_in| {
        stand_in.record(&[0x02, 0x00, 0x20, 0x00])
    });
}

/// A Hello that breaks its layout or a limit, a second Hello, and any
/// record before Hello are protocol violations.
#[test]
fn a_bad_hello_or_a_record_out_of_turn_is_a_protocol_violation() {
    relay_refuses("a second Hello", |stand_in| stand_in.record(&wire::HELLO));

    let mut relay = relay_end(relay_key(), &[hub_key().public()]);
    let mut stand_in = StandIn::dial(&mut relay, [0x11; 32], relay_key().public());
    let early = stand_in.record(&[0x41, 0x00]);
    let output = step(&mut relay, SECOND, Input::Bytes(&early));
    assert_eq!(
        stand_in.open(&output.bytes),
        [close_unit(8)],
        "a record before Hello"
    );
    assert!(output.close);

    for (what, stream, record) in [
        ("a Close before Hello", 0, &[0x02, 0x00, 0x20][..]),
        ("a session stream before Hello", 1, &wire::HELLO[..]),
    ] {
        let mut relay = relay_end(relay_key(), &[hub_key().public()]);
        let mut stand_in = StandIn::dial(&mut relay, [0x11; 32], relay_key().public());
        let early = stand_in.unit(stream, 0, record);
        let output = step(&mut relay, SECOND, Input::Bytes(&early));
        assert_eq!(stand_in.open(&output.bytes), [close_unit(8)], "{what}");
    }

    let mut hub = hub_end(hub_key(), relay_key().public());
    let mut stand_in = StandIn::answer(&mut hub, [0x22; 32]);
    let early = stand_in.record(&[0x10, 0x00]);
    let output = step(&mut hub, SECOND, Input::Bytes(&early));
    assert_eq!(
        stand_in.open(&output.bytes),
        [close_unit(8)],
        "a record before the relay's Hello"
    );
    assert_eq!(
        hub.link(),
        &Link::Ended(Ended::Left(HubClose::ProtocolViolation))
    );
}

/// A relay that refuses sends Close in place of its Hello: the hub monitor
/// learns the reason and nothing else, and closes without a reply.
#[test]
fn a_close_in_place_of_the_relays_hello_ends_the_link_with_its_reason() {
    let mut hub = hub_end(hub_key(), relay_key().public());
    let mut stand_in = StandIn::answer(&mut hub, [0x22; 32]);
    let close = stand_in.record(&[0x02, 0x00, 0x04]);
    let output = step(&mut hub, SECOND, Input::Bytes(&close));

    assert!(output.close && output.bytes.is_empty());
    let not_paired = Link::Ended(Ended::Closed(RelayClose::NotPaired));
    assert_eq!(hub.link(), &not_paired);
}

/// A Hello body as the Hello page lays it out.
fn hello_body(release: &[u8], name: &[u8], tokens: &[&[u8]], largest: u32) -> Vec<u8> {
    let text = |bytes: &[u8]| [&(bytes.len() as u32).to_be_bytes()[..], bytes].concat();
    let mut body = vec![0x01];
    body.extend(text(release));
    body.extend(text(name));
    body.extend((tokens.len() as u32).to_be_bytes());
    for token in tokens {
        body.extend(text(token));
    }
    body.extend(largest.to_be_bytes());
    body
}

/// What a relay that the stand-in dialled does with this Hello: whether it
/// answers with its own.
fn relay_accepts_hello(hello: &[u8]) -> bool {
    let mut relay = relay_end(relay_key(), &[hub_key().public()]);
    let mut stand_in = StandIn::dial(&mut relay, [0x11; 32], relay_key().public());
    let bytes = stand_in.record(hello);
    let output = step(&mut relay, SECOND, Input::Bytes(&bytes));
    let sent = stand_in.open(&output.bytes);
    match sent[..] {
        [ref only] if only == &on_control(RELAY_HELLO) => true,
        [ref only] if only == &close_unit(8) => false,
        _ => panic!("neither Hello nor Close: {sent:02x?}"),
    }
}

/// The Hello page's limits: a release of 1 to 64 bytes, a name of at most
/// 64, at most 64 capability tokens of at most 64 bytes, valid UTF-8 and a
/// largest record of at least 4,096. Tokens are read and ignored.
#[test]
fn a_hello_is_held_to_the_published_limits() {
    let byte_64: &[u8] = &[b'r'; 64];
    let accepted = [
        hello_body(b"2", b"", &[], 4_096),
        hello_body(byte_64, byte_64, &[byte_64], u32::MAX),
        hello_body(b"2", b"", &[&b"t"[..]; 64], 4_096),
        hello_body(
            "é".as_bytes(),
            "ü".as_bytes(),
            &[&b"x"[..], &b""[..]],
            4_096,
        ),
    ];
    for hello in accepted {
        assert!(relay_accepts_hello(&hello), "{hello:02x?}");
    }
    let refused = [
        ("no release", hello_body(b"", b"atlas", &[], 4_096)),
        ("a long release", hello_body(&[b'r'; 65], b"", &[], 4_096)),
        ("a long name", hello_body(b"2", &[b'n'; 65], &[], 4_096)),
        ("65 tokens", hello_body(b"2", b"", &[&b"t"[..]; 65], 4_096)),
        (
            "a long token",
            hello_body(b"2", b"", &[&[b't'; 65][..]], 4_096),
        ),
        ("a small ceiling", hello_body(b"2", b"", &[], 4_095)),
        ("a release not UTF-8", hello_body(b"\xff", b"", &[], 4_096)),
        ("a name not UTF-8", hello_body(b"2", b"\xc3", &[], 4_096)),
        (
            "a token not UTF-8",
            hello_body(b"2", b"", &[&b"\x80"[..]], 4_096),
        ),
    ];
    for (what, hello) in refused {
        assert!(!relay_accepts_hello(&hello), "{what}");
    }
    let whole = hello_body(b"2", b"atlas", &[], 4_096);
    assert!(!relay_accepts_hello(&whole[..whole.len() - 1]), "short");
    assert!(!relay_accepts_hello(&[&whole[..], &[0]].concat()), "long");
    let mut lying = whole.clone();
    lying[4] = 2;
    assert!(!relay_accepts_hello(&lying), "a length past its text");
}

/// A Hello record is at most 4,096 bytes, whatever the receiver's ceiling.
#[test]
fn a_hello_longer_than_4096_bytes_is_a_protocol_violation() {
    let tokens = [&[b't'; 64][..]; 64];
    let long = hello_body(b"2", b"", &tokens, 4_096);
    assert!(long.len() > 4_096);
    let mut hub = hub_end(hub_key(), relay_key().public());
    let mut stand_in = StandIn::answer(&mut hub, [0x22; 32]);
    let bytes = stand_in.fragmented(0, &long);
    let output = step(&mut hub, SECOND, Input::Bytes(&bytes));
    assert_eq!(stand_in.open(&output.bytes), [close_unit(8)]);
}

/// `Hello::new` makes only a Hello within the limits, and it reads back
/// what it was given.
#[test]
fn a_hello_is_made_only_within_the_limits() {
    let made = Hello::new("2.4.0", "atlas", 4_096).expect("a valid Hello");
    assert_eq!(
        (made.release(), made.name(), made.largest_record()),
        ("2.4.0", "atlas", 4_096)
    );
    let long = "x".repeat(64);
    assert!(Hello::new(&long, &long, 4_096).is_some());
    assert!(Hello::new("", "atlas", 4_096).is_none());
    assert!(Hello::new(&"x".repeat(65), "", 4_096).is_none());
    assert!(Hello::new("2", &"x".repeat(65), 4_096).is_none());
    assert!(Hello::new("2", "", 4_095).is_none());
}

/// A record longer than one unit's fragment of 16,363 bytes crosses as
/// several units on its stream, each at most 16,384 bytes of ciphertext,
/// every one but the last flagged "more follows".
#[test]
fn a_long_record_crosses_as_several_units() {
    let (hub, relay) = paired();
    let mut harness = Harness::new(hub, relay);
    harness.run();
    let before = harness.sent_by_relay().len();
    let body = vec![0xab; 2 * 16_363];
    harness.relay.send(7, 0x81, &body).expect("sent");
    harness.run();

    let sent = wire::units(&harness.sent_by_relay()[before..]);
    let lengths: Vec<_> = sent.iter().map(|unit| unit.len() - 2).collect();
    assert_eq!(lengths, [16_384, 16_384, 5 + 1 + 16]);
    let record = Record {
        stream: 7,
        tag: 0x81,
        body,
    };
    assert_eq!(harness.hub.records(), [record]);
}

/// The units of one record arrive in order and form it; units of other
/// streams may come between them, and a keepalive may come between the
/// units of a session stream's record.
#[test]
fn fragments_form_records_with_other_streams_between_them() {
    let (mut hub, mut stand_in) = linked_hub();
    let bytes = [
        stand_in.unit(3, 1, b"\x81fir"),
        stand_in.unit(0, 1, b"\x10con"),
        stand_in.unit(0, 0, b"trol"),
        stand_in.unit(3, 1, b"st"),
        stand_in.unit(0, 0, b""),
        stand_in.unit(3, 0, b"!"),
    ]
    .concat();
    let output = step(&mut hub, 2 * SECOND, Input::Bytes(&bytes));

    assert!(output.bytes.is_empty() && !output.close);
    let record = |stream, tag, body: &[u8]| Record {
        stream,
        tag,
        body: body.to_vec(),
    };
    assert_eq!(
        hub.records(),
        [record(0, 0x10, b"control"), record(3, 0x81, b"first!")]
    );
}

/// The framing rules of the shared page, each broken once: a protocol
/// violation that ends the link with Close.
#[test]
fn a_unit_that_breaks_the_framing_is_a_protocol_violation() {
    relay_refuses("a length under 21", |_| {
        [&[0x00, 0x14][..], &[0; 20]].concat()
    });
    relay_refuses("a length over 16,384", |_| vec![0x40, 0x01]);
    relay_refuses("a flag other than bit 0", |stand_in| {
        stand_in.unit(0, 0x02, b"\x41")
    });
    relay_refuses("an empty fragment that is no keepalive", |stand_in| {
        stand_in.unit(1, 0, b"")
    });
    relay_refuses("an empty fragment flagged more", |stand_in| {
        stand_in.unit(0, 1, b"")
    });
    relay_refuses("a keepalive inside a stream-0 record", |stand_in| {
        [stand_in.unit(0, 1, b"\x10"), stand_in.unit(0, 0, b"")].concat()
    });
    relay_refuses("a record over the receiver's ceiling", |stand_in| {
        stand_in.fragmented(1, &[0x41; 4_097])
    });
}

/// The control: the framing at its limits is accepted. A unit of exactly
/// 16,384 bytes of ciphertext, a record exactly at the ceiling and a
/// keepalive all pass without a reply.
#[test]
fn a_unit_at_the_limits_is_accepted() {
    let (mut relay, sent, closed) = relay_given(|stand_in| {
        [
            stand_in.unit(1, 1, &[0x41; 4_000]),
            stand_in.unit(1, 0, &[0x41; 96]),
            stand_in.unit(0, 0, b""),
        ]
        .concat()
    });
    assert!(sent.is_empty() && !closed);
    let records = relay.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].body.len(), 4_095);

    let (mut hub, sent, closed) = hub_given(|stand_in| stand_in.unit(2, 0, &[0x81; 16_363]));
    assert!(sent.is_empty() && !closed);
    assert_eq!(hub.records()[0].body.len(), 16_362);
}

/// What an end refuses to send: anything before the link is established
/// or after it ends, the tags of Hello and Close, which the link code sends
/// itself, and a record longer than the other end accepts.
#[test]
fn an_end_refuses_what_it_may_not_send() {
    let mut hub = hub_end(hub_key(), relay_key().public());
    assert_eq!(hub.send(1, 0x41, b""), Err(Refused::NotLinked));
    assert_eq!(hub.close(HubClose::Stopped), Err(Refused::NotLinked));

    let (mut hub, _) = linked_hub();
    assert_eq!(hub.send(0, 0x01, b""), Err(Refused::Reserved));
    assert_eq!(hub.send(0, 0x02, b""), Err(Refused::Reserved));
    assert_eq!(hub.send(1, 0x41, &[0; 4_095]), Ok(()));
    assert_eq!(hub.send(1, 0x41, &[0; 4_096]), Err(Refused::TooLong));
    hub.close(HubClose::Stopped).expect("closed");
    assert_eq!(hub.send(1, 0x41, b""), Err(Refused::NotLinked));
    assert_eq!(hub.close(HubClose::Stopped), Err(Refused::NotLinked));

    let (mut relay, _) = linked_relay();
    assert_eq!(relay.send(1, 0x81, &[0; 1_048_575]), Ok(()));
    assert_eq!(relay.send(1, 0x81, &[0; 1_048_576]), Err(Refused::TooLong));
}

/// A byte stream that closes before the link is established fails it, and
/// one that closes an established link without Close loses it.
#[test]
fn a_closed_byte_stream_fails_or_loses_the_link() {
    let (mut relay, _) = linked_relay();
    let output = step(&mut relay, 2 * SECOND, Input::Closed);
    assert!(output.close && output.bytes.is_empty());
    assert_eq!(relay.link(), &Link::Ended(Ended::Lost));

    let mut relay = relay_end(relay_key(), &[hub_key().public()]);
    StandIn::dial(&mut relay, [0x11; 32], relay_key().public());
    let output = step(&mut relay, SECOND, Input::Closed);
    assert!(output.close && output.bytes.is_empty());
    assert_eq!(relay.link(), &Link::Ended(Ended::Failed));
}
