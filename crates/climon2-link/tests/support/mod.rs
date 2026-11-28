//! The Link-layer harness (#492): it drives a hub monitor's end and a
//! relay's end of one byte stream, in memory, and supplies their time and
//! randomness. Nothing here touches a socket, a file, a clock or a process,
//! so a Link test runs the same on every platform.

// Each test binary compiles this module for itself and uses its own part.
#![allow(dead_code)]

pub mod pairing;

use climon2_link::{End, Input, Output, Randomness, Time};
use std::time::Duration;

/// The steps one `run` may take before the harness decides the two ends
/// will never fall quiet.
const MOST_STEPS: usize = 10_000;

/// Randomness a test supplies: the same seed always gives the same bytes
/// (the SplitMix64 generator, which is not for secrets).
pub struct Seeded(u64);

impl Seeded {
    pub fn new(seed: u64) -> Self {
        Self(seed)
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

/// One end as the harness holds it: its randomness and the byte stream that
/// carries what it sends to the other end.
struct Side {
    randomness: Seeded,
    /// Every byte this end has sent.
    sent: Vec<u8>,
    /// How many of `sent` the other end has been handed.
    delivered: usize,
    /// This end closed its byte stream and is stepped no more.
    closed: bool,
    /// The other end has been told of the close.
    close_delivered: bool,
    /// The deadline this end named at its last step.
    deadline: Option<Time>,
}

impl Side {
    fn new(seed: u64) -> Self {
        Self {
            randomness: Seeded::new(seed),
            sent: Vec::new(),
            delivered: 0,
            closed: false,
            close_delivered: false,
            deadline: None,
        }
    }

    fn step(&mut self, end: &mut impl End, now: Time, input: Input<'_>) {
        if self.closed {
            return;
        }
        let Output {
            bytes,
            close,
            deadline,
        } = end.step(now, input, &mut self.randomness);
        self.sent.extend(bytes);
        self.closed = close;
        self.deadline = deadline;
    }

    /// The next thing the other end is due from this one, if anything:
    /// a fragment of bytes, or the close.
    fn due(&mut self, fragment: usize) -> Option<Option<Vec<u8>>> {
        let waiting = &self.sent[self.delivered..];
        if !waiting.is_empty() {
            let bytes = waiting[..fragment.min(waiting.len())].to_vec();
            self.delivered += bytes.len();
            return Some(Some(bytes));
        }
        let close = self.closed && !self.close_delivered;
        self.close_delivered |= close;
        close.then_some(None)
    }
}

/// Two ends joined by in-memory byte streams. `hub` is the hub monitor's
/// end, the dialler; `relay` is the relay's end, the listener.
pub struct Harness<H, R> {
    pub hub: H,
    pub relay: R,
    now: Duration,
    fragment: usize,
    hub_side: Side,
    relay_side: Side,
}

impl<H: End, R: End> Harness<H, R> {
    /// Joins two ends at time zero, each with randomness of its own.
    pub fn new(hub: H, relay: R) -> Self {
        Self::with_seeds(hub, relay, 1, 2)
    }

    /// As `new`, with the seed of each end's randomness.
    pub fn with_seeds(hub: H, relay: R, hub_seed: u64, relay_seed: u64) -> Self {
        Self {
            hub,
            relay,
            now: Duration::ZERO,
            fragment: usize::MAX,
            hub_side: Side::new(hub_seed),
            relay_side: Side::new(relay_seed),
        }
    }

    /// Hands bytes over at most `bytes` at a time from now on. Without it
    /// everything an end has sent arrives at once.
    pub fn fragment(&mut self, bytes: usize) {
        assert!(bytes > 0, "a fragment holds at least one byte");
        self.fragment = bytes;
    }

    /// The time the ends are told.
    pub fn now(&self) -> Time {
        Time::since_start(self.now)
    }

    /// Moves the time on by `by`, then runs the ends. On the way it stops
    /// at every deadline an end has named and runs the ends there, as a
    /// caller must.
    pub fn advance(&mut self, by: Duration) {
        let target = self.now + by;
        while let Some(deadline) = self.deadline_before(target) {
            self.now = deadline;
            self.run();
        }
        self.now = target;
        self.run();
    }

    /// The earliest deadline either end has named that is still to come and
    /// falls before `target`.
    fn deadline_before(&self, target: Duration) -> Option<Duration> {
        [self.hub_side.deadline, self.relay_side.deadline]
            .into_iter()
            .flatten()
            .map(Time::elapsed)
            .filter(|deadline| self.now < *deadline && *deadline < target)
            .min()
    }

    /// The deadline the hub monitor's end named at its last step.
    pub fn hub_deadline(&self) -> Option<Time> {
        self.hub_side.deadline
    }

    /// The deadline the relay's end named at its last step.
    pub fn relay_deadline(&self) -> Option<Time> {
        self.relay_side.deadline
    }

    /// Steps each end once with nothing arriving, the hub monitor's first,
    /// then carries bytes and closes between them until both are quiet.
    pub fn run(&mut self) {
        let now = self.now();
        self.hub_side.step(&mut self.hub, now, Input::Tick);
        self.relay_side.step(&mut self.relay, now, Input::Tick);
        for _ in 0..MOST_STEPS {
            let to_relay = self.hub_side.due(self.fragment);
            if let Some(arrival) = &to_relay {
                self.relay_side.step(&mut self.relay, now, input(arrival));
            }
            let to_hub = self.relay_side.due(self.fragment);
            if let Some(arrival) = &to_hub {
                self.hub_side.step(&mut self.hub, now, input(arrival));
            }
            if to_relay.is_none() && to_hub.is_none() {
                return;
            }
        }
        panic!("the two ends never fell quiet");
    }

    /// Every byte the hub monitor's end has put on its byte stream.
    pub fn sent_by_hub(&self) -> &[u8] {
        &self.hub_side.sent
    }

    /// Every byte the relay's end has put on its byte stream.
    pub fn sent_by_relay(&self) -> &[u8] {
        &self.relay_side.sent
    }

    /// Whether the hub monitor's end has closed its byte stream.
    pub fn hub_closed(&self) -> bool {
        self.hub_side.closed
    }

    /// Whether the relay's end has closed its byte stream.
    pub fn relay_closed(&self) -> bool {
        self.relay_side.closed
    }
}

/// An arrival as the input of a step: bytes, or the close.
fn input(arrival: &Option<Vec<u8>>) -> Input<'_> {
    match arrival {
        Some(bytes) => Input::Bytes(bytes),
        None => Input::Closed,
    }
}
