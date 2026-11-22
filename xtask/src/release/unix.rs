//! `cargo xtask release-check`, which needs the Unix installer's payload rules.
use super::{PACKAGES, build, has_marker, production_directory};
use climon2_core::layout::{ExecutableHome, client_library_name};
use std::{
    env, fs, io, iter,
    path::{Path, PathBuf},
};

pub fn check(root: &Path) -> io::Result<()> {
    let executable = env::current_exe()?;
    let siblings = executable.parent().expect("xtask binary directory");
    let artifacts = test_artifacts(siblings);
    let snapshots: Vec<_> = artifacts
        .iter()
        .map(|(_, path)| fs::read(path))
        .collect::<io::Result<_>>()?;
    for ((name, _), bytes) in artifacts.iter().zip(&snapshots) {
        if !has_marker(bytes) {
            return Err(io::Error::other(format!(
                "{name} test binary lacks hooks; run cargo test --workspace first"
            )));
        }
    }
    let target = production_directory(root)?;
    for (package, _) in PACKAGES {
        build(root, &target, package, None)?;
    }
    let release = target.join("release");
    for name in shipped_names() {
        if has_marker(&fs::read(release.join(&name))?) {
            return Err(io::Error::other(format!(
                "release binary {name} contains test hooks"
            )));
        }
    }
    let scratch = tempfile::Builder::new()
        .prefix("release-check-")
        .tempdir_in(root.join("target"))?;
    let home =
        ExecutableHome::new(&scratch.path().join("executable home")).map_err(io::Error::other)?;
    let version = env!("CARGO_PKG_VERSION")
        .parse()
        .map_err(io::Error::other)?;
    let library = home.client_library(&version);
    fs::create_dir_all(library.parent().expect("a version folder"))?;
    fs::copy(release.join(client_library_name()), library)?;
    crate::library::verify(&home, &version)?;
    for ((name, path), before) in artifacts.iter().zip(snapshots) {
        if fs::read(path)? != before {
            return Err(io::Error::other(format!(
                "release build overwrote test binary {name}"
            )));
        }
    }
    eprintln!("Verified release markers absent; test binaries unchanged.");
    Ok(())
}

/// The hooked test builds of every shipped artifact but the launcher, where
/// `cargo test --workspace` leaves them.
fn test_artifacts(siblings: &Path) -> Vec<(String, PathBuf)> {
    hooked_names()
        .map(|name| {
            let path = climon2_install::cargo_output(siblings, &name);
            (name, path)
        })
        .collect()
}

/// Every artifact a release build ships: the launcher, then the hooked rest.
fn shipped_names() -> impl Iterator<Item = String> {
    iter::once(climon2_install::LAUNCHER.to_owned()).chain(hooked_names())
}

/// The shipped artifacts whose test builds carry hooks, which is all but the
/// dependency-free launcher (ADR 0002): the installer, then the version
/// folder's files, the executables before the client library.
fn hooked_names() -> impl Iterator<Item = String> {
    iter::once("climon2-install".to_owned()).chain(climon2_install::version_files())
}
