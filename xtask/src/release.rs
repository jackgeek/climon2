use std::{env, fs, io, path::Path, process::Command};

const BINARIES: &[&str] = &[
    "climon2-cli",
    "climon2-daemon",
    "climon2-client",
    "climon2-install",
];
const MARKER: &[u8] = b"CLIMON2_TEST_HOOKS_ENABLED";

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
    for package in [
        "climon2-launcher",
        "climon2-cli",
        "climon2-daemon",
        "climon2-client",
        "climon2-install",
    ] {
        let status = Command::new(env!("CARGO"))
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
            .arg(target.path())
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "release build of {package} failed: {status}"
            )));
        }
        let name = if package == "climon2-launcher" {
            "climon2"
        } else {
            package
        };
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

fn has_marker(bytes: &[u8]) -> bool {
    bytes.windows(MARKER.len()).any(|window| window == MARKER)
}
