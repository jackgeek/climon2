use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

trait IsolatedCommand {
    fn isolated(&mut self) -> &mut Self;
}

impl IsolatedCommand for Command {
    fn isolated(&mut self) -> &mut Self {
        self.env_clear();
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            self.env("LLVM_PROFILE_FILE", profile);
        }
        self
    }
}

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let root = std::env::current_dir().unwrap().join(format!(
            "target/install tests 雪 {}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn payload(&self, name: &str, version: &str, status: u8) -> PathBuf {
        let root = self.0.join(name);
        fs::create_dir_all(root.join(version)).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_climon2-install"), root.join("install")).unwrap();
        for binary in ["climon2-cli", "climon2-daemon", "climon2-client"] {
            script(&root.join(version).join(binary), &format!("exit {status}"));
        }
        script(&root.join("climon2"), "exit 11");
        root
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// #21 / #24 / #33: the public library builds runnable layouts from release payloads.
#[test]
fn fresh_install_publishes_complete_version_launcher_and_current() {
    let workspace = Workspace::new();
    let source = workspace.payload("payload", "1.2.3", 37);
    let payload = climon2_install::Payload::from_directory(&source, "1.2.3").unwrap();
    let destination = workspace.0.join("installed");
    let report = climon2_install::install(&payload, &destination).unwrap();
    assert_eq!(report.executable_home, destination.canonicalize().unwrap());
    assert_eq!(fs::read(destination.join("current")).unwrap(), b"1.2.3\n");
    for binary in ["climon2-cli", "climon2-daemon", "climon2-client"] {
        let output = Command::new(destination.join("1.2.3").join(binary))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(37));
    }
    assert_eq!(
        Command::new(destination.join("climon2"))
            .status()
            .unwrap()
            .code(),
        Some(11)
    );
    assert_eq!(fs::read_dir(destination).unwrap().count(), 3);
}

/// #21.17-19 / #33: identical versions retain their files; differing versions swap.
#[test]
fn reinstall_clobbers_only_different_same_version_and_retains_other_versions() {
    use std::os::unix::fs::MetadataExt;
    let workspace = Workspace::new();
    let source = workspace.payload("payload", "1.2.3", 17);
    let payload = climon2_install::Payload::from_directory(&source, "1.2.3").unwrap();
    let destination = workspace.0.join("installed");
    climon2_install::install(&payload, &destination).unwrap();
    let original_inode = fs::metadata(destination.join("1.2.3/climon2-cli"))
        .unwrap()
        .ino();
    script(&destination.join("climon2"), "exit 99");
    climon2_install::install(&payload, &destination).unwrap();
    assert_eq!(
        fs::metadata(destination.join("1.2.3/climon2-cli"))
            .unwrap()
            .ino(),
        original_inode
    );
    assert_eq!(
        Command::new(destination.join("climon2"))
            .status()
            .unwrap()
            .code(),
        Some(11)
    );
    let second = workspace.payload("second", "2.0.0", 22);
    let second_payload = climon2_install::Payload::from_directory(&second, "2.0.0").unwrap();
    climon2_install::install(&second_payload, &destination).unwrap();
    fs::write(destination.join("2.0.0/preserve"), "untouched").unwrap();
    fs::write(destination.join("1.2.3/stale"), "must go").unwrap();
    script(&source.join("1.2.3/climon2-cli"), "exit 44");
    climon2_install::install(&payload, &destination).unwrap();
    assert_eq!(fs::read(destination.join("current")).unwrap(), b"1.2.3\n");
    assert!(!destination.join("1.2.3/stale").exists());
    assert_eq!(
        Command::new(destination.join("1.2.3/climon2-cli"))
            .status()
            .unwrap()
            .code(),
        Some(44)
    );
    assert_eq!(
        fs::read(destination.join("2.0.0/preserve")).unwrap(),
        b"untouched"
    );
    assert_eq!(fs::read_dir(destination).unwrap().count(), 4);
}

/// #21.15-16 / #24.5 / #33: installer finds its sibling payload and never edits rc files.
#[test]
fn installer_defaults_to_home_and_prints_safe_path_hint_only_when_needed() {
    let workspace = Workspace::new();
    let source = workspace.payload("payload", env!("CARGO_PKG_VERSION"), 31);
    let home = workspace.0.join("home's 雪");
    fs::create_dir(&home).unwrap();
    fs::write(home.join(".profile"), "keep me").unwrap();
    let output = Command::new(source.join("install"))
        .isolated()
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty());
    let destination = home.join(".local/share/climon2");
    assert_eq!(
        fs::read(destination.join("current")).unwrap(),
        format!("{}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
    assert_eq!(fs::read(home.join(".profile")).unwrap(), b"keep me");
    let hint = String::from_utf8(output.stdout).unwrap();
    assert!(hint.starts_with("export PATH='"));
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("{hint}\ncommand -v climon2"))
        .isolated()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        destination.join("climon2").to_str().unwrap()
    );
    let output = Command::new(source.join("install"))
        .arg("--dir")
        .arg(&destination)
        .isolated()
        .env("PATH", &destination)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

/// #17 / #33: all installer errors are one diagnostic line and exit 125.
#[test]
fn installer_failures_are_single_line_and_do_not_create_partial_layouts() {
    let workspace = Workspace::new();
    let source = workspace.payload("payload", env!("CARGO_PKG_VERSION"), 31);
    for args in [
        vec!["--unknown"],
        vec!["--dir"],
        vec!["--dir", ""],
        vec!["--dir", "a", "extra"],
    ] {
        let output = Command::new(source.join("install"))
            .args(args)
            .isolated()
            .output()
            .unwrap();
        assert_installer_failure(output);
    }
    let output = Command::new(source.join("install"))
        .isolated()
        .output()
        .unwrap();
    assert_installer_failure(output);
    let output = Command::new(source.join("install"))
        .isolated()
        .env("HOME", "relative")
        .output()
        .unwrap();
    assert_installer_failure(output);
    fs::remove_file(
        source
            .join(env!("CARGO_PKG_VERSION"))
            .join("climon2-client"),
    )
    .unwrap();
    let destination = workspace.0.join("missing");
    let output = Command::new(source.join("install"))
        .arg("--dir")
        .arg(&destination)
        .isolated()
        .output()
        .unwrap();
    assert_installer_failure(output);
    assert!(!destination.exists());
}

fn assert_installer_failure(output: std::process::Output) {
    assert_eq!(output.status.code(), Some(125));
    assert!(output.stdout.is_empty());
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.starts_with("climon2: error: "));
    assert_eq!(diagnostic.lines().count(), 1);
}

