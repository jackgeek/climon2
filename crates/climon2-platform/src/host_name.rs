//! The host name seam (#501): the name a hub monitor and a relay suggest
//! for themselves when they pair, which is the machine's host name (P22).
//!
//! Unix only: monitor links on Windows are not yet specified.

use std::io;

/// The longest host name read, terminator included.
const MOST: usize = 256;

/// The machine's host name, as `uname -n` gives it. A name that is not
/// UTF-8 keeps what it can, with replacement characters for the rest.
pub fn get() -> io::Result<String> {
    let mut buffer = [0u8; MOST];
    // SAFETY: `buffer` is writable for its whole length, which is passed.
    let status = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    let length = buffer.iter().position(|byte| *byte == 0).unwrap_or(MOST);
    Ok(String::from_utf8_lossy(&buffer[..length]).into_owned())
}
