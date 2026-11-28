//! HOS-05 at the Link layer (#498): arbitrary bytes, from a recorded seed
//! and for a bounded number of cases, fed to every decoder of the link
//! code through the two ends, in the ordinary test run.
//!
//! Each target is a place in a connection where an end decodes what the
//! other end sent: the opening request and reply, each pairing message,
//! each handshake message, the units, and, sealed in units the link peer
//! stand-in encrypts, Hello, Close, every advertisement and session-stream
//! record and every client interface record. Half the cases of a target
//! are bytes drawn at random behind a prefix that reaches the decoder (a
//! length, a tag); the other half are a valid message of the target with
//! a random edit: cut short, a byte changed, four bytes overwritten, as a
//! length or count is, or bytes added.
//!
//! No case may panic, which in Rust is also how a read past the input
//! shows, and while an end handles one case it may hold at most
//! [`HELD_MOST`] bytes more than before, whatever lengths and counts the
//! bytes claim. This test binary's global allocator measures that.
//!
//! The box inside a pairing confirmation opens only under the run's keys,
//! so arbitrary bytes from outside never reach its decoder; a unit test in
//! `src/pairing/mod.rs` feeds it arbitrary plaintext instead.

mod samples;
mod support;
mod wire;

use climon2_link::{End, HubEnd, Input, Pin, Randomness, RelayEnd};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::Duration;
use support::Seeded;
use support::pairing::{Wire, invited, pairing_hub, pairing_relay};
use wire::{REPLY, REQUEST, StandIn, hub_end, hub_key, relay_end, relay_key, step};

/// The seed every case is drawn from.
const SEED: u64 = 0x0498;
/// The cases of each target.
const CASES: usize = 96;
/// The most bytes an end may hold beyond what it held before one case: four
/// units of 16,384 bytes. A decoder that believed a length or a count it
/// was handed would hold far more.
const HELD_MOST: isize = 4 * 16_384;
/// The longest arbitrary message a case draws.
const LONGEST: usize = 600;

struct Counting;

thread_local! {
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

fn note(delta: isize) {
    let _ = LIVE.try_with(|live| {
        let now = live.get().wrapping_add(delta);
        live.set(now);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
    });
}

// SAFETY: every call is forwarded unchanged to the system allocator; the
// counters only observe sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size().cast_signed());
        // SAFETY: forwarded with the caller's layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note(layout.size().cast_signed().wrapping_neg());
        // SAFETY: forwarded with the caller's pointer and layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// The most this thread holds while `work` runs, beyond what it held
/// before.
fn held_by(work: impl FnOnce()) -> isize {
    let start = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(start));
    work();
    PEAK.with(Cell::get) - start
}

/// The cases' randomness, with the draws a case needs.
struct Draw(Seeded);

impl Draw {
    fn below(&mut self, bound: usize) -> usize {
        let mut bytes = [0; 8];
        self.0.fill(&mut bytes);
        (u64::from_be_bytes(bytes) % bound as u64) as usize
    }

    fn bytes(&mut self, count: usize) -> Vec<u8> {
        let mut bytes = vec![0; count];
        self.0.fill(&mut bytes);
        bytes
    }

    /// Up to `LONGEST` random bytes.
    fn message(&mut self) -> Vec<u8> {
        let count = self.below(LONGEST + 1);
        self.bytes(count)
    }

    /// `valid` with one random edit.
    fn edit(&mut self, valid: &[u8]) -> Vec<u8> {
        let mut bytes = valid.to_vec();
        let at = self.below(bytes.len().max(1));
        match self.below(4) {
            0 => bytes.truncate(at),
            1 if !bytes.is_empty() => bytes[at] ^= 1 << self.below(8),
            2 if bytes.len() >= 4 => {
                let at = at.min(bytes.len() - 4);
                let word = [[0xff; 4], [0, 0, 0, 0], [0x7f, 0xff, 0xff, 0xff], {
                    let mut word = [0; 4];
                    self.0.fill(&mut word);
                    word
                }][self.below(4)];
                bytes[at..at + 4].copy_from_slice(&word);
            }
            _ => {
                let count = self.below(64) + 1;
                bytes.extend(self.bytes(count));
            }
        }
        bytes
    }

