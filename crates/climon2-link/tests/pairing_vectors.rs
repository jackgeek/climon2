//! SEC-08 at the Link layer (#495): climon2's own frozen vectors for the
//! pairing key schedule and for binding the opening exchange into the PIN
//! key exchange. The published CPace vectors run in `src/pairing/cpace.rs`,
//! and the frozen keys of the schedule in `src/pairing/keys.rs`.
//!
//! Every expected byte here was computed by `vectors/pairing_reference.py`,
//! which shares no code with the link crate, from the layouts of the
//! protocol's pairing share and pairing confirmation pages. A cryptographic
//! crate that is replaced or upgraded cannot change them unnoticed.

mod support;
mod transcript;

use climon2_link::{Invitation, PairingRun};
use support::Harness;
use support::pairing::{
    hub_introduction, invited, pairing_hub, pairing_relay, proxied, relay_introduction,
};

/// The PIN the relay mints from seed 3.
const PIN: &str = "8517-9087";
/// The opening exchange of a pairing connection in revision 1: 26 bytes one
/// way, 20 the other.
const OPENING: (&str, &str) = (
    "63326c696e6b0d0a010100010000000000000000000000000000",
    "0100010100010000000000000000000000000000",
);
/// The hub monitor's share message, its scalar drawn from seed 1.
const HUB_SHARE: &str = "0020669b3a947fbf595f1476038e68f9bfccab0a3f208a729f8961e9302708463334";
/// The relay's share message, its scalar drawn from seed 2.
const RELAY_SHARE: &str = "0020a8c40ade77998cbb3953e199198f3849c864630c6105dc4f8096443fba194e52";
/// The hub monitor's confirmation message: its Confirmation under the hub
/// confirmation key, then its key and the name `laptop` under the hub box
/// key.
const HUB_CONFIRMATION: &str = "
    0077aa2dc585a144dce482fec724f7f59b3a86ca6a5568235b98f8a7b91efbc5
    6c34ede6f6b64923a55edbd998fc3fab48df7e766ffd1fb41dff099cb3737a63
    914277c2f97bcc36e382a374361ddad86fcf3981a89b11a11c9fb83888189f91
    280e8b9757570577a5dc538a2520a4538f74fd8d41a1188fc1";
/// The relay's confirmation message: its Confirmation under the relay
/// confirmation key, then its key and the name `büild-box` under the relay
/// box key.
const RELAY_CONFIRMATION: &str = "
    007b783f0a2d6a78bdbfa2c76c816ee6a2ccb55a99e1677ebcc89a53784f7406
    83e3ef2bc3368505ae211ae82738ec8a47fae012aecdb4325d997477eccc2e95
    91ce993329b7af0698558f00c312ee24cc615ef4729d96e1740cf61bb07f574c
    f56e74f58d3b95bbd717406ae6e5ad10aa3626c80494c35b80ebeca844";

fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .map(|byte| (byte as char).to_digit(16).expect("a hex digit") as u8)
        .collect();
    assert_eq!(digits.len() % 2, 0, "whole bytes");
    digits
        .chunks(2)
        .map(|pair| pair[0] << 4 | pair[1])
        .collect()
}

/// SEC-08: with the randomness of seeds 1, 2 and 3, a whole pairing
/// connection is these bytes exactly, and each end is handed the other's
/// key and name.
#[test]
fn sec_08_a_pairing_connection_is_the_frozen_bytes() {
    let (invitations, pin) = invited();
    assert_eq!(pin.to_string(), PIN);
    let mut harness = Harness::new(pairing_hub(&pin), pairing_relay(&invitations));

    harness.run();

    let from_hub = [OPENING.0, HUB_SHARE, HUB_CONFIRMATION].concat();
    let from_relay = [OPENING.1, RELAY_SHARE, RELAY_CONFIRMATION].concat();
    assert_eq!(harness.sent_by_hub(), hex(&from_hub));
    assert_eq!(harness.sent_by_relay(), hex(&from_relay));
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
    assert!(harness.hub_closed() && harness.relay_closed());
}

/// SEC-08: the opening exchange is bound into the key exchange. With one
/// more revision in the hub monitor's list, and nothing else changed, both
/// shares and both confirmations are other bytes.
#[test]
fn sec_08_another_opening_transcript_gives_other_pairing_bytes() {
    let (invitations, pin) = invited();
    let hub = climon2_link::HubEnd::pair(
        support::pairing::revisions(&[1, 2]),
        pin,
        hub_introduction(),
    );

    let (hub, relay) = proxied(hub, pairing_relay(&invitations), |_, _, _| {});

    assert_eq!(hub.end.pairing(), &PairingRun::Paired(relay_introduction()));
    let frozen = [HUB_SHARE, HUB_CONFIRMATION, RELAY_SHARE, RELAY_CONFIRMATION].map(hex);
    let sent = [&hub.sent[1], &hub.sent[2], &relay.sent[1], &relay.sent[2]];
    for (sent, frozen) in sent.into_iter().zip(&frozen) {
        assert_eq!(sent.len(), frozen.len());
        assert_ne!(sent[2..34], frozen[2..34]);
    }
}

/// REV-07: the pairing transcript is this frozen connection, so its bytes
/// were computed by `vectors/pairing_reference.py`.
#[test]
fn rev_07_the_pairing_transcript_is_the_frozen_connection() {
    let transcript = transcript::load(1, "pairing-correct-pin");
    let from_hub = [OPENING.0, HUB_SHARE, HUB_CONFIRMATION].concat();
    let from_relay = [OPENING.1, RELAY_SHARE, RELAY_CONFIRMATION].concat();
    assert_eq!(transcript.sent_by(transcript::Side::Hub), hex(&from_hub));
    assert_eq!(
        transcript.sent_by(transcript::Side::Relay),
        hex(&from_relay)
    );
}
