//! `cargo xtask measure`: the PERF-01 measurement (#121) of an exact
//! candidate archive, run by the test-only `climon2-measure` harness built
//! in release mode. A missed target is a failed Measure case in its report,
//! not a failed command; only a harness error is.

use std::{ffi::OsString, io, path::Path, process::Command};

/// Cargo running the harness's `run` with `args`.
fn command(cargo: &str, root: &Path, args: &[OsString]) -> Command {
    let mut command = Command::new(cargo);
    command
        .args([
            "run",
            "--release",
            "--locked",
            "-p",
            "climon2-measure",
            "--",
            "run",
        ])
        .args(args)
        .current_dir(root);
    command
}

pub(crate) fn run(root: &Path, args: &[OsString]) -> io::Result<()> {
    let status = command(env!("CARGO"), root, args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "climon2-measure failed with {status}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The harness's flags pass through unchanged, after a locked release
    /// build of the harness, from the workspace root.
    #[test]
    fn measure_runs_the_release_harness_with_the_given_flags() {
        let args: Vec<OsString> = ["--archive", "a.tar.gz", "--out", "o", "--seconds", "5"]
            .map(OsString::from)
            .to_vec();
        let command = command("cargo", Path::new("/w"), &args);
        assert_eq!(command.get_program(), "cargo");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "run",
                "--release",
                "--locked",
                "-p",
                "climon2-measure",
                "--",
                "run",
                "--archive",
                "a.tar.gz",
                "--out",
                "o",
                "--seconds",
                "5",
            ]
        );
        assert_eq!(command.get_current_dir(), Some(Path::new("/w")));
    }
}
