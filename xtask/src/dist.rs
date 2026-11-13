use crate::{quality, release};
use std::{
    collections::BTreeSet,
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

struct Platform {
    suffix: &'static str,
    targets: &'static [&'static str],
}

struct BuiltPayload {
    scratch: tempfile::TempDir,
    directory: PathBuf,
    platform: Platform,
}

fn host_platform() -> io::Result<Platform> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => Ok(Platform {
            suffix: "macos-universal",
            targets: &["aarch64-apple-darwin", "x86_64-apple-darwin"],
        }),
        ("linux", "x86_64") if cfg!(target_env = "gnu") => Ok(Platform {
            suffix: "linux-x86_64-gnu",
            targets: &["x86_64-unknown-linux-gnu"],
        }),
        _ => Err(io::Error::other(
            "dist requires an arm64 macOS or x86_64 GNU/Linux host",
        )),
    }
}

fn build_payload(root: &Path) -> io::Result<BuiltPayload> {
    let version = env!("CARGO_PKG_VERSION");
    validate_versions(root, version)?;
    let platform = host_platform()?;
    fs::create_dir_all(root.join("target"))?;
    let scratch = tempfile::Builder::new()
        .prefix("dist-")
        .tempdir_in(root.join("target"))?;
    let build = scratch.path().join("build");
    for target in platform.targets {
        for (package, _) in release::PACKAGES {
            release::build(root, &build, package, Some(target))?;
        }
    }
    let name = format!("climon2-{version}-{}", platform.suffix);
    let payload = scratch.path().join(&name);
    fs::create_dir_all(payload.join(version))?;
    for (_, binary) in release::PACKAGES {
        let destination = match *binary {
            "climon2" => payload.join("climon2"),
            "climon2-install" => payload.join("install"),
            _ => payload.join(version).join(binary),
        };
        package_binary(&platform, &build, binary, &destination)?;
    }
    package_library(&platform, &build, &payload.join(version), version)?;
    for file in ["LICENSE", "README.md"] {
        fs::copy(root.join(file), payload.join(file))?;
    }
    Ok(BuiltPayload {
        scratch,
        directory: payload,
        platform,
    })
}

pub fn install(root: &Path, args: &[OsString]) -> io::Result<()> {
    let payload = build_payload(root)?;
    quality::run(Command::new(payload.directory.join("install")).args(args))
}

