#![cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"),
    all(target_os = "windows", target_arch = "x86_64", target_env = "msvc")
))]

use std::{
    env::consts::EXE_SUFFIX,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// The client library's file name in every version folder (library embedding).
#[cfg(target_os = "macos")]
const LIBRARY: &str = "libclimon2_client.dylib";
#[cfg(target_os = "linux")]
const LIBRARY: &str = "libclimon2_client.so";
#[cfg(windows)]
const LIBRARY: &str = "climon2_client.dll";

/// What the installer prints for a home that is not on `PATH`. No platform's
/// `xtask install` puts it there.
#[cfg(unix)]
const PATH_HINT: &str = "export PATH=";
#[cfg(windows)]
const PATH_HINT: &str = "climon2: PATH was not changed.";

/// The variable naming the folder the installer's default home is under, and
/// that home inside the folder.
#[cfg(unix)]
const DEFAULT_HOME: (&str, &str) = ("HOME", ".local/share/climon2");
#[cfg(windows)]
const DEFAULT_HOME: (&str, &str) = ("LOCALAPPDATA", "climon2");

/// The user's own folder, where rustup and Cargo keep theirs by default.
fn user_folder() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .unwrap()
        .into()
}

fn program(folder: &Path, name: &str) -> PathBuf {
    folder.join(format!("{name}{EXE_SUFFIX}"))
}

fn xtask() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
    command
        .arg("install")
        .env_remove("GITHUB_REF_TYPE")
        .env_remove("GITHUB_REF_NAME");
    command
}

fn assert_installed(destination: &Path) {
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(
        fs::read_to_string(destination.join("current"))
            .unwrap()
            .trim(),
        version
    );
    for path in [
        program(destination, "c2"),
        program(&destination.join(version), "climon2-cli"),
        program(&destination.join(version), "climon2-daemon"),
        program(&destination.join(version), "climon2-client"),
        program(&destination.join(version), "climon2-monitor"),
        destination.join(version).join(LIBRARY),
    ] {
        let bytes = fs::read(path).unwrap();
        assert!(
            !bytes
                .windows(b"CLIMON2_TEST_HOOKS_ENABLED".len())
                .any(|window| window == b"CLIMON2_TEST_HOOKS_ENABLED")
        );
    }
    let output = Command::new(program(destination, "c2"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!("climon2 {version}")
    );
}

#[test]
fn install_builds_production_artifacts_and_preserves_existing_versions() {
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary.path().join("executable home with spaces");
    fs::create_dir_all(destination.join("0.0.1")).unwrap();
    fs::write(destination.join("0.0.1/preserved"), "old version").unwrap();
    fs::write(destination.join("current"), "0.0.1\n").unwrap();

    let output = xtask().arg("--dir").arg(&destination).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_installed(&destination);
    assert_eq!(
        fs::read_to_string(destination.join("0.0.1/preserved")).unwrap(),
        "old version"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(PATH_HINT),
        "{output:?}"
    );
}

#[test]
fn install_uses_the_installers_default_destination() {
    let temporary = tempfile::tempdir().unwrap();
    let home = user_folder();
    let (variable, folder) = DEFAULT_HOME;
    let output = xtask()
        .env(
            "RUSTUP_HOME",
            std::env::var_os("RUSTUP_HOME").unwrap_or_else(|| home.join(".rustup").into()),
        )
        .env(
            "CARGO_HOME",
            std::env::var_os("CARGO_HOME").unwrap_or_else(|| home.join(".cargo").into()),
        )
        .env(variable, temporary.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_installed(&temporary.path().join(folder));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(PATH_HINT),
        "{output:?}"
    );
}

#[test]
fn failed_production_build_does_not_change_the_installation() {
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary.path().join("untouched");
    let output = xtask()
        .arg("--dir")
        .arg(&destination)
        .env("GITHUB_REF_TYPE", "tag")
        .env("GITHUB_REF_NAME", "v999.0.0")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("release tag v999.0.0 does not match"),
        "{output:?}"
    );
    assert!(!destination.exists());
}

#[test]
fn installer_failure_is_reported() {
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary.path().join("not a directory");
    fs::write(&destination, "preserved").unwrap();
    let output = xtask().arg("--dir").arg(&destination).output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("climon2: error:"),
        "{output:?}"
    );
    assert_eq!(fs::read_to_string(destination).unwrap(), "preserved");
}
