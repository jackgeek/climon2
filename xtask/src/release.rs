use climon2_core::layout::client_library_name;
use std::{
    env, fs, io, iter,
    path::{Path, PathBuf},
    process::Command,
};

const MARKER: &[u8] = b"CLIMON2_TEST_HOOKS_ENABLED";
pub(crate) const PACKAGES: &[(&str, &str)] = &[
    ("climon2-launcher", "climon2"),
    ("climon2-cli", "climon2-cli"),
    ("climon2-daemon", "climon2-daemon"),
    ("climon2-client", "climon2-client"),
    ("climon2-install", "climon2-install"),
    ("climon2-monitor", "climon2-monitor"),
];

pub(crate) fn build(
    root: &Path,
    directory: &Path,
    package: &str,
    target: Option<&str>,
) -> io::Result<()> {
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(root)
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .env_remove("CARGO_BUILD_BUILD_DIR")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("LLVM_PROFILE_FILE")
        .args([
            "build",
            "--release",
            "--locked",
            "-p",
            package,
            "--target-dir",
        ])
        .arg(directory);
    if cfg!(target_os = "macos") {
        command.env("MACOSX_DEPLOYMENT_TARGET", "15.0");
    }
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    crate::quality::run(&mut command)
}

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
    fs::create_dir_all(root.join("target"))?;
    let target = tempfile::Builder::new()
        .prefix("release-check-")
        .tempdir_in(root.join("target"))?;
    for (package, _) in PACKAGES {
        build(root, target.path(), package, None)?;
    }
    let release = target.path().join("release");
    for name in shipped_names() {
        if has_marker(&fs::read(release.join(&name))?) {
            return Err(io::Error::other(format!(
                "release binary {name} contains test hooks"
            )));
        }
    }
    let version = env!("CARGO_PKG_VERSION");
    let home = target.path().join("executable home");
    fs::create_dir_all(home.join(version))?;
    fs::copy(
        release.join(client_library_name()),
        home.join(version).join(client_library_name()),
    )?;
    crate::library::verify(&home, version)?;
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
    iter::once("climon2".to_owned()).chain(hooked_names())
}

/// The shipped artifacts whose test builds carry hooks, which is all but the
/// dependency-free launcher (ADR 0002): the installer, then the version
/// folder's files, the executables before the client library.
fn hooked_names() -> impl Iterator<Item = String> {
    iter::once("climon2-install".to_owned()).chain(climon2_install::version_files())
}

pub(crate) fn has_marker(bytes: &[u8]) -> bool {
    bytes.windows(MARKER.len()).any(|window| window == MARKER)
}
