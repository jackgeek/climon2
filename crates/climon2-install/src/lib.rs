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
    let executable_home = executable_home(destination)?;
    let stage = stage(payload, &executable_home)?;
    Ok((stage, executable_home))
}

fn executable_home(destination: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(destination)?;
    destination.canonicalize()
}

fn stage(payload: &Payload, executable_home: &Path) -> io::Result<transaction::Stage> {
    let stage = transaction::Stage::new(executable_home)?;
    stage.copy_payload(payload)?;
    #[cfg(feature = "test-hooks")]
    hooks::checkpoint("version-staged")?;
    Ok(stage)
}
