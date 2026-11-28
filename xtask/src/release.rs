//! What every production build shares, on every platform that builds a
//! release: the private target directory, the build command, the release
//! version checks and the test-hook marker check.
use crate::quality;
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(unix)]
pub(crate) mod unix;

const MARKER: &[u8] = b"CLIMON2_TEST_HOOKS_ENABLED";
/// The packages a macOS or Linux release builds, each with its binary.
pub(crate) const PACKAGES: &[(&str, &str)] = &[
    ("climon2-launcher", "c2"),
    ("climon2-cli", "climon2-cli"),
    ("climon2-daemon", "climon2-daemon"),
    ("climon2-client", "climon2-client"),
    ("climon2-install", "climon2-install"),
    ("climon2-monitor", "climon2-monitor"),
    ("climon2-relay", "climon2-relay"),
];

/// Where every production build goes: one directory kept between builds, so
/// each build after the first is incremental, and apart from the test builds,
/// so no test-hooks feature ever reaches it.
pub(crate) fn production_directory(root: &Path) -> io::Result<PathBuf> {
    let directory = root.join("target/production");
    fs::create_dir_all(&directory)?;
    Ok(directory)
}

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
    quality::run(&mut command)
}

pub(crate) fn has_marker(bytes: &[u8]) -> bool {
    bytes.windows(MARKER.len()).any(|window| window == MARKER)
}

/// Release builds never enable `test-hooks`, so no shipped binary carries the marker.
pub(crate) fn unhooked(path: &Path) -> io::Result<()> {
    if has_marker(&fs::read(path)?) {
        return Err(io::Error::other(format!(
            "{} contains test hooks",
            path.display()
        )));
    }
    Ok(())
}

/// Fails unless every release tag on this run and on `HEAD` names `version`,
/// and each of `packages` exists at exactly that version.
pub(crate) fn validate_versions(
    root: &Path,
    version: &str,
    packages: &[(&str, &str)],
) -> io::Result<()> {
    let expected = format!("v{version}");
    if env::var("GITHUB_REF_TYPE").as_deref() == Ok("tag") {
        let tag = env::var("GITHUB_REF_NAME").map_err(io::Error::other)?;
        validate_tag(&tag, &expected)?;
    }
    let tags = quality::output(Command::new("git").current_dir(root).args([
        "tag",
        "--points-at",
        "HEAD",
    ]))?;
    for tag in String::from_utf8_lossy(&tags.stdout).lines() {
        if tag.starts_with('v') {
            validate_tag(tag, &expected)?;
        }
    }
    let metadata = quality::output(Command::new(env!("CARGO")).current_dir(root).args([
        "metadata",
        "--no-deps",
        "--locked",
        "--format-version",
        "1",
    ]))?;
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout)?;
    let workspace = metadata["packages"]
        .as_array()
        .ok_or_else(|| io::Error::other("Cargo metadata omitted packages"))?;
    shipped_versions(workspace, packages, version)
}

/// Every shipped package exists at exactly the release version.
fn shipped_versions(
    workspace: &[serde_json::Value],
    packages: &[(&str, &str)],
    version: &str,
) -> io::Result<()> {
    for (name, _) in packages {
        let package = workspace
            .iter()
            .find(|package| package["name"] == *name)
            .ok_or_else(|| io::Error::other(format!("missing shipped package {name}")))?;
        if package["version"] != version {
            return Err(io::Error::other(format!("{name} version is not {version}")));
        }
    }
    Ok(())
}

fn validate_tag(tag: &str, expected: &str) -> io::Result<()> {
    if tag != expected {
        return Err(io::Error::other(format!(
            "release tag {tag} does not match workspace version {expected}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn packages(monitor: Option<&str>) -> Vec<Value> {
        let mut packages: Vec<_> = PACKAGES
            .iter()
            .filter(|(name, _)| *name != "climon2-monitor")
            .map(|(name, _)| json!({"name": name, "version": "1.2.3"}))
            .collect();
        if let Some(version) = monitor {
            packages.push(json!({"name": "climon2-monitor", "version": version}));
        }
        packages
    }

    /// #74: the release version check covers the shipped monitor package, so a
    /// missing or differently versioned monitor fails before any build.
    #[test]
    fn version_check_rejects_a_missing_or_wrong_version_monitor() {
        assert!(shipped_versions(&packages(Some("1.2.3")), PACKAGES, "1.2.3").is_ok());
        let wrong = shipped_versions(&packages(Some("1.2.4")), PACKAGES, "1.2.3").unwrap_err();
        assert_eq!(wrong.to_string(), "climon2-monitor version is not 1.2.3");
        let missing = shipped_versions(&packages(None), PACKAGES, "1.2.3").unwrap_err();
        assert_eq!(
            missing.to_string(),
            "missing shipped package climon2-monitor"
        );
    }
}
