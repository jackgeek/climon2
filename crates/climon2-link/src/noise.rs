//! The cryptography of a link connection: the Noise pattern `IK` as the
//! handshake pages name it, `Noise_IK_25519_ChaChaPoly_SHA256`, and the
//! transport messages that carry the units. The `snow` crate does the work;
//! this module is the only one that names it.

use crate::keys::{KEY_SIZE, PublicKey, StaticKey};
use snow::params::{CipherChoice, DHChoice, HashChoice};
use snow::resolvers::{CryptoResolver, DefaultResolver};
use snow::types::{Cipher, Dh, Hash, Random};
use snow::{Builder, HandshakeState, TransportState};

/// The Noise protocol name of link revision 1.
const PROTOCOL: &str = "Noise_IK_25519_ChaChaPoly_SHA256";
/// The size of the authentication tag that ends every encrypted part.
pub(crate) const TAG_SIZE: usize = 16;
/// The longest message Noise allows.
const MOST: usize = 65_535;

/// The suite `snow` builds, with the ephemeral private key the caller drew
/// in place of any generator of `snow`'s own.
struct Suite {
    ephemeral: [u8; KEY_SIZE],
}

/// A generator that gives out one ephemeral private key, once.
struct Drawn(Option<[u8; KEY_SIZE]>);

impl Random for Drawn {
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), snow::Error> {
        let drawn = self.0.take().ok_or(snow::Error::Rng)?;
        if bytes.len() != drawn.len() {
            return Err(snow::Error::Rng);
        }
        bytes.copy_from_slice(&drawn);
        Ok(())
    }
}

impl CryptoResolver for Suite {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        Some(Box::new(Drawn(Some(self.ephemeral))))
    }

    fn resolve_dh(&self, choice: &DHChoice) -> Option<Box<dyn Dh>> {
        DefaultResolver.resolve_dh(choice)
    }

    fn resolve_hash(&self, choice: &HashChoice) -> Option<Box<dyn Hash>> {
        DefaultResolver.resolve_hash(choice)
    }

    fn resolve_cipher(&self, choice: &CipherChoice) -> Option<Box<dyn Cipher>> {
        DefaultResolver.resolve_cipher(choice)
    }
}

/// The public key that belongs to the private key `private`.
pub(crate) fn public_of(private: &[u8; KEY_SIZE]) -> Option<[u8; KEY_SIZE]> {
    let mut exchange = DefaultResolver.resolve_dh(&DHChoice::Curve25519)?;
    exchange.set(private);
    exchange.pubkey().try_into().ok()
}

/// A handshake over `prologue` by the end that holds `own`, drawing
/// `ephemeral` as its ephemeral private key.
fn builder<'a>(
    prologue: &'a [u8],
    own: &'a StaticKey,
    ephemeral: [u8; KEY_SIZE],
) -> Option<Builder<'a>> {
    Builder::with_resolver(PROTOCOL.parse().ok()?, Box::new(Suite { ephemeral }))
        .local_private_key(own.private())
        .ok()?
        .prologue(prologue)
        .ok()
}

/// One handshake message holding `payload`.
fn write(handshake: &mut HandshakeState, payload: &[u8]) -> Option<Vec<u8>> {
    let mut message = vec![0; MOST];
    let size = handshake.write_message(payload, &mut message).ok()?;
    message.truncate(size);
    Some(message)
}

/// The payload of the handshake message `message`, when it decrypts.
fn read(handshake: &mut HandshakeState, message: &[u8]) -> Option<Vec<u8>> {
    let mut payload = vec![0; MOST];
    let size = handshake.read_message(message, &mut payload).ok()?;
    payload.truncate(size);
    Some(payload)
}

/// The hub monitor's half of a handshake, between its two messages.
pub(crate) struct Initiator(HandshakeState);

impl Initiator {
    /// Begins a handshake with the relay whose pinned key is `relay`: the
    /// state to finish it with, and the first message, holding `payload`.
    pub(crate) fn begin(
        prologue: &[u8],
        own: &StaticKey,
        relay: &PublicKey,
        ephemeral: [u8; KEY_SIZE],
        payload: &[u8],
    ) -> Option<(Self, Vec<u8>)> {
        let mut handshake = builder(prologue, own, ephemeral)?
            .remote_public_key(relay.as_bytes())
            .ok()?
            .build_initiator()
            .ok()?;
        let message = write(&mut handshake, payload)?;
        Some((Self(handshake), message))
    }

