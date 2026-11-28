//! Session streams at the Link layer (#496): STR-02's numbering and order
//! of records, and STR-03's fragments, with control records overtaking a
//! large event. Real ends meet in the harness; a link peer stand-in sends
//! what an honest end never would.

mod samples;
mod support;
mod wire;

use climon2_link::{
    EndReason, Ended, HubClose, HubEnd, HubRecord, HubStream, Input, Link, Refused, RelayClose,
    RelayEnd, RelayRecord, RelayStream, StreamEnded, StreamRefused,
};
use samples::{
    LATER, from_hub, from_relay, hub_streaming, linked_hub, linked_relay, on, open_stream,
    relay_streaming, stream_opened,
};
use support::Harness;
use wire::{StandIn, paired, step};

/// The plaintext of a Close unit that gives "protocol violation".
const VIOLATION: [u8; 8] = [0, 0, 0, 0, 0, 0x02, 0x00, 0x08];

fn open(stream: u32) -> HubRecord {
    from_hub(stream, open_stream())
}

fn request(stream: u32) -> HubRecord {
    let (request, _) = samples::requests().remove(1);
    from_hub(stream, HubStream::Request(request))
}

fn close(stream: u32) -> HubRecord {
    from_hub(stream, HubStream::Close)
}

fn opened(stream: u32) -> RelayRecord {
    from_relay(stream, stream_opened())
}

fn event(stream: u32) -> RelayRecord {
    let (event, _) = samples::events().remove(0);
    from_relay(stream, RelayStream::Event(event))
}

fn no_error() -> climon2_link::client::Error {
    climon2_link::client::Error {
        origin: 0,
        code: String::new(),
        message: String::new(),
    }
}

fn ended(stream: u32) -> RelayRecord {
    from_relay(
        stream,
        RelayStream::Ended(StreamEnded {
            reason: EndReason::ClientEnded,
            workers_cleanup: 1,
            generation: 3,
            client_outcome: 1,
            retry: 1,
            leave: 2,
            attachment_detached: 0,
            connection_closed: 1,
            error: no_error(),
        }),
    )
}

fn refused(stream: u32) -> RelayRecord {
    from_relay(
        stream,
        RelayStream::Refused(StreamRefused {
            cause: climon2_link::Cause::NotAdvertised,
            error: no_error(),
        }),
    )
}

/// The record `record` as the stand-in sends it.
fn units_of(stand_in: &mut StandIn, stream: u32, record: &[u8]) -> Vec<u8> {
    stand_in.unit(stream, 0, record)
}

/// The record of a hub monitor's record, tag included, as the stand-in
/// would send it. The encodings are those `encoding.rs` proves.
fn hub_bytes(record: &HubRecord) -> Vec<u8> {
    match &record.record {
        HubStream::Open(_) => samples::open_stream_record(),
        HubStream::Close => vec![0x23],
        HubStream::Request(_) => samples::requests().remove(1).1,
    }
}

/// The same for a relay's record on a session stream.
fn relay_bytes(record: &RelayRecord) -> Vec<u8> {
    let RelayRecord::Stream { record, .. } = record else {
        panic!("a session stream's record");
    };
    let report = |hex: &str| wire::hex(hex);
    match record {
        RelayStream::Opened(_) => samples::stream_opened_record(),
        RelayStream::Event(_) => samples::events().remove(0).1,
        RelayStream::Ended(_) => report(
            "24 01 00 00 00 01 00 00 00 00 00 00 00 03 00 00 00 01 00 00 00 01
             00 00 00 02 00 01 00 00 00 00 00 00 00 00 00 00 00 00",
        ),
        RelayStream::Refused(_) => report("22 00 01 00 00 00 00 00 00 00 00 00 00 00 00"),
        RelayStream::RequestRefused(_) => panic!("not used here"),
    }
}

/// What a linked relay does when the stand-in sends `records` in order:
/// the relay, the plaintexts it sends back and whether it closes.
fn relay_hears(records: &[HubRecord]) -> (RelayEnd, Vec<Vec<u8>>, bool) {
    let (mut relay, mut stand_in) = linked_relay();
    let mut sent = Vec::new();
    let mut closed = false;
    for record in records {
        let bytes = units_of(&mut stand_in, record.stream, &hub_bytes(record));
        let output = step(&mut relay, LATER, Input::Bytes(&bytes));
        sent.extend(stand_in.open(&output.bytes));
        closed |= output.close;
        // The relay answers each Open stream at once, as a test's relay
        // would, unless the next record is meant to come first.
        if matches!(record.record, HubStream::Open(_)) && !closed {
            let _ = relay.send(&opened(record.stream));
            let output = step(&mut relay, LATER, Input::Tick);
            stand_in.open(&output.bytes);
        }
    }
    (relay, sent, closed)
}

