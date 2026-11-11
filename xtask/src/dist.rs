use crate::{quality, release};
use std::{collections::BTreeSet, env, fs, io, path::Path, process::Command};

struct Platform {
    suffix: &'static str,
    targets: &'static [&'static str],
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

pub fn run(root: &Path) -> io::Result<()> {
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
    if release::has_marker(&fs::read(destination)?) {
        return Err(io::Error::other(format!(
            "{} contains test hooks",
            destination.display()
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

fn verify_macos_binary(path: &Path) -> io::Result<()> {
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
