//! The pairing store (spec #488, Storage): what each side of a monitor link
//! remembers about the other, in the owner-only `links/` folder of a climon2
//! home.
//!
//! - A relay keeps its identity in `relay-identity.json` and the hub
//!   monitors it remembers, with its hold, in `hub-monitors.json`.
//! - A hub monitor keeps one record per relay in `relays.json`.
//!
//! Each file is one line of version 1 JSON; the crate README gives the
//! fields. This module parses and applies policy and performs no I/O: the
//! files are read and replaced through [`KeyFiles`], which the platform's
//! key storage seam (ADR 0003) implements.
//!
//! A file that is missing means nothing was ever stored. Anything else that
//! is not a whole, valid file is damage and an `InvalidData` error naming
//! the file. It is never read as empty and never repaired. No error carries
//! a value from the file, because the files hold private keys.
use crate::json;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use std::{fmt, io};

/// The relay's identity keypair.
pub const RELAY_IDENTITY_FILE: &str = "relay-identity.json";
/// The hub monitors a relay remembers, and its hold.
pub const HUB_MONITORS_FILE: &str = "hub-monitors.json";
/// A hub monitor's record of each relay.
pub const RELAYS_FILE: &str = "relays.json";
/// The most pairings one side keeps.
pub const MAX_PAIRINGS: usize = 64;

const VERSION: u32 = 1;
const KEY_BYTES: usize = 32;

/// The files of one home's `links/` folder, by name. `load` gives `None`
/// for a file that does not exist and an error for one it cannot trust or
/// read; `store` replaces a file's whole contents atomically.
pub trait KeyFiles {
    fn load(&self, name: &str) -> io::Result<Option<Vec<u8>>>;
    fn store(&self, name: &str, contents: &[u8]) -> io::Result<()>;
}

/// A static public key: the other side's, or the public half of a keypair.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicKey([u8; KEY_BYTES]);

impl PublicKey {
    pub fn new(bytes: [u8; KEY_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; KEY_BYTES] {
        &self.0
    }
}

/// A static private key. It has no debug text and no comparison, so it
/// reaches a log or a message only through [`PrivateKey::expose`].
#[derive(Clone)]
pub struct PrivateKey([u8; KEY_BYTES]);

impl PrivateKey {
    pub fn new(bytes: [u8; KEY_BYTES]) -> Self {
        Self(bytes)
    }

    /// The key's bytes, for the handshake that needs them.
    pub fn expose(&self) -> &[u8; KEY_BYTES] {
        &self.0
    }
}

impl fmt::Debug for PrivateKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PrivateKey(withheld)")
    }
}

/// A relay's identity: one keypair per installation.
#[derive(Clone, Debug)]
pub struct Keypair {
    pub public_key: PublicKey,
    pub private_key: PrivateKey,
}

/// A hub monitor a relay remembers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HubMonitor {
    /// The hub monitor's public key for this pairing.
    pub public_key: PublicKey,
    /// The name the hub monitor suggested. The relay cannot change it.
    pub name: String,
    /// When the pairing was made, in seconds since the Unix epoch.
    pub paired_at: u64,
    /// When it last linked, or `None` if it never has.
    pub last_linked_at: Option<u64>,
}

/// A hub monitor's record of one relay.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Relay {
    /// The name the hub monitor's user chose.
    pub name: String,
    /// The one address the hub monitor dials.
    pub address: String,
    /// The relay's pinned public key.
    pub relay_public_key: PublicKey,
    /// The hub monitor's keypair for this pairing.
    pub public_key: PublicKey,
    pub private_key: PrivateKey,
    /// When the pairing was made, in seconds since the Unix epoch.
    pub paired_at: u64,
    /// When it last linked, or `None` if it never has.
    pub last_linked_at: Option<u64>,
    /// Whether the user disconnected it; it stays so across restarts.
    pub disconnected: bool,
}

