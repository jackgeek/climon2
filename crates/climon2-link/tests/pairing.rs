//! Pairing over a link invitation at the Link layer (#495): the four
//! messages of a pairing connection and the rules of the invitation they run
//! under, with the time and the randomness supplied by the test. The
//! scenario IDs are those of the monitor-link acceptance criteria.

mod support;

use climon2_link::{
    End, HubEnd, Input, Introduction, Invitation, Invitations, Opening, Output, PairingRun, Pin,
    Purpose, RelayEnd, Time,
};
use std::time::Duration;
use support::pairing::{
    Way, at, hub_introduction, invited, pairing_hub, pairing_relay, proxied, proxied_with_seeds,
    relay_introduction, revisions, uninvited,
};
use support::{Harness, Seeded};

const SECOND: Duration = Duration::from_secs(1);
const MILLISECOND: Duration = Duration::from_millis(1);
/// The size of an opening request and of an opening reply.
const OPENING: (usize, usize) = (26, 20);
/// A PIN the relay of these tests never mints.
const WRONG: &str = "1111-2222";

fn pin(typed: &str) -> Pin {
    Pin::parse(typed).expect("8 digits")
}

/// The pairing messages in `bytes`, each without its length. Whatever
/// follows the last whole message is returned as a message of its own.
fn messages(mut bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut found = Vec::new();
    while let Some((length, rest)) = bytes.split_first_chunk::<2>() {
        let length = usize::from(u16::from_be_bytes(*length)).min(rest.len());
        found.push(rest[..length].to_vec());
        bytes = &rest[length..];
    }
    if !bytes.is_empty() {
        found.push(bytes.to_vec());
    }
    found
}

/// The pairing messages an end sent after its opening message.
fn pairing_messages(sent: &[u8], opening: usize) -> Vec<Vec<u8>> {
    messages(&sent[opening..])
}

/// How many attempts the relay's invitation has left at time zero.
fn attempts_left(invitations: &Invitations) -> u8 {
    match invitations.invitation(Time::since_start(Duration::ZERO)) {
        Invitation::Live { attempts_left, .. } => attempts_left,
        other => panic!("no live invitation: {other:?}"),
    }
}

/// Two ends stepped by hand, each with randomness of its own, at moments
/// the test names.
struct Wire {
    hub: HubEnd,
    relay: RelayEnd,
    hub_randomness: Seeded,
    relay_randomness: Seeded,
}

impl Wire {
    fn new(hub: HubEnd, relay: RelayEnd) -> Self {
        Self {
            hub,
            relay,
            hub_randomness: Seeded::new(1),
            relay_randomness: Seeded::new(2),
        }
    }

    fn hub(&mut self, when: Duration, input: Input<'_>) -> Output {
        self.hub.step(at(when), input, &mut self.hub_randomness)
    }

    fn relay(&mut self, when: Duration, input: Input<'_>) -> Output {
        self.relay.step(at(when), input, &mut self.relay_randomness)
    }

    /// Runs the opening exchange at `when`: the relay accepts, the hub
    /// monitor dials, and the reply reaches the hub monitor. The hub
    /// monitor's share message, which it sends at once.
    fn open(&mut self, when: Duration) -> Output {
        let request = self.hub(when, Input::Tick).bytes;
        self.relay(when, Input::Tick);
        let reply = self.relay(when, Input::Bytes(&request)).bytes;
        self.hub(when, Input::Bytes(&reply))
    }

    /// Runs the opening exchange and both shares at `when`: the relay's
    /// share message, and the hub monitor's confirmation.
    fn shares(&mut self, when: Duration) -> (Output, Output) {
        let hub_share = self.open(when).bytes;
        let relay_share = self.relay(when, Input::Bytes(&hub_share));
        let confirmation = self.hub(when, Input::Bytes(&relay_share.bytes));
        (relay_share, confirmation)
    }
}

/// An output that closes the byte stream without sending a byte.
fn silent_close() -> Output {
    Output {
        bytes: Vec::new(),
        close: true,
        deadline: None,
    }
}

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