/// #21.7,18 / #33: interruption never publishes partial versions or advances current early.
#[cfg(feature = "test-hooks")]
#[test]
fn interruption_cleans_only_own_staging_and_rerun_recovers() {
    for mode in ["fail", "crash"] {
        for boundary in [
            "copy-cli",
            "version-staged",
            "version-published",
            "launcher-published",
        ] {
            let workspace = Workspace::new();
            let old = workspace.payload("old", "0.0.1", 17);
            let destination = workspace.0.join("installed");
            climon2_install::install(
                &climon2_install::Payload::from_directory(&old, "0.0.1").unwrap(),
                &destination,
            )
            .unwrap();
            let source = workspace.payload("new", env!("CARGO_PKG_VERSION"), 44);
            script(&source.join("climon2"), "exit 22");
            fs::create_dir(destination.join(".unrelated-stage")).unwrap();
            fs::write(destination.join(".unrelated-stage/keep"), "safe").unwrap();
            let output = Command::new(source.join("install"))
                .arg("--dir")
                .arg(&destination)
                .isolated()
                .env("CLIMON2_TEST_INSTALL_FAULT", format!("{mode}:{boundary}"))
                .output()
                .unwrap();
            if mode == "fail" {
                assert_installer_failure(output);
                assert!(!destination.join(".climon2-install-stage").exists());
            } else {
                assert_eq!(output.status.code(), Some(86));
                assert!(destination.join(".climon2-install-stage").exists());
            }
            assert_eq!(fs::read(destination.join("current")).unwrap(), b"0.0.1\n");
            let version = destination.join(env!("CARGO_PKG_VERSION"));
            if ["copy-cli", "version-staged"].contains(&boundary) {
                assert!(!version.exists());
            } else {
                assert_eq!(fs::read_dir(&version).unwrap().count(), 3);
                assert_eq!(
                    Command::new(version.join("climon2-cli"))
                        .status()
                        .unwrap()
                        .code(),
                    Some(44)
                );
            }
            let expected_launcher = if boundary == "launcher-published" {
                22
            } else {
                11
            };
            assert_eq!(
                Command::new(destination.join("climon2"))
                    .status()
                    .unwrap()
                    .code(),
                Some(expected_launcher)
            );
            let output = Command::new(source.join("install"))
                .arg("--dir")
                .arg(&destination)
                .isolated()
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert_eq!(
                fs::read(destination.join("current")).unwrap(),
                format!("{}\n", env!("CARGO_PKG_VERSION")).as_bytes()
            );
            assert_eq!(
                fs::read(destination.join(".unrelated-stage/keep")).unwrap(),
                b"safe"
            );
            assert!(!destination.join(".climon2-install-stage").exists());
            assert_eq!(
                Command::new(destination.join("0.0.1/climon2-cli"))
                    .status()
                    .unwrap()
                    .code(),
                Some(17)
            );
        }
    }
}