    /// The bytes of one case of a target: a random message behind
    /// `prefix`, or one of `valid` edited.
    fn case(&mut self, prefix: &[u8], valid: &[Vec<u8>]) -> Vec<u8> {
        match valid.is_empty() || self.below(2) == 0 {
            true => [prefix.to_vec(), self.message()].concat(),
            false => {
                let chosen = self.below(valid.len());
                self.edit(&valid[chosen])
            }
        }
    }
}

/// Runs `CASES` cases of the target `name`, each drawn from `prefix` and
/// `valid` (see [`Draw::case`]). `setup` makes a fresh end and gives the
/// work that hands it a case's bytes, which is measured. No case may hold
/// more than [`HELD_MOST`] bytes beyond the most a valid message holds.
fn target<W: FnOnce()>(
    name: &str,
    draw: &mut Draw,
    prefix: &[u8],
    valid: &[Vec<u8>],
    mut setup: impl FnMut(Vec<u8>) -> W,
) {
    let baseline = valid
        .iter()
        .map(|message| held_by(setup(message.clone())))
        .max()
        .unwrap_or(0);
    for number in 0..CASES {
        let bytes = draw.case(prefix, valid);
        let held = held_by(setup(bytes));
        assert!(
            held <= baseline + HELD_MOST,
            "{name}, case {number}: the end held {held} more bytes, a valid message {baseline}"
        );
    }
}

fn draw(salt: u64) -> Draw {
    Draw(Seeded::new(SEED ^ salt))
}

const LATER: Duration = Duration::from_secs(2);

/// HOS-05: the opening request at the relay and the opening reply at the
/// hub monitor.
#[test]
fn hos_05_the_opening_decoders_take_arbitrary_bytes() {
    let mut draw = draw(1);
    let request = [REQUEST.to_vec()];
    target(
        "opening request",
        &mut draw,
        &REQUEST[..8],
        &request,
        |bytes| {
            let mut relay = relay_end(relay_key(), &[hub_key().public()]);
            step(&mut relay, Duration::ZERO, Input::Tick);
            move || drop(step(&mut relay, Duration::ZERO, Input::Bytes(&bytes)))
        },
    );
    target(
        "opening reply",
        &mut draw,
        &[],
        &[REPLY.to_vec()],
        |bytes| {
            let mut hub = hub_end(hub_key(), relay_key().public());
            step(&mut hub, Duration::ZERO, Input::Tick);
            move || drop(step(&mut hub, Duration::ZERO, Input::Bytes(&bytes)))
        },
    );
}

/// The two ends of a pairing connection with its opening exchange done,
/// and the hub monitor's share, which the relay has not been handed.
fn pairing() -> (Wire, Vec<u8>) {
    let (invitations, pin) = invited();
    let mut wire = Wire::new(pairing_hub(&pin), pairing_relay(&invitations));
    let share = wire.open(Duration::ZERO).bytes;
    (wire, share)
}

/// HOS-05: each pairing message, at each end: the shares, which hold a
/// point of the group, and the confirmations.
#[test]
fn hos_05_the_pairing_decoders_take_arbitrary_bytes() {
    let mut draw = draw(2);
    let (mut valid, hub_share) = pairing();
    let relay_share = valid.relay(Duration::ZERO, Input::Bytes(&hub_share)).bytes;
    let hub_confirmation = valid.hub(Duration::ZERO, Input::Bytes(&relay_share)).bytes;
    let relay_confirmation = valid
        .relay(Duration::ZERO, Input::Bytes(&hub_confirmation))
        .bytes;

    let name = "the hub monitor's share";
    target(name, &mut draw, &[0x00, 0x20], &[hub_share], |bytes| {
        let (mut wire, _) = pairing();
        move || drop(wire.relay(Duration::ZERO, Input::Bytes(&bytes)))
    });
    let name = "the relay's share";
    target(name, &mut draw, &[0x00, 0x20], &[relay_share], |bytes| {
        let (mut wire, _) = pairing();
        move || drop(wire.hub(Duration::ZERO, Input::Bytes(&bytes)))
    });
    let name = "the hub monitor's confirmation";
    target(
        name,
        &mut draw,
        &[0x00, 0x77],
        &[hub_confirmation],
        |bytes| {
            let (mut wire, share) = pairing();
            wire.relay(Duration::ZERO, Input::Bytes(&share));
            move || drop(wire.relay(Duration::ZERO, Input::Bytes(&bytes)))
        },
    );
    let name = "the relay's confirmation";
    target(
        name,
        &mut draw,
        &[0x00, 0x7b],
        &[relay_confirmation],
        |bytes| {
            let (mut wire, share) = pairing();
            let relay_share = wire.relay(Duration::ZERO, Input::Bytes(&share)).bytes;
            wire.hub(Duration::ZERO, Input::Bytes(&relay_share));
            move || drop(wire.hub(Duration::ZERO, Input::Bytes(&bytes)))
        },
    );
}

