//! Pairing over a link invitation (#495): the four messages of a pairing
//! connection, as the protocol's pairing share and pairing confirmation
//! pages give them, and the link invitation that lets a run start.
//!
//! The hub monitor's end is [`hub::HubRun`] and the relay's is
//! [`relay::RelayRun`]. Each takes over its connection once the opening
//! exchange has chosen a link revision for the purpose of pairing.

mod cpace;
pub(crate) mod hub;
mod invitation;
mod keys;
pub(crate) mod relay;

pub use invitation::{Invitation, Invitations};

use crate::opening::Transcript;
use crate::{Input, Output, Randomness, Time};
use std::fmt;
use std::ops::RangeInclusive;
use std::time::Duration;

/// How long a pairing run may take, opening exchange included: from the
/// relay accepting the connection, and from the hub monitor's first step.
pub(crate) const RUN_WAIT: Duration = Duration::from_secs(30);

/// How many digits a PIN has.
const PIN_DIGITS: usize = 8;
/// One more than the largest PIN.
const PIN_COUNT: u32 = 100_000_000;
/// Four random bytes at or above this are drawn again: below it every PIN
/// is as likely as any other.
const PIN_UNBIASED: u32 = 42 * PIN_COUNT;
/// The longest name an end may suggest, in bytes.
const NAME_MOST: usize = 64;
/// The size of a static public key.
const KEY_SIZE: usize = 32;
/// What every channel identifier of a pairing key exchange starts with.
const CHANNEL: &[u8] = b"climon2 link pairing";
/// The size of the length that starts every pairing message.
const LENGTH_SIZE: usize = 2;
/// The sizes a pairing share may state: always one group element.
const SHARE_SIZES: RangeInclusive<usize> = cpace::SHARE_SIZE..=cpace::SHARE_SIZE;
/// The sizes a pairing confirmation may state: the confirmation and a box
/// that holds a key and a name of 0 to 64 bytes.
const CONFIRMATION_SIZES: RangeInclusive<usize> = 113..=177;

/// The PIN of a link invitation: 8 decimal digits. It is shown as
/// `1234-5678` and never crosses the byte stream. Its `Debug` form hides
/// the digits, so a PIN reaches a log only when its holder writes it there.
#[derive(Clone)]
pub struct Pin([u8; PIN_DIGITS]);

impl Pin {
    /// The PIN a user typed, or nothing when it is not 8 digits. Hyphens and
    /// spaces are ignored wherever they stand.
    pub fn parse(typed: &str) -> Option<Self> {
        let digits: Vec<u8> = typed
            .bytes()
            .filter(|byte| !matches!(byte, b' ' | b'-'))
            .collect();
        let digits: [u8; PIN_DIGITS] = digits.try_into().ok()?;
        digits
            .iter()
            .all(u8::is_ascii_digit)
            .then_some(Self(digits))
    }

    /// A new PIN, every one as likely as any other. It reads four bytes of
    /// `randomness` as a big-endian number and draws again when that number
    /// would favour the low PINs.
    fn draw(randomness: &mut dyn Randomness) -> Self {
        let mut bytes = <[u8; 4]>::default();
        let mut number = PIN_UNBIASED;
        while number >= PIN_UNBIASED {
            randomness.fill(&mut bytes);
            number = u32::from_be_bytes(bytes);
        }
        let mut digits = <[u8; PIN_DIGITS]>::default();
        for digit in digits.iter_mut().rev() {
            *digit = b'0' + (number % 10) as u8;
            number /= 10;
        }
        Self(digits)
    }

    /// The digits as ASCII: the password of the key exchange.
    fn digits(&self) -> &[u8; PIN_DIGITS] {
        &self.0
    }
}

impl fmt::Display for Pin {
    /// The form a relay shows: `1234-5678`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (first, second) = self.0.split_at(PIN_DIGITS / 2);
        for digit in first.iter().chain(b"-").chain(second) {
            write!(formatter, "{}", char::from(*digit))?;
        }
        Ok(())
    }
}

impl fmt::Debug for Pin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Pin(hidden)")
    }
}

/// What one end hands the other once a pairing run has confirmed the PIN:
/// its static public key for the pinned-key handshake and a suggested name.
/// The name is a label and never an identity, and a name from the other end
/// is hostile text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Introduction {
    key: [u8; KEY_SIZE],
    name: String,
}

