//! What vte 0.15.0 prints for a text token, so that a model fed through a
//! parser it shares with other models keeps its own preceding character for
//! `CSI n b` (#166).

/// The last character vte prints for `text`, if it prints any: every
/// character but the C0 and C1 controls it executes, and one replacement
/// character for each invalid sequence except a lone byte up to 0x9F, which
/// it executes as a C1 control. A sequence cut off at the end of the text
/// prints one replacement character too.
pub(crate) fn last(text: &[u8]) -> Option<char> {
    text.utf8_chunks()
        .flat_map(|chunk| {
            let valid = chunk.valid().chars().filter(|&c| !executed(c));
            valid.chain(replacement(chunk.invalid()))
        })
        .last()
}

fn executed(c: char) -> bool {
    matches!(c, '\0'..='\x1f' | '\u{80}'..='\u{9f}')
}

fn replacement(invalid: &[u8]) -> Option<char> {
    match invalid {
        [] => None,
        [byte] if *byte <= 0x9f => None,
        _ => Some(char::REPLACEMENT_CHARACTER),
    }
}
