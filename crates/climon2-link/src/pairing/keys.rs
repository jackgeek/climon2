//! The pairing key schedule and the pairing confirmation message, as the
//! protocol's pairing confirmation page gives them: four keys from the CPace
//! key by HKDF with SHA-512, a confirmation by HMAC with SHA-512, and a
//! ChaCha20-Poly1305 box that holds the sender's key and name.

use super::{Introduction, cpace, framed};
use crate::opening::Transcript;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha512;

/// The size of a Confirmation field: one HMAC with SHA-512.
const CONFIRMATION_SIZE: usize = 64;

/// Which end sent a confirmation message. Each end has keys of its own, so
/// a message sent back to its sender proves nothing.
#[derive(Clone, Copy)]
pub(crate) enum Sender {
    Hub,
    Relay,
}

impl Sender {
    /// The info strings of this end's confirmation key and box key.
    fn infos(self) -> (&'static [u8], &'static [u8]) {
        match self {
            Self::Hub => (
                b"climon2 link pairing hub confirm",
                b"climon2 link pairing hub box",
            ),
            Self::Relay => (
                b"climon2 link pairing relay confirm",
                b"climon2 link pairing relay box",
            ),
        }
    }
}

/// What both ends of a run confirm: the key CPace produced, and every byte
/// that led to it.
pub(crate) struct Confirmed {
    pub(crate) key: [u8; cpace::KEY_SIZE],
    pub(crate) transcript: Transcript,
    pub(crate) hub_share: [u8; cpace::SHARE_SIZE],
    pub(crate) relay_share: [u8; cpace::SHARE_SIZE],
}

impl Confirmed {
    /// One 32-byte key of the schedule: HKDF with SHA-512 and an empty salt
    /// over the CPace key.
    fn derive(&self, info: &[u8]) -> [u8; 32] {
        let mut key = <[u8; 32]>::default();
        Hkdf::<Sha512>::new(None, &self.key)
            .expand(info, &mut key)
            .expect("HKDF with SHA-512 gives 32 bytes");
        key
    }

    /// The HMAC of `sender`'s Confirmation field, fed with the opening
    /// transcript, then the hub monitor's share, then the relay's.
    fn mac(&self, sender: Sender) -> Hmac<Sha512> {
        let key = self.derive(sender.infos().0);
        let mut mac = Hmac::<Sha512>::new_from_slice(&key).expect("HMAC takes a key of any size");
        mac.update(self.transcript.as_bytes());
        mac.update(&self.hub_share);
        mac.update(&self.relay_share);
        mac
    }

    /// The cipher of `sender`'s box. Its key seals one box, so the nonce is
    /// twelve zero bytes.
    fn cipher(&self, sender: Sender) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new(&self.derive(sender.infos().1).into())
    }

    /// The pairing confirmation `sender` sends to hand over `introduction`,
    /// as it crosses the byte stream.
    pub(crate) fn message(&self, sender: Sender, introduction: &Introduction) -> Vec<u8> {
        let mut body = self.mac(sender).finalize().into_bytes().to_vec();
        let plaintext = Payload {
            msg: &introduction.encode(),
            aad: &body.clone(),
        };
        let sealed = self
            .cipher(sender)
            .encrypt(&Nonce::default(), plaintext)
            .expect("a box of at most 113 bytes");
        body.extend(sealed);
        framed(&body)
    }

    /// What the body of `sender`'s pairing confirmation hands over. Nothing
    /// unless its Confirmation is the one this end computes, its box opens
    /// under this end's key and the plaintext keeps the layout.
    pub(crate) fn open(&self, sender: Sender, body: &[u8]) -> Option<Introduction> {
        let (confirmation, sealed) = body.split_at_checked(CONFIRMATION_SIZE)?;
        self.mac(sender).verify_slice(confirmation).ok()?;
        let ciphertext = Payload {
            msg: sealed,
            aad: confirmation,
        };
        let plaintext = self
            .cipher(sender)
            .decrypt(&Nonce::default(), ciphertext)
            .ok()?;
        Introduction::decode(&plaintext)
    }
}

#[cfg(test)]
mod tests {
    //! SEC-08: climon2's own frozen vectors for the pairing key schedule.
    //! Every expected byte was computed by `tests/vectors/pairing_reference.py`,
    //! which shares no code with this crate, for the opening exchange, shares
    //! and introductions of `tests/pairing_vectors.rs`.

    use super::*;
    use crate::opening::{REPLY_SIZE, REQUEST_SIZE};

