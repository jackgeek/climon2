//! Pairing over a link invitation at the Link layer (#495): the four
//! messages of a pairing connection, what each end keeps of the other, and
//! what an on-path proxy can do to them, with the time and the randomness
//! supplied by the test. The rules of the invitation a run starts under are
//! in `tests/invitation.rs`. The scenario IDs are those of the monitor-link
//! acceptance criteria.

mod support;

use climon2_link::{
    End, HubEnd, Input, Introduction, Invitation, Invitations, Opening, Output, PairingRun, Pin,
    Purpose, Time,
};
use std::time::Duration;
use support::pairing::*;
use support::{Harness, Seeded};

/// PAIR-01: pairing takes four messages on a connection of its own, which
/// then closes and carries nothing else: the hub monitor's share, the
/// relay's share, the hub monitor's confirmation and the relay's. Each end
/// is handed the other's key and name; the relay hands over the identity it
/// was made with, and the invitation is consumed by the hub monitor's key.
#[test]
fn pair_01_pairing_takes_four_messages_on_its_own_connection() {
    let (invitations, pin) = invited();
    let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));

    harness.run();

    let from_hub = pairing_messages(harness.sent_by_hub(), OPENING.0);
    let from_relay = pairing_messages(harness.sent_by_relay(), OPENING.1);
    let sizes = |messages: &[Vec<u8>]| messages.iter().map(Vec::len).collect::<Vec<_>>();
    // A share is 32 bytes; a confirmation is 64 bytes and a box of the key,
    // the name's length, the name and a 16-byte tag.
    assert_eq!(sizes(&from_hub), [32, 64 + 32 + 1 + "laptop".len() + 16]);
    assert_eq!(
        sizes(&from_relay),
        [32, 64 + 32 + 1 + "büild-box".len() + 16]
    );
    assert!(harness.hub_closed() && harness.relay_closed());
    assert_eq!(
        harness.hub.pairing(),
        &PairingRun::Paired(relay_introduction())
    );
    assert_eq!(
        harness.relay.pairing(),
        &PairingRun::Paired(hub_introduction())
    );
    assert_eq!(
        invitations.invitation(harness.now()),
        Invitation::Paired(hub_introduction())
    );
}

/// PAIR-01: a PIN is typed with or without its hyphen and with spaces, and
/// pairs the same either way. Anything but 8 digits is no PIN.
#[test]
fn pair_01_hyphens_and_spaces_in_a_typed_pin_are_ignored() {
    for typed in ["8517-9087", "85179087", " 8517 9087 ", "8-5-1-7 9-0-8-7"] {
        let (invitations, _) = invited();
        let mut harness = Harness::new(pairing_hub(&pin(typed)), pairing_relay(&invitations));
        harness.run();
        assert_eq!(
            harness.hub.pairing(),
            &PairingRun::Paired(relay_introduction()),
            "{typed}"
        );
    }
    for typed in [
        "8517-908",
        "8517-90870",
        "8517-908a",
        "8517_9087",
        "",
        "٨٥١٧٩٠٨٧",
    ] {
        assert!(Pin::parse(typed).is_none(), "{typed}");
    }
}

/// PAIR-01: the relay shows a PIN as `1234-5678`; a PIN's debug form, the
/// one a log would take, hides its digits.
#[test]
fn pair_01_a_pin_shows_as_two_groups_of_four_and_hides_from_logs() {
    let (_, pin) = invited();

    assert_eq!(pin.to_string(), "8517-9087");
    assert_eq!(self::pin("00000001").to_string(), "0000-0001");
    assert_eq!(format!("{pin:?}"), "Pin(hidden)");
}

/// PAIR-01: a name is at most 64 bytes, and any text that fits is handed
/// over as it is, the empty name too.
#[test]
fn pair_01_a_name_of_up_to_64_bytes_is_handed_over() {
    let longest = "é".repeat(32);
    assert!(Introduction::new([7; 32], &format!("{longest}x")).is_none());
    for name in ["", "\u{1b}[31m", longest.as_str()] {
        let relay = Introduction::new([7; 32], name).expect("at most 64 bytes");
        let invitations = Invitations::new(relay.clone());
        let pin = invitations.mint(at(Duration::ZERO), &mut Seeded::new(9));
        let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));
        harness.run();
        assert_eq!(harness.hub.pairing(), &PairingRun::Paired(relay), "{name}");
    }
    let introduction = hub_introduction();
    assert_eq!(introduction.key(), &support::pairing::HUB_KEY);
    assert_eq!(introduction.name(), "laptop");
}

