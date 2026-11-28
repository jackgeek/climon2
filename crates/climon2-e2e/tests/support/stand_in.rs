//! The link peer stand-in (spec #488; acceptance, Evidence policy): a
//! test-only relay that speaks the monitor link's bytes itself, over
//! loopback, exact or with one deliberate defect. It is built from the link
//! code's own relay end, so an exact stand-in is an honest relay with no
//! terminal, no store and no sessions.

use climon2_link::{
    Advertisement, Carrier, Hello, Introduction, Invitation, Invitations, Link, Next, PublicKey,
    Randomness, RelayEnd, RelayRecord, Revisions, StaticKey, Time, drive,
};
use std::{
    io::{self, ErrorKind, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const WAIT: Duration = Duration::from_secs(20);

/// The one thing a stand-in relay gets wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Defect {
    /// None: it pairs, then links, as an honest relay does.
    Exact,
    /// It pairs with its identity key, then answers the hub monitor's first
    /// pinned-key link with another key.
    LinksWithAnotherKey,
}

/// A stand-in relay serving one pairing connection and the link connection
/// after it, or, advertising, every link connection until it finishes.
pub struct StandInRelay {
    address: SocketAddr,
    pin: String,
    identity: StaticKey,
    served: Option<JoinHandle<()>>,
    finishing: Arc<AtomicBool>,
}

impl StandInRelay {
    pub fn start(defect: Defect) -> Self {
        let mut randomness = Seeded::now();
        let identity = key(&mut randomness);
        let introduction = Introduction::new(*identity.public().as_bytes(), "stand-in").unwrap();
        let invitations = Invitations::new(introduction);
        let started = Instant::now();
        let pin = invitations.mint(Time::since_start(started.elapsed()), &mut randomness);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let linking = match defect {
            Defect::Exact => identity.clone(),
            Defect::LinksWithAnotherKey => key(&mut randomness),
        };
        let served = thread::spawn(move || {
            serve(&listener, &invitations, linking, started, &mut randomness);
        });
        Self {
            address,
            pin: pin.to_string(),
            identity,
            served: Some(served),
            finishing: Arc::new(AtomicBool::new(false)),
        }
    }

    /// An exact stand-in that pairs once, then serves link connections from
    /// the hub monitor it paired with until it finishes, advertising
    /// `advertised` on each once it is established. Each link stays up
    /// until the hub monitor ends it.
    pub fn advertising(advertised: Vec<Advertisement>) -> Self {
        let mut randomness = Seeded::now();
        let identity = key(&mut randomness);
        let introduction = Introduction::new(*identity.public().as_bytes(), "stand-in").unwrap();
        let invitations = Invitations::new(introduction);
        let started = Instant::now();
        let pin = invitations.mint(Time::since_start(started.elapsed()), &mut randomness);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let finishing = Arc::new(AtomicBool::new(false));
        let advertiser = Advertiser {
            listener,
            invitations,
            identity: identity.clone(),
            advertised,
            started,
            finishing: Arc::clone(&finishing),
        };
        let served = thread::spawn(move || advertiser.serve(&mut randomness));
        Self {
            address,
            pin: pin.to_string(),
            identity,
            served: Some(served),
            finishing,
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The PIN of its one invitation.
    pub fn pin(&self) -> &str {
        &self.pin
    }

    /// The key it hands over when it pairs.
    pub fn identity(&self) -> PublicKey {
        self.identity.public()
    }

    /// Waits until it has served both connections, or the first alone when
    /// no pairing came of it; an advertising stand-in stops serving once
    /// its current connection ends.
    pub fn finish(mut self) {
        self.finishing.store(true, Ordering::Relaxed);
        self.served.take().unwrap().join().unwrap();
    }
}

/// What an advertising stand-in serves.
struct Advertiser {
    listener: TcpListener,
    invitations: Invitations,
    identity: StaticKey,
    advertised: Vec<Advertisement>,
    started: Instant,
    finishing: Arc<AtomicBool>,
}

impl Advertiser {
    fn serve(&self, randomness: &mut Seeded) {
        let mut pairing = RelayEnd::new(Revisions::published(), self.invitations.clone());
        run(&mut pairing, &self.listener, self.started, randomness);
        let now = Time::since_start(self.started.elapsed());
        let Invitation::Paired(hub) = self.invitations.invitation(now) else {
            return;
        };
        let remembered = vec![PublicKey::from_bytes(*hub.key())];
        self.listener.set_nonblocking(true).unwrap();
        while !self.finishing.load(Ordering::Relaxed) {
            let Ok((stream, _)) = self.listener.accept() else {
                thread::sleep(Duration::from_millis(10));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            let hello = Hello::new("0.0.0-stand-in", "stand-in", 1_048_576).unwrap();
            let mut link = RelayEnd::new(Revisions::published(), self.invitations.clone()).linking(
                self.identity.clone(),
                remembered.clone(),
                hello,
            );
            let mut wire = Wire {
                stream,
                started: self.started,
            };
            let mut unsaid = Some(self.advertised.clone());
            let between = |end: &mut RelayEnd| Ok(advertise(end, &mut unsaid));
            drive(&mut link, &mut wire, randomness, between).unwrap();
        }
    }
}

/// Once the link is established, sends what is still unsaid.
fn advertise(end: &mut RelayEnd, unsaid: &mut Option<Vec<Advertisement>>) -> Next {
    if !matches!(end.link(), Link::Established(_)) {
        return Next::Wait;
    }
    let Some(advertised) = unsaid.take() else {
        return Next::Wait;
    };
    for advertisement in advertised {
        end.send(&RelayRecord::Advertisement(advertisement))
            .unwrap();
    }
    Next::Step
}

fn serve(
    listener: &TcpListener,
    invitations: &Invitations,
    linking: StaticKey,
    started: Instant,
    randomness: &mut Seeded,
) {
    let mut pairing = RelayEnd::new(Revisions::published(), invitations.clone());
    run(&mut pairing, listener, started, randomness);
    let Invitation::Paired(hub) = invitations.invitation(Time::since_start(started.elapsed()))
    else {
        return;
    };
    let hello = Hello::new("0.0.0-stand-in", "stand-in", 1_048_576).unwrap();
    let remembered = vec![PublicKey::from_bytes(*hub.key())];
    let mut link = RelayEnd::new(Revisions::published(), invitations.clone())
        .linking(linking, remembered, hello);
    run(&mut link, listener, started, randomness);
}

fn run(end: &mut RelayEnd, listener: &TcpListener, started: Instant, randomness: &mut Seeded) {
    let (stream, _) = listener.accept().unwrap();
    let mut wire = Wire { stream, started };
    drive(end, &mut wire, randomness, |_| Ok(Next::Wait)).unwrap();
}

fn key(randomness: &mut Seeded) -> StaticKey {
    let mut private = [0; 32];
    randomness.fill(&mut private);
    StaticKey::from_private(private)
}

struct Wire {
    stream: TcpStream,
    started: Instant,
}

impl Carrier for Wire {
    fn now(&mut self) -> Time {
        Time::since_start(self.started.elapsed())
    }

    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.stream.write_all(bytes)
    }

    fn receive(&mut self, buffer: &mut [u8], wait: Option<Duration>) -> io::Result<usize> {
        let wait = wait.unwrap_or(WAIT).max(Duration::from_millis(1));
        self.stream.set_read_timeout(Some(wait))?;
        self.stream
            .read(buffer)
            .map_err(|error| match error.kind() {
                ErrorKind::WouldBlock => ErrorKind::TimedOut.into(),
                _ => error,
            })
    }
}

/// Test randomness, seeded from the clock: the SplitMix64 generator, which
/// is not for secrets.
struct Seeded(u64);

impl Seeded {
    fn now() -> Self {
        Self(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64,
        )
    }
}

impl Randomness for Seeded {
    fn fill(&mut self, bytes: &mut [u8]) {
        for byte in bytes {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut mixed = self.0;
            mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            *byte = (mixed ^ (mixed >> 31)) as u8;
        }
    }
}