/// What a hub monitor that has opened streams 1 and 2 does when the
/// stand-in sends `records` in order: the plaintexts it sends back and
/// whether it closes, and the records it took.
fn hub_hears(records: &[RelayRecord]) -> (HubEnd, Vec<Vec<u8>>, bool) {
    let (mut hub, mut stand_in) = linked_hub();
    for stream in [1, 2] {
        hub.send(&open(stream)).expect("open");
    }
    let output = step(&mut hub, LATER, Input::Tick);
    stand_in.open(&output.bytes);
    let mut sent = Vec::new();
    let mut closed = false;
    for record in records {
        let RelayRecord::Stream { stream, .. } = record else {
            panic!("a session stream's record");
        };
        let bytes = units_of(&mut stand_in, *stream, &relay_bytes(record));
        let output = step(&mut hub, LATER, Input::Bytes(&bytes));
        sent.extend(stand_in.open(&output.bytes));
        closed |= output.close;
    }
    (hub, sent, closed)
}

fn relay_violated(records: &[HubRecord]) -> bool {
    let (relay, sent, closed) = relay_hears(records);
    let left = Link::Ended(Ended::Left(RelayClose::ProtocolViolation));
    sent.last() == Some(&VIOLATION.to_vec()) && closed && relay.link() == &left
}

fn hub_violated(records: &[RelayRecord]) -> bool {
    let (hub, sent, closed) = hub_hears(records);
    let left = Link::Ended(Ended::Left(HubClose::ProtocolViolation));
    sent == [VIOLATION] && closed && hub.link() == &left
}

/// STR-02: the hub monitor numbers streams from 1, each the previous plus
/// one, and never uses a number twice. An end asked to break the rule
/// refuses, and a relay that receives a broken number ends the link.
#[test]
fn str_02_streams_are_numbered_from_1_and_never_reused() {
    let (mut hub, _) = linked_hub();
    assert_eq!(hub.send(&open(0)), Err(Refused::OutOfTurn));
    assert_eq!(hub.send(&open(2)), Err(Refused::OutOfTurn));
    assert_eq!(hub.send(&open(1)), Ok(()));
    assert_eq!(hub.send(&open(1)), Err(Refused::OutOfTurn));
    assert_eq!(hub.send(&open(3)), Err(Refused::OutOfTurn));
    assert_eq!(hub.send(&open(2)), Ok(()));

    let (mut relay, _, closed) = relay_hears(&[open(1), open(2)]);
    assert!(!closed);
    assert_eq!(relay.records(), [open(1), open(2)]);
    assert!(relay_violated(&[open(2)]), "a first stream of 2");
    assert!(relay_violated(&[open(1), open(1)]), "stream 1 twice");
    assert!(relay_violated(&[open(1), open(3)]), "a number skipped");
}

/// STR-02: a reconnect is a new connection, whose streams start again
/// from 1 and carry new instances.
#[test]
fn str_02_a_reconnect_numbers_its_streams_from_1_again() {
    for _ in 0..2 {
        let (hub, relay) = paired();
        let mut harness = Harness::new(hub, relay);
        harness.run();
        harness
            .hub
            .send(&open(1))
            .expect("stream 1 on every connection");
        harness.run();
        assert_eq!(harness.relay.records(), [open(1)]);
    }
}

