//! What the pairing tests share (#495): the two ends' introductions, a
//! relay's link invitation, and the on-path proxy.

use super::Seeded;
use climon2_link::{
    End, HubEnd, Input, Introduction, Invitation, Invitations, Output, Pin, RelayEnd, Revisions,
    Time,
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

pub const SECOND: Duration = Duration::from_secs(1);
pub const MILLISECOND: Duration = Duration::from_millis(1);
/// The size of an opening request and of an opening reply.
pub const OPENING: (usize, usize) = (26, 20);
/// A PIN the relay of these tests never mints.
pub const WRONG: &str = "1111-2222";

pub fn pin(typed: &str) -> Pin {
    Pin::parse(typed).expect("8 digits")
}

/// The pairing messages in `bytes`, each without its length. Whatever
/// follows the last whole message is returned as a message of its own.
pub fn messages(mut bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut found = Vec::new();
    while let Some((length, rest)) = bytes.split_first_chunk::<2>() {
        let length = usize::from(u16::from_be_bytes(*length)).min(rest.len());
        found.push(rest[..length].to_vec());
        bytes = &rest[length..];
    }
    if !bytes.is_empty() {
        found.push(bytes.to_vec());
    }
    found
}

/// The pairing messages an end sent after its opening message.
pub fn pairing_messages(sent: &[u8], opening: usize) -> Vec<Vec<u8>> {
    messages(&sent[opening..])
}

/// How many attempts the relay's invitation has left at time zero.
pub fn attempts_left(invitations: &Invitations) -> u8 {
    match invitations.invitation(Time::since_start(Duration::ZERO)) {
        Invitation::Live { attempts_left, .. } => attempts_left,
        other => panic!("no live invitation: {other:?}"),
    }
}

/// Two ends stepped by hand, each with randomness of its own, at moments
/// the test names.
pub struct Wire {
    pub hub: HubEnd,
    pub relay: RelayEnd,
    pub hub_randomness: Seeded,
    pub relay_randomness: Seeded,
}

impl Wire {
    pub fn new(hub: HubEnd, relay: RelayEnd) -> Self {
        Self {
            hub,
            relay,
            hub_randomness: Seeded::new(1),
            relay_randomness: Seeded::new(2),
        }
    }

    pub fn hub(&mut self, when: Duration, input: Input<'_>) -> Output {
        self.hub.step(at(when), input, &mut self.hub_randomness)
    }

    pub fn relay(&mut self, when: Duration, input: Input<'_>) -> Output {
        self.relay.step(at(when), input, &mut self.relay_randomness)
    }

    /// Runs the opening exchange at `when`: the relay accepts, the hub
    /// monitor dials, and the reply reaches the hub monitor. The hub
    /// monitor's share message, which it sends at once.
    pub fn open(&mut self, when: Duration) -> Output {
        let request = self.hub(when, Input::Tick).bytes;
        self.relay(when, Input::Tick);
        let reply = self.relay(when, Input::Bytes(&request)).bytes;
        self.hub(when, Input::Bytes(&reply))
    }

    /// Runs the opening exchange and both shares at `when`: the relay's
    /// share message, and the hub monitor's confirmation.
    pub fn shares(&mut self, when: Duration) -> (Output, Output) {
        let hub_share = self.open(when).bytes;
        let relay_share = self.relay(when, Input::Bytes(&hub_share));
        let confirmation = self.hub(when, Input::Bytes(&relay_share.bytes));
        (relay_share, confirmation)
    }
}

/// An output that closes the byte stream without sending a byte.
pub fn silent_close() -> Output {
    Output {
        bytes: Vec::new(),
        close: true,
        deadline: None,
    }
}
