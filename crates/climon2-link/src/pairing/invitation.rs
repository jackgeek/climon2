//! The link invitation: the one PIN a relay has live, and the rules of its
//! lifetime (the pairing ceremony, #193). One relay has one [`Invitations`];
//! every connection it accepts holds a copy, so that all of them see the
//! same invitation.

use super::{Introduction, Pin};
use crate::{Randomness, Time};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// How long an invitation stays live after it is minted.
const LIFETIME: Duration = Duration::from_secs(5 * 60);
/// How many failed pairing runs an invitation survives, the last one not.
const ATTEMPTS: u8 = 3;

/// The link invitation of one relay, as its user sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invitation {
    /// The relay's user has minted none.
    None,
    /// A hub monitor that holds the PIN can pair.
    Live {
        /// The moment the invitation expires.
        expires: Time,
        /// How many more pairing runs may fail before it is used up.
        attempts_left: u8,
    },
    /// A hub monitor paired with it, which consumed it. This is the key and
    /// the name that hub monitor handed over.
    Paired(Introduction),
    /// It reached 5 minutes unused.
    Expired,
    /// Its third pairing run failed.
    UsedUp,
    /// The relay's user cancelled it.
    Cancelled,
}

/// A relay's link invitation and what the relay hands a hub monitor that
/// pairs with it. Copies share one invitation: the relay keeps one to mint
/// and cancel with, and gives one to the end of every connection it accepts.
///
/// A relay has at most one live invitation. Nothing here mints one by
/// itself, and an invitation that has ended stays ended.
#[derive(Clone)]
pub struct Invitations(Arc<Mutex<Desk>>);

struct Desk {
    /// What the relay hands over: its identity key and its name.
    relay: Introduction,
    /// How many invitations were minted. A pairing run holds the number of
    /// the invitation it started under.
    minted: u64,
    /// The PIN while the invitation is live.
    pin: Option<Pin>,
    invitation: Invitation,
    /// Whether a pairing run of this invitation is in flight.
    running: bool,
}

impl Desk {
    /// Ends the invitation as `ended`: its PIN is forgotten.
    fn end(&mut self, ended: Invitation) {
        self.invitation = ended;
        self.pin = None;
    }

    /// Expires a live invitation whose time has come.
    fn settle(&mut self, now: Time) {
        if matches!(self.invitation, Invitation::Live { expires, .. } if now >= expires) {
            self.end(Invitation::Expired);
        }
    }
}

impl Invitations {
    /// A relay with no invitation, which hands `relay`, its identity key and
    /// its name, to each hub monitor that pairs.
    pub fn new(relay: Introduction) -> Self {
        Self(Arc::new(Mutex::new(Desk {
            relay,
            minted: 0,
            pin: None,
            invitation: Invitation::None,
            running: false,
        })))
    }

    fn desk(&self) -> MutexGuard<'_, Desk> {
        // The desk holds no invariant a panic elsewhere could have broken
        // half-way, so a poisoned lock is still good.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Mints an invitation at `now` and gives its PIN, drawn from
    /// `randomness`. It expires 5 minutes on. The invitation before it, if
    /// one was live, is revoked, and a pairing run in flight under it fails.
    pub fn mint(&self, now: Time, randomness: &mut dyn Randomness) -> Pin {
        let pin = Pin::draw(randomness);
        let mut desk = self.desk();
        desk.minted += 1;
        desk.running = false;
        desk.pin = Some(pin.clone());
        desk.invitation = Invitation::Live {
            expires: Time::since_start(now.elapsed().saturating_add(LIFETIME)),
            attempts_left: ATTEMPTS,
        };
        pin
    }

    /// Cancels the live invitation, if there is one. A pairing run in flight
    /// under it fails.
    pub fn cancel(&self) {
        let mut desk = self.desk();
        if matches!(desk.invitation, Invitation::Live { .. }) {
            desk.end(Invitation::Cancelled);
        }
    }

    /// The invitation as it stands at `now`.
    pub fn invitation(&self, now: Time) -> Invitation {
        let mut desk = self.desk();
        desk.settle(now);
        desk.invitation.clone()
    }

    /// Starts a pairing run at `now`, or nothing when none can start: no
    /// invitation is live, or another run is in flight.
    pub(crate) fn seat(&self, now: Time) -> Option<Seat> {
        let mut desk = self.desk();
        desk.settle(now);
        let pin = desk.pin.clone().filter(|_| !desk.running)?;
        desk.running = true;
        Some(Seat {
            invitations: self.clone(),
            invitation: desk.minted,
            pin,
            relay: desk.relay.clone(),
            counted: false,
            over: false,
        })
    }
}

/// One pairing run's hold on the invitation it started under. While it
/// lasts no other run starts. Dropping it ends the run as a failure, which
/// uses up an attempt once the relay's share has been sent, so a run that is
/// abandoned in any way is counted like one that failed.
pub(crate) struct Seat {
    invitations: Invitations,
    /// Which invitation the run started under, by `Desk::minted`.
    invitation: u64,
    /// The invitation's PIN.
    pub(crate) pin: Pin,
    /// What the relay hands over.
    pub(crate) relay: Introduction,
    /// Whether the relay's share has been sent.
    counted: bool,
    /// Whether the run has ended and the invitation knows.
    over: bool,
}

impl Seat {
    /// Records that the relay's share is sent: from here a failure counts.
    pub(crate) fn count(&mut self) {
        self.counted = true;
    }

    /// Whether a failure of this run counts as an attempt.
    pub(crate) fn counted(&self) -> bool {
        self.counted
    }

    /// The desk, when this run's invitation is still the relay's current
    /// one: a newer invitation owes this run nothing.
    fn desk(&self) -> Option<MutexGuard<'_, Desk>> {
        Some(self.invitations.desk()).filter(|desk| desk.minted == self.invitation)
    }

    /// Ends the run as a success at `now`: `hub` paired, which consumes the
    /// invitation. False when the invitation is no longer live, and then
    /// nothing is paired.
    pub(crate) fn pair(&mut self, now: Time, hub: &Introduction) -> bool {
        let paired = self.desk().is_some_and(|mut desk| {
            desk.settle(now);
            let live = matches!(desk.invitation, Invitation::Live { .. });
            if live {
                desk.end(Invitation::Paired(hub.clone()));
                desk.running = false;
            }
            live
        });
        self.over |= paired;
        paired
    }

    /// Ends the run as a failure. `now`, when known, first expires an
    /// invitation whose time has come.
    fn fail(&mut self, now: Option<Time>) {
        if std::mem::replace(&mut self.over, true) {
            return;
        }
        let counted = self.counted;
        let Some(mut desk) = self.desk() else {
            return;
        };
        desk.running = false;
        if let Some(now) = now {
            desk.settle(now);
        }
        match &mut desk.invitation {
            Invitation::Live { attempts_left, .. } if counted => {
                *attempts_left -= 1;
                if *attempts_left == 0 {
                    desk.end(Invitation::UsedUp);
                }
            }
            _ => {}
        }
    }

    /// Ends the run as a failure at `now`.
    pub(crate) fn failed(&mut self, now: Time) {
        self.fail(Some(now));
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        self.fail(None);
    }
}