/// ADR 0004 / #33: packaged consumers reuse sibling binaries, including instrumentation.
#[test]
fn sibling_payload_runs_real_installed_launcher_with_literal_arguments() {
    let workspace = Workspace::new();
    let source = workspace.payload("payload", "3.2.1", 0);
    for binary in ["climon2-cli", "climon2-daemon", "climon2-client"] {
        fs::copy(source.join("3.2.1").join(binary), source.join(binary)).unwrap();
    }
    let launcher = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("climon2");
    assert!(
        launcher.is_file(),
        "build sibling launcher with cargo test -p climon2-launcher -p climon2-install"
    );
    fs::copy(launcher, source.join("climon2")).unwrap();
    script(
        &source.join("climon2-cli"),
        "printf '%s\\000' \"$@\"; printf '%s' \"$CHECK\"; exit 42",
    );
    let payload = climon2_install::Payload::from_siblings(&source, "3.2.1").unwrap();
    assert_eq!(payload.version().to_string(), "3.2.1");
    let destination = workspace.0.join("exe home 雪");
    climon2_install::install(&payload, &destination).unwrap();
    let output = Command::new(destination.join("climon2"))
        .args(["exec", "space 雪", "", "--option"])
        .isolated()
        .env("HOME", &workspace.0)
        .env("CHECK", "unchanged")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(42));
    assert_eq!(
        output.stdout,
        "exec\0space 雪\0\0--option\0unchanged".as_bytes()
    );
    assert!(output.stderr.is_empty());
}

/// #33: real publication errors preserve current and remove only this transaction.
#[test]
fn publication_failures_do_not_advance_current_and_keep_unrelated_files() {
    for blocked in ["climon2", "current"] {
        let workspace = Workspace::new();
        let source = workspace.payload("payload", "1.2.3", 31);
        let payload = climon2_install::Payload::from_directory(&source, "1.2.3").unwrap();
        let destination = workspace.0.join("installed");
        fs::create_dir_all(destination.join(blocked)).unwrap();
        fs::write(destination.join(blocked).join("keep"), "untouched").unwrap();
        if blocked == "climon2" {
            fs::write(destination.join("current"), "0.0.1\n").unwrap();
        }
        assert!(climon2_install::install(&payload, &destination).is_err());
        assert_eq!(
            fs::read(destination.join(blocked).join("keep")).unwrap(),
            b"untouched"
        );
        assert!(!destination.join(".climon2-install-stage").exists());
        assert_eq!(fs::read_dir(destination.join("1.2.3")).unwrap().count(), 3);
        if blocked == "climon2" {
            assert_eq!(fs::read(destination.join("current")).unwrap(), b"0.0.1\n");
        }
        fs::remove_dir_all(destination.join(blocked)).unwrap();
        climon2_install::install(&payload, &destination).unwrap();
        assert_eq!(fs::read(destination.join("current")).unwrap(), b"1.2.3\n");
    }
}