    fn hex(text: &str) -> Vec<u8> {
        let digits: Vec<u8> = text
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace())
            .map(|byte| (byte as char).to_digit(16).expect("a hex digit") as u8)
            .collect();
        digits
            .chunks(2)
            .map(|pair| pair[0] << 4 | pair[1])
            .collect()
    }

    const TRANSCRIPT: &str = "63326c696e6b0d0a010100010000000000000000000000000000
                              0100010100010000000000000000000000000000";
    const HUB_SHARE: &str = "669b3a947fbf595f1476038e68f9bfccab0a3f208a729f8961e9302708463334";
    const RELAY_SHARE: &str = "a8c40ade77998cbb3953e199198f3849c864630c6105dc4f8096443fba194e52";
    const CPACE_KEY: &str = "64863003a3c969421387291e2744768d6718e112d40a8052fac75230e56f4e12
                             fcdd9037683ce82f4a3ce0a9cd7dff18e4a391d55899faaf6c6c61cf0df9b940";
    const HUB_CONFIRM_KEY: &str =
        "172f925fa36fcc351268f4c00eb0f972cece9e653bdb5729e2be3bc3fde140c7";
    const RELAY_CONFIRM_KEY: &str =
        "0cabc7354957bc6236d7df7b08e9df28aaaa553f7aa61cf7888558018b1f4721";
    const HUB_BOX_KEY: &str = "094639b1340216a5489a78919203f98212828dbd3ec59e84a24edf2ead32037e";
    const RELAY_BOX_KEY: &str = "ba84fae3b872ed718b7512078842ab546bb8cc8b84b1a391972c8ba7d6034a0f";
    const HUB_CONFIRMATION: &str = "
        0077aa2dc585a144dce482fec724f7f59b3a86ca6a5568235b98f8a7b91efbc5
        6c34ede6f6b64923a55edbd998fc3fab48df7e766ffd1fb41dff099cb3737a63
        914277c2f97bcc36e382a374361ddad86fcf3981a89b11a11c9fb83888189f91
        280e8b9757570577a5dc538a2520a4538f74fd8d41a1188fc1";
    const RELAY_CONFIRMATION: &str = "
        007b783f0a2d6a78bdbfa2c76c816ee6a2ccb55a99e1677ebcc89a53784f7406
        83e3ef2bc3368505ae211ae82738ec8a47fae012aecdb4325d997477eccc2e95
        91ce993329b7af0698558f00c312ee24cc615ef4729d96e1740cf61bb07f574c
        f56e74f58d3b95bbd717406ae6e5ad10aa3626c80494c35b80ebeca844";

    fn confirmed() -> Confirmed {
        let transcript = hex(TRANSCRIPT);
        let (request, reply) = transcript.split_at(REQUEST_SIZE);
        Confirmed {
            key: hex(CPACE_KEY).try_into().unwrap(),
            transcript: Transcript::new(
                request.try_into().unwrap(),
                <&[u8; REPLY_SIZE]>::try_from(reply).unwrap(),
            ),
            hub_share: hex(HUB_SHARE).try_into().unwrap(),
            relay_share: hex(RELAY_SHARE).try_into().unwrap(),
        }
    }

    fn hub() -> Introduction {
        let key: Vec<u8> = (0x40..0x60).collect();
        Introduction::new(key.try_into().unwrap(), "laptop").unwrap()
    }

    fn relay() -> Introduction {
        let key: Vec<u8> = (0xa0..0xc0).collect();
        Introduction::new(key.try_into().unwrap(), "büild-box").unwrap()
    }

    #[test]
    fn sec_08_the_four_keys_are_the_frozen_bytes() {
        let confirmed = confirmed();
        let keys = [Sender::Hub, Sender::Relay].map(|sender| {
            let (confirm, boxed) = sender.infos();
            [confirmed.derive(confirm), confirmed.derive(boxed)]
        });

        assert_eq!(keys[0][0].to_vec(), hex(HUB_CONFIRM_KEY));
        assert_eq!(keys[0][1].to_vec(), hex(HUB_BOX_KEY));
        assert_eq!(keys[1][0].to_vec(), hex(RELAY_CONFIRM_KEY));
        assert_eq!(keys[1][1].to_vec(), hex(RELAY_BOX_KEY));
    }

    #[test]
    fn sec_08_each_confirmation_is_the_frozen_bytes() {
        let confirmed = confirmed();

        assert_eq!(
            confirmed.message(Sender::Hub, &hub()),
            hex(HUB_CONFIRMATION)
        );
        assert_eq!(
            confirmed.message(Sender::Relay, &relay()),
            hex(RELAY_CONFIRMATION)
        );
    }

    #[test]
    fn a_confirmation_opens_only_as_its_senders() {
        let confirmed = confirmed();
        let hub_body = &hex(HUB_CONFIRMATION)[2..];
        let relay_body = &hex(RELAY_CONFIRMATION)[2..];

        assert_eq!(confirmed.open(Sender::Hub, hub_body), Some(hub()));
        assert_eq!(confirmed.open(Sender::Relay, relay_body), Some(relay()));
        assert_eq!(confirmed.open(Sender::Relay, hub_body), None);
        assert_eq!(confirmed.open(Sender::Hub, relay_body), None);
    }

    #[test]
    fn a_confirmation_that_is_short_or_altered_does_not_open() {
        let confirmed = confirmed();
        let body = hex(HUB_CONFIRMATION)[2..].to_vec();

        assert_eq!(confirmed.open(Sender::Hub, &body[..63]), None);
        assert_eq!(confirmed.open(Sender::Hub, &body[..64]), None);
        for at in [0, 63, 64, body.len() - 1] {
            let mut altered = body.clone();
            altered[at] ^= 0x01;
            assert_eq!(confirmed.open(Sender::Hub, &altered), None, "{at}");
        }
    }

    #[test]
    fn every_input_of_the_confirmation_is_bound() {
        let body = hex(HUB_CONFIRMATION)[2..].to_vec();
        let changes: [fn(&mut Confirmed); 4] = [
            |confirmed| confirmed.key[0] ^= 1,
            |confirmed| {
                let mut bytes = *confirmed.transcript.as_bytes();
                bytes[45] ^= 1;
                let (request, reply) = bytes.split_at(REQUEST_SIZE);
                confirmed.transcript = Transcript::new(
                    request.try_into().unwrap(),
                    <&[u8; REPLY_SIZE]>::try_from(reply).unwrap(),
                );
            },
            |confirmed| confirmed.hub_share[31] ^= 1,
            |confirmed| confirmed.relay_share[0] ^= 1,
        ];
        for change in changes {
            let mut confirmed = confirmed();
            change(&mut confirmed);
            assert_eq!(confirmed.open(Sender::Hub, &body), None);
        }
    }
}
