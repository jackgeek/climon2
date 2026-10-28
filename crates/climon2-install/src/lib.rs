//! Atomic installation of a release payload into a climon2 executable home.
//! See `tests/install.rs` for the #21/#24/#33 behavioral contract.
mod command;
#[cfg(feature = "test-hooks")]
mod hooks;
mod payload;
mod platform;
mod publish;
mod transaction;

pub use command::run;
pub use payload::Payload;

use std::io;
use std::path::{Path, PathBuf};

/// The canonical executable home containing the committed install layout.
#[derive(Debug)]
pub struct InstallReport {
    pub executable_home: PathBuf,
}

/// Install a validated payload; other installed versions are never removed.
pub fn install(payload: &Payload, destination: &Path) -> io::Result<InstallReport> {
    let (stage, executable_home) = prepare(payload, destination)?;
    publish::commit(&stage, &executable_home, payload)?;
    Ok(InstallReport { executable_home })
}

fn prepare(payload: &Payload, destination: &Path) -> io::Result<(transaction::Stage, PathBuf)> {
    payload.validate()?;
    std::fs::create_dir_all(destination)?;
    let executable_home = destination.canonicalize()?;
    let stage = transaction::Stage::new(&executable_home)?;
    stage.copy_payload(payload)?;
    #[cfg(feature = "test-hooks")]
    hooks::checkpoint("version-staged")?;
    Ok((stage, executable_home))
}