    /// Finishes the handshake with the relay's `reply`: its payload and the
    /// transport, or nothing when the reply does not decrypt.
    pub(crate) fn finish(mut self, reply: &[u8]) -> Option<(Vec<u8>, Transport)> {
        let payload = read(&mut self.0, reply)?;
        Some((payload, Transport(self.0.into_transport_mode().ok()?)))
    }
}

/// A handshake as the relay has answered it.
pub(crate) struct Answered {
    /// The static public key the hub monitor proved it holds.
    pub(crate) hub: PublicKey,
    /// The second message, to send back.
    pub(crate) reply: Vec<u8>,
    pub(crate) transport: Transport,
}

/// Answers the first handshake message `initiation` as the relay whose
/// identity key is `own`, with a reply holding `payload`: the payload of the
/// hub monitor's message and the answer. Nothing when the message does not
/// decrypt.
pub(crate) fn answer(
    prologue: &[u8],
    own: &StaticKey,
    ephemeral: [u8; KEY_SIZE],
    initiation: &[u8],
    payload: &[u8],
) -> Option<(Vec<u8>, Answered)> {
    let mut handshake = builder(prologue, own, ephemeral)?.build_responder().ok()?;
    let received = read(&mut handshake, initiation)?;
    let hub = PublicKey::from_bytes(handshake.get_remote_static()?.try_into().ok()?);
    let reply = write(&mut handshake, payload)?;
    let transport = Transport(handshake.into_transport_mode().ok()?);
    let answered = Answered {
        hub,
        reply,
        transport,
    };
    Some((received, answered))
}

/// The two cipher states of an established connection, one for each
/// direction, each with its own counter.
pub(crate) struct Transport(TransportState);

impl Transport {
    /// The next transport message this end sends, holding `plaintext`.
    /// Nothing when `plaintext` is too long for one message.
    pub(crate) fn seal(&mut self, plaintext: &[u8]) -> Option<Vec<u8>> {
        let mut message = vec![0; plaintext.len() + TAG_SIZE];
        let size = self.0.write_message(plaintext, &mut message).ok()?;
        (size == message.len()).then_some(message)
    }

