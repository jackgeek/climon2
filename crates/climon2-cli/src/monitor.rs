//! `c2 monitor` and `c2 m`: hand off to this version's monitor.
use crate::Error;
use climon2_core::layout::VersionBinary;
use std::process::Command;

/// Returns only on failure: success hands off to the monitor of the CLI's own
/// version, found with the layout code used for every sibling, as a client
/// hand-off does: on Unix it replaces this process, on Windows this process
/// waits for it and exits with its status.
pub fn run() -> Result<u32, Error> {
    let layout = crate::executable_layout()?;
    let monitor = Command::new(layout.binary(layout.version(), VersionBinary::Monitor));
    let error = crate::native::exec_client(monitor);
    Err(Error::new(
        125,
        format!(
            "cannot start the climon2 {} monitor: {error}",
            layout.version()
        ),
    ))
}