/// STR-02: the relay answers Open stream with Stream opened or Stream
/// refused before anything else, sends nothing after Stream ended or Stream
/// refused, and sends nothing on a stream the hub monitor never opened. An
/// end asked to break the order refuses; a hub monitor that receives it
/// ends the link.
#[test]
fn str_02_the_relay_answers_first_and_stream_ended_is_last() {
    let (mut relay, _, _) = relay_hears(&[]);
    assert_eq!(
        relay.send(&opened(1)),
        Err(Refused::OutOfTurn),
        "not opened"
    );

    let (mut relay, mut stand_in) = linked_relay();
    let bytes = units_of(&mut stand_in, 1, &samples::open_stream_record());
    step(&mut relay, LATER, Input::Bytes(&bytes));
    assert_eq!(relay.send(&event(1)), Err(Refused::OutOfTurn), "unanswered");
    assert_eq!(relay.send(&ended(1)), Err(Refused::OutOfTurn), "unanswered");
    assert_eq!(relay.send(&opened(1)), Ok(()));
    assert_eq!(relay.send(&opened(1)), Err(Refused::OutOfTurn), "twice");
    assert_eq!(relay.send(&refused(1)), Err(Refused::OutOfTurn), "answered");
    assert_eq!(relay.send(&event(1)), Ok(()));
    assert_eq!(relay.send(&ended(1)), Ok(()));
    for after in [event(1), ended(1), opened(1), refused(1)] {
        assert_eq!(relay.send(&after), Err(Refused::OutOfTurn), "{after:?}");
    }

    let bytes = units_of(&mut stand_in, 2, &samples::open_stream_record());
    step(&mut relay, LATER, Input::Bytes(&bytes));
    assert_eq!(relay.send(&refused(2)), Ok(()));
    for after in [event(2), ended(2), opened(2), refused(2)] {
        assert_eq!(relay.send(&after), Err(Refused::OutOfTurn), "{after:?}");
    }

    let in_order = [opened(1), event(1), ended(1), refused(2)];
    let (mut hub, sent, closed) = hub_hears(&in_order);
    assert!(sent.is_empty() && !closed);
    assert_eq!(hub.records(), in_order);
    for out_of_turn in [
        &[event(1)][..],
        &[ended(1)],
        &[opened(1), opened(1)],
        &[opened(1), refused(1)],
        &[refused(1), event(1)],
        &[opened(1), ended(1), event(1)],
        &[opened(1), ended(1), ended(1)],
        &[opened(3)],
    ] {
        assert!(hub_violated(out_of_turn), "{out_of_turn:?}");
    }
}

/// STR-02: the hub monitor sends requests only after Stream opened and
/// before its own Close stream, and nothing after Close stream. Close
/// stream may come before the relay answers.
#[test]
fn str_02_the_hub_monitor_sends_in_turn() {
    let (mut hub, mut stand_in) = linked_hub();
    assert_eq!(hub.send(&request(1)), Err(Refused::OutOfTurn), "not opened");
    assert_eq!(hub.send(&close(1)), Err(Refused::OutOfTurn), "not opened");
    hub.send(&open(1)).expect("open");
    assert_eq!(hub.send(&request(1)), Err(Refused::OutOfTurn), "unanswered");
    assert_eq!(hub.send(&close(1)), Ok(()), "unanswered");
    assert_eq!(hub.send(&close(1)), Err(Refused::OutOfTurn), "closed");
    let output = step(&mut hub, LATER, Input::Tick);
    stand_in.open(&output.bytes);

    let (mut hub, _) = hub_streaming();
    assert_eq!(hub.send(&request(1)), Ok(()));
    assert_eq!(hub.send(&close(1)), Ok(()));
    assert_eq!(hub.send(&request(1)), Err(Refused::OutOfTurn), "closed");

    assert!(
        !relay_violated(&[open(1), request(1), close(1)]),
        "the control"
    );
    assert!(relay_violated(&[request(1)]), "not opened");
    assert!(relay_violated(&[close(1)]), "not opened");
    assert!(
        relay_violated(&[open(1), close(1), close(1)]),
        "closed twice"
    );
    assert!(
        relay_violated(&[open(1), close(1), request(1)]),
        "after Close"
    );
}

/// A Request before the relay has answered is out of turn too.
#[test]
fn a_request_before_the_relays_answer_ends_the_link() {
    let (mut relay, mut stand_in) = linked_relay();
    let bytes = [
        units_of(&mut stand_in, 1, &samples::open_stream_record()),
        units_of(&mut stand_in, 1, &hub_bytes(&request(1))),
    ]
    .concat();
    let output = step(&mut relay, LATER, Input::Bytes(&bytes));
    assert_eq!(stand_in.open(&output.bytes), [VIOLATION]);
}

