//! The relay's keys: one letter per action, as its key bar lists them. Only
//! stop exists so far.

/// The stop key.
pub(crate) const STOP: u8 = b's';
const ESCAPE: u8 = 0x1b;

/// Whether `input`, one read from the terminal, asks the relay to stop: the
/// stop key is typed before any escape sequence starts. What follows an
/// escape in the same read belongs to its sequence, whatever letters it
/// carries.
pub(crate) fn stops(input: &[u8]) -> bool {
    input
        .iter()
        .take_while(|byte| **byte != ESCAPE)
        .any(|byte| *byte == STOP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stop_key_stops() {
        assert!(stops(b"s"));
        assert!(stops(b"xs"));
        assert!(stops(b"sx"));
    }

    #[test]
    fn other_keys_do_not_stop() {
        for input in [&b""[..], b"x", b"S", b"\x03", b"\r", b"q", b"t", b"r"] {
            assert!(!stops(input), "{input:?}");
        }
    }

    #[test]
    fn an_escape_sequence_carrying_the_letter_does_not_stop() {
        assert!(!stops(b"\x1bs"));
        assert!(!stops(b"\x1b[s"));
        assert!(!stops(b"\x1b"));
        assert!(!stops(b"x\x03q\r\x1bs"));
        assert!(!stops(b"x\x1b[1;5s"));
    }

    #[test]
    fn the_stop_key_before_an_escape_sequence_stops() {
        assert!(stops(b"s\x1b[A"));
        assert!(stops(b"xs\x1bs"));
    }
}