/// PAIR-02: a relay has one live invitation, and minting revokes the one
/// before. The old PIN is then a wrong PIN.
#[test]
fn pair_02_a_new_invitation_revokes_the_old() {
    let (invitations, old) = invited();
    let new = invitations.mint(at(Duration::ZERO), &mut Seeded::new(4));
    assert_ne!(old.to_string(), new.to_string());

    let mut harness = Harness::new(pairing_hub(&old), pairing_relay(&invitations));
    harness.run();
    assert_eq!(
        harness.relay.pairing(),
        &PairingRun::Failed { counted: true }
    );
    assert_eq!(attempts_left(&invitations), 2);

    let mut harness = Harness::new(pairing_hub(&new), pairing_relay(&invitations));
    harness.run();
    assert_eq!(
        harness.hub.pairing(),
        &PairingRun::Paired(relay_introduction())
    );
}

/// PAIR-02: a pairing run in flight when its invitation is revoked,
/// cancelled or expires fails at its next step, sends nothing more and
/// leaves the relay's next invitation untouched. Each run starts 10 seconds
/// before the invitation would expire and is handed its next message as it
/// does.
#[test]
fn pair_02_a_run_in_flight_fails_when_its_invitation_ends() {
    let start = 290 * SECOND;
    let expiry = 300 * SECOND;
    type Ending = (&'static str, fn(&Invitations));
    let endings: [Ending; 3] = [
        ("revoked", |invitations| {
            invitations.mint(at(290 * SECOND), &mut Seeded::new(4));
        }),
        ("cancelled", Invitations::cancel),
        ("expired", |_| {}),
    ];
    for (ending, end) in endings {
        // Before the hub monitor's share.
        let (invitations, pin) = invited();
        let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
        let hub_share = wire.open(start).bytes;
        end(&invitations);
        let output = wire.relay(expiry, Input::Bytes(&hub_share));
        assert_eq!(output, silent_close(), "{ending}");
        assert_eq!(
            wire.relay.pairing(),
            &PairingRun::Failed { counted: false },
            "{ending}"
        );

        // Before the hub monitor's confirmation.
        let (invitations, pin) = invited();
        let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
        let (_, confirmation) = wire.shares(start);
        end(&invitations);
        let output = wire.relay(expiry, Input::Bytes(&confirmation.bytes));
        assert_eq!(output, silent_close(), "{ending}");
        assert_eq!(
            wire.relay.pairing(),
            &PairingRun::Failed { counted: true },
            "{ending}"
        );
        let after = invitations.invitation(at(expiry));
        let expected = match ending {
            "revoked" => Invitation::Live {
                expires: at(590 * SECOND),
                attempts_left: 3,
            },
            "cancelled" => Invitation::Cancelled,
            _ => Invitation::Expired,
        };
        assert_eq!(after, expected, "{ending}");
    }
}

/// PAIR-02, with supplied time: an invitation expires 5 minutes after it is
/// minted and not before. Expiry is final, and nothing mints another.
#[test]
fn pair_02_expiry_at_5_minutes_is_final() {
    let (invitations, pin) = invited();
    let five_minutes = 300 * SECOND;

    assert_eq!(
        invitations.invitation(at(five_minutes - MILLISECOND)),
        Invitation::Live {
            expires: at(five_minutes),
            attempts_left: 3
        }
    );
    assert_eq!(
        invitations.invitation(at(five_minutes)),
        Invitation::Expired
    );
    assert_eq!(
        invitations.invitation(at(10 * five_minutes)),
        Invitation::Expired
    );
    // Time on the caller's clock never runs back; an expired invitation
    // stays expired even if it is asked about an earlier moment.
    assert_eq!(
        invitations.invitation(at(Duration::ZERO)),
        Invitation::Expired
    );

    let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));
    harness.advance(five_minutes);
    assert_eq!(harness.relay.opening(), &Opening::PairingUnavailable);
    assert_eq!(harness.hub.opening(), &Opening::PairingUnavailable);
}

