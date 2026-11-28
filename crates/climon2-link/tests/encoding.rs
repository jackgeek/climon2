//! STR-03 at the Link layer (#496): every advertisement, session-stream and
//! client interface record crosses the link in the layout its page
//! publishes, built by the one mechanical rule, and a body that breaks its
//! layout ends the link. A link peer stand-in at the other end sees and
//! sends the bytes.

mod samples;
mod support;
mod wire;

use climon2_link::{
    Cause, EndReason, Ended, HubClose, HubEnd, HubStream, Input, Link, RelayClose, RelayEnd,
    RelayStream, RequestRefused, StreamEnded, StreamRefused,
};
use samples::{LATER, from_hub, from_relay, hub_streaming, linked_hub, on, relay_streaming};
use wire::{StandIn, hex, step};

/// The plaintext of a Close unit that gives "protocol violation".
const VIOLATION: [u8; 8] = [0, 0, 0, 0, 0, 0x02, 0x00, 0x08];

/// STR-03: each request crosses as the record the mechanical rule makes of
/// its C structure: the hub monitor sends exactly those bytes, and a relay
/// that receives them takes the request.
#[test]
fn str_03_each_request_has_the_layout_of_its_structure() {
    for (request, bytes) in samples::requests() {
        let record = from_hub(1, HubStream::Request(request));
        let (mut hub, mut stand_in) = hub_streaming();
        hub.send(&record).expect("sent");
        let output = step(&mut hub, LATER, Input::Tick);
        assert_eq!(stand_in.open(&output.bytes), [on(1, &bytes)], "{record:?}");

        let (mut relay, mut stand_in) = relay_streaming();
        let unit = stand_in.unit(1, 0, &bytes);
        let output = step(&mut relay, LATER, Input::Bytes(&unit));
        assert!(output.bytes.is_empty() && !output.close, "{record:?}");
        assert_eq!(relay.records(), [record]);
    }
}

/// STR-03: each event, and each answer a Response carries, crosses as the
/// record the mechanical rule makes of its C structure, both ways.
#[test]
fn str_03_each_event_has_the_layout_of_its_structure() {
    for (event, bytes) in samples::events().into_iter().chain(samples::responses()) {
        let record = from_relay(1, RelayStream::Event(event));
        let (mut relay, mut stand_in) = relay_streaming();
        relay.send(&record).expect("sent");
        let output = step(&mut relay, LATER, Input::Tick);
        assert_eq!(stand_in.open(&output.bytes), [on(1, &bytes)], "{record:?}");

        let (mut hub, mut stand_in) = hub_streaming();
        let unit = stand_in.unit(1, 0, &bytes);
        let output = step(&mut hub, LATER, Input::Bytes(&unit));
        assert!(output.bytes.is_empty() && !output.close, "{record:?}");
        assert_eq!(hub.records(), [record]);
    }
}