/// A hub monitor's record on a stream that has ended may have crossed the
/// ending on the wire: the relay ignores it, and the link and its other
/// streams carry on. The hub monitor, which knows the stream has ended,
/// does not send one.
#[test]
fn a_record_that_crossed_the_streams_end_is_ignored() {
    let (mut relay, mut stand_in) = relay_streaming();
    relay.send(&ended(1)).expect("ended");
    let output = step(&mut relay, LATER, Input::Tick);
    stand_in.open(&output.bytes);
    let crossed = [
        units_of(&mut stand_in, 1, &hub_bytes(&request(1))),
        units_of(&mut stand_in, 1, &[0x23]),
        units_of(&mut stand_in, 2, &samples::open_stream_record()),
    ]
    .concat();
    let output = step(&mut relay, LATER, Input::Bytes(&crossed));
    assert!(output.bytes.is_empty() && !output.close);
    assert_eq!(relay.records(), [open(2)]);

    let (mut hub, mut stand_in) = hub_streaming();
    let bytes = units_of(&mut stand_in, 1, &relay_bytes(&ended(1)));
    step(&mut hub, LATER, Input::Bytes(&bytes));
    assert_eq!(hub.records(), [ended(1)]);
    assert_eq!(hub.send(&request(1)), Err(Refused::OutOfTurn));
    assert_eq!(hub.send(&close(1)), Err(Refused::OutOfTurn));
}

/// After Close stream the hub monitor discards what the relay sent before
/// its Stream ended, and takes the Stream ended or Stream refused that ends
/// the stream.
#[test]
fn after_close_stream_the_hub_monitor_takes_only_the_ending() {
    let (mut hub, mut stand_in) = hub_streaming();
    hub.send(&close(1)).expect("close");
    let output = step(&mut hub, LATER, Input::Tick);
    assert_eq!(stand_in.open(&output.bytes), [on(1, &[0x23])]);
    let bytes = [
        units_of(&mut stand_in, 1, &relay_bytes(&event(1))),
        units_of(&mut stand_in, 1, &relay_bytes(&ended(1))),
    ]
    .concat();
    let output = step(&mut hub, LATER, Input::Bytes(&bytes));
    assert!(output.bytes.is_empty() && !output.close);
    assert_eq!(hub.records(), [ended(1)]);

    for ending in [ended(1), refused(1)] {
        let (mut hub, mut stand_in) = linked_hub();
        hub.send(&open(1)).expect("open");
        hub.send(&close(1)).expect("close before the answer");
        let output = step(&mut hub, LATER, Input::Tick);
        stand_in.open(&output.bytes);
        let answer = match ending {
            RelayRecord::Stream {
                record: RelayStream::Ended(_),
                ..
            } => units_of(&mut stand_in, 1, &samples::stream_opened_record()),
            _ => Vec::new(),
        };
        let bytes = [answer, units_of(&mut stand_in, 1, &relay_bytes(&ending))].concat();
        let output = step(&mut hub, LATER, Input::Bytes(&bytes));
        assert!(output.bytes.is_empty() && !output.close);
        assert_eq!(hub.records(), [ending]);
    }
}

/// A linked pair of real ends with streams 1 and 2 opened and answered.
fn two_streams() -> Harness<HubEnd, RelayEnd> {
    let (hub, relay) = paired();
    let mut harness = Harness::new(hub, relay);
    harness.run();
    for stream in [1, 2] {
        harness.hub.send(&open(stream)).expect("open");
    }
    harness.run();
    for stream in [1, 2] {
        harness.relay.send(&opened(stream)).expect("opened");
    }
    harness.run();
    assert_eq!(harness.hub.records(), [opened(1), opened(2)]);
    assert_eq!(harness.relay.records(), [open(1), open(2)]);
    harness
}

