//! Test tools for a link connection (#494): the on-path proxy between two
//! real ends, and the link peer stand-in, which speaks the link's bytes
//! itself so that a test can send one deliberate defect. Neither ships.

// Each test binary compiles this module for itself and uses its own part.
#![allow(dead_code)]

use crate::support::Seeded;
use climon2_link::{
    End, Hello, HubEnd, Input, Output, Pairing, PublicKey, Randomness, RelayEnd, Revisions,
    StaticKey, Time,
};
use snow::params::{CipherChoice, DHChoice, HashChoice};
use snow::resolvers::{CryptoResolver, DefaultResolver};
use snow::types::{Cipher, Dh, Hash, Random};
use std::collections::VecDeque;
use std::time::Duration;

pub const SECOND: Duration = Duration::from_secs(1);
pub const NANOSECOND: Duration = Duration::from_nanos(1);

/// The opening request page's example: a hub monitor that links and
/// supports link revision 1.
pub const REQUEST: [u8; 26] = *b"c2link\r\n\x02\x01\x00\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
/// The opening reply page's first example: revision 1 chosen.
pub const REPLY: [u8; 20] = *b"\x01\x00\x01\x01\x00\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
/// The Hello page's example record: release `2.4.0`, name `atlas`, no
/// capabilities, records up to 1,048,576 bytes.
pub const HELLO: [u8; 27] = [
    0x01, 0x00, 0x00, 0x00, 0x05, 0x32, 0x2e, 0x34, 0x2e, 0x30, 0x00, 0x00, 0x00, 0x05, 0x61, 0x74,
    0x6c, 0x61, 0x73, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00,
];
/// The size on the wire of a handshake initiation and of a handshake reply,
/// lengths included.
pub const INITIATION: usize = 98;
pub const HANDSHAKE_REPLY: usize = 50;

pub fn hex(text: &str) -> Vec<u8> {
    let text: String = text.split_whitespace().collect();
    let digit = |at: usize| u8::from_str_radix(&text[at..at + 2], 16).expect("hex");
    (0..text.len()).step_by(2).map(digit).collect()
}

pub fn revision_1() -> Revisions {
    Revisions::new(&[1]).expect("a valid revision list")
}

/// The Hello of the Hello page's example.
pub fn hello() -> Hello {
    Hello::new("2.4.0", "atlas", 1_048_576).expect("a valid Hello")
}

/// A static key whose private key is `byte` 32 times.
pub fn key(byte: u8) -> StaticKey {
    StaticKey::from_private([byte; 32])
}

/// The hub monitor's static key and the relay's identity key of most tests.
pub fn hub_key() -> StaticKey {
    key(0x11)
}
pub fn relay_key() -> StaticKey {
    key(0x22)
}

/// A hub monitor's end that links with the relay it pinned as `relay`.
pub fn hub_end(own: StaticKey, relay: PublicKey) -> HubEnd {
    HubEnd::dialling(revision_1(), own, relay, hello())
}

/// The Hello of the relays of most tests: the smallest ceiling allowed.
pub fn relay_hello() -> Hello {
    Hello::new("2.5.1", "forge", 4_096).expect("a valid Hello")
}

/// That Hello's record, tag included.
pub fn relay_hello_record() -> Vec<u8> {
    b"\x01\0\0\0\x052.5.1\0\0\0\x05forge\0\0\0\0\0\0\x10\0".to_vec()
}

/// A relay's end that remembers the hub monitors `remembered`.
pub fn relay_end(identity: StaticKey, remembered: &[PublicKey]) -> RelayEnd {
    RelayEnd::new(revision_1(), Pairing::Unavailable).linking(
        identity,
        remembered.to_vec(),
        relay_hello(),
    )
}

/// The two ends of one remembered pairing.
pub fn paired() -> (HubEnd, RelayEnd) {
    (
        hub_end(hub_key(), relay_key().public()),
        relay_end(relay_key(), &[hub_key().public()]),
    )
}

/// Randomness that gives out exactly the bytes a test wrote, in order.
pub struct Fixed(pub VecDeque<u8>);

impl Fixed {
    pub fn new(bytes: &[u8]) -> Self {
        Self(bytes.iter().copied().collect())
    }
}

