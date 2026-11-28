//! Violations at the Link layer (#497): LIM-06 of the monitor-link
//! acceptance criteria. A link peer stand-in breaks one rule at a time. A
//! breach of one session stream's own rules ends only that stream; any
//! other breach closes the link with "protocol violation".

mod samples;
mod support;
mod wire;

use climon2_link::{
    Advertisement, HubEnd, HubStream, Input, Link, OpenStream, Refused, RelayEnd, RelayStream,
};
use samples::{LATER, from_hub, from_relay, on, open_stream_record};
use wire::{StandIn, hex, step};

/// The plaintext of a Close unit that gives "protocol violation".
const VIOLATION: [u8; 8] = [0, 0, 0, 0, 0, 0x02, 0x00, 0x08];

/// The most credit stream 1 can still be granted after its Open stream's
/// 1,048,576 bytes.
const ROOM: u32 = 4_294_967_295 - 1_048_576;

fn credit(amount: u32) -> Vec<u8> {
    [&[0x25][..], &amount.to_be_bytes()].concat()
}

fn request() -> Vec<u8> {
    samples::requests().remove(1).1
}

fn close() -> Vec<u8> {
    vec![0x23]
}

/// The stand-in's units holding `records`, each whole, on `stream`.
fn units_on(stand_in: &mut StandIn, stream: u32, records: &[Vec<u8>]) -> Vec<u8> {
    let unit = |record: &Vec<u8>| stand_in.unit(stream, 0, record);
    records.iter().map(unit).collect::<Vec<_>>().concat()
}

/// A relay linked with the stand-in, which has opened stream 1 and, when
/// `answered`, been answered with Stream opened.
fn relay_opened(answered: bool) -> (RelayEnd, StandIn) {
    if answered {
        return samples::relay_streaming();
    }
    let (mut relay, mut stand_in) = samples::linked_relay();
    let open = stand_in.unit(1, 0, &open_stream_record());
    step(&mut relay, LATER, Input::Bytes(&open));
    assert_eq!(relay.records().len(), 1, "Open stream");
    (relay, stand_in)
}

/// The hub monitor's breaches of one session stream's own rules, each
/// with whether the relay has answered the stream first.
fn hub_breaches() -> Vec<(&'static str, bool, Vec<Vec<u8>>)> {
    vec![
        ("a Credit of nothing", true, vec![credit(0)]),
        ("credit past the most", true, vec![credit(ROOM + 1)]),
        (
            "credit filled, then more",
            true,
            vec![credit(ROOM), credit(1)],
        ),
        ("a request before the answer", false, vec![request()]),
        ("Credit before the answer", false, vec![credit(1)]),
        (
            "a request after Close stream",
            true,
            vec![close(), request()],
        ),
        ("Close stream twice", false, vec![close(), close()]),
    ]
}

/// LIM-06: a hub monitor's breach of one stream's rules ends only that
/// stream. The relay sends nothing itself and tells its caller, which ends
/// the stream with Stream ended "stream violation". The link carries on,
/// and a new stream opens on it.
#[test]
fn lim_06_a_hub_monitors_per_stream_breach_ends_only_that_stream() {
    for (what, answered, records) in hub_breaches() {
        let (mut relay, mut stand_in) = relay_opened(answered);
        let units = units_on(&mut stand_in, 1, &records);
        let output = step(&mut relay, LATER, Input::Bytes(&units));
        assert!(output.bytes.is_empty() && !output.close, "{what}");
        assert!(matches!(relay.link(), Link::Established(_)), "{what}");
        assert_eq!(relay.violations(), [1], "{what}");
        assert!(relay.violations().is_empty(), "{what}: told once");

        let open = stand_in.unit(2, 0, &open_stream_record());
        step(&mut relay, LATER, Input::Bytes(&open));
        let open = from_hub(2, samples::open_stream());
        assert_eq!(relay.records().last(), Some(&open), "{what}");
    }
}

/// The control: the same records in turn are taken, with no violation.
#[test]
fn records_in_turn_break_no_stream() {
    let in_turn = [credit(1), credit(ROOM - 1), request(), close()];
    let (mut relay, mut stand_in) = relay_opened(true);
    let units = units_on(&mut stand_in, 1, &in_turn);
    step(&mut relay, LATER, Input::Bytes(&units));
    assert_eq!(relay.records().len(), 4);
    assert!(relay.violations().is_empty());
}

