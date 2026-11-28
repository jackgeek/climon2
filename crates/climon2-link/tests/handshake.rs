//! The pinned-key handshake at the Link layer (#494): Noise `IK` bound to
//! the opening exchange, then Hello each way. The expected bytes come from
//! the protocol pages and from climon2's frozen vector, which an
//! independent implementation of the Noise pattern computed
//! (`vectors/noise_ik.py`), not the crate under test.

mod support;
mod wire;

use climon2_link::{
    End, Ended, HubEnd, Input, Link, Opening, PairingRun, Peer, Randomness, RelayEnd, Revisions,
    StaticKey, Time,
};
use std::time::Duration;
use support::pairing::{invited, uninvited};
use support::{Harness, Seeded};
use wire::{
    Fixed, HANDSHAKE_REPLY, INITIATION, OnPath, REPLY, REQUEST, SECOND, StandIn, Way, hello, hex,
    hub_end, hub_key, paired, relay_end, relay_key, revision_1, step,
};

/// climon2's frozen vector: the hub monitor's static private key is 32
/// bytes of `0x11` and its ephemeral private key the bytes `0x40` to
/// `0x5f`; the relay's identity private key is 32 bytes of `0x22` and its
/// ephemeral private key the bytes `0x60` to `0x7f`. The prologue is the
/// opening exchange of the protocol pages' examples, and the Hellos are the
/// Hello page's example and `2.5.1`, `forge`, 4,096.
const HUB_PUBLIC: &str = "7b4e909bbe7ffe44c465a220037d608ee35897d31ef972f07f74892cb0f73f13";
const RELAY_PUBLIC: &str = "0faa684ed28867b97f4a6a2dee5df8ce974e76b7018e3f22a1c4cf2678570f20";
const FROZEN_INITIATION: &str = "0060
    79a631eede1bf9c98f12032cdeadd0e7a079398fc786b88cc846ec89af85a51a
    7536e8c1c5cf4b1269f1abdaa6fe74555b166c90b6048f0df7c788d9f6d261e7
    ed4f93dc50c8bd2ed307a78e1c3b8cd8fc8b9a23d268fcc725acf0b695fe7d29";
const FROZEN_REPLY: &str = "0030
    675dd574ed7789310b3d2e7681f3790b466c773b1521fecf36577958371ea52f
    01c346d64d43de31bbfdf4884b7f2414";
const FROZEN_HUB_HELLO: &str = "0030
    4b9941a3d6b7fea4e0b57423f85ec4c368511337274a7903f48468196c9bd02d
    26a2e8657f2e58b7cf4fea2657048bb6";
const FROZEN_RELAY_HELLO: &str = "0030
    732fc751f438812b4e770092722580afa9451cb18a240c830d7d6155666e1c77
    89a751ce5af1634eece83412a87a750d";

fn fixed(from: u8) -> Fixed {
    Fixed::new(&(from..from + 32).collect::<Vec<_>>())
}

