//! The relay's keys: one letter per action, as its key bar lists them.

/// What a key asks of the relay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Key {
    /// Mint a link invitation (P6, X6).
    Pair,
    /// Stop the relay.
    Stop,
}

/// What the key bar lists, in its order.
pub(crate) const BAR: &str = " p Pair  s Stop";
const ESCAPE: u8 = 0x1b;

/// The first action key in `input`, one read from the terminal, typed
/// before any escape sequence starts. What follows an escape in the same
/// read belongs to its sequence, whatever letters it carries.
pub(crate) fn action(input: &[u8]) -> Option<Key> {
    input
        .iter()
        .take_while(|byte| **byte != ESCAPE)
        .find_map(|byte| match byte {
            b'p' => Some(Key::Pair),
            b's' => Some(Key::Stop),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stop_key_stops_and_the_pair_key_pairs() {
        assert_eq!(action(b"s"), Some(Key::Stop));
        assert_eq!(action(b"xs"), Some(Key::Stop));
        assert_eq!(action(b"sx"), Some(Key::Stop));
        assert_eq!(action(b"p"), Some(Key::Pair));
        assert_eq!(action(b"xp"), Some(Key::Pair));
    }

    #[test]
    fn the_first_action_key_in_a_read_wins() {
        assert_eq!(action(b"ps"), Some(Key::Pair));
        assert_eq!(action(b"sp"), Some(Key::Stop));
    }

    #[test]
    fn other_keys_do_nothing() {
        for input in [&b""[..], b"x", b"S", b"P", b"\x03", b"\r", b"q", b"t", b"r"] {
            assert_eq!(action(input), None, "{input:?}");
        }
    }

    #[test]
    fn an_escape_sequence_carrying_the_letter_does_nothing() {
        for input in [
            &b"\x1bs"[..],
            b"\x1b[s",
            b"\x1b",
            b"x\x03q\r\x1bs",
            b"x\x1b[1;5s",
            b"\x1bp",
        ] {
            assert_eq!(action(input), None, "{input:?}");
        }
    }

    #[test]
    fn an_action_key_before_an_escape_sequence_acts() {
        assert_eq!(action(b"s\x1b[A"), Some(Key::Stop));
        assert_eq!(action(b"xs\x1bs"), Some(Key::Stop));
        assert_eq!(action(b"p\x1b[A"), Some(Key::Pair));
    }
}