    /// The plaintext of `message`, when it is the next transport message
    /// the other end sent, unaltered.
    pub(crate) fn open(&mut self, message: &[u8]) -> Option<Vec<u8>> {
        let mut plaintext = vec![0; message.len()];
        let size = self.0.read_message(message, &mut plaintext).ok()?;
        plaintext.truncate(size);
        Some(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        let digit = |at: usize| u8::from_str_radix(&text[at..at + 2], 16).expect("hex");
        (0..text.len()).step_by(2).map(digit).collect()
    }

    fn key(text: &str) -> [u8; KEY_SIZE] {
        hex(text).try_into().expect("32 bytes")
    }

    // The `Noise_IK_25519_ChaChaPoly_SHA256` vector of the cacophony test
    // vectors, as the snow repository publishes it in
    // `tests/vectors/cacophony.txt`.
    const PROLOGUE: &str = "4a6f686e2047616c74";
    const INIT_STATIC: &str = "e61ef9919cde45dd5f82166404bd08e38bceb5dfdfded0a34c8df7ed542214d1";
    const INIT_EPHEMERAL: &str = "893e28b9dc6ca8d611ab664754b8ceb7bac5117349a4439a6b0569da977c464a";
    const RESP_PUBLIC: &str = "31e0303fd6418d2f8c0e78b91f22e8caed0fbe48656dcf4767e4834f701b8f62";
    const RESP_STATIC: &str = "4a3acbfdb163dec651dfa3194dece676d437029c62a408b4c5ea9114246e4893";
    const RESP_EPHEMERAL: &str = "bbdb4cdbd309f1a1f2e1456967fe288cadd6f712d65dc7b7793d5e63da6b375b";
    /// Each message's payload and ciphertext, the initiator's first.
    const MESSAGES: [(&str, &str); 6] = [
        (
            "4c756477696720766f6e204d69736573",
            "ca35def5ae56cec33dc2036731ab14896bc4c75dbb07a61f879f8e3afa4c7944718da798efbcd91528520204f904b9bd6c7413dccdc214d951e15253e39987f18146e8cd0873654207148333479d4d16c289f0294b29960a72f48e0b7bba2e89083169825e59642148d492020664ccf7",
        ),
        (
            "4d757272617920526f746862617264",
            "95ebc60d2b1fa672c1f46a8aa265ef51bfe38e7ccb39ec5be34069f1448088435361e70b2ed446e6c9ec387d1d6b3b840f194e373979d241b203c4acafccf5",
        ),
        (
            "462e20412e20486179656b",
            "050e9f3c8fac16b68dbce8f8c4bfbf6617c897f9ada4aa29aa19c8",
        ),
        (
            "4361726c204d656e676572",
            "344233a6cabb7141d80f3da2fedc311d9646bbb0f505afe403a667",
        ),
        (
            "4a65616e2d426170746973746520536179",
            "62cdeeb172ad7ade7aa7d9e069da5790f12331bfa00177787a1d0810c67dc3b2b4",
        ),
        (
            "457567656e2042f6686d20766f6e2042617765726b",
            "029bead1b40992327044d409d9a1f3ad8f36c3c452775d557e18bbeb2e8dfcead32d514024",
        ),
    ];

    fn vector_handshake() -> (Transport, Transport) {
        let prologue = hex(PROLOGUE);
        let hub_key = StaticKey::from_private(key(INIT_STATIC));
        let relay_key = StaticKey::from_private(key(RESP_STATIC));
        let (initiator, initiation) = Initiator::begin(
            &prologue,
            &hub_key,
            &PublicKey::from_bytes(key(RESP_PUBLIC)),
            key(INIT_EPHEMERAL),
            &hex(MESSAGES[0].0),
        )
        .expect("the first message");
        assert_eq!(initiation, hex(MESSAGES[0].1));

        let (received, answered) = answer(
            &prologue,
            &relay_key,
            key(RESP_EPHEMERAL),
            &initiation,
            &hex(MESSAGES[1].0),
        )
        .expect("the second message");
        assert_eq!(received, hex(MESSAGES[0].0));
        assert_eq!(answered.reply, hex(MESSAGES[1].1));
        assert_eq!(answered.hub, hub_key.public());

        let (payload, hub) = initiator.finish(&answered.reply).expect("the handshake");
        assert_eq!(payload, hex(MESSAGES[1].0));
        (hub, answered.transport)
    }

    /// SEC-08: the published Noise vector, through both handshake messages
    /// and two transport messages each way.
    #[test]
    fn sec_08_the_published_noise_ik_vector_gives_its_bytes() {
        let (mut hub, mut relay) = vector_handshake();
        for (index, (payload, ciphertext)) in MESSAGES.iter().enumerate().skip(2) {
            let (sender, receiver) = match index % 2 {
                0 => (&mut hub, &mut relay),
                _ => (&mut relay, &mut hub),
            };
            let message = sender.seal(&hex(payload)).expect("a transport message");
            assert_eq!(message, hex(ciphertext), "message {index}");
            assert_eq!(
                receiver.open(&message),
                Some(hex(payload)),
                "message {index}"
            );
        }
    }

    #[test]
    fn a_public_key_is_derived_from_its_private_key() {
        assert_eq!(public_of(&key(RESP_STATIC)), Some(key(RESP_PUBLIC)));
    }

    #[test]
    fn a_transport_message_opens_once_in_order_and_unaltered() {
        let (mut hub, mut relay) = vector_handshake();
        let first = hub.seal(b"first").expect("a message");
        let second = hub.seal(b"second").expect("a message");
        assert_eq!(relay.open(&second), None, "out of order");
        let mut altered = first.clone();
        altered[0] ^= 1;
        assert_eq!(relay.open(&altered), None, "altered");
        assert_eq!(relay.open(&first), Some(b"first".to_vec()));
        assert_eq!(relay.open(&first), None, "repeated");
        assert_eq!(relay.open(&second), Some(b"second".to_vec()));
    }

    #[test]
    fn a_message_longer_than_noise_allows_is_not_sealed() {
        let (mut hub, _) = vector_handshake();
        assert!(hub.seal(&vec![0; MOST - TAG_SIZE + 1]).is_none());
        assert_eq!(
            hub.seal(&vec![0; MOST - TAG_SIZE]).map(|m| m.len()),
            Some(MOST)
        );
    }

    #[test]
    fn a_handshake_fails_on_another_prologue_or_another_relay_key() {
        let hub_key = StaticKey::from_private(key(INIT_STATIC));
        let relay_key = StaticKey::from_private(key(RESP_STATIC));
        let begin = |prologue: &[u8], relay: &StaticKey| {
            Initiator::begin(
                prologue,
                &hub_key,
                &relay.public(),
                key(INIT_EPHEMERAL),
                &[],
            )
            .expect("the first message")
            .1
        };
        let answers = |prologue: &[u8], initiation: &[u8]| {
            answer(prologue, &relay_key, key(RESP_EPHEMERAL), initiation, &[]).is_some()
        };
        assert!(answers(b"one", &begin(b"one", &relay_key)));
        assert!(!answers(b"two", &begin(b"one", &relay_key)));
        assert!(!answers(b"one", &begin(b"one", &hub_key)));
    }
}
