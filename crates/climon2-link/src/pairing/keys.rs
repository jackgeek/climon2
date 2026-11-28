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
        let mut key = [0; 32];
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