/// PAIR-01: until its run ends, an end says its pairing is pending, and an
/// end made only to open says so for good.
#[test]
fn pair_01_a_run_is_pending_until_it_ends() {
    let (invitations, pin) = invited();
    let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
    assert_eq!(wire.hub.pairing(), &PairingRun::Pending);
    assert_eq!(wire.relay.pairing(), &PairingRun::Pending);

    let (_, confirmation) = wire.shares(Duration::ZERO);

    assert!(!confirmation.close);
    assert_eq!(wire.hub.pairing(), &PairingRun::Pending);
    assert_eq!(wire.relay.pairing(), &PairingRun::Pending);
    let mut harness = Harness::new(
        HubEnd::new(Purpose::Pair, revisions(&[1])),
        pairing_relay(&invited().0),
    );
    harness.run();
    assert_eq!(harness.hub.pairing(), &PairingRun::Pending);
}

/// PAIR-05: a dialler without the PIN gets the relay's share and never a
/// confirmation, because the hub monitor confirms first. It learns nothing
/// more from the run, and the PIN's digits never cross the wire.
#[test]
fn pair_05_a_dialler_without_the_pin_never_gets_a_confirmation() {
    let (invitations, pin) = invited();
    let mut harness = Harness::new(pairing_hub(&self::pin(WRONG)), pairing_relay(&invitations));

    harness.run();

    let from_relay = pairing_messages(harness.sent_by_relay(), OPENING.1);
    assert_eq!(from_relay.len(), 1);
    assert_eq!(from_relay[0].len(), 32);
    assert!(harness.relay_closed());
    assert_eq!(
        harness.relay.pairing(),
        &PairingRun::Failed { counted: true }
    );
    assert_digits_never_cross(&pin, &harness);
}

/// PAIR-05: a relay without the PIN gets the hub monitor's share and one
/// confirmation, one guess, and the hub monitor keeps nothing of it.
#[test]
fn pair_05_a_relay_without_the_pin_gets_one_guess_and_nothing_kept() {
    let invitations = uninvited();
    invitations.mint(at(Duration::ZERO), &mut Seeded::new(11));
    let (_, pin) = invited();
    let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));

    harness.run();

    let from_hub = pairing_messages(harness.sent_by_hub(), OPENING.0);
    assert_eq!(from_hub.len(), 2);
    assert_eq!(harness.hub.pairing(), &PairingRun::Failed { counted: true });
    assert_digits_never_cross(&pin, &harness);
}

/// Neither end sent the PIN's digits, as text or as one number.
fn assert_digits_never_cross<H, R>(pin: &Pin, harness: &Harness<H, R>)
where
    H: End,
    R: End,
{
    let digits: Vec<u8> = pin.to_string().bytes().filter(u8::is_ascii_digit).collect();
    let number: u32 = std::str::from_utf8(&digits).unwrap().parse().unwrap();
    for sent in [harness.sent_by_hub(), harness.sent_by_relay()] {
        for needle in [&digits[..], &number.to_be_bytes(), &number.to_le_bytes()] {
            assert!(!sent.windows(needle.len()).any(|window| window == needle));
        }
    }
}

/// What the ends of a pairing run behind the on-path proxy came to, and the
/// invitation after it.
fn proxied_run(edit: impl FnMut(Way, usize, &mut Vec<u8>)) -> (PairingRun, PairingRun, Invitation) {
    let (invitations, pin) = invited();
    let (hub, relay) = proxied(pairing_hub(&pin), pairing_relay(&invitations), edit);
    (
        hub.end.pairing().clone(),
        relay.end.pairing().clone(),
        invitations.invitation(at(Duration::ZERO)),
    )
}

/// The invitation minted at time zero with two attempts left.
const TWO_LEFT: Invitation = Invitation::Live {
    expires: Time::since_start(Duration::from_secs(300)),
    attempts_left: 2,
};

/// A run that failed at both ends after the relay's share: it counts, and
/// neither end pinned a key.
const FAILED_AND_COUNTED: (PairingRun, PairingRun, Invitation) = (
    PairingRun::Failed { counted: true },
    PairingRun::Failed { counted: true },
    TWO_LEFT,
);

