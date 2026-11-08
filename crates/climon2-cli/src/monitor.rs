//! `climon2 monitor` and `climon2 mon`: hand off to this version's monitor.
use crate::Error;
use climon2_core::layout::{ExecutableLayout, VersionBinary};
use std::{io, os::unix::process::CommandExt, process::Command};

/// Returns only on failure: success replaces this process with the monitor of
/// the CLI's own version, found with the layout code used for every sibling.
pub fn run() -> Result<u32, Error> {
    let executable = std::env::current_exe()?.canonicalize()?;
    let layout = ExecutableLayout::from_cli_path(&executable).map_err(io::Error::other)?;
    let error = Command::new(layout.binary(layout.version(), VersionBinary::Monitor)).exec();
    Err(Error::new(
        125,
        format!(
            "cannot start the climon2 {} monitor: {error}",
            layout.version()
        ),
    ))
}