/// A record of one pairing, known by the other side's public key.
pub trait Pairing {
    fn peer(&self) -> &PublicKey;
}

impl Pairing for HubMonitor {
    fn peer(&self) -> &PublicKey {
        &self.public_key
    }
}

impl Pairing for Relay {
    fn peer(&self) -> &PublicKey {
        &self.relay_public_key
    }
}

/// One side's pairings: at most [`MAX_PAIRINGS`], no two with the same peer.
#[derive(Clone, Debug)]
pub struct Pairings<T>(Vec<T>);

/// The refusal to keep one pairing more than [`MAX_PAIRINGS`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreFull;

impl fmt::Display for StoreFull {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{MAX_PAIRINGS} pairings are already kept, which is the most a side keeps"
        )
    }
}

impl std::error::Error for StoreFull {}

impl<T> Default for Pairings<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T: Pairing> Pairings<T> {
    /// The records, oldest first.
    pub fn records(&self) -> &[T] {
        &self.0
    }

    /// Keeps `record`. A record for the same peer is replaced where it
    /// stands; a new peer is refused once the store is full.
    pub fn insert(&mut self, record: T) -> Result<(), StoreFull> {
        match self.position(record.peer()) {
            Some(index) => self.0[index] = record,
            None if self.0.len() < MAX_PAIRINGS => self.0.push(record),
            None => return Err(StoreFull),
        }
        Ok(())
    }

    /// Forgets the pairing with `peer`, and gives back its record.
    pub fn remove(&mut self, peer: &PublicKey) -> Option<T> {
        self.position(peer).map(|index| self.0.remove(index))
    }

    fn position(&self, peer: &PublicKey) -> Option<usize> {
        self.0.iter().position(|record| record.peer() == peer)
    }

    /// The records of a file, which must already obey the store's rules.
    fn from_file(records: Vec<T>) -> Result<Self, &'static str> {
        if records.len() > MAX_PAIRINGS {
            return Err("more than 64 pairings");
        }
        let mut pairings = Self::default();
        for record in records {
            if pairings.position(record.peer()).is_some() {
                return Err("two pairings share a public key");
            }
            pairings.0.push(record);
        }
        Ok(pairings)
    }
}

/// What a relay remembers: its hub monitors and its hold.
#[derive(Clone, Debug, Default)]
pub struct RelayPairings {
    /// Whether the relay refuses every remembered hub monitor.
    pub hold: bool,
    pub hub_monitors: Pairings<HubMonitor>,
}

/// The relay's identity, at the relay's start. A missing identity is made
/// with `new_identity` and stored; this is the only place one is created,
/// so no command and no monitor ever makes one. A damaged identity is an
/// error and is never replaced.
pub fn relay_identity_at_start(
    files: &impl KeyFiles,
    new_identity: impl FnOnce() -> io::Result<Keypair>,
) -> io::Result<Keypair> {
    if let Some(file) = read::<IdentityFile>(files, RELAY_IDENTITY_FILE)? {
        return Ok(Keypair {
            public_key: file.public_key,
            private_key: file.private_key,
        });
    }
    let identity = new_identity()?;
    let file = IdentityFile {
        version: VERSION,
        public_key: identity.public_key,
        private_key: identity.private_key.clone(),
    };
    write(files, RELAY_IDENTITY_FILE, &file)?;
    Ok(identity)
}

/// The hub monitors a relay remembers; none, and no hold, when it has never
/// stored any.
pub fn load_hub_monitors(files: &impl KeyFiles) -> io::Result<RelayPairings> {
    let Some(file) = read::<HubMonitorsFile>(files, HUB_MONITORS_FILE)? else {
        return Ok(RelayPairings::default());
    };
    Ok(RelayPairings {
        hold: file.hold,
        hub_monitors: Pairings::from_file(file.hub_monitors)
            .map_err(|reason| damaged(HUB_MONITORS_FILE, reason))?,
    })
}

