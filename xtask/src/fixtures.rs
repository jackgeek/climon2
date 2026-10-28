use crate::quality;
use std::{ffi::OsStr, io, path::Path, process::Command};

pub fn check(root: &Path, base: &OsStr) -> io::Result<()> {
    let commit = quality::output(
        Command::new("git")
            .current_dir(root)
            .args(["rev-parse", "--verify", "--end-of-options"])
            .arg(format!("{}^{{commit}}", base.to_string_lossy())),
    )?;
    let commit = String::from_utf8(commit.stdout).map_err(io::Error::other)?;
    let changed = quality::output(
        Command::new("git")
            .current_dir(root)
            .args(["diff", "--name-only", "-z", "--diff-filter=DMRT"])
            .arg(commit.trim())
            .args(["--", "crates/climon2-core/tests/goldens"]),
    )?;
    if !changed.stdout.is_empty() {
        return Err(io::Error::other("metadata fixtures are append-only"));
    }
    Ok(())
}