/// PAIR-02: an invitation minted later expires 5 minutes after its own mint,
/// and one minted at the end of the caller's clock never wraps round.
#[test]
fn pair_02_each_invitation_expires_5_minutes_after_its_mint() {
    let invitations = uninvited();
    invitations.mint(at(42 * SECOND), &mut Seeded::new(3));
    assert_eq!(
        invitations.invitation(at(42 * SECOND)),
        Invitation::Live {
            expires: at(342 * SECOND),
            attempts_left: 3
        }
    );

    invitations.mint(at(Duration::MAX), &mut Seeded::new(3));
    assert_eq!(
        invitations.invitation(at(Duration::MAX - SECOND)),
        Invitation::Live {
            expires: at(Duration::MAX),
            attempts_left: 3
        }
    );
}

/// PAIR-02: cancelling ends the live invitation and nothing else. With no
/// invitation live it changes nothing.
#[test]
fn pair_02_cancelling_ends_only_a_live_invitation() {
    let invitations = uninvited();
    invitations.cancel();
    assert_eq!(invitations.invitation(at(Duration::ZERO)), Invitation::None);

    let (invitations, pin) = invited();
    invitations.cancel();
    assert_eq!(
        invitations.invitation(at(Duration::ZERO)),
        Invitation::Cancelled
    );
    let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));
    harness.run();
    assert_eq!(harness.relay.opening(), &Opening::PairingUnavailable);

    let (invitations, pin) = invited();
    Harness::new(pairing_hub(&pin), pairing_relay(&invitations)).run();
    invitations.cancel();
    assert_eq!(
        invitations.invitation(at(Duration::ZERO)),
        Invitation::Paired(hub_introduction())
    );
}

/// PAIR-02: an invitation pairs one hub monitor. The same PIN once more
/// finds no invitation open, and the first pairing stands.
#[test]
fn pair_02_an_invitation_pairs_one_hub_monitor() {
    let (invitations, pin) = invited();
    Harness::new(pairing_hub(&pin), pairing_relay(&invitations)).run();

    let other = Introduction::new([9; 32], "other").expect("a short name");
    let hub = HubEnd::pair(revisions(&[1]), pin, other);
    let mut harness = Harness::new(hub, pairing_relay(&invitations));
    harness.run();

    assert_eq!(harness.hub.opening(), &Opening::PairingUnavailable);
    assert_eq!(
        invitations.invitation(harness.now()),
        Invitation::Paired(hub_introduction())
    );
}

/// PAIR-03: three runs with a wrong PIN use the invitation up; each counts
/// at both ends. The right PIN then finds no invitation open.
#[test]
fn pair_03_three_wrong_pins_use_up_the_invitation() {
    let (invitations, pin) = invited();
    for left in [2, 1, 0] {
        let mut harness = Harness::new(pairing_hub(&self::pin(WRONG)), pairing_relay(&invitations));
        harness.run();
        assert_eq!(
            harness.relay.pairing(),
            &PairingRun::Failed { counted: true }
        );
        assert_eq!(harness.hub.pairing(), &PairingRun::Failed { counted: true });
        if left > 0 {
            assert_eq!(attempts_left(&invitations), left);
        }
    }
    assert_eq!(
        invitations.invitation(at(Duration::ZERO)),
        Invitation::UsedUp
    );

    let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));
    harness.run();
    assert_eq!(harness.hub.opening(), &Opening::PairingUnavailable);
}

