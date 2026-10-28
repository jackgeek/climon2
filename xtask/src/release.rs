use std::{env, fs, io, path::Path, process::Command};

const BINARIES: &[&str] = &[
    "climon2-cli",
    "climon2-daemon",
    "climon2-client",
    "climon2-install",
];
const MARKER: &[u8] = b"CLIMON2_TEST_HOOKS_ENABLED";
pub(crate) const PACKAGES: &[(&str, &str)] = &[
    ("climon2-launcher", "climon2"),
    ("climon2-cli", "climon2-cli"),
    ("climon2-daemon", "climon2-daemon"),
    ("climon2-client", "climon2-client"),
    ("climon2-install", "climon2-install"),
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
        .env("MACOSX_DEPLOYMENT_TARGET", "15.0")
        .args([
            "build",
            "--release",
            "--locked",
            "-p",
            package,
            "--target-dir",
        ])
        .arg(directory);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    crate::quality::run(&mut command)
}

pub fn check(root: &Path) -> io::Result<()> {
    let executable = env::current_exe()?;
    let siblings = executable.parent().expect("xtask binary directory");
    let snapshots: Vec<_> = BINARIES
        .iter()
        .map(|name| fs::read(siblings.join(name)))
        .collect::<io::Result<_>>()?;
    for (name, bytes) in BINARIES.iter().zip(&snapshots) {
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
    for (package, name) in PACKAGES {
        build(root, target.path(), package, None)?;
        if has_marker(&fs::read(target.path().join("release").join(name))?) {
            return Err(io::Error::other(format!(
                "release binary {name} contains test hooks"
            )));
        }
    }
    for (name, before) in BINARIES.iter().zip(snapshots) {
        if fs::read(siblings.join(name))? != before {
            return Err(io::Error::other(format!(
                "release build overwrote test binary {name}"
            )));
        }
    }
    eprintln!("Verified release markers absent; test binaries unchanged.");
    Ok(())
}

pub(crate) fn has_marker(bytes: &[u8]) -> bool {
    bytes.windows(MARKER.len()).any(|window| window == MARKER)
}