/// SEC-08: a link connection run with the frozen vector's keys and
/// randomness puts exactly the vector's bytes on the wire, so replacing or
/// upgrading the cryptographic crate cannot change them unnoticed. The
/// prologue is the opening transcript, so the vector also freezes how the
/// opening exchange is bound into the handshake.
#[test]
fn sec_08_a_link_connection_gives_climon2s_frozen_bytes() {
    let (mut hub, mut relay) = paired();
    let (mut hub_randomness, mut relay_randomness) = (fixed(0x40), fixed(0x60));
    let now = Time::since_start(Duration::ZERO);
    let mut hub_step = |input| hub.step(now, input, &mut hub_randomness);
    let mut relay_step = |input| relay.step(now, input, &mut relay_randomness);

    assert_eq!(hub_key().public().as_bytes()[..], hex(HUB_PUBLIC));
    assert_eq!(relay_key().public().as_bytes()[..], hex(RELAY_PUBLIC));
    assert_eq!(hub_step(Input::Tick).bytes, REQUEST);
    relay_step(Input::Tick);
    assert_eq!(relay_step(Input::Bytes(&REQUEST)).bytes, REPLY);
    let initiation = hub_step(Input::Bytes(&REPLY)).bytes;
    assert_eq!(initiation, hex(FROZEN_INITIATION));
    let reply = relay_step(Input::Bytes(&initiation)).bytes;
    assert_eq!(reply, hex(FROZEN_REPLY));
    let hub_hello = hub_step(Input::Bytes(&reply)).bytes;
    assert_eq!(hub_hello, hex(FROZEN_HUB_HELLO));
    let relay_hello = relay_step(Input::Bytes(&hub_hello)).bytes;
    assert_eq!(relay_hello, hex(FROZEN_RELAY_HELLO));
    let last = hub_step(Input::Bytes(&relay_hello));
    assert!(last.bytes.is_empty() && !last.close);

    assert_eq!(
        hub.link(),
        &Link::Established(Peer {
            key: relay_key().public(),
            hello: wire::relay_hello(),
        })
    );
    assert_eq!(
        relay.link(),
        &Link::Established(Peer {
            key: hub_key().public(),
            hello: hello(),
        })
    );
}

/// The two ends of one remembered pairing, joined through an on-path proxy
/// that changes nothing until a test gives it an edit, and linked.
fn linked() -> Harness<HubEnd, OnPath<RelayEnd>> {
    let (hub, relay) = paired();
    let mut harness = Harness::new(hub, OnPath::new(relay));
    harness.run();
    assert!(matches!(harness.hub.link(), Link::Established(_)));
    assert!(matches!(harness.relay.end.link(), Link::Established(_)));
    harness
}

/// The handshake messages have the layouts of the handshake pages: a
/// length of 96 and of 48, then that many bytes. The Hello that follows is
/// one unit: the Hello page's 27-byte example behind the 5 bytes of stream
/// and flags, and a 16-byte tag.
#[test]
fn the_handshake_messages_have_the_published_layout() {
    let harness = linked();

    let from_hub = &harness.sent_by_hub()[REQUEST.len()..];
    assert_eq!(from_hub[..2], [0x00, 0x60]);
    assert_eq!(from_hub.len(), INITIATION + 2 + 5 + 27 + 16);
    assert_eq!(from_hub[INITIATION..INITIATION + 2], [0x00, 0x30]);
    let from_relay = &harness.sent_by_relay()[REPLY.len()..];
    assert_eq!(from_relay[..2], [0x00, 0x30]);
    assert_eq!(
        from_relay[HANDSHAKE_REPLY..HANDSHAKE_REPLY + 2],
        [0x00, 0x30]
    );
    assert!(!harness.hub_closed() && !harness.relay_closed());
}

/// Every run of the 16 bytes in `bytes`.
fn windows(bytes: &[u8]) -> Vec<&[u8]> {
    bytes.windows(16).collect()
}

/// SEC-01: two connections of the same remembered pairing each draw a
/// fresh ephemeral key from the caller's randomness, so they share no
/// session key and no ciphertext, even of the same Hello.
#[test]
fn sec_01_each_connection_uses_fresh_ephemeral_keys() {
    let connection = |hub_seed: u64, relay_seed: u64| {
        let (hub, relay) = paired();
        let mut harness = Harness::with_seeds(hub, relay, hub_seed, relay_seed);
        harness.run();
        assert!(matches!(harness.hub.link(), Link::Established(_)));
        assert!(matches!(harness.relay.link(), Link::Established(_)));
        let mut drawn = [0; 32];
        Seeded::new(hub_seed).fill(&mut drawn);
        let ephemeral = StaticKey::from_private(drawn).public();
        let sent = harness.sent_by_hub()[REQUEST.len()..].to_vec();
        assert_eq!(sent[2..34], ephemeral.as_bytes()[..], "drawn, not made up");
        let received = harness.sent_by_relay()[REPLY.len()..].to_vec();
        [sent, received].concat()
    };
    let first = connection(1, 2);
    let second = connection(3, 4);

    let shared: Vec<_> = windows(&first)
        .into_iter()
        .filter(|window| windows(&second).contains(window))
        .collect();
    assert!(shared.is_empty(), "shared ciphertext: {shared:02x?}");
}