/// PAIR-06 and REV-03 on a pairing connection: an on-path proxy that edits
/// the revision list of either opening message, in a way both ends still
/// accept, leaves them with different transcripts, so the key exchange that
/// follows fails. The run counts and pins nothing, and neither end tries
/// another revision.
#[test]
fn pair_06_rev_03_an_edited_opening_fails_the_key_exchange() {
    // The request offers revisions 1 and 2, not only 1.
    let more_in_the_request = |way, index, bytes: &mut Vec<u8>| {
        if (way, index) == (Way::ToRelay, 0) {
            bytes[9] = 2;
            bytes[13] = 2;
        }
    };
    // The reply lists revisions 1 and 3, not only 1.
    let more_in_the_reply = |way, index, bytes: &mut Vec<u8>| {
        if (way, index) == (Way::ToHub, 0) {
            bytes[3] = 2;
            bytes[7] = 3;
        }
    };
    assert_eq!(proxied_run(more_in_the_request), FAILED_AND_COUNTED);
    assert_eq!(proxied_run(more_in_the_reply), FAILED_AND_COUNTED);
}

/// PAIR-06: the proxy sends the hub monitor its own share back as the
/// relay's. The run fails, counts and pins nothing.
#[test]
fn pair_06_a_share_reflected_to_its_sender_fails() {
    let mut hub_share = Vec::new();
    let reflect = |way, index, bytes: &mut Vec<u8>| match (way, index) {
        (Way::ToRelay, 1) => hub_share = bytes.clone(),
        (Way::ToHub, 1) => *bytes = hub_share.clone(),
        _ => {}
    };
    assert_eq!(proxied_run(reflect), FAILED_AND_COUNTED);
}

/// PAIR-06: each confirmation is bound to its sender's role. The hub
/// monitor's confirmation sent back to it in place of the relay's is
/// refused, so the hub monitor keeps nothing. The relay, which checked the
/// real one first, has paired: that is PAIR-07, a run cut short at its end.
#[test]
fn pair_06_a_confirmation_reflected_to_its_sender_is_refused() {
    let mut hub_confirmation = Vec::new();
    let reflect = |way, index, bytes: &mut Vec<u8>| match (way, index) {
        (Way::ToRelay, 2) => hub_confirmation = bytes.clone(),
        (Way::ToHub, 2) => *bytes = hub_confirmation.clone(),
        _ => {}
    };
    let (hub, relay, _) = proxied_run(reflect);
    assert_eq!(hub, PairingRun::Failed { counted: true });
    assert_eq!(relay, PairingRun::Paired(hub_introduction()));
}

/// PAIR-06: the proxy splices a share from one run into another, the hub
/// monitor's or the relay's. Each spliced run fails, counts and pins
/// nothing.
#[test]
fn pair_06_shares_spliced_from_another_run_fail() {
    let (invitations, pin) = invited();
    let (first_hub, first_relay) =
        proxied(pairing_hub(&pin), pairing_relay(&invitations), |_, _, _| {});
    assert_eq!(
        first_hub.end.pairing(),
        &PairingRun::Paired(relay_introduction())
    );
    for (from, way) in [
        (&first_hub.sent[1], Way::ToRelay),
        (&first_relay.sent[1], Way::ToHub),
    ] {
        let (invitations, pin) = invited();
        let splice = |each, index, bytes: &mut Vec<u8>| {
            if (each, index) == (way, 1) {
                *bytes = from.clone();
            }
        };
        let (hub, relay) = proxied_with_seeds(
            pairing_hub(&pin),
            pairing_relay(&invitations),
            (5, 6),
            splice,
        );
        assert_ne!(&hub.sent[1], from);
        let outcome = (
            hub.end.pairing().clone(),
            relay.end.pairing().clone(),
            invitations.invitation(at(Duration::ZERO)),
        );
        assert_eq!(outcome, FAILED_AND_COUNTED, "{way:?}");
    }
}