impl Randomness for Fixed {
    fn fill(&mut self, bytes: &mut [u8]) {
        for byte in bytes {
            *byte = self.0.pop_front().expect("the test supplied enough bytes");
        }
    }
}

/// Steps `end` once at the time `at`, with randomness of no interest.
pub fn step(end: &mut impl End, at: Duration, input: Input<'_>) -> Output {
    end.step(
        Time::since_start(at),
        input,
        &mut Seeded::new(at.as_nanos() as u64 + 7),
    )
}

/// What the on-path proxy changes, batch by batch. It sees each batch of
/// bytes one end hands the byte stream and returns what arrives instead.
pub type Edit = Box<dyn FnMut(Way, Vec<u8>) -> Vec<u8>>;

/// A direction of the byte stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    ToRelay,
    ToHub,
}

/// The on-path proxy, as the relay's end sees it: it stands between the
/// relay's end and the harness's byte streams, so every byte either way
/// passes through `edit`. A close passes as it is: the proxy cannot hide
/// one.
pub struct OnPath<E> {
    pub end: E,
    pub edit: Edit,
}

impl<E> OnPath<E> {
    /// A proxy that changes nothing until a test gives it an edit.
    pub fn new(end: E) -> Self {
        Self {
            end,
            edit: Box::new(|_, bytes| bytes),
        }
    }
}

impl<E: End> End for OnPath<E> {
    fn step(&mut self, now: Time, input: Input<'_>, randomness: &mut dyn Randomness) -> Output {
        let arriving;
        let input = match input {
            Input::Bytes(bytes) => {
                arriving = (self.edit)(Way::ToRelay, bytes.to_vec());
                match arriving.is_empty() {
                    true => Input::Tick,
                    false => Input::Bytes(&arriving),
                }
            }
            other => other,
        };
        let output = self.end.step(now, input, randomness);
        Output {
            bytes: match output.bytes.is_empty() {
                true => output.bytes,
                false => (self.edit)(Way::ToHub, output.bytes),
            },
            ..output
        }
    }
}

/// Splits bytes that hold only whole units into those units, lengths
/// included.
pub fn units(mut bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut units = Vec::new();
    while !bytes.is_empty() {
        let length = usize::from(u16::from_be_bytes([bytes[0], bytes[1]]));
        let (unit, rest) = bytes.split_at(2 + length);
        units.push(unit.to_vec());
        bytes = rest;
    }
    units
}

/// The Noise suite of link revision 1 as `snow` builds it, with a fixed
/// ephemeral private key.
struct Suite([u8; 32]);

struct Ephemeral([u8; 32]);

impl Random for Ephemeral {
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), snow::Error> {
        bytes.copy_from_slice(&self.0);
        Ok(())
    }
}

impl CryptoResolver for Suite {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        Some(Box::new(Ephemeral(self.0)))
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

fn builder<'a>(private: &'a [u8; 32], prologue: &'a [u8], ephemeral: u8) -> snow::Builder<'a> {
    let name = "Noise_IK_25519_ChaChaPoly_SHA256".parse().expect("a name");
    snow::Builder::with_resolver(name, Box::new(Suite([ephemeral; 32])))
        .local_private_key(private)
        .expect("a private key")
        .prologue(prologue)
        .expect("a prologue")
}

fn framed(message: &[u8]) -> Vec<u8> {
    let mut bytes = (message.len() as u16).to_be_bytes().to_vec();
    bytes.extend(message);
    bytes
}

/// The link peer stand-in: it runs the opening exchange and the handshake
/// against one real end, then sends and reads whatever units a test asks.
pub struct StandIn {
    transport: snow::TransportState,
    /// What the real end sent in the step that completed the handshake.
    pub first: Output,
}

impl StandIn {
    /// Dials `relay` at time zero as a hub monitor whose static private key
    /// is `own` and which pinned `pinned`, up to the end of the handshake.
    pub fn dial(relay: &mut RelayEnd, own: [u8; 32], pinned: PublicKey) -> Self {
        step(relay, Duration::ZERO, Input::Tick);
        let reply = step(relay, Duration::ZERO, Input::Bytes(&REQUEST)).bytes;
        let prologue = [&REQUEST[..], &reply].concat();
        let mut handshake = builder(&own, &prologue, 0x33)
            .remote_public_key(pinned.as_bytes())
            .expect("a public key")
            .build_initiator()
            .expect("an initiator");
        let mut message = [0; 96];
        handshake
            .write_message(&[], &mut message)
            .expect("an initiation");
        let first = step(relay, Duration::ZERO, Input::Bytes(&framed(&message)));
        assert_eq!(first.bytes.len(), HANDSHAKE_REPLY, "a handshake reply");
        handshake
            .read_message(&first.bytes[2..], &mut [])
            .expect("a reply that decrypts");
        Self {
            transport: handshake.into_transport_mode().expect("a transport"),
            first,
        }
    }