/// SEC-01: each end of an established link knows the other by the static
/// key it pinned, and records cross both ways on any stream.
#[test]
fn sec_01_an_established_link_carries_records_both_ways() {
    let mut harness = linked();

    harness.hub.send(1, 0x41, b"to the relay").expect("sent");
    harness.hub.send(0, 0x7f, b"").expect("sent");
    harness
        .relay
        .end
        .send(1, 0x81, b"to the hub")
        .expect("sent");
    harness.run();

    let record = |stream, tag, body: &[u8]| climon2_link::Record {
        stream,
        tag,
        body: body.to_vec(),
    };
    assert_eq!(
        harness.relay.end.records(),
        [record(1, 0x41, b"to the relay"), record(0, 0x7f, b"")]
    );
    assert_eq!(harness.hub.records(), [record(1, 0x81, b"to the hub")]);
    assert!(harness.relay.end.records().is_empty(), "taken once");
    assert!(harness.hub.records().is_empty(), "taken once");
}

/// A link connection survives a byte stream that hands its bytes over one
/// at a time: no handshake message or unit depends on arriving whole.
#[test]
fn a_link_is_established_one_byte_at_a_time() {
    let (hub, relay) = paired();
    let mut harness = Harness::new(hub, relay);
    harness.fragment(1);
    harness.run();

    assert!(matches!(harness.hub.link(), Link::Established(_)));
    assert!(matches!(harness.relay.link(), Link::Established(_)));
}

fn revisions(list: &[u16]) -> Revisions {
    Revisions::new(list).expect("a valid revision list")
}

/// REV-03: the proxy edits a revision list so that each end still accepts
/// the opening exchange it saw. The two transcripts then differ, the relay
/// cannot decrypt the handshake initiation, and it closes without a byte;
/// the hub monitor's dial fails. Neither end tries another revision.
#[test]
fn rev_03_an_edited_revision_list_fails_the_key_exchange() {
    // The hub monitor supports revision 1 and the relay 1 and 2. The proxy
    // adds revision 3 to the request, or revision 3 to the reply.
    let add_to_request = |way: Way, mut bytes: Vec<u8>| {
        if way == Way::ToRelay && bytes.len() == REQUEST.len() {
            bytes[9] = 2;
            bytes[12..14].copy_from_slice(&[0x00, 0x03]);
        }
        bytes
    };
    let add_to_reply = |way: Way, mut bytes: Vec<u8>| {
        if way == Way::ToHub && bytes.len() == REPLY.len() {
            bytes[3] = 3;
            bytes[8..10].copy_from_slice(&[0x00, 0x03]);
        }
        bytes
    };
    let edits: [(&str, wire::Edit); 2] = [
        ("request", Box::new(add_to_request)),
        ("reply", Box::new(add_to_reply)),
    ];
    for (edited, edit) in edits {
        let relay = RelayEnd::new(revisions(&[1, 2]), uninvited()).linking(
            relay_key(),
            vec![hub_key().public()],
            wire::relay_hello(),
        );
        let mut on_path = OnPath::new(relay);
        on_path.edit = edit;
        let mut harness = Harness::new(hub_end(hub_key(), relay_key().public()), on_path);
        harness.run();

        let opened = |opening: &Opening| matches!(opening, Opening::Opened(o) if o.revision == 1);
        assert!(opened(harness.hub.opening()), "{edited}");
        assert!(opened(harness.relay.end.opening()), "{edited}");
        assert_eq!(
            harness.relay.end.link(),
            &Link::Ended(Ended::Failed),
            "{edited}"
        );
        assert_eq!(harness.hub.link(), &Link::Ended(Ended::Failed), "{edited}");
        assert_eq!(harness.sent_by_relay().len(), REPLY.len(), "{edited}");
        assert_eq!(
            harness.sent_by_hub().len(),
            REQUEST.len() + INITIATION,
            "{edited}"
        );
        assert!(harness.relay_closed() && harness.hub_closed(), "{edited}");
    }
}