/// PAIR-03: once the relay has sent its share, every way a run can fail
/// counts: a hang-up, a timeout, and a confirmation that is malformed,
/// oversized, cut short or wrong. The relay closes without a byte.
#[test]
fn pair_03_every_failure_after_the_relays_share_counts() {
    let mut wrong = vec![0x00, 177];
    wrong.extend([0; 177]);
    let failures: [(&str, Duration, Vec<Input<'_>>); 7] = [
        ("hang-up", Duration::ZERO, vec![Input::Closed]),
        ("timeout", 30 * SECOND, vec![Input::Tick]),
        (
            "too short",
            Duration::ZERO,
            vec![Input::Bytes(&[0x00, 112])],
        ),
        (
            "oversized",
            Duration::ZERO,
            vec![Input::Bytes(&[0x00, 178])],
        ),
        ("empty", Duration::ZERO, vec![Input::Bytes(&[0x00, 0x00])]),
        (
            "cut short",
            Duration::ZERO,
            vec![Input::Bytes(&[0x00, 113, 1, 2, 3]), Input::Closed],
        ),
        ("wrong", Duration::ZERO, vec![Input::Bytes(&wrong)]),
    ];
    for (failure, when, inputs) in failures {
        let (invitations, pin) = invited();
        let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
        let (relay_share, _) = wire.shares(Duration::ZERO);
        assert_eq!(messages(&relay_share.bytes).len(), 1, "{failure}");

        let outputs: Vec<Output> = inputs
            .into_iter()
            .map(|input| wire.relay(when, input))
            .collect();

        assert_eq!(outputs.last(), Some(&silent_close()), "{failure}");
        assert!(outputs.iter().all(|output| output.bytes.is_empty()));
        assert_eq!(
            wire.relay.pairing(),
            &PairingRun::Failed { counted: true },
            "{failure}"
        );
        assert_eq!(attempts_left(&invitations), 2, "{failure}");
    }
}

/// PAIR-03: nothing before the relay's share counts: a connection that
/// closes at once or after the opening, garbage, no shared revision, and a
/// share that is malformed or no valid group element. The relay sends no
/// share to any of them.
#[test]
fn pair_03_no_failure_before_the_relays_share_counts() {
    let neutral = [&[0x00, 0x20][..], &[0; 32]].concat();
    let mut invalid = vec![0x00, 0x20];
    invalid.extend([0xff; 32]);
    let shares: [&[u8]; 5] = [
        &[0x00, 0x1f],
        &[0x00, 0x21],
        &[0xff, 0xff],
        &neutral,
        &invalid,
    ];
    for share in shares {
        let (invitations, pin) = invited();
        let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
        wire.open(Duration::ZERO);
        let output = wire.relay(Duration::ZERO, Input::Bytes(share));
        assert_eq!(output, silent_close(), "{share:02x?}");
        assert_eq!(wire.relay.pairing(), &PairingRun::Failed { counted: false });
        assert_eq!(attempts_left(&invitations), 3);
    }

    let (invitations, pin) = invited();
    let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
    wire.open(Duration::ZERO);
    assert_eq!(wire.relay(SECOND, Input::Closed), silent_close());
    assert_eq!(wire.relay.pairing(), &PairingRun::Failed { counted: false });
    assert_eq!(attempts_left(&invitations), 3);

    let mut relay = pairing_relay(&invitations);
    let mut randomness = Seeded::new(0);
    relay.step(at(Duration::ZERO), Input::Tick, &mut randomness);
    relay.step(at(Duration::ZERO), Input::Closed, &mut randomness);
    let mut relay = pairing_relay(&invitations);
    relay.step(
        at(Duration::ZERO),
        Input::Bytes(&[0x55; 64]),
        &mut randomness,
    );
    let hub = HubEnd::pair(revisions(&[2]), pin.clone(), hub_introduction());
    Harness::new(hub, pairing_relay(&invitations)).run();
    assert_eq!(attempts_left(&invitations), 3);

    // And the invitation still pairs.
    let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));
    harness.run();
    assert_eq!(
        harness.relay.pairing(),
        &PairingRun::Paired(hub_introduction())
    );
}

/// PAIR-03: the hub monitor, too, counts its failure from the relay's share
/// on: a relay that closes before it is no attempt.
#[test]
fn pair_03_the_hub_monitor_counts_from_the_relays_share() {
    let (invitations, pin) = invited();
    let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
    wire.open(Duration::ZERO);
    assert_eq!(wire.hub(SECOND, Input::Closed), silent_close());
    assert_eq!(wire.hub.pairing(), &PairingRun::Failed { counted: false });

    let (invitations, pin) = invited();
    let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
    wire.shares(Duration::ZERO);
    assert_eq!(wire.hub(SECOND, Input::Closed), silent_close());
    assert_eq!(wire.hub.pairing(), &PairingRun::Failed { counted: true });
}