impl Introduction {
    /// A static public key and a suggested name, or nothing when the name is
    /// longer than 64 bytes.
    pub fn new(key: [u8; KEY_SIZE], name: &str) -> Option<Self> {
        let name = name.to_owned();
        (name.len() <= NAME_MOST).then_some(Self { key, name })
    }

    /// The X25519 static public key.
    pub fn key(&self) -> &[u8; KEY_SIZE] {
        &self.key
    }

    /// The suggested display name: at most 64 bytes of UTF-8.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The plaintext of a box: the key, the name's length and the name.
    fn encode(&self) -> Vec<u8> {
        let mut bytes = self.key.to_vec();
        bytes.push(self.name.len() as u8);
        bytes.extend(self.name.bytes());
        bytes
    }

    /// The introduction a box held, or nothing when the plaintext breaks
    /// the layout: a name longer than 64 bytes, not as long as its length
    /// says, or not UTF-8.
    fn decode(bytes: &[u8]) -> Option<Self> {
        let (key, rest) = bytes.split_first_chunk()?;
        let (length, name) = rest.split_first()?;
        let name = str::from_utf8(name).ok()?;
        (name.len() == usize::from(*length)).then(|| Self::new(*key, name))?
    }
}

/// How far one end's pairing run has got, and how it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairingRun {
    /// The run has not ended. A connection whose opening exchange did not
    /// open for pairing never leaves this.
    Pending,
    /// The run succeeded: the other end proved it holds the PIN and handed
    /// over its key and name. The connection is closed.
    Paired(Introduction),
    /// The run failed and the connection is closed. This end keeps nothing
    /// of the other.
    Failed {
        /// Whether the relay's share had crossed the byte stream, the moment
        /// from which a run that fails is one of the invitation's attempts.
        counted: bool,
    },
}