/// #21.17-18 / #33: crash after a same-version swap leaves a complete runnable version.
#[cfg(feature = "test-hooks")]
#[test]
fn same_version_swap_crash_is_recoverable_without_touching_other_versions() {
    let workspace = Workspace::new();
    let source = workspace.payload("payload", env!("CARGO_PKG_VERSION"), 17);
    let payload =
        climon2_install::Payload::from_directory(&source, env!("CARGO_PKG_VERSION")).unwrap();
    let destination = workspace.0.join("installed");
    climon2_install::install(&payload, &destination).unwrap();
    let old = workspace.payload("other", "0.0.1", 19);
    climon2_install::install(
        &climon2_install::Payload::from_directory(&old, "0.0.1").unwrap(),
        &destination,
    )
    .unwrap();
    script(
        &source.join(env!("CARGO_PKG_VERSION")).join("climon2-cli"),
        "exit 44",
    );
    let output = Command::new(source.join("install"))
        .arg("--dir")
        .arg(&destination)
        .isolated()
        .env("CLIMON2_TEST_INSTALL_FAULT", "crash:version-published")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(86));
    assert_eq!(fs::read(destination.join("current")).unwrap(), b"0.0.1\n");
    assert_eq!(
        Command::new(
            destination
                .join(env!("CARGO_PKG_VERSION"))
                .join("climon2-cli")
        )
        .status()
        .unwrap()
        .code(),
        Some(44)
    );
    let output = Command::new(source.join("install"))
        .arg("--dir")
        .arg(&destination)
        .isolated()
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!destination.join(".climon2-install-stage").exists());
    assert_eq!(
        Command::new(destination.join("0.0.1/climon2-cli"))
            .status()
            .unwrap()
            .code(),
        Some(19)
    );
}

/// #24 / #33: malformed or symlinked payload files are rejected before writes.
#[test]
fn invalid_payloads_and_version_targets_never_escape_their_layout() {
    use std::os::unix::fs::symlink;
    let workspace = Workspace::new();
    let source = workspace.payload("payload", "1.2.3", 31);
    for version in ["", "../escape", "/absolute", "1.2.3\n", "v1.2.3"] {
        assert!(climon2_install::Payload::from_directory(&source, version).is_err());
    }
    let cli = source.join("1.2.3/climon2-cli");
    fs::remove_file(&cli).unwrap();
    symlink(source.join("1.2.3/climon2-client"), &cli).unwrap();
    assert!(climon2_install::Payload::from_directory(&source, "1.2.3").is_err());
    fs::remove_file(&cli).unwrap();
    script(&cli, "exit 31");
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(climon2_install::Payload::from_directory(&source, "1.2.3").is_err());
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o755)).unwrap();
    let payload = climon2_install::Payload::from_directory(&source, "1.2.3").unwrap();
    let destination = workspace.0.join("installed");
    fs::create_dir(&destination).unwrap();
    let outside = workspace.0.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), "untouched").unwrap();
    symlink(&outside, destination.join("1.2.3")).unwrap();
    assert!(climon2_install::install(&payload, &destination).is_err());
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"untouched");
    assert!(!destination.join("current").exists());
    assert!(!destination.join(".climon2-install-stage").exists());
}

/// #17 / #33: a failed PATH-hint write is an installer error, not a panic.
#[test]
fn installer_output_failure_uses_own_error_contract() {
    let workspace = Workspace::new();
    let source = workspace.payload("payload", env!("CARGO_PKG_VERSION"), 31);
    let destination = workspace.0.join("installed");
    let mut child = Command::new(source.join("install"))
        .arg("--dir")
        .arg(destination)
        .isolated()
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    assert_installer_failure(child.wait_with_output().unwrap());
}