/// After a stream violation the relay ignores what the hub monitor sends
/// on that stream, and tells its caller nothing more. The relay may still
/// answer the stream, and ends it with Stream ended "stream violation".
#[test]
fn a_violated_stream_is_ignored_until_the_relay_ends_it() {
    for answered in [true, false] {
        let (mut relay, mut stand_in) = relay_opened(answered);
        let breach = units_on(&mut stand_in, 1, &[credit(0)]);
        step(&mut relay, LATER, Input::Bytes(&breach));
        assert_eq!(relay.violations(), [1]);
        relay.records();

        let more = units_on(&mut stand_in, 1, &[request(), credit(1), close()]);
        let output = step(&mut relay, LATER, Input::Bytes(&more));
        assert!(output.bytes.is_empty() && !output.close);
        assert!(relay.records().is_empty() && relay.violations().is_empty());

        if !answered {
            let opened = from_relay(1, samples::stream_opened());
            relay.send(&opened).expect("the answer");
        }
        relay.send(&stream_violation()).expect("Stream ended");
        let output = step(&mut relay, LATER, Input::Tick);
        let sent = stand_in.open(&output.bytes);
        assert_eq!(sent.last(), Some(&on(1, &stream_violation_record())));
    }
}

/// Stream ended for a stream violation, with the Stream ended page's run
/// report.
fn stream_violation() -> climon2_link::RelayRecord {
    from_relay(
        1,
        RelayStream::Ended(climon2_link::StreamEnded {
            reason: climon2_link::EndReason::StreamViolation,
            workers_cleanup: 1,
            generation: 3,
            client_outcome: 1,
            retry: 1,
            leave: 2,
            attachment_detached: 0,
            connection_closed: 1,
            error: climon2_link::client::Error {
                origin: 0,
                code: String::new(),
                message: String::new(),
            },
        }),
    )
}

fn stream_violation_record() -> Vec<u8> {
    hex(
        "24 03 00 00 00 01 00 00 00 00 00 00 00 03 00 00 00 01 00 00 00 01
         00 00 00 02 00 01 00 00 00 00 00 00 00 00 00 00 00 00",
    )
}

/// A breach of the link's own rules on a session stream still closes the
/// link: a stream number skipped or reused, and a record on a stream that
/// never began.
#[test]
fn lim_06_a_hub_monitors_link_breach_closes_the_link() {
    let breaches: [(&str, u32, Vec<u8>); 4] = [
        ("Open stream on a live stream", 1, open_stream_record()),
        ("a stream number skipped", 3, open_stream_record()),
        ("Credit on a stream never begun", 2, credit(1)),
        ("a request on a stream never begun", 2, request()),
    ];
    for (what, stream, record) in breaches {
        let (mut relay, mut stand_in) = relay_opened(true);
        let unit = stand_in.unit(stream, 0, &record);
        let output = step(&mut relay, LATER, Input::Bytes(&unit));
        assert_eq!(stand_in.open(&output.bytes), [VIOLATION], "{what}");
        assert!(output.close, "{what}");
        assert!(relay.violations().is_empty(), "{what}");
    }
}

/// A hub monitor linked with the stand-in, which opened stream 1 with
/// `credit` bytes of first credit and, when `answered`, was answered with
/// Stream opened.
fn hub_opened(credit: u32, answered: bool) -> (HubEnd, StandIn) {
    let (mut hub, mut stand_in) = samples::linked_hub();
    let open = OpenStream {
        session: samples::SESSION_ID,
        generation: 3,
        credit,
    };
    hub.send(&from_hub(1, HubStream::Open(open))).expect("Open");
    let output = step(&mut hub, LATER, Input::Tick);
    assert_eq!(stand_in.open(&output.bytes).len(), 1, "Open stream");
    if answered {
        let unit = stand_in.unit(1, 0, &samples::stream_opened_record());
        step(&mut hub, LATER, Input::Bytes(&unit));
        assert_eq!(hub.records().len(), 1, "Stream opened");
    }
    (hub, stand_in)
}

/// The first sample event and its record.
fn event() -> Vec<u8> {
    samples::events().remove(0).1
}

fn ended() -> Vec<u8> {
    stream_violation_record()
}