/// The key exchange inputs both ends of a pairing run must hold alike: the
/// PIN as the password, and the opening transcript in the channel
/// identifier.
fn exchange<R>(pin: &Pin, transcript: &Transcript, run: impl FnOnce(cpace::Inputs<'_>) -> R) -> R {
    let channel = [CHANNEL, transcript.as_bytes()].concat();
    run(cpace::Inputs {
        password: pin.digits(),
        channel: &channel,
        session: &[],
        initiator_data: &[],
        responder_data: &[],
    })
}

/// A pairing share as it crosses the byte stream.
fn share_message(share: &[u8; cpace::SHARE_SIZE]) -> Vec<u8> {
    framed(share)
}

/// A pairing message: the size of `body`, then `body`.
fn framed(body: &[u8]) -> Vec<u8> {
    let mut bytes = (body.len() as u16).to_be_bytes().to_vec();
    bytes.extend(body);
    bytes
}

/// What the bytes that have arrived hold of the next pairing message.
enum Framed {
    /// Part of it is still to come.
    Partial,
    /// All of it: the bytes behind its length.
    Whole(Vec<u8>),
    /// Its length is one the layout does not allow.
    Malformed,
}

/// The byte stream as one end of a pairing run reads it: what has arrived
/// and not been read, and the deadline of the whole run.
struct Arrived {
    deadline: Time,
    bytes: Vec<u8>,
}

impl Arrived {
    /// Takes one step's input. False when the run can no longer succeed:
    /// the deadline has passed or the other end closed the byte stream.
    fn take(&mut self, now: Time, input: Input<'_>) -> bool {
        if let Input::Bytes(bytes) = input {
            self.bytes.extend(bytes);
        }
        now <= self.deadline && input != Input::Closed
    }

    /// The next message, when its length is one of `sizes`. A length
    /// outside them is malformed at once, before the rest arrives.
    fn message(&mut self, sizes: RangeInclusive<usize>) -> Framed {
        let Some(length) = self.bytes.first_chunk() else {
            return Framed::Partial;
        };
        let end = LENGTH_SIZE + usize::from(u16::from_be_bytes(*length));
        if !sizes.contains(&(end - LENGTH_SIZE)) {
            return Framed::Malformed;
        }
        if self.bytes.len() < end {
            return Framed::Partial;
        }
        Framed::Whole(self.bytes.drain(..end).skip(LENGTH_SIZE).collect())
    }

    /// The output of a step that leaves the run waiting, having sent
    /// `bytes`. Nothing when `now` is the deadline: a run still unfinished
    /// then has failed.
    fn waiting(&self, now: Time, bytes: Vec<u8>) -> Option<Output> {
        (now < self.deadline).then_some(Output {
            bytes,
            close: false,
            deadline: Some(self.deadline),
        })
    }
}

/// The output of a step that ends a run: `bytes`, then the close.
fn closing(bytes: Vec<u8>) -> Output {
    Output {
        bytes,
        close: true,
        deadline: None,
    }
}

/// What one end does with the next pairing message it has framed: `read`
/// takes it once it is whole, and what `read` gives to send joins `sent`.
/// Whether the end reads on: false while the message is still partial.
/// Nothing when the run has failed.
fn read_message(
    framed: Framed,
    sent: &mut Vec<u8>,
    read: impl FnOnce(Vec<u8>) -> Option<Vec<u8>>,
) -> Option<bool> {
    match framed {
        Framed::Partial => Some(false),
        Framed::Malformed => None,
        Framed::Whole(body) => {
            sent.extend(read(body)?);
            Some(true)
        }
    }
}

/// The output of a step of a run that has got as far as `outcome` and has
/// `sent` to send, or nothing when the run has failed: a run that paired
/// sends and closes, and one still pending waits until its deadline.
fn concluded(
    outcome: &PairingRun,
    arrived: &Arrived,
    now: Time,
    sent: Option<Vec<u8>>,
) -> Option<Output> {
    let bytes = sent?;
    if matches!(outcome, PairingRun::Paired(_)) {
        return Some(closing(bytes));
    }
    arrived.waiting(now, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Randomness that hands out the bytes a test wrote, in order.
    struct Scripted(Vec<u8>);

    impl Randomness for Scripted {
        fn fill(&mut self, bytes: &mut [u8]) {
            for byte in bytes {
                *byte = self.0.remove(0);
            }
        }
    }

    fn drawn(numbers: &[u32]) -> String {
        let bytes = numbers.iter().flat_map(|number| number.to_be_bytes());
        Pin::draw(&mut Scripted(bytes.collect())).to_string()
    }

    #[test]
    fn a_pin_is_four_random_bytes_as_a_number_below_4_2_billion() {
        assert_eq!(drawn(&[0]), "0000-0000");
        assert_eq!(drawn(&[12_345_678]), "1234-5678");
        assert_eq!(drawn(&[4_199_999_999]), "9999-9999");
        assert_eq!(drawn(&[100_000_000]), "0000-0000");
    }

    #[test]
    fn a_draw_that_would_favour_the_low_pins_is_drawn_again() {
        assert_eq!(drawn(&[4_200_000_000, 7]), "0000-0007");
        assert_eq!(drawn(&[u32::MAX, 4_200_000_001, 87_654_321]), "8765-4321");
    }

    fn plaintext(length: u8, name: &[u8]) -> Vec<u8> {
        let mut bytes = vec![5; KEY_SIZE];
        bytes.push(length);
        bytes.extend(name);
        bytes
    }

    #[test]
    fn a_box_holds_a_key_a_length_and_that_many_bytes_of_utf_8() {
        let introduction = Introduction::new([5; KEY_SIZE], "büild").unwrap();
        assert_eq!(introduction.encode(), plaintext(6, "büild".as_bytes()));
        assert_eq!(
            Introduction::decode(&plaintext(6, "büild".as_bytes())),
            Some(introduction)
        );
        let empty = Introduction::new([5; KEY_SIZE], "").unwrap();
        assert_eq!(Introduction::decode(&plaintext(0, b"")), Some(empty));
    }

    #[test]
    fn a_box_that_breaks_its_layout_holds_nothing() {
        let long = [b'x'; 65];
        for broken in [
            plaintext(5, b"build-box"),
            plaintext(9, b"build"),
            plaintext(2, &[0xc3, 0x28]),
            plaintext(65, &long),
            vec![5; KEY_SIZE],
            vec![5; KEY_SIZE - 1],
        ] {
            assert_eq!(Introduction::decode(&broken), None, "{broken:?}");
        }
    }
}