/// The relay's own records on a session stream, each next to the bytes
/// its page's example gives. Each is sent on its own stream 1 after Stream
/// opened, except Stream refused, which answers Open stream.
fn stream_reports() -> Vec<(RelayStream, Vec<u8>, bool)> {
    let refused = StreamRefused {
        cause: Cause::OutOfResources,
        error: no_error(),
    };
    let ended = StreamEnded {
        reason: EndReason::ClientEnded,
        workers_cleanup: 1,
        generation: 3,
        client_outcome: 1,
        retry: 1,
        leave: 2,
        attachment_detached: 0,
        connection_closed: 1,
        error: no_error(),
    };
    let request_refused = RequestRefused {
        request_id: 9,
        code: 5,
        error: no_error(),
    };
    vec![
        (
            RelayStream::Refused(refused),
            hex("22 00 05 00 00 00 00 00 00 00 00 00 00 00 00"),
            false,
        ),
        (
            RelayStream::Ended(ended),
            hex("24 01 00 00 00 01 00 00 00 00 00 00 00 03 00 00
                 00 01 00 00 00 01 00 00 00 02 00 01 00 00 00 00
                 00 00 00 00 00 00 00 00"),
            true,
        ),
        (
            RelayStream::RequestRefused(request_refused),
            hex("26 00 00 00 00 00 00 00 09 00 00 00 05 00 00 00
                 00 00 00 00 00 00 00 00 00"),
            true,
        ),
    ]
}

fn no_error() -> climon2_link::client::Error {
    climon2_link::client::Error {
        origin: 0,
        code: String::new(),
        message: String::new(),
    }
}

/// A relay linked with the stand-in, which has opened stream 1 and, when
/// `answered`, been answered with Stream opened.
fn relay_opened(answered: bool) -> (RelayEnd, StandIn) {
    if answered {
        return relay_streaming();
    }
    let (mut relay, mut stand_in) = samples::linked_relay();
    let open = stand_in.unit(1, 0, &samples::open_stream_record());
    step(&mut relay, LATER, Input::Bytes(&open));
    assert_eq!(relay.records().len(), 1, "Open stream");
    (relay, stand_in)
}

/// The same for a hub monitor that opened stream 1.
fn hub_opened(answered: bool) -> (HubEnd, StandIn) {
    if answered {
        return hub_streaming();
    }
    let (mut hub, mut stand_in) = linked_hub();
    hub.send(&from_hub(1, samples::open_stream()))
        .expect("open");
    let output = step(&mut hub, LATER, Input::Tick);
    assert_eq!(stand_in.open(&output.bytes).len(), 1, "Open stream");
    (hub, stand_in)
}

/// The examples of the Stream refused, Stream ended and Request refused
/// pages, both ways.
#[test]
fn str_03_the_relays_stream_records_have_their_published_layouts() {
    for (report, bytes, answered) in stream_reports() {
        let record = from_relay(1, report);
        let (mut relay, mut stand_in) = relay_opened(answered);
        relay.send(&record).expect("sent");
        let output = step(&mut relay, LATER, Input::Tick);
        assert_eq!(stand_in.open(&output.bytes), [on(1, &bytes)], "{record:?}");

        let (mut hub, mut stand_in) = hub_opened(answered);
        let unit = stand_in.unit(1, 0, &bytes);
        step(&mut hub, LATER, Input::Bytes(&unit));
        assert_eq!(hub.records(), [record]);
    }
}

/// Whether a relay that has answered stream 1 ends the link with "protocol
/// violation" when the stand-in sends `units`, delivering nothing.
fn relay_refuses(units: impl FnOnce(&mut StandIn) -> Vec<u8>) -> bool {
    let (mut relay, mut stand_in) = relay_streaming();
    let units = units(&mut stand_in);
    let output = step(&mut relay, LATER, Input::Bytes(&units));
    let left = Link::Ended(Ended::Left(RelayClose::ProtocolViolation));
    let refused = stand_in.open(&output.bytes) == [VIOLATION] && output.close;
    refused && relay.link() == &left && relay.records().is_empty()
}

/// The same for a hub monitor whose stream 1 the stand-in has answered.
fn hub_refuses(units: impl FnOnce(&mut StandIn) -> Vec<u8>) -> bool {
    let (mut hub, mut stand_in) = hub_streaming();
    let units = units(&mut stand_in);
    let output = step(&mut hub, LATER, Input::Bytes(&units));
    let left = Link::Ended(Ended::Left(HubClose::ProtocolViolation));
    let refused = stand_in.open(&output.bytes) == [VIOLATION] && output.close;
    refused && hub.link() == &left && hub.records().is_empty()
}

/// Each record cut short, cut to its tag alone, or with a byte more than
/// its layout.
fn broken(record: &[u8]) -> [Vec<u8>; 3] {
    [
        record[..record.len() - 1].to_vec(),
        record[..1].to_vec(),
        [record, &[0]].concat(),
    ]
}

/// A record whose body is shorter or longer than its layout is a protocol
/// violation that ends the link, whichever record it is.
#[test]
fn a_record_that_breaks_its_layout_ends_the_link() {
    for (_, record) in samples::requests() {
        for broken in broken(&record) {
            assert!(
                relay_refuses(|stand_in| stand_in.unit(1, 0, &broken)),
                "{broken:02x?}"
            );
        }
    }
    let events = samples::events().into_iter().chain(samples::responses());
    let reports = stream_reports()
        .into_iter()
        .filter(|(.., answered)| *answered);
    let records = events
        .map(|(_, record)| record)
        .chain(reports.map(|(_, record, _)| record))
        .chain([samples::stream_opened_record()]);
    for record in records {
        for broken in broken(&record) {
            assert!(
                hub_refuses(|stand_in| stand_in.unit(1, 0, &broken)),
                "{broken:02x?}"
            );
        }
    }
    for broken in broken(&samples::session_record()) {
        assert!(
            hub_refuses(|stand_in| stand_in.unit(0, 0, &broken)),
            "{broken:02x?}"
        );
    }
}

/// The control: the same records whole are taken, so each refusal above
/// is the layout's and not the stand-in's.
#[test]
fn a_record_in_its_layout_is_taken() {
    let (_, request) = &samples::requests()[0];
    assert!(!relay_refuses(|stand_in| stand_in.unit(1, 0, request)));
    let (_, event) = &samples::events()[0];
    assert!(!hub_refuses(|stand_in| stand_in.unit(1, 0, event)));
    assert!(!hub_refuses(|stand_in| stand_in.unit(
        0,
        0,
        &samples::session_record()
    )));
}

/// What no layout allows, though every length is right: a value that
/// selects no layout, a client interface revision other than 1, text that
/// is not UTF-8, and a count larger than the body holds.
#[test]
fn a_value_that_breaks_a_layout_ends_the_link() {
    let mut no_answer = hex(&format!(
        "81 {} 00 00 00 0d 00 00 00 00 00 00 00 00 00 00 00 00",
        "00 00 00 00 00 00 00 03 00 00 00 00 00 00 00 09 00 00 00 01"
    ));
    let unknown_answer = no_answer.clone();
    no_answer[24] = 0x0c;
    assert!(
        !hub_refuses(|stand_in| stand_in.unit(1, 0, &no_answer)),
        "the control"
    );
    assert!(
        hub_refuses(|stand_in| stand_in.unit(1, 0, &unknown_answer)),
        "result tag 13"
    );

    for cause in [0_u8, 7, 0xff] {
        let refused = [&[0x22, 0x00, cause][..], &[0; 12]].concat();
        assert!(hub_refused_on_answer(&refused), "an unknown cause {cause}");
    }
    let mut ended = stream_reports()[1].1.clone();
    for reason in [0_u8, 4, 0xff] {
        ended[1] = reason;
        assert!(
            hub_refuses(|stand_in| stand_in.unit(1, 0, &ended)),
            "reason {reason}"
        );
    }

    let mut open = samples::open_stream_record();
    open[28] = 2;
    let (mut relay, mut stand_in) = samples::linked_relay();
    let unit = stand_in.unit(1, 0, &open);
    let output = step(&mut relay, LATER, Input::Bytes(&unit));
    assert_eq!(
        stand_in.open(&output.bytes),
        [VIOLATION],
        "interface revision 2"
    );

    let not_utf8 = hex("21 00 00 00 01 00 00 00 01 ff");
    let (mut hub, mut stand_in) = hub_opened(false);
    let unit = stand_in.unit(1, 0, &not_utf8);
    let output = step(&mut hub, LATER, Input::Bytes(&unit));
    assert_eq!(
        stand_in.open(&output.bytes),
        [VIOLATION],
        "a token not UTF-8"
    );

    let counted = hex("49 00 00 00 00 00 00 00 09 ff ff ff ff 00 00 00 01 61");
    assert!(
        relay_refuses(|stand_in| stand_in.unit(1, 0, &counted)),
        "a count of 2^32 - 1"
    );
}

/// Whether a hub monitor that opened stream 1 ends the link when the
/// stand-in answers with `record`.
fn hub_refused_on_answer(record: &[u8]) -> bool {
    let (mut hub, mut stand_in) = hub_opened(false);
    let unit = stand_in.unit(1, 0, record);
    let output = step(&mut hub, LATER, Input::Bytes(&unit));
    stand_in.open(&output.bytes) == [VIOLATION] && hub.records().is_empty()
}

/// REV-04: a record with a tag the receiver does not know, or one only the
/// receiver itself sends, ends the link with "protocol violation". It is
/// never skipped: a good record behind it is not delivered.
#[test]
fn rev_04_an_unknown_record_tag_ends_the_link() {
    let (_, good_request) = samples::requests().remove(0);
    let to_relay = [
        (0, 0x03),
        (0, 0x10),
        (0, 0x20),
        (0, 0x41),
        (0, 0xff),
        (1, 0x00),
        (1, 0x21),
        (1, 0x24),
        (1, 0x25),
        (1, 0x26),
        (1, 0x27),
        (1, 0x40),
        (1, 0x4c),
        (1, 0x7f),
        (1, 0x81),
    ];
    for (stream, tag) in to_relay {
        let refused = relay_refuses(|stand_in| {
            [
                stand_in.unit(stream, 0, &[tag]),
                stand_in.unit(1, 0, &good_request),
            ]
            .concat()
        });
        assert!(refused, "tag {tag:#04x} on stream {stream} to the relay");
    }
    let (_, good_event) = samples::events().remove(0);
    let to_hub = [
        (0, 0x03),
        (0, 0x13),
        (0, 0x20),
        (0, 0x21),
        (0, 0x81),
        (1, 0x00),
        (1, 0x10),
        (1, 0x20),
        (1, 0x23),
        (1, 0x25),
        (1, 0x27),
        (1, 0x41),
        (1, 0x80),
        (1, 0x8f),
        (1, 0xff),
    ];
    for (stream, tag) in to_hub {
        let refused = hub_refuses(|stand_in| {
            [
                stand_in.unit(stream, 0, &[tag]),
                stand_in.unit(1, 0, &good_event),
            ]
            .concat()
        });
        assert!(
            refused,
            "tag {tag:#04x} on stream {stream} to the hub monitor"
        );
    }
}
