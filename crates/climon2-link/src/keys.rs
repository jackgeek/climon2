//! Static keys: the long-term X25519 key pairs that identify the two ends of
//! a pairing. A relay has one. A hub monitor has one for each relay it has
//! paired with.

use crate::noise;
use std::fmt;

/// The size of a private or a public key.
pub(crate) const KEY_SIZE: usize = 32;

/// A static public key: what one end pins of the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PublicKey([u8; KEY_SIZE]);

impl PublicKey {
    /// The public key with these bytes.
    pub const fn from_bytes(bytes: [u8; KEY_SIZE]) -> Self {
        Self(bytes)
    }

    /// The key's bytes.
    pub const fn as_bytes(&self) -> &[u8; KEY_SIZE] {
        &self.0
    }
}

/// A static key pair. Its private half never leaves this type, and `Debug`
/// shows the public half only.
#[derive(Clone, PartialEq, Eq)]
pub struct StaticKey {
    private: [u8; KEY_SIZE],
    public: PublicKey,
}

impl StaticKey {
    /// The key pair whose private key is `private`: 32 random bytes the
    /// caller drew once and stores.
    pub fn from_private(private: [u8; KEY_SIZE]) -> Self {
        let public = noise::public_of(&private).expect("X25519 is always built");
        Self {
            private,
            public: PublicKey(public),
        }
    }

    /// The public half, which the other end pins.
    pub fn public(&self) -> PublicKey {
        self.public
    }

    pub(crate) fn private(&self) -> &[u8; KEY_SIZE] {
        &self.private
    }
}

impl fmt::Debug for StaticKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StaticKey")
            .field("public", &self.public)
            .finish_non_exhaustive()
    }
}