/// PAIR-03: while one run is in flight a second is refused at once, with
/// "pairing unavailable", and does not count. The first still pairs.
#[test]
fn pair_03_a_concurrent_run_is_refused_at_once_and_does_not_count() {
    let (invitations, pin) = invited();
    let mut first = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
    let hub_share = first.open(Duration::ZERO).bytes;

    let mut second = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));
    second.run();
    assert_eq!(second.relay.opening(), &Opening::PairingUnavailable);
    assert_eq!(second.sent_by_relay()[0], 3);
    assert!(second.relay_closed());
    assert_eq!(attempts_left(&invitations), 3);

    let relay_share = first.relay(Duration::ZERO, Input::Bytes(&hub_share)).bytes;
    let confirmation = first.hub(Duration::ZERO, Input::Bytes(&relay_share)).bytes;
    first.relay(Duration::ZERO, Input::Bytes(&confirmation));
    assert_eq!(
        first.relay.pairing(),
        &PairingRun::Paired(hub_introduction())
    );
}

/// PAIR-03: a run ends when it fails, so the next one can start; a run that
/// is abandoned, its end dropped, frees the invitation too and counts once
/// the relay's share was sent.
#[test]
fn pair_03_an_abandoned_run_frees_the_invitation() {
    let (invitations, pin) = invited();
    let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
    wire.open(Duration::ZERO);
    drop(wire);
    assert_eq!(attempts_left(&invitations), 3);

    let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
    wire.shares(Duration::ZERO);
    drop(wire);
    assert_eq!(attempts_left(&invitations), 2);

    let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));
    harness.run();
    assert_eq!(
        harness.relay.pairing(),
        &PairingRun::Paired(hub_introduction())
    );
}

/// PAIR-03, with supplied time: the whole run must finish 30 seconds after
/// the relay accepted the connection. A confirmation that arrives exactly
/// then pairs; one that arrives later fails and counts. The relay names
/// that deadline to its caller from the opening on.
#[test]
fn pair_03_a_run_that_passes_30_seconds_fails_at_the_relay() {
    let thirty = 30 * SECOND;
    for (when, paired) in [(thirty, true), (thirty + MILLISECOND, false)] {
        let (invitations, pin) = invited();
        let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
        let request = wire.hub(Duration::ZERO, Input::Tick).bytes;
        wire.relay(Duration::ZERO, Input::Tick);
        let reply = wire.relay(5 * SECOND, Input::Bytes(&request));
        assert_eq!(reply.deadline, Some(at(thirty)));
        let hub_share = wire.hub(5 * SECOND, Input::Bytes(&reply.bytes)).bytes;
        let relay_share = wire.relay(20 * SECOND, Input::Bytes(&hub_share));
        assert_eq!(relay_share.deadline, Some(at(thirty)));
        assert_eq!(
            wire.relay(29 * SECOND, Input::Tick).deadline,
            Some(at(thirty))
        );
        let confirmation = wire
            .hub(20 * SECOND, Input::Bytes(&relay_share.bytes))
            .bytes;

        let output = wire.relay(when, Input::Bytes(&confirmation));

        assert!(output.close);
        assert_eq!(output.bytes.is_empty(), !paired);
        let expected = match paired {
            true => PairingRun::Paired(hub_introduction()),
            false => PairingRun::Failed { counted: true },
        };
        assert_eq!(wire.relay.pairing(), &expected);
    }

    // A tick at the deadline with the run unfinished fails it.
    let (invitations, pin) = invited();
    let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
    wire.open(Duration::ZERO);
    assert!(!wire.relay(thirty - MILLISECOND, Input::Tick).close);
    assert_eq!(wire.relay(thirty, Input::Tick), silent_close());
    assert_eq!(wire.relay.pairing(), &PairingRun::Failed { counted: false });
}

