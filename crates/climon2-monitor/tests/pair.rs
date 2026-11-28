//! #501 (PAIR-01, PAIR-04, P14-20, ST3, ST15): the hub monitor's side of
//! pairing, through the monitor library's own interface. A relay made here
//! from the link code listens on loopback; the library dials it, pairs,
//! completes its first pinned-key link and records the relay in the home's
//! `links/relays.json`.
#![cfg(unix)]

use climon2_core::{layout::HomeLayout, pairing};
use climon2_link::{
    Carrier, Ended, Hello, HubClose, Introduction, Invitation, Invitations, Link, Next, PublicKey,
    Randomness, RelayEnd, RelayLink, Revisions, StaticKey, Time, drive,
};
use climon2_monitor::links::{self, Paired, Pin, Unpaired};
use climon2_platform::{
    clock, host_name,
    key_storage::KeyStore,
    link_socket::{self, LinkListener, LinkStream},
    randomness,
};
use std::{
    io,
    net::SocketAddr,
    thread::{self, JoinHandle},
    time::Duration,
};

const WAIT: Duration = Duration::from_secs(20);
const LARGEST_RECORD: u32 = 1_048_576;

struct System;

impl Randomness for System {
    fn fill(&mut self, bytes: &mut [u8]) {
        randomness::fill(bytes).unwrap();
    }
}

struct Wire(LinkStream);

impl Carrier for Wire {
    fn now(&mut self) -> Time {
        Time::since_start(clock::monotonic())
    }

    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes)
    }

    fn receive(&mut self, buffer: &mut [u8], wait: Option<Duration>) -> io::Result<usize> {
        self.0.read(buffer, wait.unwrap_or(WAIT))
    }
}

fn now() -> Time {
    Time::since_start(clock::monotonic())
}

fn key() -> StaticKey {
    let mut private = [0; 32];
    randomness::fill(&mut private).unwrap();
    StaticKey::from_private(private)
}

/// What the test's relay saw of the two connections a pairing makes.
struct Seen {
    paired: Invitation,
    link: RelayLink,
}

/// A relay made from the link code, with its own identity and name. Once
/// served, it takes one pairing connection and then one link connection,
/// linking with the key it paired with under `link_identity`: its own
/// identity unless a test makes it lie.
struct TestRelay {
    name: String,
    address: SocketAddr,
    identity: StaticKey,
    invitations: Invitations,
    listener: Option<LinkListener>,
    served: Option<JoinHandle<Seen>>,
}

impl TestRelay {
    fn named(name: &str) -> Self {
        Self::with_identity(name, key())
    }

    fn with_identity(name: &str, identity: StaticKey) -> Self {
        let introduction = Introduction::new(*identity.public().as_bytes(), name).unwrap();
        let listener = link_socket::listen("127.0.0.1:0".parse().unwrap()).unwrap();
        Self {
            name: name.to_owned(),
            address: listener.local_address().unwrap(),
            identity,
            invitations: Invitations::new(introduction),
            listener: Some(listener),
            served: None,
        }
    }

    fn serve(&mut self, link_identity: StaticKey) {
        self.serve_revisions(link_identity, Revisions::published());
    }

    fn serve_revisions(&mut self, link_identity: StaticKey, revisions: Revisions) {
        let listener = self.listener.take().unwrap();
        let invitations = self.invitations.clone();
        let name = self.name.clone();
        self.served = Some(thread::spawn(move || {
            let mut pairing = RelayEnd::new(revisions, invitations.clone());
            let stream = listener.accept(WAIT).unwrap();
            drive(&mut pairing, &mut Wire(stream), &mut System, |_| {
                Ok(Next::Wait)
            })
            .unwrap();
            let paired = invitations.invitation(now());
            let Invitation::Paired(hub) = &paired else {
                return Seen {
                    paired,
                    link: Link::Pending,
                };
            };
            let hello = Hello::new("9.9.9", &name, LARGEST_RECORD).unwrap();
            let remembered = vec![PublicKey::from_bytes(*hub.key())];
            let mut linking = RelayEnd::new(Revisions::published(), invitations).linking(
                link_identity,
                remembered,
                hello,
            );
            let stream = listener.accept(WAIT).unwrap();
            drive(&mut linking, &mut Wire(stream), &mut System, |_| {
                Ok(Next::Wait)
            })
            .unwrap();
            Seen {
                paired,
                link: linking.link().clone(),
            }
        }));
    }

    fn seen(&mut self) -> Seen {
        self.served.take().unwrap().join().unwrap()
    }
}

/// A relay named `name`, serving, with an invitation open, and its PIN.
fn invited(name: &str) -> (TestRelay, Pin) {
    let mut relay = TestRelay::named(name);
    let identity = relay.identity.clone();
    relay.serve(identity);
    let pin = relay.invitations.mint(now(), &mut System);
    (relay, pin)
}