pub fn run(root: &Path) -> io::Result<()> {
    let payload = build_payload(root)?;
    let scratch = payload.scratch;
    let name = payload.directory.file_name().expect("release payload name");
    let platform = payload.platform;
    let version = env!("CARGO_PKG_VERSION");
    let archive_name = format!("{}.tar.gz", name.to_string_lossy());
    quality::run(
        Command::new("tar")
            .arg("-czf")
            .arg(scratch.path().join(&archive_name))
            .arg("-C")
            .arg(scratch.path())
            .arg(name),
    )?;
    fs::write(
        scratch.path().join("install.sh"),
        stamped_script(&fs::read_to_string(root.join("install.sh"))?, version)?,
    )?;
    let mut checksum = checksum_command(&platform);
    let sums = quality::output(
        checksum
            .args([&archive_name, "install.sh"])
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

fn checksum_command(platform: &Platform) -> Command {
    if platform.suffix == "macos-universal" {
        let mut command = Command::new("shasum");
        command.args(["-a", "256"]);
        command
    } else {
        Command::new("sha256sum")
    }
}

fn package_binary(
    platform: &Platform,
    build: &Path,
    binary: &str,
    destination: &Path,
) -> io::Result<()> {
    if platform.targets.len() == 1 {
        fs::copy(
            build.join(platform.targets[0]).join("release").join(binary),
            destination,
        )?;
        verify_linux_binary(destination)?;
    } else {
        let mut lipo = Command::new("lipo");
        lipo.arg("-create");
        for target in platform.targets {
            lipo.arg(build.join(target).join("release").join(binary));
        }
        quality::run(lipo.arg("-output").arg(destination))?;
        quality::run(
            Command::new("codesign")
                .args(["--force", "-s", "-"])
                .arg(destination),
        )?;
        verify_macos_binary(destination)?;
    }
    unhooked(destination)
}

/// The client library is packaged like an executable, then checked as the
/// library of exactly this `version`.
fn package_library(
    platform: &Platform,
    build: &Path,
    folder: &Path,
    version: &str,
) -> io::Result<()> {
    let name = climon2_core::layout::client_library_name();
    let destination = folder.join(&name);
    package_binary(platform, build, &name, &destination)?;
    crate::library::verify(&destination, version)
}

/// Release builds never enable `test-hooks`, so no shipped binary carries the marker.
pub(crate) fn unhooked(path: &Path) -> io::Result<()> {
    if release::has_marker(&fs::read(path)?) {
        return Err(io::Error::other(format!(
            "{} contains test hooks",
            path.display()
        )));
    }
    Ok(())
}

fn verify_linux_binary(path: &Path) -> io::Result<()> {
    let bytes = fs::read(path)?;
    // ELF64, little-endian, current ELF version, AMD64 e_machine.
    if !bytes.starts_with(b"\x7fELF\x02\x01\x01") || bytes.get(18..20) != Some(&[62, 0]) {
        return Err(io::Error::other(format!(
            "{} is not an x86_64 Linux ELF executable",
            path.display()
        )));
    }
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

pub(crate) fn verify_macos_binary(path: &Path) -> io::Result<()> {
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

    /// This workspace's monitor test build: an executable of this host's one
    /// architecture.
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

    /// The monitor test build, with the test-hook marker only if `hooked`,
    /// whatever features built it: a stand-in that differs from a release
    /// build in at most that one respect.
    fn monitor_stand_in(hooked: bool) -> Vec<u8> {
        const MARKER: &[u8] = b"CLIMON2_TEST_HOOKS_ENABLED";
        let mut bytes = fs::read(built_monitor()).unwrap();
        while let Some(start) = bytes
            .windows(MARKER.len())
            .position(|window| window == MARKER)
        {
            bytes[start..start + MARKER.len()].fill(b'-');
        }
        if hooked {
            bytes.extend_from_slice(MARKER);
        }
        bytes
    }

    /// #74 (Linux amendment): packaging for the Linux release archive accepts
    /// an x86-64 monitor without test hooks, and rejects one-defect stand-ins:
    /// an AArch64 ELF, and an x86-64 binary carrying the test-hook marker.
    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    #[test]
    fn linux_packaging_rejects_a_non_x86_64_or_hooked_monitor() {
        const AARCH64: u16 = 183;
        let platform = host_platform().unwrap();
        let scratch = tempfile::tempdir_in(built_monitor().parent().unwrap()).unwrap();
        let build = scratch.path().join("build");
        let release = build.join(platform.targets[0]).join("release");
        fs::create_dir_all(&release).unwrap();
        let packaged = scratch.path().join("climon2-monitor");
        let package = |bytes: &[u8]| {
            fs::write(release.join("climon2-monitor"), bytes).unwrap();
            package_binary(&platform, &build, "climon2-monitor", &packaged)
                .map_err(|error| error.to_string())
        };
        let plain = monitor_stand_in(false);
        package(&plain).unwrap();
        assert_eq!(fs::read(&packaged).unwrap(), plain);
        let mut arm = plain;
        arm[18..20].copy_from_slice(&AARCH64.to_le_bytes());
        assert_eq!(
            package(&arm).unwrap_err(),
            format!(
                "{} is not an x86_64 Linux ELF executable",
                packaged.display()
            )
        );
        assert_eq!(
            package(&monitor_stand_in(true)).unwrap_err(),
            format!("{} contains test hooks", packaged.display())
        );
    }

    /// #74: the universal-binary check rejects a monitor that is not exactly
    /// arm64 plus x86_64, such as this host's single-architecture test build.
    #[cfg(target_os = "macos")]
    #[test]
    fn binary_check_rejects_a_single_architecture_monitor() {
        let built = built_monitor();
        let scratch = tempfile::tempdir_in(built.parent().unwrap()).unwrap();
        let thin = scratch.path().join("climon2-monitor");
        fs::write(&thin, monitor_stand_in(false)).unwrap();
        quality::run(
            Command::new("codesign")
                .args(["--force", "-s", "-"])
                .arg(&thin),
        )
        .unwrap();
        let error = verify_macos_binary(&thin).unwrap_err().to_string();
        assert!(error.contains("is not exactly arm64 + x86_64"), "{error}");
    }

    /// #74: the release binary check accepts a monitor without the test-hook
    /// marker and rejects the same binary carrying it.
    #[cfg(target_os = "macos")]
    #[test]
    fn binary_check_rejects_a_hooked_monitor() {
        let scratch = tempfile::tempdir_in(built_monitor().parent().unwrap()).unwrap();
        let monitor = scratch.path().join("climon2-monitor");
        fs::write(&monitor, monitor_stand_in(false)).unwrap();
        unhooked(&monitor).unwrap();
        fs::write(&monitor, monitor_stand_in(true)).unwrap();
        let error = unhooked(&monitor).unwrap_err().to_string();
        assert_eq!(error, format!("{} contains test hooks", monitor.display()));
    }
}
