//! Where terminal output may be cut without splitting a UTF-8 character.
//! Only the Windows console needs it: each `WriteFile` to a console is
//! decoded on its own, so a character split across two writes is shown as two
//! replacement characters (ADR 0003, "Client console").

/// The length of the longest prefix of `bytes` that does not end inside a
/// character the next bytes could still complete. Only an incomplete but
/// so far valid sequence at the very end is held back; invalid bytes pass,
/// so output is never held for good.
pub fn whole_characters(bytes: &[u8]) -> usize {
    // A held-back tail is at most three bytes: four would be a whole
    // character, so looking at four is the same as looking at three.
    let earliest = bytes.len().saturating_sub(3usize);
    (earliest..bytes.len())
        .find(|&start| unfinished(&bytes[start..]))
        .unwrap_or(bytes.len())
}

/// Whether `tail` is the start of one character and nothing else.
fn unfinished(tail: &[u8]) -> bool {
    std::str::from_utf8(tail)
        .is_err_and(|error| error.valid_up_to() == 0 && error.error_len().is_none())
}

#[cfg(test)]
mod tests {
    use super::whole_characters;

    #[test]
    fn whole_text_is_written_whole() {
        assert_eq!(whole_characters(b""), 0);
        assert_eq!(whole_characters(b"plain"), 5);
        assert_eq!(whole_characters("caf\u{e9} \u{1f600}".as_bytes()), 10);
    }

    #[test]
    fn the_start_of_a_character_at_the_end_is_held_back() {
        // U+00E9, U+20AC and U+1F600, each cut after every byte but its last.
        assert_eq!(whole_characters(b"ab\xc3"), 2);
        assert_eq!(whole_characters(b"ab\xe2"), 2);
        assert_eq!(whole_characters(b"ab\xe2\x82"), 2);
        assert_eq!(whole_characters(b"\xf0"), 0);
        assert_eq!(whole_characters(b"a\xf0\x9f"), 1);
        assert_eq!(whole_characters(b"a\xf0\x9f\x98"), 1);
        assert_eq!(whole_characters(b"\xf0\x9f\x98\x80\xe2\x82"), 4);
    }

    #[test]
    fn invalid_bytes_are_never_held_back() {
        assert_eq!(whole_characters(b"a\xff"), 2);
        assert_eq!(whole_characters(b"a\x80"), 2);
        assert_eq!(whole_characters(b"\xc3\x28"), 2);
        assert_eq!(whole_characters(b"a\xf0\x28"), 3);
        // A lead byte no character starts with.
        assert_eq!(whole_characters(b"a\xf8\x80\x80"), 4);
    }
}