/// The control: the same ends, with no edit, link.
#[test]
fn ends_with_different_revision_lists_link_on_the_highest_shared() {
    let relay = RelayEnd::new(revisions(&[1, 2]), uninvited()).linking(
        relay_key(),
        vec![hub_key().public()],
        wire::relay_hello(),
    );
    let mut harness = Harness::new(hub_end(hub_key(), relay_key().public()), relay);
    harness.run();

    assert!(matches!(harness.hub.link(), Link::Established(_)));
    assert!(matches!(harness.relay.link(), Link::Established(_)));
}

/// A hub monitor that holds no keys runs the opening exchange alone: it has
/// no link, and nothing can be sent on one.
#[test]
fn an_end_without_keys_has_no_link() {
    let mut hub = HubEnd::new(climon2_link::Purpose::Link, revision_1());
    let mut relay = RelayEnd::new(revision_1(), uninvited());

    assert_eq!(hub.link(), &Link::Pending);
    assert_eq!(relay.link(), &Link::Pending);
    let refused = Err(climon2_link::Refused::NotLinked);
    assert_eq!(hub.send(1, 0x41, b""), refused);
    assert_eq!(relay.send(1, 0x81, b""), refused);
    assert_eq!(hub.close(climon2_link::HubClose::Stopped), refused);
    assert_eq!(relay.close(climon2_link::RelayClose::Stopping), refused);
    assert!(hub.records().is_empty() && relay.records().is_empty());

    step(&mut hub, Duration::ZERO, Input::Tick);
    let output = step(&mut hub, Duration::ZERO, Input::Bytes(&REPLY));
    assert!(output.bytes.is_empty() && !output.close);
}

/// A static key shows its public half when debugged, never its private
/// half.
#[test]
fn a_static_key_never_shows_its_private_key() {
    let shown = format!("{:?}", hub_key());

    assert!(
        shown.contains(&format!("{:?}", hub_key().public())),
        "{shown}"
    );
    assert!(!shown.contains("17, 17"), "{shown}");
}

/// A handshake message whose length is not the published one ends the
/// connection with no reply as soon as the length arrives, and so do bytes
/// behind a whole handshake message, which nothing sends before it is
/// answered.
#[test]
fn a_handshake_message_of_another_length_ends_the_connection_at_once() {
    let initiation = {
        let mut hub = hub_end(hub_key(), relay_key().public());
        step(&mut hub, Duration::ZERO, Input::Tick);
        step(&mut hub, Duration::ZERO, Input::Bytes(&REPLY)).bytes
    };
    let crowded = [&initiation[..], &[0]].concat();
    for (what, bytes) in [
        ("length 97", &[0x00, 0x61][..]),
        ("a byte behind", &crowded),
    ] {
        let mut relay = relay_end(relay_key(), &[hub_key().public()]);
        step(&mut relay, Duration::ZERO, Input::Tick);
        step(&mut relay, Duration::ZERO, Input::Bytes(&REQUEST));
        let output = step(&mut relay, SECOND, Input::Bytes(bytes));
        assert!(output.close && output.bytes.is_empty(), "{what}");
        assert_eq!(relay.link(), &Link::Ended(Ended::Failed), "{what}");
    }

    let mut hub = hub_end(hub_key(), relay_key().public());
    step(&mut hub, Duration::ZERO, Input::Tick);
    step(&mut hub, Duration::ZERO, Input::Bytes(&REPLY));
    let output = step(&mut hub, SECOND, Input::Bytes(&[0x00, 0x2f]));
    assert!(output.close && output.bytes.is_empty());
    assert_eq!(hub.link(), &Link::Ended(Ended::Failed));
}