/// STR-03: a large event on one stream, queued with another stream's
/// events and a control record, crosses in units of at most 16,384 bytes,
/// each one fragment of one record. The control record goes first, the two
/// streams take turns, and every record arrives complete and in order.
#[test]
fn str_03_control_records_overtake_a_large_event() {
    let mut harness = two_streams();
    let before = harness.sent_by_relay().len();
    let large = from_relay(1, RelayStream::Event(samples::display(100_000, 0xab)));
    let small = |byte| from_relay(2, RelayStream::Event(samples::display(10, byte)));
    let session = samples::advertise(climon2_link::Advertisement::Session(samples::session()));
    for record in [&large, &small(1), &small(2), &session] {
        harness.relay.send(record).expect("sent");
    }
    harness.run();

    let units = wire::units(&harness.sent_by_relay()[before..]);
    assert!(units.iter().all(|unit| unit.len() - 2 <= 16_384));
    // Each unit's stream, read from its length: a fragment of the large
    // event fills a unit, and the other records fit one each.
    let lengths: Vec<_> = units.iter().map(|unit| unit.len() - 2).collect();
    let large_unit = 16_384;
    let small_unit = 5 + 16 + 1 + 8 + 16 + 4 + 10;
    let session_unit = 5 + 16 + samples::session_record().len();
    let last = 5 + 16 + (1 + 8 + 16 + 4 + 100_000) - 6 * 16_363;
    assert_eq!(
        lengths,
        [
            session_unit,
            large_unit,
            small_unit,
            large_unit,
            small_unit,
            large_unit,
            large_unit,
            large_unit,
            large_unit,
            last,
        ]
    );
    assert_eq!(
        harness.hub.records(),
        [session, small(1), small(2), large],
        "complete, each stream in order"
    );
}

/// An end sends at most 8 units in one step. With more to send it names
/// the present moment as its deadline, and a record queued before the next
/// step goes ahead of what is left.
#[test]
fn an_end_sends_8_units_a_step_and_lets_control_go_ahead() {
    let (mut relay, mut stand_in) = relay_streaming();
    let large = from_relay(1, RelayStream::Event(samples::display(200_000, 0xcd)));
    relay.send(&large).expect("sent");
    let first = step(&mut relay, LATER, Input::Tick);
    assert_eq!(wire::units(&first.bytes).len(), 8);
    assert_eq!(first.deadline, Some(climon2_link::Time::since_start(LATER)));
    assert!(!first.close);

    let session = samples::advertise(climon2_link::Advertisement::SnapshotComplete);
    relay.send(&session).expect("sent");
    let second = step(&mut relay, LATER, Input::Tick);
    let opened = stand_in.open(&[first.bytes, second.bytes.clone()].concat());
    assert_eq!(opened[8], on(0, &[0x12]), "the control record goes next");
    assert_eq!(
        wire::units(&second.bytes).len(),
        6,
        "and the rest of the event"
    );
    assert_eq!(second.deadline, None, "nothing more to send");
}

/// A record longer than one unit's fragment of 16,363 bytes crosses as
/// several units on its stream, each at most 16,384 bytes of ciphertext,
/// every one but the last flagged "more follows".
#[test]
fn a_long_record_crosses_as_several_units() {
    let (mut relay, mut stand_in) = relay_streaming();
    let body = 2 * 16_363 - (1 + 8 + 16 + 4);
    let large = from_relay(1, RelayStream::Event(samples::display(body, 0xab)));
    relay.send(&large).expect("sent");
    let output = step(&mut relay, LATER, Input::Tick);
    let lengths: Vec<_> = wire::units(&output.bytes)
        .iter()
        .map(|unit| unit.len() - 2)
        .collect();
    assert_eq!(lengths, [16_384, 16_384]);
    let flags: Vec<_> = stand_in
        .open(&output.bytes)
        .iter()
        .map(|plaintext| plaintext[..5].to_vec())
        .collect();
    assert_eq!(flags, [[0, 0, 0, 1, 1], [0, 0, 0, 1, 0]]);
}

/// A record that would be longer than the other end accepts is refused,
/// and one exactly that long is sent.
#[test]
fn an_end_refuses_a_record_longer_than_the_other_end_accepts() {
    let (mut relay, _) = relay_streaming();
    let most = 1_048_576 - (1 + 8 + 16 + 4);
    let event = |size| from_relay(1, RelayStream::Event(samples::display(size, 0)));
    assert_eq!(relay.send(&event(most)), Ok(()));
    assert_eq!(relay.send(&event(most + 1)), Err(Refused::TooLong));

    let mut harness = two_streams();
    let input = |size| {
        let (mut request, _) = samples::requests().remove(1);
        if let climon2_link::client::Arguments::Input(input) = &mut request.arguments {
            input.bytes = vec![b'x'; size];
        }
        from_hub(1, HubStream::Request(request))
    };
    let most = 4_096 - (1 + 8 + 24 + 4);
    assert_eq!(harness.hub.send(&input(most)), Ok(()));
    assert_eq!(harness.hub.send(&input(most + 1)), Err(Refused::TooLong));
    harness.run();
    assert_eq!(harness.relay.records(), [input(most)]);
}
