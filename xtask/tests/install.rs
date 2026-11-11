#![cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64", target_env = "gnu")
))]

use std::{fs, path::Path, process::Command};

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
        destination.join("climon2"),
        destination.join(version).join("climon2-cli"),
        destination.join(version).join("climon2-daemon"),
        destination.join(version).join("climon2-client"),
    ] {
        let bytes = fs::read(path).unwrap();
        assert!(
            !bytes
                .windows(b"CLIMON2_TEST_HOOKS_ENABLED".len())
                .any(|window| window == b"CLIMON2_TEST_HOOKS_ENABLED")
        );
    }
    let output = Command::new(destination.join("climon2"))
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("export PATH="));
}

#[test]
fn install_uses_the_installers_default_destination() {
    let temporary = tempfile::tempdir().unwrap();
    let home = std::env::var_os("HOME").unwrap();
    let output = xtask()
        .env(
            "RUSTUP_HOME",
            std::env::var_os("RUSTUP_HOME")
                .unwrap_or_else(|| Path::new(&home).join(".rustup").into()),
        )
        .env(
            "CARGO_HOME",
            std::env::var_os("CARGO_HOME")
                .unwrap_or_else(|| Path::new(&home).join(".cargo").into()),
        )
        .env("HOME", temporary.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_installed(&temporary.path().join(".local/share/climon2"));
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