    /// Answers the dial of `hub` at time zero as a relay whose identity
    /// private key is `identity`, up to the end of the handshake and the
    /// hub monitor's Hello, which `first` holds.
    pub fn answer(hub: &mut HubEnd, identity: [u8; 32]) -> Self {
        let request = step(hub, Duration::ZERO, Input::Tick).bytes;
        let initiation = step(hub, Duration::ZERO, Input::Bytes(&REPLY)).bytes;
        assert_eq!(initiation.len(), INITIATION, "a handshake initiation");
        let prologue = [&request[..], &REPLY].concat();
        let mut handshake = builder(&identity, &prologue, 0x44)
            .build_responder()
            .expect("a responder");
        handshake
            .read_message(&initiation[2..], &mut [])
            .expect("an initiation that decrypts");
        let mut message = [0; 48];
        handshake.write_message(&[], &mut message).expect("a reply");
        let first = step(hub, Duration::ZERO, Input::Bytes(&framed(&message)));
        let mut stand_in = Self {
            transport: handshake.into_transport_mode().expect("a transport"),
            first,
        };
        let hello = stand_in.open(&stand_in.first.bytes.clone());
        assert_eq!(
            hello,
            [[&[0; 5][..], &HELLO].concat()],
            "the hub monitor's Hello"
        );
        stand_in
    }

    /// Whether a relay whose identity private key is `identity` can read
    /// the handshake initiation `initiation`, length included, over
    /// `prologue`.
    pub fn can_read(initiation: &[u8], identity: [u8; 32], prologue: &[u8]) -> bool {
        let mut handshake = builder(&identity, prologue, 0x44)
            .build_responder()
            .expect("a responder");
        handshake
            .read_message(&initiation[2..], &mut [0; 96])
            .is_ok()
    }

    /// The next unit this end sends, holding `plaintext` whatever it is.
    pub fn seal(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let mut ciphertext = vec![0; plaintext.len() + 16];
        self.transport
            .write_message(plaintext, &mut ciphertext)
            .expect("a transport message");
        framed(&ciphertext)
    }

    /// The next unit this end sends: one fragment on `stream`.
    pub fn unit(&mut self, stream: u32, flags: u8, fragment: &[u8]) -> Vec<u8> {
        let mut plaintext = stream.to_be_bytes().to_vec();
        plaintext.push(flags);
        plaintext.extend(fragment);
        self.seal(&plaintext)
    }

    /// The units that carry `record` on `stream`, each fragment as long as
    /// a unit allows.
    pub fn fragmented(&mut self, stream: u32, record: &[u8]) -> Vec<u8> {
        let fragments: Vec<_> = record.chunks(16_363).collect();
        let last = fragments.len() - 1;
        let unit =
            |(at, fragment): (usize, &&[u8])| self.unit(stream, u8::from(at < last), fragment);
        fragments
            .iter()
            .enumerate()
            .map(unit)
            .collect::<Vec<_>>()
            .concat()
    }

    /// A whole record on stream 0 in one unit.
    pub fn record(&mut self, record: &[u8]) -> Vec<u8> {
        self.unit(0, 0, record)
    }

    /// The plaintext of every unit in `bytes`, which the real end sent.
    pub fn open(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let open = |unit: Vec<u8>| {
            let mut plaintext = vec![0; unit.len()];
            let size = self
                .transport
                .read_message(&unit[2..], &mut plaintext)
                .expect("a unit that decrypts");
            plaintext.truncate(size);
            plaintext
        };
        units(bytes).into_iter().map(open).collect()
    }
}
