use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

/// The client library: a cdylib artifact of `climon2-client`.
pub(crate) const LIBRARY: &str = "libclimon2_client.dylib";
const BINARIES: &[&str] = &[
    "climon2-cli",
    "climon2-daemon",
    "climon2-client",
    "climon2-install",
    LIBRARY,
];
const MARKER: &[u8] = b"CLIMON2_TEST_HOOKS_ENABLED";
pub(crate) const PACKAGES: &[(&str, &str)] = &[
    ("climon2-launcher", "climon2"),
    ("climon2-cli", "climon2-cli"),
    ("climon2-daemon", "climon2-daemon"),
    ("climon2-client", "climon2-client"),
    ("climon2-install", "climon2-install"),
    ("climon2-client", LIBRARY),
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
        .map(|name| fs::read(test_artifact(siblings, name)))
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
        let artifact = target.path().join("release").join(name);
        if has_marker(&fs::read(&artifact)?) {
            return Err(io::Error::other(format!(
                "release binary {name} contains test hooks"
            )));
        }
        if *name == LIBRARY {
            crate::library::verify(&artifact, env!("CARGO_PKG_VERSION"))?;
        }
    }
    for (name, before) in BINARIES.iter().zip(snapshots) {
        if fs::read(test_artifact(siblings, name))? != before {
            return Err(io::Error::other(format!(
                "release build overwrote test binary {name}"
            )));
        }
    }
    eprintln!("Verified release markers absent; test binaries unchanged.");
    Ok(())
}

/// Cargo uplifts binaries but leaves a dependency's cdylib in `deps/`.
fn test_artifact(siblings: &Path, name: &str) -> PathBuf {
    if name == LIBRARY {
        siblings.join("deps").join(name)
    } else {
        siblings.join(name)
    }
}

pub(crate) fn has_marker(bytes: &[u8]) -> bool {
    bytes.windows(MARKER.len()).any(|window| window == MARKER)
}
