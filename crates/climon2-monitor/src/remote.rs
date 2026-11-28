//! The hub monitor's links to the relays it remembers (#502; spec #488,
//! ADR 0011): one thread per relay that is not disconnected dials it,
//! holds the link and hands what the relay advertises to the monitor,
//! which applies it on its own thread. A link that ends is dialled again
//! after `monitor.limits.retry.initial_ms`; the redial backoff, its reset
//! and the slower redial against a refusing relay come with #504. Each dial
//! reads the relay's record afresh, so a changed address is dialled and a
//! relay that was forgotten or disconnected is not. The bytes are the link
//! code's (`climon2-link`).
//!
//! A monitor whose home has no `links/` folder remembers no relay and
//! creates none.
use climon2_core::{layout::HomeLayout, pairing::Relay};
use climon2_link::{Advertisement, PublicKey};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};

/// What a relay's link tells the monitor. Only macOS and Linux hold links.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) enum Heard {
    /// A link connection to the relay is established: it starts with
    /// nothing held, so whatever the previous one advertised is gone.
    Linked(PublicKey),
    /// The relay advertised something on the link.
    Advertised(PublicKey, Advertisement),
}

/// How the hub monitor links: the ceiling its Hello states, and how long
/// it waits before dialling a relay again.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) struct Dialling {
    pub(crate) largest_record: u32,
    pub(crate) redial: Duration,
}

/// The links to every remembered relay, held until [`Remote::stop`].
pub(crate) struct Remote {
    heard: Receiver<Heard>,
    /// Every remembered relay's key and the name the user knows it by.
    relays: Vec<(PublicKey, String)>,
    stopping: Arc<AtomicBool>,
}

impl Remote {
    /// Starts a link to each relay remembered in `home` that is not
    /// disconnected. A store that cannot be read is an error.
    pub(crate) fn start(home: &HomeLayout, dialling: Dialling) -> io::Result<Self> {
        let remembered = remembered(home)?;
        let (report, heard) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let relays = remembered.iter().map(named).collect();
        #[cfg(unix)]
        for relay in remembered.iter().filter(|relay| !relay.disconnected) {
            hold::start(home, relay, dialling, &report, &stopping)?;
        }
        #[cfg(not(unix))]
        let _ = (dialling, report);
        Ok(Self {
            heard,
            relays,
            stopping,
        })
    }

    /// Every remembered relay, by key, with its name.
    pub(crate) fn relays(&self) -> &[(PublicKey, String)] {
        &self.relays
    }

    /// What the links heard since the last call.
    pub(crate) fn heard(&self) -> Vec<Heard> {
        self.heard.try_iter().collect()
    }