fn home() -> (tempfile::TempDir, HomeLayout) {
    let directory = tempfile::tempdir().unwrap();
    let home = HomeLayout::at(directory.path()).unwrap();
    (directory, home)
}

fn relays(home: &HomeLayout) -> Vec<pairing::Relay> {
    let store = KeyStore::open(&home.links()).unwrap();
    pairing::load_relays(&store).unwrap().records().to_vec()
}

fn pair(home: &HomeLayout, address: &str, pin: &Pin) -> Result<Paired, Unpaired> {
    links::pair(home, address, pin.clone(), LARGEST_RECORD)
}

fn pin(typed: &str) -> Pin {
    Pin::parse(typed).unwrap()
}

/// PAIR-01 (P15-17, P19-20, P23, ST3, ST15): pairing is its own connection,
/// then the first pinned-key link succeeds, and only then is the relay
/// recorded and reported, under its suggested name and its fingerprint.
/// That link is closed with "hub monitor disconnected" and the record stays
/// connected.
#[test]
fn a_pairing_links_once_then_records_and_reports_the_relay() {
    let (_directory, home) = home();
    let (mut relay, pin) = invited("buildbox");
    let typed = format!("localhost:{}", relay.address.port());
    let paired = pair(&home, &typed, &pin).unwrap();
    assert_eq!(
        paired,
        Paired {
            name: "buildbox".to_owned(),
            fingerprint: relay.identity.public().fingerprint(),
        }
    );
    let seen = relay.seen();
    let Invitation::Paired(hub) = seen.paired else {
        panic!("the relay paired: {:?}", seen.paired);
    };
    assert_eq!(hub.name(), host_name::get().unwrap());
    let Link::Ended(Ended::Closed(HubClose::Disconnected)) = seen.link else {
        panic!("the first link is closed as disconnected: {:?}", seen.link);
    };
    let [record] = relays(&home).try_into().unwrap();
    assert_eq!(record.name, "buildbox");
    assert_eq!(record.address, typed, "the address is kept as typed");
    assert_eq!(
        record.relay_public_key.as_bytes(),
        relay.identity.public().as_bytes()
    );
    assert_eq!(record.public_key.as_bytes(), hub.key());
    let own = StaticKey::from_private(*record.private_key.expose());
    assert_eq!(
        own.public().as_bytes(),
        hub.key(),
        "the stored keypair is whole"
    );
    assert!(!record.disconnected);
    let wall = clock::wall().as_secs();
    assert!(record.paired_at.abs_diff(wall) < 60, "{}", record.paired_at);
    assert_eq!(record.last_linked_at, Some(record.paired_at));
}

/// P15-16 (PAIR-01): each pairing makes a fresh keypair. Pairing again with
/// a relay already recorded replaces its record; another relay gets one of
/// its own.
#[test]
fn each_pairing_makes_a_fresh_keypair_and_replaces_only_its_relays_record() {
    let (_directory, home) = home();
    let (mut first, pin) = invited("buildbox");
    pair(&home, &first.address.to_string(), &pin).unwrap();
    first.seen();
    let [before] = relays(&home).try_into().unwrap();
    let mut again = TestRelay::with_identity("buildbox", first.identity.clone());
    again.serve(first.identity.clone());
    let pin = again.invitations.mint(now(), &mut System);
    pair(&home, &again.address.to_string(), &pin).unwrap();
    again.seen();
    let [replaced] = relays(&home).try_into().unwrap();
    assert_eq!(replaced.relay_public_key, before.relay_public_key);
    assert_ne!(replaced.public_key, before.public_key, "a fresh keypair");
    assert_eq!(replaced.address, again.address.to_string());
    let (mut other, pin) = invited("laptop");
    pair(&home, &other.address.to_string(), &pin).unwrap();
    other.seen();
    let records = relays(&home);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].public_key, replaced.public_key, "{records:?}");
    assert_ne!(records[1].public_key, replaced.public_key);
}

