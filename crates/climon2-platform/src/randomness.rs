//! The randomness seam (ADR 0003): bytes from the operating system's
//! generator, for keys, PINs and nonces. The link code never draws its own;
//! its caller draws them here and passes them in.

use std::io;

/// Fills `bytes` from the operating system's generator: `getrandom` on
/// Linux, `getentropy` on macOS and `ProcessPrng` on Windows. A failure
/// leaves no usable bytes, and there is no fallback generator.
pub fn fill(bytes: &mut [u8]) -> io::Result<()> {
    getrandom::fill(bytes).map_err(io::Error::from)
}