    /// Every link closes as "hub monitor stopped", and none is dialled
    /// again.
    pub(crate) fn stop(&self) {
        self.stopping.store(true, Ordering::Relaxed);
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The relays `home` remembers; none without a `links/` folder.
#[cfg(unix)]
fn remembered(home: &HomeLayout) -> io::Result<Vec<Relay>> {
    if !home.links().exists() {
        return Ok(Vec::new());
    }
    let store = climon2_platform::key_storage::KeyStore::open(&home.links())?;
    let relays = climon2_core::pairing::load_relays(&store)?;
    Ok(relays.records().to_vec())
}

/// Monitor links are for macOS and Linux only (spec #488, Out of scope).
#[cfg(not(unix))]
fn remembered(_: &HomeLayout) -> io::Result<Vec<Relay>> {
    Ok(Vec::new())
}

/// A relay's key and name, as the machine column shows it.
fn named(relay: &Relay) -> (PublicKey, String) {
    let key = PublicKey::from_bytes(*relay.relay_public_key.as_bytes());
    (key, relay.name.clone())
}

#[cfg(unix)]
mod hold {
    //! One relay's link, held on a thread of its own.
    use super::{Dialling, Heard};
    use climon2_core::{
        layout::HomeLayout,
        pairing::{self, Relay},
    };
    use climon2_link::{
        Carrier, Hello, HubClose, HubEnd, Link, Next, PublicKey, Randomness, RelayRecord,
        Revisions, StaticKey, Time, display_name, drive,
    };
    use climon2_platform::{
        clock, host_name,
        key_storage::KeyStore,
        link_socket::{self, LinkStream},
        randomness,
    };
    use std::{
        io,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc::Sender,
        },
        thread,
        time::{Duration, Instant},
    };

    /// How long a dial may take to connect (spec #488, Link lifecycle).
    const CONNECT_WAIT: Duration = Duration::from_secs(10);
    /// The longest one wait lasts, so that a stopping monitor is noticed.
    const TICK: Duration = Duration::from_millis(50);

    /// Holds `relay`'s link on a thread of its own, reporting on `report`
    /// until `stopping`.
    pub(super) fn start(
        home: &HomeLayout,
        relay: &Relay,
        dialling: Dialling,
        report: &Sender<Heard>,
        stopping: &Arc<AtomicBool>,
    ) -> io::Result<()> {
        let held = Held {
            store: KeyStore::open(&home.links())?,
            relay: PublicKey::from_bytes(*relay.relay_public_key.as_bytes()),
            dialling,
            report: report.clone(),
            stopping: Arc::clone(stopping),
        };
        thread::Builder::new()
            .name("relay link".into())
            .spawn(move || held.hold())?;
        Ok(())
    }

    /// One relay's link and what it needs.
    struct Held {
        store: KeyStore,
        relay: PublicKey,
        dialling: Dialling,
        report: Sender<Heard>,
        stopping: Arc<AtomicBool>,
    }

    impl Held {
        /// Dials and holds the link, then dials again after the wait,
        /// while the relay is still remembered and not disconnected.
        fn hold(&self) {
            while !self.stopping() {
                let Some(relay) = self.current() else {
                    return;
                };
                if let Err(error) = self.link(&relay) {
                    tracing::info!(%error, relay = relay.name, "relay link ended");
                }
                self.rest();
            }
        }

        fn stopping(&self) -> bool {
            self.stopping.load(Ordering::Relaxed)
        }

        /// The relay's record as the store holds it now, while it is
        /// remembered and not disconnected.
        fn current(&self) -> Option<Relay> {
            let relays = pairing::load_relays(&self.store)
                .inspect_err(|error| tracing::warn!(%error, "relay store unreadable"))
                .ok()?;
            let key = |relay: &&Relay| relay.relay_public_key.as_bytes() == self.relay.as_bytes();
            let relay = relays.records().iter().find(key)?;
            (!relay.disconnected).then(|| relay.clone())
        }

        /// One link connection to `relay`, driven until it ends.
        fn link(&self, relay: &Relay) -> io::Result<()> {
            let stream = link_socket::dial_named(&relay.address, CONNECT_WAIT)?;
            let own = StaticKey::from_private(*relay.private_key.expose());
            let name = display_name(&host_name::get().unwrap_or_default());
            let largest = self.dialling.largest_record;
            let hello = Hello::new(env!("CARGO_PKG_VERSION"), &name, largest)
                .expect("this release's Hello is valid");
            let mut end = HubEnd::dialling(Revisions::published(), own, self.relay, hello);
            let mut linked = false;
            let between = |end: &mut HubEnd| Ok(self.between(end, &mut linked));
            drive(&mut end, &mut Wire(stream), &mut System, between)
        }

        /// Between two steps of the link: its establishment and what the
        /// relay advertised go to the monitor, and a stopping monitor
        /// closes it.
        fn between(&self, end: &mut HubEnd, linked: &mut bool) -> Next {
            self.heard(end, linked);
            if !self.stopping() {
                return Next::Wait;
            }
            match end.close(HubClose::Stopped) {
                Ok(()) => Next::Step,
                Err(_) => Next::Stop,
            }
        }

        /// The link's establishment, once, and what the relay advertised
        /// since the last step go to the monitor. Session streams come with
        /// #503.
        fn heard(&self, end: &mut HubEnd, linked: &mut bool) {
            if !*linked && matches!(end.link(), Link::Established(_)) {
                *linked = true;
                let _ = self.report.send(Heard::Linked(self.relay));
            }
            for record in end.records() {
                if let RelayRecord::Advertisement(advertisement) = record {
                    let _ = self
                        .report
                        .send(Heard::Advertised(self.relay, advertisement));
                }
            }
        }

        /// Waits before the next dial, or until the monitor stops.
        fn rest(&self) {
            let until = Instant::now() + self.dialling.redial;
            while !self.stopping() && Instant::now() < until {
                thread::sleep(TICK);
            }
        }
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
            self.0
                .read(buffer, wait.map_or(TICK, |wait| wait.min(TICK)))
        }
    }

    /// The operating system's generator, which is all the link code draws
    /// on.
    struct System;

    impl Randomness for System {
        fn fill(&mut self, bytes: &mut [u8]) {
            randomness::fill(bytes).expect("the operating system's generator gives bytes");
        }
    }
}
