use crate::{quality, release};
use std::{collections::BTreeSet, env, fs, io, path::Path, process::Command};

const TARGETS: &[&str] = &["aarch64-apple-darwin", "x86_64-apple-darwin"];

pub fn run(root: &Path) -> io::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    validate_versions(root, version)?;
    if env::consts::OS != "macos" || env::consts::ARCH != "aarch64" {
        return Err(io::Error::other("dist requires an arm64 macOS host"));
    }
    fs::create_dir_all(root.join("target"))?;
    let scratch = tempfile::Builder::new()
        .prefix("dist-")
        .tempdir_in(root.join("target"))?;
    let build = scratch.path().join("build");
    for target in TARGETS {
        for (package, _) in release::PACKAGES {
            release::build(root, &build, package, Some(target))?;
        }
    }
    let name = format!("climon2-{version}-macos-universal");
    let payload = scratch.path().join(&name);
    fs::create_dir_all(payload.join(version))?;
    for (_, binary) in release::PACKAGES {
        let destination = match *binary {
            "climon2" => payload.join("climon2"),
            "climon2-install" => payload.join("install"),
            _ => payload.join(version).join(binary),
        };
        let mut lipo = Command::new("lipo");
        lipo.arg("-create");
        for target in TARGETS {
            lipo.arg(build.join(target).join("release").join(binary));
        }
        quality::run(lipo.arg("-output").arg(&destination))?;
        quality::run(
            Command::new("codesign")
                .args(["--force", "-s", "-"])
                .arg(&destination),
        )?;
        verify_binary(&destination)?;
    }
    for file in ["LICENSE", "README.md"] {
        fs::copy(root.join(file), payload.join(file))?;
    }
    let archive_name = format!("{name}.tar.gz");
    quality::run(
        Command::new("tar")
            .arg("-czf")
            .arg(scratch.path().join(&archive_name))
            .arg("-C")
            .arg(scratch.path())
            .arg(&name),
    )?;
    fs::write(
        scratch.path().join("install.sh"),
        stamped_script(&fs::read_to_string(root.join("install.sh"))?, version)?,
    )?;
    let sums = quality::output(
        Command::new("shasum")
            .args(["-a", "256", &archive_name, "install.sh"])
            .current_dir(scratch.path()),
    )?;
    fs::write(scratch.path().join("SHA256SUMS"), sums.stdout)?;
    let destination = root.join("target/dist");
    fs::create_dir_all(&destination)?;
    for file in [archive_name.as_str(), "install.sh", "SHA256SUMS"] {
        fs::rename(scratch.path().join(file), destination.join(file))?;
    }
    eprintln!("Release artifacts: {}", destination.display());
    Ok(())
}

fn validate_versions(root: &Path, version: &str) -> io::Result<()> {
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
    let packages = metadata["packages"]
        .as_array()
        .ok_or_else(|| io::Error::other("Cargo metadata omitted packages"))?;
    shipped_versions(packages, version)
}

/// Every shipped package exists at exactly the release version.
fn shipped_versions(packages: &[serde_json::Value], version: &str) -> io::Result<()> {
    for (name, _) in release::PACKAGES {
        let package = packages
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

fn verify_binary(path: &Path) -> io::Result<()> {
    quality::run(
        Command::new("codesign")
            .args(["--verify", "--strict"])
            .arg(path),
    )?;
    let archs = quality::output(Command::new("lipo").arg("-archs").arg(path))?;
    let archs = String::from_utf8(archs.stdout).map_err(io::Error::other)?;
    if archs.split_whitespace().collect::<BTreeSet<_>>() != BTreeSet::from(["arm64", "x86_64"]) {
        return Err(io::Error::other(format!(
            "{} is not exactly arm64 + x86_64: {archs}",
            path.display()
        )));
    }
    unhooked(path)
}

/// Release builds never enable `test-hooks`, so no shipped binary carries the marker.
fn unhooked(path: &Path) -> io::Result<()> {
    if release::has_marker(&fs::read(path)?) {
        return Err(io::Error::other(format!(
            "{} contains test hooks",
            path.display()
        )));
    }
    Ok(())
}

fn stamped_script(template: &str, version: &str) -> io::Result<String> {
    let mut found = 0;
    let mut output = String::new();
    for line in template.lines() {
        if line.starts_with("VERSION=") {
            output.push_str(&format!("VERSION='{version}'"));
            found += 1;
        } else {
            output.push_str(line);
        }
        output.push('\n');
    }
    if found != 1 {
        return Err(io::Error::other(
            "install.sh must contain one VERSION assignment",
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::path::PathBuf;

    fn packages(monitor: Option<&str>) -> Vec<Value> {
        let mut packages: Vec<_> = release::PACKAGES
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
        assert!(shipped_versions(&packages(Some("1.2.3")), "1.2.3").is_ok());
        let wrong = shipped_versions(&packages(Some("1.2.4")), "1.2.3").unwrap_err();
        assert_eq!(wrong.to_string(), "climon2-monitor version is not 1.2.3");
        let missing = shipped_versions(&packages(None), "1.2.3").unwrap_err();
        assert_eq!(
            missing.to_string(),
            "missing shipped package climon2-monitor"
        );
    }

    /// This workspace's monitor test build: one architecture, with test hooks.
    fn built_monitor() -> PathBuf {
        let built = env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("climon2-monitor");
        assert!(built.is_file(), "run cargo test --workspace first");
        built
    }

    /// #74: the universal-binary check rejects a monitor that is not exactly
    /// arm64 plus x86_64, such as this host's single-architecture test build.
    #[test]
    fn binary_check_rejects_a_single_architecture_monitor() {
        let built = built_monitor();
        let scratch = tempfile::tempdir_in(built.parent().unwrap()).unwrap();
        let thin = scratch.path().join("climon2-monitor");
        fs::copy(&built, &thin).unwrap();
        quality::run(
            Command::new("codesign")
                .args(["--force", "-s", "-"])
                .arg(&thin),
        )
        .unwrap();
        let error = verify_binary(&thin).unwrap_err().to_string();
        assert!(error.contains("is not exactly arm64 + x86_64"), "{error}");
    }

    /// #74: the release binary check rejects a monitor that carries the
    /// test-hook marker, such as this workspace's hook-enabled test build.
    #[test]
    fn binary_check_rejects_a_hooked_monitor() {
        let built = built_monitor();
        let error = unhooked(&built).unwrap_err().to_string();
        assert_eq!(error, format!("{} contains test hooks", built.display()));
    }
}
