//! `c2 monitor pair` on Windows. Monitor links are specified for macOS and
//! Linux only (spec #488), so here pairing refuses at once.
use climon2_monitor::Error;

/// Refuses, whatever the address, with the climon2 error code.
pub(crate) fn run(_address: &str) -> Result<u32, Error> {
    Err(Error::new(
        climon2_core::exit::CLIMON2_ERROR,
        "c2 monitor pair runs on macOS and Linux only",
    ))
}