/// The relay's breaches of one session stream's own rules, each with the
/// credit stream 1 was opened with and whether the stand-in answered it.
fn relay_breaches() -> Vec<(&'static str, u32, bool, Vec<Vec<u8>>)> {
    let length = event().len() as u32;
    let opened = samples::stream_opened_record();
    vec![
        ("an Event past the credit", length - 1, true, vec![event()]),
        (
            "Events past the credit",
            length,
            true,
            vec![event(), event()],
        ),
        ("an Event before the answer", length, false, vec![event()]),
        ("Stream opened twice", length, true, vec![opened]),
        (
            "Stream ended before the answer",
            length,
            false,
            vec![ended()],
        ),
    ]
}

/// LIM-06: a relay's breach of one stream's rules ends only that stream.
/// The hub monitor sends Close stream, tells its caller and carries on
/// with the link: an advertisement behind the breach arrives.
#[test]
fn lim_06_a_relays_per_stream_breach_ends_only_that_stream() {
    for (what, credit, answered, records) in relay_breaches() {
        let (mut hub, mut stand_in) = hub_opened(credit, answered);
        let units = [
            units_on(&mut stand_in, 1, &records),
            stand_in.unit(0, 0, &[0x12]),
        ]
        .concat();
        let output = step(&mut hub, LATER, Input::Bytes(&units));
        assert_eq!(stand_in.open(&output.bytes), [on(1, &close())], "{what}");
        assert!(!output.close, "{what}");
        assert!(matches!(hub.link(), Link::Established(_)), "{what}");
        assert_eq!(hub.violations(), [1], "{what}");
        let snapshot = samples::advertise(Advertisement::SnapshotComplete);
        assert_eq!(hub.records().last(), Some(&snapshot), "{what}");
    }
}

/// The control: an Event that its credit covers exactly is taken.
#[test]
fn an_event_within_the_credit_breaks_no_stream() {
    let (mut hub, mut stand_in) = hub_opened(event().len() as u32, true);
    let unit = stand_in.unit(1, 0, &event());
    let output = step(&mut hub, LATER, Input::Bytes(&unit));
    assert!(output.bytes.is_empty());
    assert_eq!(hub.records().len(), 1);
    assert!(hub.violations().is_empty());
}

/// After a stream violation the hub monitor sends nothing more on the
/// stream, discards what the relay sends there without telling its caller
/// again, and takes the Stream ended that ends it. A hub monitor that had
/// already sent Close stream sends no second one.
#[test]
fn a_violated_stream_is_discarded_until_the_relay_ends_it() {
    let (mut hub, mut stand_in) = hub_opened(0, true);
    let breach = units_on(&mut stand_in, 1, &[event()]);
    step(&mut hub, LATER, Input::Bytes(&breach));
    assert_eq!(hub.violations(), [1]);
    let request = from_hub(1, HubStream::Request(samples::requests().remove(1).0));
    assert_eq!(hub.send(&request), Err(Refused::OutOfTurn));

    let opened = samples::stream_opened_record();
    let more = units_on(&mut stand_in, 1, &[event(), opened, ended()]);
    let output = step(&mut hub, LATER, Input::Bytes(&more));
    assert!(output.bytes.is_empty() && !output.close);
    assert!(hub.violations().is_empty(), "told once");
    let records = hub.records();
    assert_eq!(records, [stream_violation()], "only the ending");

    let (mut hub, mut stand_in) = hub_opened(0, true);
    hub.send(&from_hub(1, HubStream::Close))
        .expect("Close stream");
    let output = step(&mut hub, LATER, Input::Tick);
    assert_eq!(stand_in.open(&output.bytes), [on(1, &close())]);
    let breach = units_on(&mut stand_in, 1, &[event()]);
    let output = step(&mut hub, LATER, Input::Bytes(&breach));
    assert!(output.bytes.is_empty(), "no second Close stream");
    assert_eq!(hub.violations(), [1]);
}

/// A breach of the link's own rules by the relay still closes the link: a
/// record on a stream that has ended or never began.
#[test]
fn lim_06_a_relays_link_breach_closes_the_link() {
    let breaches: [(&str, u32, Vec<Vec<u8>>); 3] = [
        ("an Event after Stream ended", 1, vec![ended(), event()]),
        ("an Event on a stream never begun", 2, vec![event()]),
        (
            "Stream opened on a stream never begun",
            2,
            vec![samples::stream_opened_record()],
        ),
    ];
    for (what, stream, records) in breaches {
        let (mut hub, mut stand_in) = hub_opened(1_048_576, true);
        let units = units_on(&mut stand_in, stream, &records);
        let output = step(&mut hub, LATER, Input::Bytes(&units));
        assert_eq!(stand_in.open(&output.bytes), [VIOLATION], "{what}");
        assert!(output.close, "{what}");
        assert!(hub.violations().is_empty(), "{what}");
    }
}

