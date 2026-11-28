//! Credit at the Link layer (#497): the Link-layer parts of LIM-04 of the
//! monitor-link acceptance criteria. A link peer stand-in grants credit to
//! a real relay, and sends events past the credit to a real hub monitor.

mod samples;
mod support;
mod wire;

use climon2_link::client::Error;
use climon2_link::{
    EndReason, HubStream, Input, Refused, RelayEnd, RelayRecord, RelayStream, RequestRefused,
    StreamEnded,
};
use samples::{LATER, from_hub, from_relay, hub_streaming, on, relay_streaming};
use wire::{StandIn, hex, step};

/// The Credit page's example: 16,384 more bytes.
fn credit_record() -> Vec<u8> {
    hex("25 00 00 40 00")
}

/// Credit crosses in the Credit page's layout, both ways.
#[test]
fn lim_04_credit_has_the_published_layout() {
    let (mut hub, mut stand_in) = hub_streaming();
    hub.send(&from_hub(1, HubStream::Credit(16_384)))
        .expect("Credit");
    let output = step(&mut hub, LATER, Input::Tick);
    assert_eq!(stand_in.open(&output.bytes), [on(1, &credit_record())]);

    let (mut relay, mut stand_in) = relay_streaming();
    let unit = stand_in.unit(1, 0, &credit_record());
    step(&mut relay, LATER, Input::Bytes(&unit));
    assert_eq!(relay.records(), [from_hub(1, HubStream::Credit(16_384))]);
}

/// A Display event on stream 1 carrying `size` bytes. Its record is 29
/// bytes and the display: the tag, the generation, the period, the
/// restoration ID and the length of the bytes (the Event page's example
/// carries two bytes in 31).
fn display(size: usize) -> RelayRecord {
    from_relay(1, RelayStream::Event(samples::display(size, 0x68)))
}

/// Grants stream 1 `amount` more bytes from the stand-in.
fn grant(relay: &mut RelayEnd, stand_in: &mut StandIn, amount: u32) {
    let record = [&[0x25][..], &amount.to_be_bytes()].concat();
    let unit = stand_in.unit(1, 0, &record);
    let output = step(relay, LATER, Input::Bytes(&unit));
    assert!(
        output.bytes.is_empty(),
        "nothing queued, and nothing in answer"
    );
}

/// A relay linked with the stand-in, which opened stream 1 with `credit`
/// bytes of first credit, answered with Stream opened.
fn relay_with_credit(credit: u32) -> (RelayEnd, StandIn) {
    let (mut relay, mut stand_in) = samples::linked_relay();
    let mut open = samples::open_stream_record();
    open.truncate(open.len() - 4);
    open.extend(credit.to_be_bytes());
    let unit = stand_in.unit(1, 0, &open);
    step(&mut relay, LATER, Input::Bytes(&unit));
    assert_eq!(relay.records().len(), 1, "Open stream");
    relay
        .send(&from_relay(1, samples::stream_opened()))
        .expect("Stream opened");
    let output = step(&mut relay, LATER, Input::Tick);
    assert_eq!(stand_in.open(&output.bytes).len(), 1, "Stream opened");
    (relay, stand_in)
}

fn no_error() -> Error {
    Error {
        origin: 0,
        code: String::new(),
        message: String::new(),
    }
}

/// The relay takes an Event only when the stream's credit covers all of
/// its record, tag included. It holds no Event past the credit: its caller
/// keeps the rest in the client library until a Credit arrives.
#[test]
fn lim_04_an_event_takes_its_records_length_in_credit() {
    let (mut relay, mut stand_in) = relay_with_credit(61);
    relay.send(&display(2)).expect("31 bytes of 61");
    assert_eq!(relay.send(&display(2)), Err(Refused::Credit), "31 of 30");
    relay.send(&display(1)).expect("30 bytes of 30");
    assert_eq!(relay.send(&display(0)), Err(Refused::Credit), "29 of none");
    let output = step(&mut relay, LATER, Input::Tick);
    assert_eq!(stand_in.open(&output.bytes).len(), 2, "two events");

    grant(&mut relay, &mut stand_in, 28);
    assert_eq!(relay.records(), [from_hub(1, HubStream::Credit(28))]);
    assert_eq!(relay.send(&display(0)), Err(Refused::Credit), "29 of 28");
    grant(&mut relay, &mut stand_in, 1);
    relay.send(&display(0)).expect("29 bytes of 29");
    let output = step(&mut relay, LATER, Input::Tick);
    assert_eq!(stand_in.open(&output.bytes).len(), 1, "a third event");
}

/// Credit counts Events and nothing else: a stream with none still sends
/// Request refused and Stream ended.
#[test]
fn lim_04_only_events_use_credit() {
    let (mut relay, _) = relay_with_credit(0);
    assert_eq!(relay.send(&display(0)), Err(Refused::Credit));
    let refused = RelayStream::RequestRefused(RequestRefused {
        request_id: 9,
        code: 5,
        error: no_error(),
    });
    relay
        .send(&from_relay(1, refused))
        .expect("Request refused");
    let ended = RelayStream::Ended(StreamEnded {
        reason: EndReason::ClientEnded,
        workers_cleanup: 1,
        generation: 3,
        client_outcome: 1,
        retry: 1,
        leave: 2,
        attachment_detached: 0,
        connection_closed: 1,
        error: no_error(),
    });
    relay.send(&from_relay(1, ended)).expect("Stream ended");
}

/// The hub monitor grants no Credit of nothing, none that would take the
/// stream past 4,294,967,295 bytes, and none before the relay answers or
/// after its own Close stream. Each Event it receives frees its length to
/// grant again.
#[test]
fn lim_04_the_hub_monitor_grants_only_credit_a_stream_can_hold() {
    let (mut hub, _) = samples::linked_hub();
    hub.send(&from_hub(1, samples::open_stream()))
        .expect("Open");
    let early = hub.send(&from_hub(1, HubStream::Credit(1)));
    assert_eq!(early, Err(Refused::OutOfTurn), "before the answer");

    let (mut hub, mut stand_in) = hub_streaming();
    let grant = |amount| from_hub(1, HubStream::Credit(amount));
    let room = 4_294_967_295 - 1_048_576;
    assert_eq!(hub.send(&grant(0)), Err(Refused::Credit), "nothing");
    assert_eq!(hub.send(&grant(room + 1)), Err(Refused::Credit));
    hub.send(&grant(room))
        .expect("up to the most a stream holds");
    assert_eq!(hub.send(&grant(1)), Err(Refused::Credit), "one past it");

    let (_, event) = samples::events().remove(0);
    let unit = stand_in.unit(1, 0, &event);
    step(&mut hub, LATER, Input::Bytes(&unit));
    assert_eq!(hub.records().len(), 1, "the event");
    let length = event.len() as u32;
    assert_eq!(hub.send(&grant(length + 1)), Err(Refused::Credit));
    hub.send(&grant(length)).expect("the event's length");

    hub.send(&from_hub(1, HubStream::Close))
        .expect("Close stream");
    let late = hub.send(&grant(1));
    assert_eq!(late, Err(Refused::OutOfTurn), "after Close stream");
}
