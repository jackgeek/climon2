//! The opening exchange at the Link layer (#493): the frozen first message
//! each way on every connection, which picks the highest link revision both
//! ends share or refuses. The expected bytes are the examples and layouts of
//! the protocol's opening request and opening reply pages, written out here
//! by hand.

mod support;

use climon2_link::{
    End, HubEnd, Input, Opened, Opening, Output, Purpose, Randomness, RelayEnd, Revisions, Time,
};
use std::time::Duration;
use support::pairing::{invited, uninvited};
use support::{Harness, Seeded};

/// The opening request page's example: link, revision 1 only.
const REQUEST_LINK_1: [u8; 26] = [
    0x63, 0x32, 0x6c, 0x69, 0x6e, 0x6b, 0x0d, 0x0a, 0x02, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0,
];
/// The opening reply page's first example: revision 1 chosen.
const REPLY_CHOSEN_1: [u8; 20] = [
    0x01, 0x00, 0x01, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

fn revisions(list: &[u16]) -> Revisions {
    Revisions::new(list).expect("a valid revision list")
}

fn hub(purpose: Purpose, list: &[u16]) -> HubEnd {
    HubEnd::new(purpose, revisions(list))
}

/// A relay with a live link invitation, so that a pairing run can start.
fn relay(list: &[u16]) -> RelayEnd {
    RelayEnd::new(revisions(list), invited().0)
}

/// The bytes of a transcript an end holds once the opening chose a revision.
fn transcript(opening: &Opening) -> Vec<u8> {
    match opening {
        Opening::Opened(Opened { transcript, .. }) => transcript.as_bytes().to_vec(),
        other => panic!("the opening chose no revision: {other:?}"),
    }
}

/// REV-01: the request is the 26 bytes and the reply the 20 bytes the
/// protocol pages give, and each end holds both, in order, for the key
/// exchange that follows.
#[test]
fn rev_01_the_opening_is_the_26_and_20_bytes_the_protocol_gives() {
    let mut harness = Harness::new(hub(Purpose::Link, &[1]), relay(&[1]));

    harness.run();

    assert_eq!(harness.sent_by_hub(), REQUEST_LINK_1);
    assert_eq!(harness.sent_by_relay(), REPLY_CHOSEN_1);
    let both = [&REQUEST_LINK_1[..], &REPLY_CHOSEN_1[..]].concat();
    assert_eq!(both.len(), 46);
    assert_eq!(transcript(harness.hub.opening()), both);
    assert_eq!(transcript(harness.relay.opening()), both);
    for opening in [harness.hub.opening(), harness.relay.opening()] {
        let Opening::Opened(opened) = opening else {
            unreachable!()
        };
        assert_eq!((opened.purpose, opened.revision), (Purpose::Link, 1));
    }
    assert!(!harness.hub_closed() && !harness.relay_closed());
}

const SECOND: Duration = Duration::from_secs(1);
const MILLISECOND: Duration = Duration::from_millis(1);

/// A link peer stand-in: it speaks the bytes the test wrote, once, and
/// writes down what reaches it.
struct StandIn {
    says: Option<Vec<u8>>,
    /// Whether it speaks at its first step, as a dialler does, or when the
    /// first bytes reach it, as a listener does.
    dials: bool,
    received: Vec<u8>,
}

impl StandIn {
    fn dialler(says: &[u8]) -> Self {
        Self {
            says: Some(says.to_vec()),
            dials: true,
            received: Vec::new(),
        }
    }

    fn listener(says: &[u8]) -> Self {
        Self {
            dials: false,
            ..Self::dialler(says)
        }
    }
}

impl End for StandIn {
    fn step(&mut self, _: Time, input: Input<'_>, _: &mut dyn Randomness) -> Output {
        if let Input::Bytes(bytes) = input {
            self.received.extend(bytes);
        }
        let speaks = self.dials || !self.received.is_empty();
        Output {
            bytes: self.says.take_if(|_| speaks).unwrap_or_default(),
            ..Output::default()
        }
    }
}

/// An opening request with this purpose byte, count byte and slots.
fn request(purpose: u8, count: u8, slots: &[u16]) -> Vec<u8> {
    let mut bytes = b"c2link\r\n".to_vec();
    bytes.extend([purpose, count]);
    bytes.extend(list(slots));
    bytes
}

/// An opening reply with this result byte, chosen revision, count byte and
/// slots.
fn reply(result: u8, chosen: u16, count: u8, slots: &[u16]) -> Vec<u8> {
    let mut bytes = vec![result, (chosen >> 8) as u8, chosen as u8, count];
    bytes.extend(list(slots));
    bytes
}

/// Eight slots on the wire: `slots` first, then zeroes.
fn list(slots: &[u16]) -> Vec<u8> {
    let mut bytes = vec![0; 16];
    for (index, slot) in slots.iter().enumerate() {
        bytes[2 * index] = (slot >> 8) as u8;
        bytes[2 * index + 1] = *slot as u8;
    }
    bytes
}

/// Steps one end by hand at `at` after the start.
fn step(end: &mut impl End, at: Duration, input: Input<'_>) -> Output {
    end.step(Time::since_start(at), input, &mut Seeded::new(0))
}

/// An output that sends nothing, keeps the byte stream open and names no
/// deadline.
fn nothing() -> Output {
    Output {
        bytes: Vec::new(),
        close: false,
        deadline: None,
    }
}

/// An output that closes the byte stream without sending a byte.
fn silent_close() -> Output {
    Output {
        close: true,
        ..nothing()
    }
}

fn waiting_until(deadline: Duration) -> Output {
    Output {
        deadline: Some(Time::since_start(deadline)),
        ..nothing()
    }
}

/// The revision an opening chose.
fn chosen(opening: &Opening) -> u16 {
    match opening {
        Opening::Opened(opened) => opened.revision,
        other => panic!("the opening chose no revision: {other:?}"),
    }
}

/// The revision a hub monitor and a relay with these lists settle on.
fn negotiated(hub_list: &[u16], relay_list: &[u16]) -> u16 {
    let mut harness = Harness::new(hub(Purpose::Link, hub_list), relay(relay_list));
    harness.run();
    let revision = chosen(harness.hub.opening());
    assert_eq!(chosen(harness.relay.opening()), revision);
    revision
}

/// REV-01: the two ends use the highest link revision they share, whichever
/// end knows a higher one the other does not.
#[test]
fn rev_01_the_ends_use_the_highest_revision_they_share() {
    assert_eq!(negotiated(&[1, 9], &[1]), 1);
    assert_eq!(negotiated(&[1], &[1, 9]), 1);
    assert_eq!(negotiated(&[1, 2, 5], &[1, 2, 7]), 2);
    assert_eq!(negotiated(&[2, 5, 7], &[1, 5, 7, 9]), 7);
    assert_eq!(negotiated(&[3], &[1, 2, 3, 4, 5, 6, 7, 8]), 3);
}

/// REV-01, with stand-ins on each side: the bytes of both messages when one
/// end offers revision 1 and a higher one the other does not know.
#[test]
fn rev_01_a_higher_unknown_revision_is_listed_and_not_chosen() {
    // A stand-in hub monitor offering 1 and 0x0902 dials the relay.
    let stand_in = StandIn::dialler(&request(2, 2, &[1, 0x0902]));
    let mut harness = Harness::new(stand_in, relay(&[1]));
    harness.run();
    assert_eq!(harness.sent_by_relay(), REPLY_CHOSEN_1);
    assert_eq!(chosen(harness.relay.opening()), 1);

    // The hub monitor dials a stand-in relay that supports 1 and 0x0902.
    let stand_in = StandIn::listener(&reply(1, 1, 2, &[1, 0x0902]));
    let mut harness = Harness::new(hub(Purpose::Link, &[1]), stand_in);
    harness.run();
    assert_eq!(harness.sent_by_hub(), REQUEST_LINK_1);
    assert_eq!(chosen(harness.hub.opening()), 1);
    assert!(!harness.hub_closed());

    // The real ends write the same lists, high byte first.
    let mut harness = Harness::new(hub(Purpose::Link, &[1, 0x0902]), relay(&[1, 0x0902]));
    harness.run();
    assert_eq!(harness.sent_by_hub(), request(2, 2, &[1, 0x0902]));
    assert_eq!(harness.sent_by_relay(), reply(1, 0x0902, 2, &[1, 0x0902]));
}

/// REV-01: an end that supports eight revisions, the most a message holds,
/// fills every slot.
#[test]
fn rev_01_eight_revisions_fill_every_slot() {
    let eight = [1, 2, 3, 4, 5, 6, 7, 0xfffe];
    let mut harness = Harness::new(hub(Purpose::Pair, &eight), relay(&eight));

    harness.run();

    assert_eq!(harness.sent_by_hub(), request(1, 8, &eight));
    assert_eq!(harness.sent_by_relay(), reply(1, 0xfffe, 8, &eight));
    let Opening::Opened(opened) = harness.relay.opening() else {
        panic!("the relay chose no revision")
    };
    assert_eq!(opened.purpose, Purpose::Pair);
    let Opening::Opened(opened) = harness.hub.opening() else {
        panic!("the hub monitor has no revision")
    };
    assert_eq!(opened.purpose, Purpose::Pair);
}

/// REV-01: every connection negotiates from scratch. The same relay settings
/// give each connection the revision its own request allows, whatever an
/// earlier connection chose, and the same connection gives the same bytes.
#[test]
fn rev_01_every_connection_negotiates_from_scratch() {
    let connect = |hub_list: &[u16]| {
        let mut harness = Harness::new(hub(Purpose::Link, hub_list), relay(&[1, 2]));
        harness.run();
        (
            chosen(harness.hub.opening()),
            harness.sent_by_hub().to_vec(),
            harness.sent_by_relay().to_vec(),
        )
    };

    let first = connect(&[1, 2]);
    let older = connect(&[1]);
    let again = connect(&[1, 2]);

    assert_eq!((first.0, older.0, again.0), (2, 1, 2));
    assert_eq!(first, again);
}

/// A byte stream keeps no message boundaries: the opening exchange works
/// when each message arrives one byte at a time.
#[test]
fn the_opening_survives_arriving_one_byte_at_a_time() {
    let mut harness = Harness::new(hub(Purpose::Link, &[1]), relay(&[1]));
    harness.fragment(1);

    harness.run();

    assert_eq!(harness.sent_by_relay(), REPLY_CHOSEN_1);
    assert_eq!(chosen(harness.hub.opening()), 1);
    assert_eq!(chosen(harness.relay.opening()), 1);
}

/// Until the exchange ends, each end says its opening is pending.
#[test]
fn an_opening_is_pending_until_it_ends() {
    let mut hub = hub(Purpose::Link, &[1]);
    let mut relay = relay(&[1]);
    assert_eq!(hub.opening(), &Opening::Pending);
    assert_eq!(relay.opening(), &Opening::Pending);

    step(&mut hub, Duration::ZERO, Input::Tick);
    step(
        &mut relay,
        Duration::ZERO,
        Input::Bytes(&REQUEST_LINK_1[..25]),
    );

    assert_eq!(hub.opening(), &Opening::Pending);
    assert_eq!(relay.opening(), &Opening::Pending);
}

/// With no shared link revision the relay answers with the refusal the
/// protocol page gives and closes; each end then holds the other's list.
#[test]
fn no_shared_revision_is_refused_with_both_lists_known() {
    // The opening reply page's second example.
    let refusal = [
        0x02, 0x00, 0x00, 0x02, 0x00, 0x02, 0x00, 0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    for purpose in [Purpose::Link, Purpose::Pair] {
        let listener = RelayEnd::new(revisions(&[2, 3]), uninvited());
        let mut harness = Harness::new(hub(purpose, &[1]), listener);

        harness.run();

        assert_eq!(harness.sent_by_relay(), refusal);
        assert!(harness.relay_closed() && harness.hub_closed());
        let theirs = revisions(&[2, 3]);
        assert_eq!(harness.hub.opening(), &Opening::Incompatible { theirs });
        let theirs = revisions(&[1]);
        assert_eq!(harness.relay.opening(), &Opening::Incompatible { theirs });
        assert_eq!(harness.sent_by_hub().len(), 26);
    }
}

/// A hub monitor that comes to pair when the relay can start no pairing run
/// gets result 3, and the relay closes. A hub monitor that comes to link is
/// not refused for it.
#[test]
fn pairing_is_refused_when_the_relay_can_start_no_run() {
    let listener = || RelayEnd::new(revisions(&[1]), uninvited());

    let mut harness = Harness::new(hub(Purpose::Pair, &[1]), listener());
    harness.run();
    assert_eq!(harness.sent_by_hub(), request(1, 1, &[1]));
    assert_eq!(harness.sent_by_relay(), reply(3, 0, 1, &[1]));
    assert!(harness.relay_closed() && harness.hub_closed());
    assert_eq!(harness.hub.opening(), &Opening::PairingUnavailable);
    assert_eq!(harness.relay.opening(), &Opening::PairingUnavailable);

    let mut harness = Harness::new(hub(Purpose::Link, &[1]), listener());
    harness.run();
    assert_eq!(harness.sent_by_relay(), REPLY_CHOSEN_1);
    assert!(!harness.relay_closed() && !harness.hub_closed());
}

/// What a relay does with these bytes as the whole of what a dialler sends.
fn relay_given(bytes: &[u8]) -> (Vec<u8>, bool, Opening) {
    let mut harness = Harness::new(StandIn::dialler(bytes), relay(&[1, 2]));
    harness.run();
    (
        harness.sent_by_relay().to_vec(),
        harness.relay_closed(),
        harness.relay.opening().clone(),
    )
}

/// SEC-04, the opening half: a request that breaks the layout in any way is
/// closed without a byte in reply.
#[test]
fn sec_04_a_malformed_opening_request_is_closed_without_a_byte() {
    let mut malformed = vec![
        ("purpose 0", request(0, 1, &[1])),
        ("purpose 3", request(3, 1, &[1])),
        ("count 0", request(2, 0, &[])),
        ("count 0 with a revision", request(2, 0, &[1])),
        ("count 9", request(2, 9, &[1, 2, 3, 4, 5, 6, 7, 8])),
        ("count 255", request(2, 255, &[1, 2, 3, 4, 5, 6, 7, 8])),
        ("revision zero", request(2, 1, &[0])),
        ("revision zero first", request(2, 2, &[0, 1])),
        ("a repeat", request(2, 2, &[1, 1])),
        ("descending", request(2, 2, &[2, 1])),
        ("descending later", request(2, 3, &[1, 3, 2])),
        ("a count short of the list", request(2, 1, &[1, 2])),
        (
            "a non-zero last slot",
            request(2, 1, &[1, 0, 0, 0, 0, 0, 0, 2]),
        ),
        ("a count past the list", request(2, 2, &[1])),
    ];
    for index in 0..8 {
        let mut bytes = REQUEST_LINK_1.to_vec();
        bytes[index] ^= 0x20;
        malformed.push(("a wrong marker", bytes));
    }
    malformed.push((
        "a web request",
        b"GET / HTTP/1.1\r\nHost: relay\r\n".to_vec(),
    ));

    for (what, bytes) in malformed {
        assert!(bytes.len() >= 26, "{what}");
        let outcome = relay_given(&bytes);
        assert_eq!(outcome, (Vec::new(), true, Opening::Failed), "{what}");
    }
    // The same relay answers the well-formed request.
    assert_eq!(relay_given(&REQUEST_LINK_1).0.len(), 20);
}

/// SEC-04, the opening half: a partial request is closed without a byte at
/// 10 seconds from the accept and not before, and the relay names that
/// deadline to its caller.
#[test]
fn sec_04_a_partial_opening_request_is_closed_at_ten_seconds() {
    for partial in [&REQUEST_LINK_1[..0], &REQUEST_LINK_1[..25]] {
        let mut harness = Harness::new(StandIn::dialler(partial), relay(&[1]));

        harness.run();
        assert_eq!(
            harness.relay_deadline(),
            Some(Time::since_start(10 * SECOND))
        );
        harness.advance(10 * SECOND - MILLISECOND);
        assert!(!harness.relay_closed());
        assert_eq!(harness.relay.opening(), &Opening::Pending);
        harness.advance(MILLISECOND);

        assert!(harness.relay_closed());
        assert_eq!(harness.sent_by_relay(), b"");
        assert_eq!(harness.relay.opening(), &Opening::Failed);
    }
}

/// SEC-04, the opening half: the 10 seconds run from the accept, which is
/// the relay's first step. A request whose last byte arrives exactly then is
/// in time; one that arrives later is late and gets no byte.
#[test]
fn sec_04_a_late_opening_request_is_closed_without_a_byte() {
    let accepted_at = 5 * SECOND;
    let deadline = 15 * SECOND;
    let accepted = || {
        let mut relay = relay(&[1]);
        let first = step(&mut relay, accepted_at, Input::Tick);
        assert_eq!(first, waiting_until(deadline));
        let part = step(&mut relay, 9 * SECOND, Input::Bytes(&REQUEST_LINK_1[..25]));
        assert_eq!(part, waiting_until(deadline));
        relay
    };
    let last = Input::Bytes(&REQUEST_LINK_1[25..]);

    let mut in_time = accepted();
    let output = step(&mut in_time, deadline, last);
    assert_eq!(output.bytes, REPLY_CHOSEN_1);
    assert_eq!((output.close, output.deadline), (false, None));

    let mut late = accepted();
    let output = step(&mut late, deadline + Duration::from_nanos(1), last);
    assert_eq!(output, silent_close());
    assert_eq!(late.opening(), &Opening::Failed);

    // A whole request that arrives late, with no step at the deadline.
    let mut late = relay(&[1]);
    step(&mut late, Duration::ZERO, Input::Tick);
    let output = step(&mut late, 11 * SECOND, Input::Bytes(&REQUEST_LINK_1));
    assert_eq!(output, silent_close());
}

/// SEC-04, the opening half: a dialler that closes before its request is
/// whole gets the close back and no byte.
#[test]
fn sec_04_a_dialler_that_hangs_up_mid_request_gets_no_byte() {
    let mut relay = relay(&[1]);
    step(&mut relay, Duration::ZERO, Input::Tick);
    step(&mut relay, SECOND, Input::Bytes(&REQUEST_LINK_1[..10]));

    let output = step(&mut relay, 2 * SECOND, Input::Closed);

    assert_eq!(output, silent_close());
    assert_eq!(relay.opening(), &Opening::Failed);
}

/// A relay whose first step already carries the request, with no tick
/// before it, accepts the connection at that step.
#[test]
fn a_relay_first_stepped_with_the_request_answers_it() {
    let mut relay = relay(&[1]);

    let output = step(&mut relay, 20 * SECOND, Input::Bytes(&REQUEST_LINK_1));

    assert_eq!(output.bytes, REPLY_CHOSEN_1);
}

/// Neither end sends anything behind its opening message until the other
/// end has answered it: the hub monitor's next message needs the reply, and
/// the relay's needs the hub monitor's. Bytes that arrive with a whole
/// opening message, behind it, therefore make it malformed: the relay closes
/// without a byte in reply, and for the hub monitor it is a failed dial.
#[test]
fn bytes_behind_an_opening_message_make_it_malformed() {
    for purpose in [Purpose::Link, Purpose::Pair] {
        let mut request = REQUEST_LINK_1;
        request[8] = if purpose == Purpose::Pair { 0x01 } else { 0x02 };
        let mut relay = relay(&[1]);
        let crowded = [&request[..], b"m"].concat();
        let output = step(&mut relay, Duration::ZERO, Input::Bytes(&crowded));
        assert_eq!(output, silent_close(), "{purpose:?}");
        assert_eq!(relay.opening(), &Opening::Failed, "{purpose:?}");

        let mut hub = hub(purpose, &[1]);
        step(&mut hub, Duration::ZERO, Input::Tick);
        let crowded = [&REPLY_CHOSEN_1[..], b"m"].concat();
        let output = step(&mut hub, Duration::ZERO, Input::Bytes(&crowded));
        assert_eq!(output, silent_close(), "{purpose:?}");
        assert_eq!(hub.opening(), &Opening::Failed, "{purpose:?}");
    }
}

/// Bytes that arrive after a whole opening message, in a later arrival, are
/// not part of it: the exchange ends as it would without them.
#[test]
fn bytes_after_an_opening_message_are_not_part_of_it() {
    let mut relay = relay(&[1]);
    let output = step(&mut relay, Duration::ZERO, Input::Bytes(&REQUEST_LINK_1));
    assert_eq!(output.bytes, REPLY_CHOSEN_1);
    for input in [Input::Bytes(b"more"), Input::Tick, Input::Closed] {
        assert_eq!(step(&mut relay, 60 * SECOND, input), nothing());
    }
    assert_eq!(chosen(relay.opening()), 1);

    let mut hub = hub(Purpose::Link, &[1]);
    step(&mut hub, Duration::ZERO, Input::Tick);
    let output = step(&mut hub, Duration::ZERO, Input::Bytes(&REPLY_CHOSEN_1));
    assert_eq!(output, nothing());
    for input in [Input::Bytes(b"more"), Input::Tick, Input::Closed] {
        assert_eq!(step(&mut hub, 60 * SECOND, input), nothing());
    }
    assert_eq!(chosen(hub.opening()), 1);
}

/// What a hub monitor that supports revisions 1 and 2 makes of these bytes
/// as the whole of what a relay answers.
fn hub_given(purpose: Purpose, bytes: &[u8]) -> (Opening, bool, usize) {
    let mut harness = Harness::new(hub(purpose, &[1, 2]), StandIn::listener(bytes));
    harness.run();
    (
        harness.hub.opening().clone(),
        harness.hub_closed(),
        harness.sent_by_hub().len(),
    )
}

/// A reply that breaks the layout, names a revision the hub monitor did not
/// offer, or is not what a relay with the list it states would answer, is a
/// failed dial: the hub monitor closes and sends nothing more.
#[test]
fn a_malformed_opening_reply_is_a_failed_dial() {
    let malformed = [
        ("result 0", reply(0, 0, 1, &[1])),
        ("result 4", reply(4, 0, 1, &[1])),
        ("a revision not offered", reply(1, 3, 3, &[1, 2, 3])),
        ("a chosen revision of zero", reply(1, 0, 1, &[1])),
        (
            "a revision below the highest shared",
            reply(1, 1, 2, &[1, 2]),
        ),
        ("a revision the relay does not list", reply(1, 2, 1, &[1])),
        ("a refusal that names a revision", reply(2, 1, 1, &[3])),
        (
            "a refusal although a revision is shared",
            reply(2, 0, 2, &[2, 3]),
        ),
        (
            "pairing unavailable naming a revision",
            reply(3, 1, 1, &[1]),
        ),
        ("count 0", reply(1, 1, 0, &[])),
        ("count 9", reply(1, 1, 9, &[1, 2, 3, 4, 5, 6, 7, 8])),
        ("revision zero", reply(1, 1, 2, &[0, 1])),
        ("a repeat", reply(1, 1, 2, &[1, 1])),
        ("descending", reply(1, 2, 2, &[2, 1])),
        ("a non-zero unused slot", reply(1, 1, 1, &[1, 0, 2])),
    ];
    for purpose in [Purpose::Link, Purpose::Pair] {
        for (what, bytes) in &malformed {
            assert_eq!(bytes.len(), 20, "{what}");
            let outcome = hub_given(purpose, bytes);
            assert_eq!(outcome, (Opening::Failed, true, 26), "{what}");
        }
    }
    // Pairing unavailable answers only a hub monitor that came to pair.
    let unavailable = reply(3, 0, 1, &[1]);
    let outcome = hub_given(Purpose::Link, &unavailable);
    assert_eq!(outcome, (Opening::Failed, true, 26));
    let outcome = hub_given(Purpose::Pair, &unavailable);
    assert_eq!(outcome, (Opening::PairingUnavailable, true, 26));
    // The same hub monitor takes the well-formed replies.
    let (opening, closed, _) = hub_given(Purpose::Link, &reply(1, 2, 3, &[2, 3, 4]));
    assert_eq!((chosen(&opening), closed), (2, false));
    let theirs = revisions(&[3, 4]);
    let outcome = hub_given(Purpose::Link, &reply(2, 0, 2, &[3, 4]));
    assert_eq!(outcome, (Opening::Incompatible { theirs }, true, 26));
}

/// The hub monitor waits 10 seconds from sending its request for the whole
/// reply, names that deadline to its caller, and then treats the dial as
/// failed.
#[test]
fn a_partial_opening_reply_is_a_failed_dial_at_ten_seconds() {
    for partial in [&REPLY_CHOSEN_1[..0], &REPLY_CHOSEN_1[..19]] {
        let mut harness = Harness::new(hub(Purpose::Link, &[1]), StandIn::listener(partial));

        harness.run();
        assert_eq!(harness.hub_deadline(), Some(Time::since_start(10 * SECOND)));
        harness.advance(10 * SECOND - MILLISECOND);
        assert!(!harness.hub_closed());
        assert_eq!(harness.hub.opening(), &Opening::Pending);
        harness.advance(MILLISECOND);

        assert!(harness.hub_closed());
        assert_eq!(harness.hub.opening(), &Opening::Failed);
        assert_eq!(harness.sent_by_hub(), REQUEST_LINK_1);
    }
}

/// The 10 seconds run from the step that sends the request. A reply whose
/// last byte arrives exactly then is in time; a later one is a failed dial,
/// and so is a relay that closes first.
#[test]
fn a_late_or_cut_off_opening_reply_is_a_failed_dial() {
    let sent_at = 5 * SECOND;
    let deadline = 15 * SECOND;
    let dialled = || {
        let mut hub = hub(Purpose::Link, &[1]);
        let first = step(&mut hub, sent_at, Input::Tick);
        assert_eq!(first.bytes, REQUEST_LINK_1);
        assert_eq!(
            (first.close, first.deadline),
            (false, Some(Time::since_start(deadline)))
        );
        let part = step(&mut hub, 9 * SECOND, Input::Bytes(&REPLY_CHOSEN_1[..19]));
        assert_eq!(part, waiting_until(deadline));
        hub
    };
    let last = Input::Bytes(&REPLY_CHOSEN_1[19..]);

    let mut in_time = dialled();
    assert_eq!(step(&mut in_time, deadline, last), nothing());
    assert_eq!(chosen(in_time.opening()), 1);

    let mut late = dialled();
    let output = step(&mut late, deadline + Duration::from_nanos(1), last);
    assert_eq!(output, silent_close());
    assert_eq!(late.opening(), &Opening::Failed);

    let mut cut_off = dialled();
    assert_eq!(
        step(&mut cut_off, 10 * SECOND, Input::Closed),
        silent_close()
    );
    assert_eq!(cut_off.opening(), &Opening::Failed);
}

/// A revision list is one to eight revisions, ascending, with no zero and
/// no repeat; nothing else can be offered.
#[test]
fn a_revision_list_is_one_to_eight_ascending_revisions() {
    assert_eq!(revisions(&[1]).as_slice(), [1]);
    assert_eq!(
        revisions(&[1, 2, 3, 4, 5, 6, 7, 8]).as_slice(),
        [1, 2, 3, 4, 5, 6, 7, 8]
    );
    assert_eq!(revisions(&[7, 300, 65535]).as_slice(), [7, 300, 65535]);
    for bad in [
        &[][..],
        &[0],
        &[0, 1],
        &[1, 1],
        &[2, 1],
        &[1, 3, 2],
        &[1, 2, 3, 4, 5, 6, 7, 8, 9],
    ] {
        assert_eq!(Revisions::new(bad), None, "{bad:?}");
    }
}

/// One connection with the on-path proxy between the ends: it may edit each
/// opening message in flight. Gives back both ends and every byte each sent.
fn through_proxy(
    mut hub: HubEnd,
    mut relay: RelayEnd,
    edit_request: impl Fn(&mut Vec<u8>),
    edit_reply: impl Fn(&mut Vec<u8>),
) -> (HubEnd, RelayEnd, Vec<u8>, Vec<u8>) {
    let start = Duration::ZERO;
    let mut from_hub = step(&mut hub, start, Input::Tick);
    step(&mut relay, start, Input::Tick);
    let mut request = from_hub.bytes.clone();
    edit_request(&mut request);
    let mut from_relay = step(&mut relay, start, Input::Bytes(&request));
    let mut answer = from_relay.bytes.clone();
    edit_reply(&mut answer);
    let after = step(&mut hub, start, Input::Bytes(&answer));
    from_hub.bytes.extend(after.bytes);
    // Whatever each end does next when the other closes, and at the end of
    // every deadline.
    if from_relay.close && !after.close {
        from_hub
            .bytes
            .extend(step(&mut hub, start, Input::Closed).bytes);
    }
    if after.close && !from_relay.close {
        from_relay
            .bytes
            .extend(step(&mut relay, start, Input::Closed).bytes);
    }
    if !from_relay.close {
        from_relay
            .bytes
            .extend(step(&mut relay, 60 * SECOND, Input::Tick).bytes);
    }
    (hub, relay, from_hub.bytes, from_relay.bytes)
}

/// Whether both ends chose a revision and hold the same transcript: the
/// only state in which the key exchange that follows can succeed.
fn agreed(hub: &HubEnd, relay: &RelayEnd) -> bool {
    match (hub.opening(), relay.opening()) {
        (Opening::Opened(ours), Opening::Opened(theirs)) => ours.transcript == theirs.transcript,
        _ => false,
    }
}

/// Removes revision 2 from a list of 1 and 2 that starts at `count`.
fn strip_revision_2(bytes: &mut [u8], count: usize) {
    assert_eq!(bytes[count..count + 5], [2, 0, 1, 0, 2]);
    bytes[count] = 1;
    bytes[count + 4] = 0;
}

/// REV-03: the on-path proxy edits the revision lists to force the older
/// revision, on a link and on a pairing connection. The ends are left with
/// different transcripts, which the key exchange that follows takes as
/// input, or the dial fails. Neither end tries again with another revision:
/// each sends its one opening message and nothing more.
#[test]
fn rev_03_edited_revision_lists_leave_no_agreed_opening() {
    for purpose in [Purpose::Link, Purpose::Pair] {
        let ends = || (hub(purpose, &[1, 2]), relay(&[1, 2]));
        let untouched = |_: &mut Vec<u8>| {};
        let strip_request = |bytes: &mut Vec<u8>| strip_revision_2(bytes, 9);
        let strip_reply = |bytes: &mut Vec<u8>| strip_revision_2(bytes, 3);

        // Without the proxy's edits the ends agree on revision 2.
        let (hub, relay) = ends();
        let (hub, relay, ..) = through_proxy(hub, relay, untouched, untouched);
        assert!(agreed(&hub, &relay));
        assert_eq!(chosen(hub.opening()), 2);

        // The request loses revision 2: the relay chooses 1, and the hub
        // monitor sees a relay that lists 2 and did not choose it.
        let (hub, relay) = ends();
        let (hub, relay, hub_sent, relay_sent) =
            through_proxy(hub, relay, strip_request, untouched);
        assert_eq!(hub.opening(), &Opening::Failed);
        assert_eq!(chosen(relay.opening()), 1);
        assert!(!agreed(&hub, &relay));
        assert_eq!((hub_sent.len(), relay_sent.len()), (26, 20));

        // Both messages lose revision 2, so each end sees a consistent
        // exchange that chose revision 1. Their transcripts differ: each
        // holds its own message as it sent it.
        let (hub, relay) = ends();
        let (hub, relay, hub_sent, relay_sent) =
            through_proxy(hub, relay, strip_request, strip_reply);
        assert_eq!((chosen(hub.opening()), chosen(relay.opening())), (1, 1));
        assert!(!agreed(&hub, &relay));
        assert_eq!(transcript(hub.opening())[..26], hub_sent);
        assert_eq!(transcript(relay.opening())[26..], relay_sent);
        assert_ne!(
            transcript(hub.opening())[..26],
            transcript(relay.opening())[..26]
        );
        assert_ne!(
            transcript(hub.opening())[26..],
            transcript(relay.opening())[26..]
        );
        assert_eq!((hub_sent.len(), relay_sent.len()), (26, 20));

        // Only the reply's list is edited.
        let (hub, relay) = ends();
        let add_revision_3 = |bytes: &mut Vec<u8>| (bytes[3], bytes[9]) = (3, 3);
        let (hub, relay, hub_sent, relay_sent) =
            through_proxy(hub, relay, untouched, add_revision_3);
        assert_eq!((chosen(hub.opening()), chosen(relay.opening())), (2, 2));
        assert!(!agreed(&hub, &relay));
        assert_eq!((hub_sent.len(), relay_sent.len()), (26, 20));
    }
}

/// REV-03 and PAIR-06's opening half: whatever single byte of either opening
/// message the on-path proxy changes, to whatever value, the two ends never
/// hold the same transcript, and neither sends a second opening message.
#[test]
fn rev_03_no_edit_of_one_byte_leaves_an_agreed_opening() {
    for purpose in [Purpose::Link, Purpose::Pair] {
        for index in 0..46 {
            for mask in 1..=255 {
                let edit = |at: Option<usize>| {
                    move |bytes: &mut Vec<u8>| {
                        if let Some(byte) = at.and_then(|at| bytes.get_mut(at)) {
                            *byte ^= mask;
                        }
                    }
                };
                let (hub, relay, hub_sent, relay_sent) = through_proxy(
                    hub(purpose, &[1, 2, 4]),
                    relay(&[1, 2, 3]),
                    edit((index < 26).then_some(index)),
                    edit(index.checked_sub(26)),
                );
                assert!(!agreed(&hub, &relay), "byte {index} xor {mask}");
                assert_eq!(hub_sent.len(), 26, "byte {index} xor {mask}");
                assert!(
                    [0, 20].contains(&relay_sent.len()),
                    "byte {index} xor {mask}"
                );
            }
        }
    }
}
