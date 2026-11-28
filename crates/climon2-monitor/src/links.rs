//! The hub monitor's side of monitor links (spec #488, ADR 0011). So far:
//! pairing with a relay by its link invitation, as `c2 monitor pair` does
//! it (#501). The bytes are the link code's (`climon2-link`); this module
//! dials, keeps the store in the home's `links/` folder, and decides when a
//! relay counts as paired: only once its first pinned-key link succeeds
//! (P20, ST15).

use climon2_core::{
    layout::HomeLayout,
    pairing::{self, PrivateKey, PublicKey as StoredKey, Relay},
};
use climon2_link::{
    Carrier, Ended, Hello, HubClose, HubEnd, Introduction, Link, Next, Opening, PairingRun,
    PublicKey, Randomness, Revisions, StaticKey, Time, drive,
};
use climon2_platform::{
    clock, host_name,
    key_storage::KeyStore,
    link_socket::{self, LinkStream},
    randomness,
};
use std::{fmt, io, time::Duration};

/// The PIN a user types for a relay's link invitation.
pub use climon2_link::Pin;

/// How long a dial may take to connect (spec #488, Link lifecycle).
const CONNECT_WAIT: Duration = Duration::from_secs(10);
/// How long one wait for bytes lasts when the link code names no deadline.
const IDLE_WAIT: Duration = Duration::from_secs(60);
/// The longest name this side suggests, in bytes.
const NAME_MOST: usize = 64;

/// A relay this hub monitor has just paired with, as its user sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paired {
    /// The name it is remembered by: the one the relay suggested.
    pub name: String,
    /// The relay's key fingerprint, to compare with the relay's screen.
    pub fingerprint: String,
}

/// Why a pairing did not happen. Each names the address as typed. Nothing
/// is remembered of a relay that was not paired with.
#[derive(Debug)]
pub enum Unpaired {
    /// The home's pairing store cannot be read or written.
    Store(io::Error),
    /// Nothing answered at the address.
    Unreachable(String, io::Error),
    /// The relay's user has no link invitation open (P25).
    NoInvitation(String),
    /// The pairing run failed: a wrong PIN, interference, or no relay.
    Failed(String),
    /// The relay shares no link revision with this monitor.
    Incompatible(String, Vec<u16>),
    /// The pairing ran, but the first pinned-key link did not succeed.
    FirstLink(String),
}

impl fmt::Display for Unpaired {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let failed =
            |address: &str, why: &str| format!("pairing with the relay at {address} failed: {why}");
        let text = match self {
            Self::Store(error) => error.to_string(),
            Self::Unreachable(address, error) => {
                format!("cannot reach the relay at {address}: {error}")
            }
            Self::NoInvitation(address) => {
                format!("the relay at {address} has no link invitation open")
            }
            Self::Failed(address) => failed(address, "wrong PIN or interference"),
            Self::Incompatible(address, theirs) => failed(address, &incompatible(theirs)),
            Self::FirstLink(address) => failed(address, "its first link did not succeed"),
        };
        formatter.write_str(&text)
    }
}

fn incompatible(theirs: &[u16]) -> String {
    let ours = Revisions::published();
    format!(
        "it shares no link revision with this monitor (relay {theirs:?}, monitor {:?})",
        ours.as_slice()
    )
}

/// Pairs with the relay at `address`, a host and a port as the user typed
/// them, by the `pin` of its link invitation. The hub monitor makes a fresh
/// keypair for this pairing, runs the pairing, then dials the first
/// pinned-key link and closes it as "hub monitor disconnected". Only then
/// does it record the relay in `home`'s `relays.json`, connected, under the
/// name the relay suggested: replacing the record of a relay it already
/// knew. `largest_record` is the receive ceiling its Hello states.
pub fn pair(
    home: &HomeLayout,
    address: &str,
    pin: Pin,
    largest_record: u32,
) -> Result<Paired, Unpaired> {
    let store = KeyStore::open(&home.links()).map_err(Unpaired::Store)?;
    let mut relays = pairing::load_relays(&store).map_err(Unpaired::Store)?;
    let (own, private) = fresh_key();
    let name = own_name();
    let introduction =
        Introduction::new(*own.public().as_bytes(), &name).expect("a name cut to 64 bytes fits");
    let relay = run_pairing(
        address,
        HubEnd::pair(Revisions::published(), pin, introduction),
    )?;
    let relay_key = PublicKey::from_bytes(*relay.key());
    let hello = Hello::new(env!("CARGO_PKG_VERSION"), &name, largest_record)
        .expect("this release's Hello is valid");
    first_link(
        address,
        HubEnd::dialling(Revisions::published(), own.clone(), relay_key, hello),
    )?;
    let now = clock::wall().as_secs();
    let record = Relay {
        name: printable(relay.name()),
        address: address.to_owned(),
        relay_public_key: StoredKey::new(*relay_key.as_bytes()),
        public_key: StoredKey::new(*own.public().as_bytes()),
        private_key: private,
        paired_at: now,
        last_linked_at: Some(now),
        disconnected: false,
    };
    let paired = Paired {
        name: record.name.clone(),
        fingerprint: relay_key.fingerprint(),
    };
    relays.insert(record).map_err(|_| Unpaired::Store(full()))?;
    pairing::save_relays(&store, &relays).map_err(Unpaired::Store)?;
    Ok(paired)
}