/// PAIR-06: a flipped bit anywhere in either end's share or confirmation
/// fails the run, which counts and pins nothing.
#[test]
fn pair_06_a_flipped_bit_in_any_pairing_message_fails() {
    for (way, index, at) in [
        (Way::ToRelay, 2, 2),
        (Way::ToRelay, 2, 70),
        (Way::ToRelay, 2, 120),
        (Way::ToHub, 1, 20),
    ] {
        let flip = |each, number, bytes: &mut Vec<u8>| {
            if (each, number) == (way, index) {
                bytes[at] ^= 0x01;
            }
        };
        let (hub, relay, invitation) = proxied_run(flip);
        assert_eq!(invitation, TWO_LEFT, "{way:?} {index} {at}");
        assert_ne!(hub, PairingRun::Paired(relay_introduction()));
        assert_eq!(relay, PairingRun::Failed { counted: true });
    }
    for at in [2, 70, 124] {
        let flip = |each, number, bytes: &mut Vec<u8>| {
            if (each, number) == (Way::ToHub, 2) {
                bytes[at] ^= 0x80;
            }
        };
        let (hub, relay, invitation) = proxied_run(flip);
        assert_eq!(hub, PairingRun::Failed { counted: true }, "{at}");
        assert_eq!(relay, PairingRun::Paired(hub_introduction()), "{at}");
        assert_eq!(invitation, Invitation::Paired(hub_introduction()));
    }
}

/// PAIR-07: the hub monitor stops after the relay has checked its
/// confirmation, before the relay's own reaches it. The relay has consumed
/// the invitation and holds the hub monitor's key for its user to keep or
/// forget; the hub monitor has nothing to report as paired.
#[test]
fn pair_07_a_hub_monitor_that_stops_at_the_end_leaves_the_relay_paired() {
    let (invitations, pin) = invited();
    let drop_last = |way, index, bytes: &mut Vec<u8>| {
        if (way, index) == (Way::ToHub, 2) {
            bytes.clear();
        }
    };

    let (hub, relay) = proxied(pairing_hub(&pin), pairing_relay(&invitations), drop_last);

    assert_eq!(relay.end.pairing(), &PairingRun::Paired(hub_introduction()));
    assert_eq!(
        invitations.invitation(at(Duration::ZERO)),
        Invitation::Paired(hub_introduction())
    );
    assert_eq!(hub.end.pairing(), &PairingRun::Failed { counted: true });
}

/// PAIR-08, the Link-layer part: with one hub monitor paired, minting, a
/// failed run, expiry and cancelling change nothing for a connection opened
/// to link, which opens alike at each. A later invitation then pairs a
/// second hub monitor, and the relay is handed its key.
#[test]
fn pair_08_an_invitation_leaves_link_connections_alone_until_a_second_pairing() {
    let link = |invitations: &Invitations| {
        let mut harness = Harness::new(
            HubEnd::new(Purpose::Link, revisions(&[1])),
            pairing_relay(invitations),
        );
        harness.run();
        (
            harness.sent_by_relay().to_vec(),
            harness.relay.opening().clone(),
            harness.relay_closed(),
        )
    };
    let before = link(&uninvited());
    let (invitations, pin) = invited();
    Harness::new(pairing_hub(&pin), pairing_relay(&invitations)).run();
    assert_eq!(link(&invitations), before);

    invitations.mint(at(Duration::ZERO), &mut Seeded::new(5));
    assert_eq!(link(&invitations), before);
    Harness::new(pairing_hub(&self::pin(WRONG)), pairing_relay(&invitations)).run();
    assert_eq!(attempts_left(&invitations), 2);
    assert_eq!(link(&invitations), before);
    assert_eq!(
        invitations.invitation(at(300 * SECOND)),
        Invitation::Expired
    );
    assert_eq!(link(&invitations), before);
    invitations.mint(at(300 * SECOND), &mut Seeded::new(6));
    invitations.cancel();
    assert_eq!(link(&invitations), before);

    let second = Introduction::new([0x22; 32], "desk").expect("a short name");
    let pin = invitations.mint(at(400 * SECOND), &mut Seeded::new(7));
    let hub = HubEnd::pair(revisions(&[1]), pin, second.clone());
    let mut harness = Harness::new(hub, pairing_relay(&invitations));
    harness.advance(400 * SECOND);
    assert_eq!(harness.relay.pairing(), &PairingRun::Paired(second.clone()));
    assert_eq!(
        invitations.invitation(harness.now()),
        Invitation::Paired(second)
    );
}

/// A byte stream keeps no message boundaries: a pairing run works one byte
/// at a time, and gives the same bytes as when each message arrives whole.
#[test]
fn a_pairing_run_survives_arriving_one_byte_at_a_time() {
    let run = |fragment: Option<usize>| {
        let (invitations, pin) = invited();
        let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));
        if let Some(bytes) = fragment {
            harness.fragment(bytes);
        }
        harness.run();
        assert_eq!(
            harness.relay.pairing(),
            &PairingRun::Paired(hub_introduction())
        );
        (
            harness.sent_by_hub().to_vec(),
            harness.sent_by_relay().to_vec(),
        )
    };

    assert_eq!(run(Some(1)), run(None));
}

