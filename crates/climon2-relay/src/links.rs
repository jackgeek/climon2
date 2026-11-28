//! The relay's side of monitor links (spec #488, ADR 0011), built on the
//! link code: it accepts every connection on its listener and runs the link
//! code's relay end over each, on a thread of its own. So far that is
//! pairing under the relay's link invitation, and links with the hub
//! monitors it remembers (#501). What a link carries comes later.
//!
//! The remembered hub monitors have one writer at a time: [`Remembered`]
//! holds them behind one lock, with the store they are written to.

use climon2_core::pairing::{self, HubMonitor, PublicKey as StoredKey, RelayPairings};
use climon2_link::{
    Carrier, Hello, Introduction, Invitations, Link, Next, PairingRun, PublicKey, Randomness,
    RelayEnd, Revisions, StaticKey, Time, display_name, drive,
};
use climon2_platform::{
    clock,
    key_storage::KeyStore,
    link_socket::{LinkListener, LinkStream},
    randomness,
};
use std::{
    io,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

/// How long the acceptor waits for a connection before it checks whether
/// the relay is stopping.
const ACCEPT_TICK: Duration = Duration::from_millis(50);
/// How long the acceptor rests after the listener fails to accept.
const ACCEPT_REST: Duration = Duration::from_millis(100);
/// How long one wait for bytes lasts when the link code names no deadline.
const IDLE_WAIT: Duration = Duration::from_secs(60);
/// The longest record this relay accepts: `relay.limits` arrives later, and
/// this is the default receive ceiling of `payload.wire_frame_bytes`.
const LARGEST_RECORD: u32 = 1_048_576;

/// The time now on this process's clock, as the link code takes it.
pub(crate) fn now() -> Time {
    Time::since_start(clock::monotonic())
}

/// The hub monitors this relay remembers, and the store that keeps them.
#[derive(Clone)]
pub(crate) struct Remembered(Arc<Mutex<Store>>);

struct Store {
    files: KeyStore,
    pairings: RelayPairings,
}

impl Remembered {
    /// What `files` holds, read once at the relay's start.
    pub(crate) fn new(files: KeyStore, pairings: RelayPairings) -> Self {
        Self(Arc::new(Mutex::new(Store { files, pairings })))
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        // Every change is written whole before the lock is let go, so a
        // panic elsewhere leaves nothing half-done.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The keys of every remembered hub monitor.
    fn keys(&self) -> Vec<PublicKey> {
        let store = self.store();
        let records = store.pairings.hub_monitors.records();
        let key = |record: &HubMonitor| PublicKey::from_bytes(*record.public_key.as_bytes());
        records.iter().map(key).collect()
    }

    /// Remembers `hub`, which has just paired, at `now` in seconds since
    /// the Unix epoch. A hub monitor already remembered is replaced; none
    /// is evicted (P17).
    fn pin(&self, hub: &Introduction, now: u64) -> io::Result<()> {
        let record = HubMonitor {
            public_key: StoredKey::new(*hub.key()),
            name: display_name(hub.name()),
            paired_at: now,
            last_linked_at: None,
        };
        self.change(|pairings| pairings.hub_monitors.insert(record).map_err(|_| full()))
    }

    /// Records that the hub monitor with `key` linked at `now`, under the
    /// name its Hello suggests.
    fn linked(&self, key: &PublicKey, name: &str, now: u64) -> io::Result<()> {
        let key = StoredKey::new(*key.as_bytes());
        self.change(|pairings| {
            let records = pairings.hub_monitors.records();
            let Some(record) = records.iter().find(|record| record.public_key == key) else {
                return Ok(());
            };
            let mut record = record.clone();
            record.last_linked_at = Some(now);
            record.name = display_name(name);
            pairings.hub_monitors.insert(record).map_err(|_| full())
        })
    }

    /// Applies `change` and writes the result; a change that fails, or is
    /// not written, is not kept.
    fn change(&self, change: impl FnOnce(&mut RelayPairings) -> io::Result<()>) -> io::Result<()> {
        let mut store = self.store();
        let mut pairings = store.pairings.clone();
        change(&mut pairings)?;
        pairing::save_hub_monitors(&store.files, &pairings)?;
        store.pairings = pairings;
        Ok(())
    }
}

fn full() -> io::Error {
    io::Error::other(format!(
        "this relay already remembers {} hub monitors",
        pairing::MAX_PAIRINGS
    ))
}

/// What every connection's end is made from.
struct Relay {
    identity: StaticKey,
    hello: Hello,
    invitations: Invitations,
    remembered: Remembered,
}

/// The relay's links: its listener, accepting on a thread of its own until
/// this is dropped.
pub(crate) struct Links {
    invitations: Invitations,
    stopping: Arc<AtomicBool>,
    acceptor: Option<JoinHandle<()>>,
}

impl Links {
    /// Starts accepting on `listener`. `identity` is the relay's identity
    /// key, `name` the name it suggests and `remembered` its hub monitors.
    pub(crate) fn start(
        listener: LinkListener,
        identity: StaticKey,
        name: &str,
        remembered: Remembered,
    ) -> Self {
        let introduction = Introduction::new(*identity.public().as_bytes(), name)
            .expect("a name cut to 64 bytes fits");
        let invitations = Invitations::new(introduction);
        let relay = Arc::new(Relay {
            identity,
            hello: Hello::new(env!("CARGO_PKG_VERSION"), name, LARGEST_RECORD)
                .expect("this release's Hello is valid"),
            invitations: invitations.clone(),
            remembered,
        });
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopping);
        let acceptor = thread::spawn(move || accept(&listener, &stop, &relay));
        Self {
            invitations,
            stopping,
            acceptor: Some(acceptor),
        }
    }

    /// The relay's link invitation, which its user mints and cancels.
    pub(crate) fn invitations(&self) -> &Invitations {
        &self.invitations
    }
}

impl Drop for Links {
    /// The listener closes once the acceptor has seen the relay stop.
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        if let Some(acceptor) = self.acceptor.take() {
            let _ = acceptor.join();
        }
    }
}

