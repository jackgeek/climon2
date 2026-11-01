//! Window titles (plan 1.8.1). Titles are kept raw and encoded at every
//! emission, so ordinary Unicode survives and control characters never do.
use std::ffi::{OsStr, OsString};

/// Percent-encodes each UTF-8 byte of C0, DEL and C1 as uppercase `%XX`.
pub fn encode(title: &str) -> String {
    title.chars().map(encode_char).collect()
}

/// `char::is_control` is exactly U+0000..U+001F and U+007F..U+009F.
fn encode_char(character: char) -> String {
    let text = character.to_string();
    if !character.is_control() {
        return text;
    }
    text.bytes().map(|byte| format!("%{byte:02X}")).collect()
}

/// `climon2 exec <command> <args...>`, converting non-UTF-8 lossily.
pub fn default_title(command: &OsStr, args: &[OsString]) -> String {
    std::iter::once(command)
        .chain(args.iter().map(OsString::as_os_str))
        .fold(String::from("climon2 exec"), |mut title, word| {
            title.push(' ');
            title.push_str(&word.to_string_lossy());
            title
        })
}