/// PAIR-03, with supplied time: the hub monitor gives its run the same 30
/// seconds, from its first step.
#[test]
fn pair_03_a_run_that_passes_30_seconds_fails_at_the_hub_monitor() {
    let thirty = 30 * SECOND;
    for (when, paired) in [(thirty, true), (thirty + MILLISECOND, false)] {
        let (invitations, pin) = invited();
        let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
        let request = wire.hub(2 * SECOND, Input::Tick).bytes;
        wire.relay(2 * SECOND, Input::Tick);
        let reply = wire.relay(2 * SECOND, Input::Bytes(&request)).bytes;
        let hub_share = wire.hub(4 * SECOND, Input::Bytes(&reply));
        assert_eq!(hub_share.deadline, Some(at(thirty + 2 * SECOND)));
        let relay_share = wire.relay(4 * SECOND, Input::Bytes(&hub_share.bytes)).bytes;
        let confirmation = wire.hub(4 * SECOND, Input::Bytes(&relay_share));
        assert_eq!(confirmation.deadline, Some(at(thirty + 2 * SECOND)));
        let relay_confirmation = wire
            .relay(4 * SECOND, Input::Bytes(&confirmation.bytes))
            .bytes;

        let output = wire.hub(when + 2 * SECOND, Input::Bytes(&relay_confirmation));

        assert_eq!(output, silent_close());
        let expected = match paired {
            true => PairingRun::Paired(relay_introduction()),
            false => PairingRun::Failed { counted: true },
        };
        assert_eq!(wire.hub.pairing(), &expected);
    }
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

/// On a connection opened to pair, bytes that arrive behind a whole opening
/// message are the start of the pairing run. A relay handed everything the
/// hub monitor sends in one arrival answers everything at once, and a hub
/// monitor handed everything the relay sends in one arrival pairs.
#[test]
fn bytes_behind_an_opening_message_start_the_pairing_run() {
    let (invitations, pin) = invited();
    let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));
    harness.run();
    let (from_hub, from_relay) = (harness.sent_by_hub(), harness.sent_by_relay());

    let (invitations, _) = invited();
    let mut relay = pairing_relay(&invitations);
    let answer = relay.step(
        at(Duration::ZERO),
        Input::Bytes(from_hub),
        &mut Seeded::new(2),
    );
    assert_eq!(answer.bytes, from_relay);
    assert!(answer.close);
    assert_eq!(relay.pairing(), &PairingRun::Paired(hub_introduction()));

    let mut hub = pairing_hub(&pin);
    let mut randomness = Seeded::new(1);
    hub.step(at(Duration::ZERO), Input::Tick, &mut randomness);
    let answer = hub.step(
        at(Duration::ZERO),
        Input::Bytes(from_relay),
        &mut randomness,
    );
    assert_eq!(answer.bytes, from_hub[OPENING.0..]);
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

/// PAIR-02: an invitation that expired before a counted run failed stays
/// expired; the failure does not turn it into one used up.
#[test]
fn pair_02_a_failure_after_expiry_leaves_the_invitation_expired() {
    let (invitations, pin) = invited();
    for _ in 0..2 {
        Harness::new(pairing_hub(&self::pin(WRONG)), pairing_relay(&invitations)).run();
    }
    assert_eq!(attempts_left(&invitations), 1);
    let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
    wire.shares(290 * SECOND);

    assert_eq!(wire.relay(301 * SECOND, Input::Closed), silent_close());

    assert_eq!(wire.relay.pairing(), &PairingRun::Failed { counted: true });
    assert_eq!(
        invitations.invitation(at(301 * SECOND)),
        Invitation::Expired
    );
}

/// PAIR-03: a connection opened to link takes no pairing run, so a pairing
/// run can start while it is open.
#[test]
fn pair_03_a_connection_opened_to_link_holds_no_pairing_run() {
    let (invitations, pin) = invited();
    let mut link = Harness::new(
        HubEnd::new(Purpose::Link, revisions(&[1])),
        pairing_relay(&invitations),
    );
    link.run();

    let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));
    harness.run();

    assert_eq!(
        harness.relay.pairing(),
        &PairingRun::Paired(hub_introduction())
    );
    assert_eq!(link.relay.pairing(), &PairingRun::Pending);
}