/// HOS-05: a PIN as a user types it.
#[test]
fn hos_05_the_pin_reader_takes_arbitrary_text() {
    let mut draw = draw(3);
    let typed = [b"1234-5678".to_vec(), b"12 34 56 78".to_vec()];
    target("a typed PIN", &mut draw, b"1234-", &typed, |bytes| {
        let typed = String::from_utf8_lossy(&bytes).into_owned();
        move || {
            Pin::parse(&typed);
        }
    });
}

/// A relay's end with its opening exchange done, awaiting the handshake.
fn relay_awaiting_initiation() -> RelayEnd {
    let mut relay = relay_end(relay_key(), &[hub_key().public()]);
    step(&mut relay, Duration::ZERO, Input::Tick);
    step(&mut relay, Duration::ZERO, Input::Bytes(&REQUEST));
    relay
}

/// A hub monitor's end with its initiation sent, awaiting the reply, and
/// that initiation.
fn hub_awaiting_reply() -> (HubEnd, Vec<u8>) {
    let mut hub = hub_end(hub_key(), relay_key().public());
    step(&mut hub, Duration::ZERO, Input::Tick);
    let initiation = step(&mut hub, Duration::ZERO, Input::Bytes(&REPLY)).bytes;
    (hub, initiation)
}

/// HOS-05: the handshake initiation at the relay and the handshake reply
/// at the hub monitor.
#[test]
fn hos_05_the_handshake_decoders_take_arbitrary_bytes() {
    let mut draw = draw(4);
    let (_, initiation) = hub_awaiting_reply();
    let reply = step(
        &mut relay_awaiting_initiation(),
        Duration::ZERO,
        Input::Bytes(&initiation),
    )
    .bytes;
    let name = "the handshake initiation";
    target(name, &mut draw, &[0x00, 0x60], &[initiation], |bytes| {
        let mut relay = relay_awaiting_initiation();
        move || drop(step(&mut relay, Duration::ZERO, Input::Bytes(&bytes)))
    });
    let name = "the handshake reply";
    target(name, &mut draw, &[0x00, 0x30], &[reply], |bytes| {
        let (mut hub, _) = hub_awaiting_reply();
        move || drop(step(&mut hub, Duration::ZERO, Input::Bytes(&bytes)))
    });
}

/// Every record the relay may receive on a session stream after Stream
/// opened, whole: each client interface request, Credit and Close stream.
fn hub_stream_records() -> Vec<Vec<u8>> {
    let mut records: Vec<Vec<u8>> = samples::requests()
        .into_iter()
        .map(|(_, bytes)| bytes)
        .collect();
    records.push(vec![0x25, 0, 0, 0x10, 0]);
    records.push(vec![0x23]);
    records
}

/// Every record the hub monitor may receive on a session stream: each
/// client interface event and response, and the relay's own reports.
fn relay_stream_records() -> Vec<Vec<u8>> {
    let events = samples::events().into_iter().chain(samples::responses());
    let mut records: Vec<Vec<u8>> = events.map(|(_, bytes)| bytes).collect();
    records.push(samples::stream_opened_record());
    records
}

/// Units on `stream` that each hold one of `records` whole.
fn on(stream: u32, records: &[Vec<u8>]) -> Vec<Vec<u8>> {
    records
        .iter()
        .map(|record| samples::on(stream, record))
        .collect()
}

/// The work that hands `end` the unit the stand-in seals around
/// `plaintext`, whatever it is.
fn sealed<E: End>(
    (mut end, mut stand_in): (E, StandIn),
    plaintext: &[u8],
) -> impl FnOnce() + use<E> {
    let unit = stand_in.seal(plaintext);
    move || drop(step(&mut end, LATER, Input::Bytes(&unit)))
}

