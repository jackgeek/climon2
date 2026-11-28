//! Suggested names (P22): what each end calls itself, its host name by
//! default, and what it is told the other end is called.

/// The longest name an end may suggest, in bytes.
const MOST: usize = 64;

/// `text` as a name may be shown, stored or suggested: without control
/// characters, and cut at a character boundary to at most 64 bytes. A name
/// from the other end is hostile text, so it passes through here before
/// any screen, file or log sees it.
pub fn display_name(text: &str) -> String {
    let mut name: String = text
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    while name.len() > MOST {
        name.pop();
    }
    name
}
