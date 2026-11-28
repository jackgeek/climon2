//! The link invitation at the Link layer (#495): its lifetime, its
//! attempts and the one pairing run it lets start at a time, with the time
//! and the randomness supplied by the test. The scenario IDs are those of
//! the monitor-link acceptance criteria.

mod support;

use climon2_link::{
    End, HubEnd, Input, Introduction, Invitation, Invitations, Opening, Output, PairingRun, Purpose,
};
use std::time::Duration;
use support::pairing::*;
use support::{Harness, Seeded};

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