/// No honest end sends anything behind its opening message before it has
/// the other end's answer: the hub monitor's share is keyed by the opening
/// transcript, reply included, and the relay's share answers the hub
/// monitor's. So bytes that arrive with a whole opening message, behind it,
/// make it malformed on a pairing connection too. The relay closes with no
/// reply and seats no run, so the invitation keeps all its attempts; the hub
/// monitor's dial fails before it sends its share.
#[test]
fn bytes_behind_an_opening_message_fail_a_pairing_connection() {
    let (invitations, pin) = invited();
    let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));
    harness.run();
    let (from_hub, from_relay) = (harness.sent_by_hub(), harness.sent_by_relay());

    let (invitations, pin) = invited();
    let mut relay = pairing_relay(&invitations);
    let answer = relay.step(
        at(Duration::ZERO),
        Input::Bytes(from_hub),
        &mut Seeded::new(2),
    );
    assert_eq!(answer, silent_close());
    assert_eq!(relay.opening(), &Opening::Failed);
    assert_eq!(relay.pairing(), &PairingRun::Pending);
    assert_eq!(attempts_left(&invitations), 3);
    let mut honest = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));
    honest.run();
    assert_eq!(
        honest.relay.pairing(),
        &PairingRun::Paired(hub_introduction())
    );

    let mut hub = pairing_hub(&pin);
    let mut randomness = Seeded::new(1);
    hub.step(at(Duration::ZERO), Input::Tick, &mut randomness);
    let answer = hub.step(
        at(Duration::ZERO),
        Input::Bytes(from_relay),
        &mut randomness,
    );
    assert_eq!(answer, silent_close());
    assert_eq!(hub.opening(), &Opening::Failed);
    assert_eq!(hub.pairing(), &PairingRun::Pending);
}

/// Inside a pairing run a byte stream keeps no message boundaries: an end
/// handed several pairing messages in one arrival reads them all. A relay
/// handed the rest of what the hub monitor sends at once answers it all, and
/// a hub monitor handed the rest of what the relay sends at once pairs.
#[test]
fn a_pairing_run_reads_every_message_in_one_arrival() {
    let (invitations, pin) = invited();
    let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));
    harness.run();
    let (from_hub, from_relay) = (harness.sent_by_hub(), harness.sent_by_relay());
    let (request, run_from_hub) = from_hub.split_at(OPENING.0);
    let (reply, run_from_relay) = from_relay.split_at(OPENING.1);

    let (invitations, _) = invited();
    let mut relay = pairing_relay(&invitations);
    let mut randomness = Seeded::new(2);
    let answer = relay.step(at(Duration::ZERO), Input::Bytes(request), &mut randomness);
    assert_eq!(answer.bytes, reply);
    let answer = relay.step(
        at(Duration::ZERO),
        Input::Bytes(run_from_hub),
        &mut randomness,
    );
    assert_eq!(answer.bytes, run_from_relay);
    assert!(answer.close);
    assert_eq!(relay.pairing(), &PairingRun::Paired(hub_introduction()));

    let mut hub = pairing_hub(&pin);
    let mut randomness = Seeded::new(1);
    hub.step(at(Duration::ZERO), Input::Tick, &mut randomness);
    let share = hub.step(at(Duration::ZERO), Input::Bytes(reply), &mut randomness);
    let answer = hub.step(
        at(Duration::ZERO),
        Input::Bytes(run_from_relay),
        &mut randomness,
    );
    assert_eq!([share.bytes, answer.bytes].concat(), run_from_hub);
    assert!(answer.close);
    assert_eq!(hub.pairing(), &PairingRun::Paired(relay_introduction()));
}

/// Once a run has ended, its end does nothing more, whatever it is handed.
#[test]
fn an_ended_run_does_nothing_more() {
    let (invitations, pin) = invited();
    let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));
    harness.run();
    let mut randomness = Seeded::new(0);
    for input in [Input::Tick, Input::Bytes(b"more"), Input::Closed] {
        let hub = harness.hub.step(at(Duration::ZERO), input, &mut randomness);
        let relay = harness
            .relay
            .step(at(Duration::ZERO), input, &mut randomness);
        assert_eq!(hub, Output::default());
        assert_eq!(relay, Output::default());
    }
    assert_eq!(
        harness.relay.pairing(),
        &PairingRun::Paired(hub_introduction())
    );
}