/// Accepts until the relay stops, giving each connection a thread.
fn accept(listener: &LinkListener, stopping: &AtomicBool, relay: &Arc<Relay>) {
    while !stopping.load(Ordering::Relaxed) {
        match listener.accept(ACCEPT_TICK) {
            Ok(stream) => {
                let relay = Arc::clone(relay);
                thread::spawn(move || connection(stream, &relay));
            }
            Err(error) => {
                if let Some(rest) = rest(&error) {
                    tracing::warn!(%error, "relay cannot accept a connection");
                    thread::sleep(rest);
                }
            }
        }
    }
}

/// How long to rest after a failed accept: not at all when nothing arrived
/// in time, and briefly when the listener itself failed.
fn rest(error: &io::Error) -> Option<Duration> {
    (error.kind() != io::ErrorKind::TimedOut).then_some(ACCEPT_REST)
}

/// Runs the relay's end of one connection to its end.
fn connection(stream: LinkStream, relay: &Relay) {
    let remembered = relay.remembered.keys();
    let end = RelayEnd::new(Revisions::published(), relay.invitations.clone());
    let mut end = end.linking(relay.identity.clone(), remembered, relay.hello.clone());
    let mut kept = Kept::default();
    let between = |end: &mut RelayEnd| keep(end, &relay.remembered, &mut kept);
    if let Err(error) = drive(&mut end, &mut Wire(stream), &mut System, between) {
        tracing::error!(%error, "relay cannot remember a hub monitor");
    }
}

/// What a connection has written to the store so far.
#[derive(Default)]
struct Kept {
    pinned: bool,
    linked: bool,
}

/// Between two steps of a connection: a hub monitor that has just paired
/// is remembered before the relay's confirmation is sent, and the first
/// moment a link is established is recorded.
fn keep(end: &mut RelayEnd, remembered: &Remembered, kept: &mut Kept) -> io::Result<Next> {
    let wall = clock::wall().as_secs();
    if let (PairingRun::Paired(hub), false) = (end.pairing(), kept.pinned) {
        remembered.pin(hub, wall)?;
        kept.pinned = true;
    }
    if let (Link::Established(peer), false) = (end.link(), kept.linked) {
        remembered.linked(&peer.key, peer.hello.name(), wall)?;
        kept.linked = true;
    }
    Ok(Next::Wait)
}

/// One TCP connection and this process's clock, as the link code is
/// driven over them.
struct Wire(LinkStream);

impl Carrier for Wire {
    fn now(&mut self) -> Time {
        now()
    }

    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes)
    }

    fn receive(&mut self, buffer: &mut [u8], wait: Option<Duration>) -> io::Result<usize> {
        self.0.read(buffer, wait.unwrap_or(IDLE_WAIT))
    }
}

/// The operating system's generator, which is all the link code draws on.
pub(crate) struct System;

