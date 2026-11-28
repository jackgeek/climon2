//! `c2 monitor` and `c2 relay`: hand off to an executable of the CLI's own
//! version.
use crate::Error;
use climon2_core::{layout::VersionBinary, monitor::MonitorArgs, relay::RelayArgs};
use std::{ffi::OsString, process::Command};

/// `c2 monitor` and `c2 m`: this version's monitor, given the verb and its
/// arguments, if any.
pub fn monitor(arguments: &MonitorArgs) -> Result<u32, Error> {
    run(VersionBinary::Monitor, "monitor", arguments.arguments())
}

/// `c2 relay` and `c2 r`: this version's relay, given the arguments the CLI
/// was given.
pub fn relay(arguments: &RelayArgs) -> Result<u32, Error> {
    run(VersionBinary::Relay, "relay", arguments.arguments())
}

/// Returns only on failure: success hands off to `binary` of the CLI's own
/// version, found with the layout code used for every sibling, as a client
/// hand-off does: on Unix it replaces this process, on Windows this process
/// waits for it and exits with its status.
fn run(binary: VersionBinary, name: &str, arguments: Vec<OsString>) -> Result<u32, Error> {
    let layout = crate::executable_layout()?;
    let mut sibling = Command::new(layout.binary(layout.version(), binary));
    sibling.args(arguments);
    let error = crate::native::exec_client(sibling);
    Err(Error::new(
        125,
        format!(
            "cannot start the climon2 {} {name}: {error}",
            layout.version()
        ),
    ))
}