fn full() -> io::Error {
    io::Error::other(format!(
        "this monitor already remembers {} relays",
        pairing::MAX_PAIRINGS
    ))
}

/// A new static keypair, and its private key as the store keeps it.
fn fresh_key() -> (StaticKey, PrivateKey) {
    let mut private = [0; 32];
    System.fill(&mut private);
    (StaticKey::from_private(private), PrivateKey::new(private))
}

/// The name this hub monitor suggests: its host name, cut to 64 bytes.
fn own_name() -> String {
    cut(host_name::get().unwrap_or_default())
}

/// `name`, cut at a character boundary to at most 64 bytes.
fn cut(mut name: String) -> String {
    while name.len() > NAME_MOST {
        name.pop();
    }
    name
}

/// `name` without its control characters: a name the other side suggested
/// is hostile text.
fn printable(name: &str) -> String {
    name.chars()
        .filter(|character| !character.is_control())
        .collect()
}

/// Runs the pairing connection to its end, and gives the relay's
/// introduction when it paired.
fn run_pairing(address: &str, mut end: HubEnd) -> Result<Introduction, Unpaired> {
    let mut wire = dial(address)?;
    drive(&mut end, &mut wire, &mut System, |_| Ok(Next::Wait)).expect("nothing refuses a step");
    match (end.opening(), end.pairing()) {
        (_, PairingRun::Paired(relay)) => Ok(relay.clone()),
        (Opening::PairingUnavailable, _) => Err(Unpaired::NoInvitation(address.to_owned())),
        (Opening::Incompatible { theirs }, _) => Err(Unpaired::Incompatible(
            address.to_owned(),
            theirs.as_slice().to_vec(),
        )),
        _ => Err(Unpaired::Failed(address.to_owned())),
    }
}

/// Runs the first pinned-key link until it is established, then closes it
/// as "hub monitor disconnected".
fn first_link(address: &str, mut end: HubEnd) -> Result<(), Unpaired> {
    let mut wire = dial(address).map_err(|_| Unpaired::FirstLink(address.to_owned()))?;
    drive(&mut end, &mut wire, &mut System, disconnect_once_linked)
        .expect("nothing refuses a step");
    match end.link() {
        Link::Ended(Ended::Left(HubClose::Disconnected)) => Ok(()),
        _ => Err(Unpaired::FirstLink(address.to_owned())),
    }
}

/// Between two steps of the first link: once it is established, close it.
fn disconnect_once_linked(end: &mut HubEnd) -> io::Result<Next> {
    match end.link() {
        Link::Established(_) => {
            end.close(HubClose::Disconnected)
                .expect("an established link closes");
            Ok(Next::Step)
        }
        _ => Ok(Next::Wait),
    }
}

fn dial(address: &str) -> Result<Wire, Unpaired> {
    link_socket::dial_named(address, CONNECT_WAIT)
        .map(Wire)
        .map_err(|error| Unpaired::Unreachable(address.to_owned(), error))
}

/// One TCP connection and this process's clock, as the link code is
/// driven over them.
struct Wire(LinkStream);

impl Carrier for Wire {
    fn now(&mut self) -> Time {
        Time::since_start(clock::monotonic())
    }

    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes)
    }

    fn receive(&mut self, buffer: &mut [u8], wait: Option<Duration>) -> io::Result<usize> {
        self.0.read(buffer, wait.unwrap_or(IDLE_WAIT))
    }
}

/// The operating system's generator, which is all the link code draws on.
struct System;

impl Randomness for System {
    fn fill(&mut self, bytes: &mut [u8]) {
        randomness::fill(bytes).expect("the operating system's generator gives bytes");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_cut_to_64_bytes_at_a_character_boundary() {
        assert_eq!(cut("a".repeat(64)), "a".repeat(64));
        assert_eq!(cut("a".repeat(65)), "a".repeat(64));
        let wide = format!("{}é", "a".repeat(63));
        assert_eq!(cut(wide), "a".repeat(63));
        assert_eq!(cut("buildbox".to_owned()), "buildbox");
    }
}