/// P25, CM24 (PAIR-04): a relay nobody can reach.
#[test]
fn an_unreachable_relay_is_named_as_such_and_nothing_is_stored() {
    let (_directory, home) = home();
    let listener = link_socket::listen("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_address().unwrap().to_string();
    drop(listener);
    let refused = pair(&home, &address, &pin("12345678")).unwrap_err();
    assert!(matches!(refused, Unpaired::Unreachable(..)), "{refused:?}");
    assert!(
        refused
            .to_string()
            .starts_with(&format!("cannot reach the relay at {address}: ")),
        "{refused}"
    );
    assert!(relays(&home).is_empty());
}

/// P25, CM24 (PAIR-04): a relay whose user has opened no invitation.
#[test]
fn a_relay_with_no_invitation_open_is_named_as_such() {
    let (_directory, home) = home();
    let mut relay = TestRelay::named("buildbox");
    let identity = relay.identity.clone();
    relay.serve(identity);
    let address = relay.address.to_string();
    let refused = pair(&home, &address, &pin("12345678")).unwrap_err();
    assert!(matches!(refused, Unpaired::NoInvitation(_)), "{refused:?}");
    assert_eq!(
        refused.to_string(),
        format!("the relay at {address} has no link invitation open")
    );
    assert_eq!(relay.seen().paired, Invitation::None);
    assert!(relays(&home).is_empty());
}

/// P25, CM24 (PAIR-04): the wrong PIN fails the pairing and pins nothing.
#[test]
fn a_wrong_pin_fails_the_pairing() {
    let (_directory, home) = home();
    let (mut relay, pin) = invited("buildbox");
    let mut wrong = pin.to_string().into_bytes();
    let last = wrong.last_mut().unwrap();
    *last = b'0' + (*last - b'0' + 1) % 10;
    let wrong = String::from_utf8(wrong).unwrap();
    let address = relay.address.to_string();
    let refused = pair(&home, &address, &self::pin(&wrong)).unwrap_err();
    assert!(matches!(refused, Unpaired::Failed(_)), "{refused:?}");
    assert_eq!(
        refused.to_string(),
        format!("pairing with the relay at {address} failed: wrong PIN or interference")
    );
    let Invitation::Live { attempts_left, .. } = relay.seen().paired else {
        panic!("a failed run leaves the invitation live");
    };
    assert_eq!(attempts_left, 2);
    assert!(relays(&home).is_empty());
}

/// P20, PAIR-01: "paired" only once the first pinned-key link succeeds. A
/// relay that pairs and then links with another key than the one it handed
/// over is not paired with, and nothing is recorded.
#[test]
fn a_relay_whose_first_link_fails_is_not_paired_with() {
    let (_directory, home) = home();
    let mut relay = TestRelay::named("buildbox");
    relay.serve(key());
    let pin = relay.invitations.mint(now(), &mut System);
    let address = relay.address.to_string();
    let refused = pair(&home, &address, &pin).unwrap_err();
    assert!(matches!(refused, Unpaired::FirstLink(_)), "{refused:?}");
    assert_eq!(
        refused.to_string(),
        format!("pairing with the relay at {address} failed: its first link did not succeed")
    );
    assert!(matches!(relay.seen().paired, Invitation::Paired(_)));
    assert!(relays(&home).is_empty());
}

/// ST5, CM20: a damaged store fails the command before anything is dialled.
#[test]
fn a_damaged_store_fails_before_anything_is_dialled() {
    let (_directory, home) = home();
    KeyStore::open(&home.links())
        .unwrap()
        .store("relays.json", b"{broken")
        .unwrap();
    let relay = TestRelay::named("buildbox");
    let pin = relay.invitations.mint(now(), &mut System);
    let refused = pair(&home, &relay.address.to_string(), &pin).unwrap_err();
    assert!(matches!(refused, Unpaired::Store(_)), "{refused:?}");
    assert!(
        refused
            .to_string()
            .starts_with("damaged pairing file relays.json"),
        "{refused}"
    );
    let Invitation::Live { attempts_left, .. } = relay.invitations.invitation(now()) else {
        panic!("the invitation is untouched");
    };
    assert_eq!(attempts_left, 3);
}

/// R16 (hostile names): a relay's suggested name keeps no control
/// character, whether recorded or reported.
#[test]
fn a_suggested_name_keeps_no_control_character() {
    let (_directory, home) = home();
    let (mut relay, pin) = invited("build\x1b[2Jbox\n");
    let paired = pair(&home, &relay.address.to_string(), &pin).unwrap();
    relay.seen();
    assert_eq!(paired.name, "build[2Jbox");
    assert_eq!(relays(&home)[0].name, "build[2Jbox");
}

/// V15-16: a relay that shares no link revision is named as incompatible,
/// with both lists.
#[test]
fn a_relay_sharing_no_link_revision_is_named_as_incompatible() {
    let (_directory, home) = home();
    let mut relay = TestRelay::named("buildbox");
    let identity = relay.identity.clone();
    relay.serve_revisions(identity, Revisions::new(&[2, 3]).unwrap());
    let pin = relay.invitations.mint(now(), &mut System);
    let address = relay.address.to_string();
    let refused = pair(&home, &address, &pin).unwrap_err();
    assert_eq!(
        refused.to_string(),
        format!(
            "pairing with the relay at {address} failed: it shares no link revision \
             with this monitor (relay [2, 3], monitor [1])"
        )
    );
    relay.seen();
    assert!(relays(&home).is_empty());
}
