use crate::quality;
use std::{io, path::Path, process::Command};

const GOLDENS: &str = ":(top)crates/climon2-core/tests/goldens";

fn git(root: &Path, args: &[&str]) -> io::Result<String> {
    let output = quality::output(Command::new("git").current_dir(root).args(args))?;
    String::from_utf8(output.stdout).map_err(io::Error::other)
}

/// ADR 0005/#26: an existing metadata golden is never modified, deleted, renamed or
/// retyped, in any commit or in the working tree. This needs no comparison base,
/// so new branches and release tags are checked the same way; shallow history
/// cannot prove it, so it fails closed.
pub fn check(root: &Path) -> io::Result<()> {
    if git(root, &["rev-parse", "--is-shallow-repository"])?.trim() != "false" {
        return Err(io::Error::other(
            "metadata fixture check needs the full git history (fetch-depth: 0)",
        ));
    }
    let committed = git(
        root,
        &[
            "log",
            "--no-renames",
            "--format=%H",
            "--diff-filter=DMT",
            "--",
            GOLDENS,
        ],
    )?;
    let uncommitted = git(
        root,
        &[
            "diff",
            "--no-renames",
            "--name-only",
            "--diff-filter=DMT",
            "HEAD",
            "--",
            GOLDENS,
        ],
    )?;
    if !committed.trim().is_empty() || !uncommitted.trim().is_empty() {
        return Err(io::Error::other("metadata fixtures are append-only"));
    }
    Ok(())
}