/// A Hello that arrives at the very moment of the deadline is in time, at
/// either end, and once the link is established no deadline is named.
#[test]
fn a_hello_at_the_deadline_is_in_time() {
    let mut relay = relay_end(relay_key(), &[hub_key().public()]);
    let mut stand_in = StandIn::dial(&mut relay, [0x11; 32], relay_key().public());
    let hello = stand_in.record(&wire::HELLO);
    let output = step(&mut relay, 30 * SECOND, Input::Bytes(&hello));
    assert!(!output.close && output.deadline.is_none());
    assert!(matches!(relay.link(), Link::Established(_)));

    let mut hub = hub_end(hub_key(), relay_key().public());
    let mut stand_in = StandIn::answer(&mut hub, [0x22; 32]);
    let hello = stand_in.record(&wire::relay_hello_record());
    let output = step(&mut hub, 30 * SECOND, Input::Bytes(&hello));
    assert!(!output.close && output.deadline.is_none());
    assert!(matches!(hub.link(), Link::Established(_)));
}

/// The relay's 30 seconds run from accepting the connection, not from the
/// opening request.
#[test]
fn the_relays_deadline_runs_from_accepting() {
    let mut relay = relay_end(relay_key(), &[hub_key().public()]);
    step(&mut relay, 4 * SECOND, Input::Tick);
    let output = step(&mut relay, 9 * SECOND, Input::Bytes(&REQUEST));

    assert_eq!(output.bytes, REPLY);
    assert_eq!(output.deadline, Some(Time::since_start(34 * SECOND)));
}

/// A hub monitor holding keys whose opening exchange fails has no link: it
/// closes and sends nothing more.
#[test]
fn a_failed_opening_fails_the_link() {
    let mut hub = hub_end(hub_key(), relay_key().public());
    step(&mut hub, Duration::ZERO, Input::Tick);
    let mut refusal = REPLY;
    refusal[0] = 2;
    refusal[2] = 0;
    refusal[5] = 2;
    let output = step(&mut hub, Duration::ZERO, Input::Bytes(&refusal));

    assert!(matches!(hub.opening(), Opening::Incompatible { .. }));
    assert!(output.close && output.bytes.is_empty());
    assert_eq!(hub.link(), &Link::Ended(Ended::Failed));

    let mut relay = relay_end(relay_key(), &[hub_key().public()]);
    let mut malformed = REQUEST;
    malformed[0] = b'C';
    let output = step(&mut relay, Duration::ZERO, Input::Bytes(&malformed));
    assert!(output.close && output.bytes.is_empty());
    assert_eq!(relay.link(), &Link::Ended(Ended::Failed));
}

/// A relay holding keys that a hub monitor reaches to pair starts no
/// handshake: the connection carries a pairing run, which reads a handshake
/// initiation as a malformed share.
#[test]
fn a_relay_reached_to_pair_starts_no_handshake() {
    let mut relay = RelayEnd::new(revision_1(), invited().0).linking(
        relay_key(),
        vec![hub_key().public()],
        wire::relay_hello(),
    );
    let mut request = REQUEST;
    request[8] = 0x01;
    let output = step(&mut relay, Duration::ZERO, Input::Bytes(&request));
    assert_eq!(output.bytes, REPLY);
    assert_eq!(relay.pairing(), &PairingRun::Pending);

    let mut hub = hub_end(hub_key(), relay_key().public());
    step(&mut hub, Duration::ZERO, Input::Tick);
    let initiation = step(&mut hub, Duration::ZERO, Input::Bytes(&REPLY)).bytes;
    let output = step(&mut relay, SECOND, Input::Bytes(&initiation));
    assert!(output.bytes.is_empty(), "no handshake reply");
    assert!(output.close);
    assert_eq!(relay.link(), &Link::Pending);
    assert_eq!(relay.pairing(), &PairingRun::Failed { counted: false });
}