impl Randomness for System {
    fn fill(&mut self, bytes: &mut [u8]) {
        randomness::fill(bytes).expect("the operating system's generator gives bytes");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use climon2_link::{Ended, HubClose, HubEnd, Invitation, Pin, RelayClose};
    use climon2_platform::link_socket;

    const WAIT: Duration = Duration::from_secs(20);

    fn key() -> StaticKey {
        let mut private = [0; 32];
        System.fill(&mut private);
        StaticKey::from_private(private)
    }

    /// A relay's links over a listener on loopback, remembering into a
    /// folder of its own.
    struct Running {
        _folder: tempfile::TempDir,
        files: KeyStore,
        identity: StaticKey,
        address: std::net::SocketAddr,
        links: Links,
    }

    fn running() -> Running {
        let folder = tempfile::tempdir().unwrap();
        let files = KeyStore::open(&folder.path().join("links")).unwrap();
        let listener = link_socket::listen("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = listener.local_address().unwrap();
        let identity = key();
        let remembered = Remembered::new(
            KeyStore::open(&folder.path().join("links")).unwrap(),
            RelayPairings::default(),
        );
        let links = Links::start(listener, identity.clone(), "buildbox", remembered);
        Running {
            _folder: folder,
            files,
            identity,
            address,
            links,
        }
    }

    impl Running {
        fn remembered(&self) -> Vec<HubMonitor> {
            let pairings = pairing::load_hub_monitors(&self.files).unwrap();
            pairings.hub_monitors.records().to_vec()
        }

        fn mint(&self) -> Pin {
            self.links.invitations().mint(now(), &mut System)
        }

        fn dial(&self) -> Wire {
            Wire(link_socket::dial(self.address, WAIT).unwrap())
        }

        /// A hub monitor named `name` pairs with `pin`; its key, and the
        /// relay it paired with.
        fn pair(&self, pin: Pin, name: &str) -> (StaticKey, PairingRun) {
            let own = key();
            let introduction = Introduction::new(*own.public().as_bytes(), name).unwrap();
            let mut hub = HubEnd::pair(Revisions::published(), pin, introduction);
            drive(&mut hub, &mut self.dial(), &mut System, |_| Ok(Next::Wait)).unwrap();
            (own, hub.pairing().clone())
        }

        /// A hub monitor with `own` links, suggesting `name`, and closes the
        /// link once it is established; how its link ended.
        fn link(&self, own: StaticKey, name: &str) -> Link<HubClose, RelayClose> {
            let hello = Hello::new("9.9.9", name, LARGEST_RECORD).unwrap();
            let relay = self.identity.public();
            let mut hub = HubEnd::dialling(Revisions::published(), own, relay, hello);
            let close = |hub: &mut HubEnd| match hub.link() {
                Link::Established(_) => hub.close(HubClose::Disconnected).map(|()| Next::Step),
                _ => Ok(Next::Wait),
            };
            let close = |hub: &mut HubEnd| close(hub).map_err(|_| io::Error::other("refused"));
            drive(&mut hub, &mut self.dial(), &mut System, close).unwrap();
            hub.link().clone()
        }
    }

    /// P17, P20, PAIR-01: a hub monitor that pairs is remembered, under the
    /// name it suggested, before the relay sends its confirmation: by the
    /// time the hub monitor has paired, the store holds it.
    #[test]
    fn a_hub_monitor_that_pairs_is_remembered_before_it_learns_it_paired() {
        let relay = running();
        let pin = relay.mint();
        let (own, paired) = relay.pair(pin, "laptop\x07");
        let PairingRun::Paired(introduction) = paired else {
            panic!("the hub monitor paired: {paired:?}");
        };
        assert_eq!(introduction.key(), relay.identity.public().as_bytes());
        assert_eq!(introduction.name(), "buildbox");
        let [hub] = relay.remembered().try_into().unwrap();
        assert_eq!(hub.public_key.as_bytes(), own.public().as_bytes());
        assert_eq!(hub.name, "laptop");
        assert!(hub.paired_at.abs_diff(clock::wall().as_secs()) < 60);
        assert_eq!(hub.last_linked_at, None);
        let Invitation::Paired(_) = relay.links.invitations().invitation(now()) else {
            panic!("the invitation is consumed");
        };
    }

    /// P17: a second pairing evicts nobody.
    #[test]
    fn every_hub_monitor_that_pairs_is_remembered() {
        let relay = running();
        let (first, _) = relay.pair(relay.mint(), "laptop");
        let (second, _) = relay.pair(relay.mint(), "desktop");
        let keys: Vec<_> = relay
            .remembered()
            .iter()
            .map(|hub| *hub.public_key.as_bytes())
            .collect();
        assert_eq!(
            keys,
            [*first.public().as_bytes(), *second.public().as_bytes()]
        );
    }

    /// PAIR-01, ST2, Hello rules: a remembered hub monitor links, and the
    /// relay records when, under the name its Hello suggests.
    #[test]
    fn a_remembered_hub_monitor_links_and_the_link_is_recorded() {
        let relay = running();
        let (own, _) = relay.pair(relay.mint(), "laptop");
        let link = relay.link(own, "laptop-2");
        assert_eq!(link, Link::Ended(Ended::Left(HubClose::Disconnected)));
        let [hub] = relay.remembered().try_into().unwrap();
        assert_eq!(hub.name, "laptop-2");
        let linked = hub.last_linked_at.expect("the link is recorded");
        assert!(linked.abs_diff(clock::wall().as_secs()) < 60);
    }

    /// SEC-04: a key the relay does not remember gets "not paired".
    #[test]
    fn an_unknown_hub_monitor_is_refused_as_not_paired() {
        let relay = running();
        relay.pair(relay.mint(), "laptop");
        let link = relay.link(key(), "stranger");
        assert_eq!(link, Link::Ended(Ended::Closed(RelayClose::NotPaired)));
        assert_eq!(relay.remembered()[0].last_linked_at, None);
    }

    /// X8: a stopped relay listens no more.
    #[test]
    fn dropping_the_links_closes_the_listener() {
        let relay = running();
        let address = relay.address;
        drop(relay.links);
        assert!(link_socket::dial(address, WAIT).is_err());
    }

    #[test]
    fn the_acceptor_rests_only_after_the_listener_fails() {
        assert_eq!(rest(&io::ErrorKind::TimedOut.into()), None);
        assert_eq!(rest(&io::Error::from_raw_os_error(24)), Some(ACCEPT_REST));
    }
}
