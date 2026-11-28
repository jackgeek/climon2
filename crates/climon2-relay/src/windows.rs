//! The relay on Windows. Monitor links are specified for macOS and Linux
//! only (spec #488), so here the relay refuses to start.
use crate::Error;
use std::ffi::OsString;

/// Refuses, whatever the arguments, with the climon2 error code.
pub fn run(_arguments: impl IntoIterator<Item = OsString>) -> Result<u32, Error> {
    Err(Error::new(
        climon2_core::exit::CLIMON2_ERROR,
        "c2 relay runs on macOS and Linux only",
    ))
}