/// A relay's end dialled by the stand-in, which has not sent its Hello.
fn relay_before_hello() -> (RelayEnd, StandIn) {
    let mut relay = relay_end(relay_key(), &[hub_key().public()]);
    let stand_in = StandIn::dial(&mut relay, [0x11; 32], relay_key().public());
    (relay, stand_in)
}

/// A hub monitor's end answered by the stand-in, which has not sent its
/// Hello.
fn hub_before_hello() -> (HubEnd, StandIn) {
    let mut hub = hub_end(hub_key(), relay_key().public());
    let stand_in = StandIn::answer(&mut hub, [0x22; 32]);
    (hub, stand_in)
}

/// HOS-05: the units and every record inside them, at each end. Some
/// cases hand an end raw bytes as units, which do not decrypt; the rest
/// are units the stand-in seals, so their plaintext reaches the unit's own
/// layout and the record decoders behind it.
#[test]
fn hos_05_the_unit_and_record_decoders_take_arbitrary_bytes() {
    let mut draw = draw(5);
    target(
        "a unit at the relay",
        &mut draw,
        &[0x00, 0x40],
        &[],
        |bytes| {
            let (mut relay, _) = samples::linked_relay();
            move || drop(step(&mut relay, LATER, Input::Bytes(&bytes)))
        },
    );
    let name = "a unit at the hub monitor";
    target(name, &mut draw, &[0x00, 0x40], &[], |bytes| {
        let (mut hub, _) = samples::linked_hub();
        move || drop(step(&mut hub, LATER, Input::Bytes(&bytes)))
    });

    let control = [0, 0, 0, 0, 0];
    let first = [0, 0, 0, 0, 0, 0x01];
    let name = "the hub monitor's Hello at the relay";
    let hello = on(0, &[wire::HELLO.to_vec()]);
    target(name, &mut draw, &first, &hello, |plaintext| {
        sealed(relay_before_hello(), &plaintext)
    });
    let name = "the relay's Hello or Close at the hub monitor";
    let hello_or_close = on(0, &[samples::RELAY_HELLO.to_vec(), vec![0x02, 0x00, 0x05]]);
    target(name, &mut draw, &first, &hello_or_close, |plaintext| {
        sealed(hub_before_hello(), &plaintext)
    });
    let name = "stream 0 at the relay";
    let close = on(0, &[vec![0x02, 0x00, 0x20]]);
    target(name, &mut draw, &control, &close, |plaintext| {
        sealed(samples::linked_relay(), &plaintext)
    });
    let name = "stream 0 at the hub monitor";
    let advertisements = on(
        0,
        &[
            samples::session_record(),
            vec![0x12],
            [[0x11].as_slice(), &samples::SESSION_ID].concat(),
            vec![0x02, 0x00, 0x02],
        ],
    );
    target(name, &mut draw, &control, &advertisements, |plaintext| {
        sealed(samples::linked_hub(), &plaintext)
    });
    let name = "a session stream at the relay";
    let mut requests = on(1, &hub_stream_records());
    requests.extend(on(2, &[samples::open_stream_record()]));
    target(name, &mut draw, &[0, 0, 0, 1, 0], &requests, |plaintext| {
        sealed(samples::relay_streaming(), &plaintext)
    });
    let name = "a session stream at the hub monitor";
    let events = on(1, &relay_stream_records());
    target(name, &mut draw, &[0, 0, 0, 1, 0], &events, |plaintext| {
        sealed(samples::hub_streaming(), &plaintext)
    });
}

/// The control: the allocator sees what an end holds, so a case that
/// made an end hold a large record would fail. A relay's event of 200,000
/// bytes, queued in one step, holds more than the bound.
#[test]
fn hos_05_the_measure_sees_what_an_end_holds() {
    let (mut relay, _) = samples::relay_streaming();
    let event = climon2_link::RelayStream::Event(samples::display(200_000, b'x'));
    let held = held_by(|| {
        relay
            .send(&samples::from_relay(1, event))
            .expect("an event within credit");
    });
    assert!(held > HELD_MOST, "held {held}");
}
