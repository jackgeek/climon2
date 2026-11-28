//! What the pairing tests share (#495): the two ends' introductions, a
//! relay's link invitation, and the on-path proxy.

use super::Seeded;
use climon2_link::{
    End, HubEnd, Input, Introduction, Invitations, Output, Pin, RelayEnd, Revisions, Time,
};
use std::time::Duration;

/// The static public key the hub monitor hands over in these tests.
pub const HUB_KEY: [u8; 32] = [
    0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e, 0x4f,
    0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f,
];
/// The relay's identity key in these tests.
pub const RELAY_KEY: [u8; 32] = [
    0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf,
    0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbb, 0xbc, 0xbd, 0xbe, 0xbf,
];
/// The seed of the randomness these tests mint their PIN from.
pub const MINT_SEED: u64 = 3;

pub fn hub_introduction() -> Introduction {
    Introduction::new(HUB_KEY, "laptop").expect("a short name")
}

pub fn relay_introduction() -> Introduction {
    Introduction::new(RELAY_KEY, "büild-box").expect("a short name")
}

pub fn revisions(list: &[u16]) -> Revisions {
    Revisions::new(list).expect("a valid revision list")
}

/// A relay's invitations with none minted.
pub fn uninvited() -> Invitations {
    Invitations::new(relay_introduction())
}

/// A relay's invitations with one minted at time zero, and its PIN.
pub fn invited() -> (Invitations, Pin) {
    let invitations = uninvited();
    let pin = invitations.mint(at(Duration::ZERO), &mut Seeded::new(MINT_SEED));
    (invitations, pin)
}

/// The moment `elapsed` after the start.
pub fn at(elapsed: Duration) -> Time {
    Time::since_start(elapsed)
}

/// A hub monitor's end that pairs with `pin`, supporting revision 1.
pub fn pairing_hub(pin: &Pin) -> HubEnd {
    HubEnd::pair(revisions(&[1]), pin.clone(), hub_introduction())
}

/// A relay's end under `invitations`, supporting revision 1.
pub fn pairing_relay(invitations: &Invitations) -> RelayEnd {
    RelayEnd::new(revisions(&[1]), invitations.clone())
}

/// Which way a message travels through the on-path proxy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    /// From the hub monitor to the relay.
    ToRelay,
    /// From the relay to the hub monitor.
    ToHub,
}

/// One end behind the on-path proxy, and what it did there.
pub struct Proxied<E> {
    pub end: E,
    randomness: Seeded,
    /// What the end sent at its last steps and the proxy has not passed on.
    waiting: Output,
    /// Every message the end sent, as it left the end.
    pub sent: Vec<Vec<u8>>,
    /// Whether the end closed its byte stream.
    pub closed: bool,
}

impl<E: End> Proxied<E> {
    fn new(end: E, seed: u64) -> Self {
        Self {
            end,
            randomness: Seeded::new(seed),
            waiting: Output::default(),
            sent: Vec::new(),
            closed: false,
        }
    }

    /// Steps the end at time zero, unless it has closed its byte stream.
    fn step(&mut self, input: Input<'_>) {
        if self.closed {
            return;
        }
        let now = at(Duration::ZERO);
        let output = self.end.step(now, input, &mut self.randomness);
        self.closed = output.close;
        self.waiting.bytes.extend(output.bytes);
        self.waiting.close |= output.close;
    }

    /// Passes on to `other` what this end sent since the last pass, the
    /// bytes through `edit`. Whether there was anything to pass on.
    fn pass<O: End>(
        &mut self,
        other: &mut Proxied<O>,
        edit: impl FnOnce(usize, &mut Vec<u8>),
    ) -> bool {
        let Output { bytes, close, .. } = std::mem::take(&mut self.waiting);
        let spoke = !bytes.is_empty();
        if spoke {
            let mut message = bytes.clone();
            edit(self.sent.len(), &mut message);
            self.sent.push(bytes);
            if !message.is_empty() {
                other.step(Input::Bytes(&message));
            }
        }
        if close {
            other.step(Input::Closed);
        }
        spoke || close
    }
}

/// Runs one connection with the on-path proxy between the ends, all at
/// time zero, until both are quiet. Each end sends at most one message a
/// step, and the proxy hands `edit` every message with its direction and
/// its number in that direction, counted from zero: the opening message,
/// the share, the confirmation. `edit` may change the bytes in any way; a
/// message it empties is dropped. Closes are passed on as they are.
pub fn proxied<H: End, R: End>(
    hub: H,
    relay: R,
    edit: impl FnMut(Way, usize, &mut Vec<u8>),
) -> (Proxied<H>, Proxied<R>) {
    proxied_with_seeds(hub, relay, (1, 2), edit)
}

/// As `proxied`, with the seed of each end's randomness, hub monitor first.
pub fn proxied_with_seeds<H: End, R: End>(
    hub: H,
    relay: R,
    seeds: (u64, u64),
    mut edit: impl FnMut(Way, usize, &mut Vec<u8>),
) -> (Proxied<H>, Proxied<R>) {
    let (mut hub, mut relay) = (Proxied::new(hub, seeds.0), Proxied::new(relay, seeds.1));
    hub.step(Input::Tick);
    relay.step(Input::Tick);
    for _ in 0..16 {
        let to_relay = hub.pass(&mut relay, |index, bytes| edit(Way::ToRelay, index, bytes));
        let to_hub = relay.pass(&mut hub, |index, bytes| edit(Way::ToHub, index, bytes));
        if !to_relay && !to_hub {
            return (hub, relay);
        }
    }
    panic!("the two ends never fell quiet");
}