/// The Session page's example with a command of `size` bytes in place of
/// `vim`, which makes its record 92 bytes and the command.
fn session_record_with(size: usize) -> Vec<u8> {
    let sample = samples::session_record();
    let command = [&(size as u32).to_be_bytes()[..], &vec![b'x'; size]].concat();
    [&sample[..21], &command, &sample[28..]].concat()
}

/// What a linked hub monitor does when the stand-in sends `records` on
/// stream 0 in one batch, taking none of them until all have arrived:
/// whether it closes the link with "protocol violation".
fn hub_overflows(records: &[Vec<u8>]) -> bool {
    let (mut hub, mut stand_in) = samples::linked_hub();
    let units = records.iter().map(|r| stand_in.fragmented(0, r));
    let units = units.collect::<Vec<_>>().concat();
    let output = step(&mut hub, LATER, Input::Bytes(&units));
    let sent = stand_in.open(&output.bytes);
    match sent.as_slice() {
        [] => false,
        [close] if *close == VIOLATION && output.close => true,
        other => panic!("unexpected units {other:02x?}"),
    }
}

/// LIM-06: the hub monitor holds at most 1,024 stream-0 records that its
/// caller has not yet taken. The 1,025th closes the link.
#[test]
fn lim_06_more_than_1024_unprocessed_control_records_close_the_link() {
    let session = samples::session_record();
    assert!(!hub_overflows(&vec![session.clone(); 1_024]));
    assert!(hub_overflows(&vec![session; 1_025]));
}

/// LIM-06: the same records hold at most 1,048,576 bytes, tags included.
/// One byte more closes the link.
#[test]
fn lim_06_more_than_1048576_unprocessed_control_bytes_close_the_link() {
    let half = session_record_with(524_288 - 92);
    assert_eq!(half.len(), 524_288);
    let longer = session_record_with(524_288 - 92 + 1);
    assert!(!hub_overflows(&[half.clone(), half.clone()]));
    assert!(hub_overflows(&[half.clone(), longer]));
    assert!(hub_overflows(&[half.clone(), half.clone(), vec![0x12]]));
}

/// Records the caller has taken leave the queue: the same flood taken in
/// two parts is within the bound.
#[test]
fn taken_control_records_free_the_queue() {
    let (mut hub, mut stand_in) = samples::linked_hub();
    let session = samples::session_record();
    for _ in 0..2 {
        let units = (0..1_000).map(|_| stand_in.unit(0, 0, &session));
        let units = units.collect::<Vec<_>>().concat();
        let output = step(&mut hub, LATER, Input::Bytes(&units));
        assert!(output.bytes.is_empty() && !output.close);
        assert_eq!(hub.records().len(), 1_000);
    }
}

/// What a hub monitor whose stream 1 is answered does when the stand-in
/// sends `units`: whether it closes the link with "protocol violation".
fn hub_closes_on(units: impl FnOnce(&mut StandIn) -> Vec<u8>) -> bool {
    let (mut hub, mut stand_in) = samples::hub_streaming();
    let units = units(&mut stand_in);
    let output = step(&mut hub, LATER, Input::Bytes(&units));
    stand_in.open(&output.bytes) == [VIOLATION] && output.close && hub.violations().is_empty()
}

/// LIM-06: an oversized unit, a record above the hub monitor's stated
/// ceiling of 1,048,576 bytes, even on a session stream, and an
/// advertisement flood past 128 sessions each close the link.
#[test]
fn lim_06_an_oversized_unit_record_or_flood_closes_the_link() {
    assert!(hub_closes_on(|_| [&[0x40, 0x01][..], &[0; 16_385]].concat()));
    let past_ceiling = [&event()[..1], &vec![0; 1_048_576]].concat();
    assert!(hub_closes_on(
        |stand_in| stand_in.fragmented(1, &past_ceiling)
    ));
    let flood = |stand_in: &mut StandIn| {
        let session = |id: u8| [&[0x10][..], &[id; 16], &samples::session_record()[17..]].concat();
        let units = (0..=128).map(|id| stand_in.unit(0, 0, &session(id)));
        units.collect::<Vec<_>>().concat()
    };
    assert!(hub_closes_on(flood));
}
