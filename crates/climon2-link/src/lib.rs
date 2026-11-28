//! The link code of monitor links (ADR 0011): what a hub monitor and a relay
//! say to each other over a plain byte stream.
//!
//! The code here performs no input or output and reads no clock. Its caller
//! owns the byte stream and drives one [`End`] of it: every step hands the
//! end the [`Time`], what arrived and a source of [`Randomness`], and gets
//! back what to send. That keeps one implementation for every platform, and
//! lets a test supply the time and the randomness (ONE-01, ONE-02 of the
//! monitor-link acceptance criteria).

mod backlog;
pub mod client;
mod drive;
mod hub;
mod keys;
mod layout;
mod link;
mod liveness;
mod name;
mod noise;
mod opening;
mod pairing;
mod record;
mod relay;
mod rules;
mod stream;
mod unit;

pub use drive::{Carrier, Next, drive};
pub use hub::HubEnd;
pub use keys::{PublicKey, StaticKey};
pub use link::{Ended, HubLink, Link, Peer, Refused, RelayLink};
pub use name::display_name;
pub use opening::{Opened, Opening, Purpose, Revisions, Transcript};
pub use pairing::{Introduction, Invitation, Invitations, PairingRun, Pin};
pub use record::{Hello, HubClose, RelayClose};
pub use relay::RelayEnd;
pub use stream::{
    Advertisement, Cause, EndReason, HubRecord, HubStream, OpenStream, RelayRecord, RelayStream,
    RequestRefused, Session, SessionId, StreamEnded, StreamOpened, StreamRefused,
};

use std::time::Duration;

/// A moment on the caller's clock: the time elapsed since a start the caller
/// fixed once, which only ever grows. The link code learns the time from no
/// other source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Time(Duration);

impl Time {
    /// The moment `elapsed` after the caller's start.
    pub const fn since_start(elapsed: Duration) -> Self {
        Self(elapsed)
    }

    /// How long after the caller's start this moment is.
    pub const fn elapsed(self) -> Duration {
        self.0
    }

    /// The moment `by` after this one.
    pub(crate) fn after(self, by: Duration) -> Self {
        Self(self.0.saturating_add(by))
    }
}

/// The caller's source of random bytes. The link code draws randomness from
/// no other source; a shipped caller hands it the operating system's
/// generator.
pub trait Randomness {
    /// Fills `bytes` with random bytes.
    fn fill(&mut self, bytes: &mut [u8]);
}

/// What one step hands an [`End`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input<'a> {
    /// Nothing arrived. The caller steps an end this way when a byte stream
    /// starts and whenever time has passed.
    Tick,
    /// The next bytes of the byte stream, in order. A byte stream keeps no
    /// message boundaries, so they may end anywhere.
    Bytes(&'a [u8]),
    /// The other end closed the byte stream; no more bytes will arrive.
    Closed,
}

/// What an [`End`] asks of its caller after one step.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// The bytes to send next on the byte stream; often none.
    pub bytes: Vec<u8>,
    /// Whether to close the byte stream once `bytes` are sent. A caller
    /// never steps an end again after it asks this.
    pub close: bool,
    /// The moment the end has a deadline, if it has one. When nothing
    /// arrives sooner, the caller steps the end with [`Input::Tick`] at this
    /// moment and not later. Each step's answer replaces the one before.
    pub deadline: Option<Time>,
}

/// One end of one byte stream between a hub monitor and a relay.
pub trait End {
    /// Takes one input at the time `now` and says what to do next.
    fn step(&mut self, now: Time, input: Input<'_>, randomness: &mut dyn Randomness) -> Output;
}