/// Replaces what a relay remembers.
pub fn save_hub_monitors(files: &impl KeyFiles, remembered: &RelayPairings) -> io::Result<()> {
    let file = HubMonitorsFile {
        version: VERSION,
        hold: remembered.hold,
        hub_monitors: remembered.hub_monitors.0.clone(),
    };
    write(files, HUB_MONITORS_FILE, &file)
}

/// A hub monitor's relays; none when it has never stored any.
pub fn load_relays(files: &impl KeyFiles) -> io::Result<Pairings<Relay>> {
    let Some(file) = read::<RelaysFile>(files, RELAYS_FILE)? else {
        return Ok(Pairings::default());
    };
    Pairings::from_file(file.relays).map_err(|reason| damaged(RELAYS_FILE, reason))
}

/// Replaces a hub monitor's relays.
pub fn save_relays(files: &impl KeyFiles, relays: &Pairings<Relay>) -> io::Result<()> {
    let file = RelaysFile {
        version: VERSION,
        relays: relays.0.clone(),
    };
    write(files, RELAYS_FILE, &file)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IdentityFile {
    version: u32,
    public_key: PublicKey,
    private_key: PrivateKey,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HubMonitorsFile {
    version: u32,
    hold: bool,
    hub_monitors: Vec<HubMonitor>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RelaysFile {
    version: u32,
    relays: Vec<Relay>,
}

/// The one field every version of every file has. The other fields are read
/// only once the version is known to be this release's.
#[derive(Deserialize)]
struct Versioned {
    version: u32,
}

/// The file `name`, parsed, or `None` when it does not exist.
fn read<T: DeserializeOwned>(files: &impl KeyFiles, name: &str) -> io::Result<Option<T>> {
    let Some(bytes) = files.load(name)? else {
        return Ok(None);
    };
    let version = json::strict_object::<Versioned>(&bytes)
        .map_err(|error| malformed(name, &error))?
        .version;
    if version != VERSION {
        let reason = format!("version {version} is not one this release reads");
        return Err(damaged(name, &reason));
    }
    json::strict_object(&bytes)
        .map(Some)
        .map_err(|error| malformed(name, &error))
}

/// Replaces the file `name` with one line of JSON.
fn write(files: &impl KeyFiles, name: &str, file: &impl Serialize) -> io::Result<()> {
    let mut line = serde_json::to_vec(file)?;
    line.push(b'\n');
    files.store(name, &line)
}

/// A parse failure, by position only: the parser's own message can quote
/// the file, and the file holds private keys.
fn malformed(name: &str, error: &serde_json::Error) -> io::Error {
    let reason = format!(
        "malformed at line {} column {}",
        error.line(),
        error.column()
    );
    damaged(name, &reason)
}

fn damaged(name: &str, reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("damaged pairing file {name}: {reason}"),
    )
}

/// A key in a file: 64 lowercase hexadecimal digits.
fn key_to_hex(bytes: &[u8; KEY_BYTES]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn key_from_hex(text: &str) -> Option<[u8; KEY_BYTES]> {
    let digits = text.as_bytes();
    let lowercase_hex = |digit: &u8| matches!(digit, b'0'..=b'9' | b'a'..=b'f');
    if digits.len() != 2 * KEY_BYTES || !digits.iter().all(lowercase_hex) {
        return None;
    }
    let mut bytes = [0; KEY_BYTES];
    for (byte, pair) in bytes.iter_mut().zip(text.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(bytes)
}

fn deserialize_key<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<[u8; KEY_BYTES], D::Error> {
    let text = String::deserialize(deserializer)?;
    key_from_hex(&text).ok_or_else(|| serde::de::Error::custom("not a key"))
}

impl Serialize for PublicKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&key_to_hex(&self.0))
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_key(deserializer).map(Self)
    }
}

impl Serialize for PrivateKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&key_to_hex(&self.0))
    }
}

impl<'de> Deserialize<'de> for PrivateKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_key(deserializer).map(Self)
    }
}
