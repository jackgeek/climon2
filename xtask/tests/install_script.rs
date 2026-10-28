#![cfg(target_os = "macos")]

use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

struct Release {
    root: TempDir,
    assets: PathBuf,
    archive: PathBuf,
}

impl Release {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let version = env!("CARGO_PKG_VERSION");
        let name = format!("climon2-{version}-macos-universal");
        let assets = root.path().join("releases").join(format!("v{version}"));
        let payload = root.path().join("payload").join(&name);
        fs::create_dir_all(payload.join(version)).unwrap();
        fs::create_dir_all(&assets).unwrap();
        let siblings = Path::new(env!("CARGO_BIN_EXE_xtask")).parent().unwrap();
        fs::copy(siblings.join("climon2-install"), payload.join("install")).unwrap();
        fs::copy(siblings.join("climon2"), payload.join("climon2")).unwrap();
        for binary in ["climon2-cli", "climon2-daemon", "climon2-client"] {
            let path = payload.join(version).join(binary);
            fs::write(&path, "#!/bin/sh\nprintf 'script-payload-ok\\n'\n").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let archive = assets.join(format!("{name}.tar.gz"));
        let status = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(payload.parent().unwrap())
            .arg(name)
            .status()
            .unwrap();
        assert!(status.success());
        let release = Self {
            root,
            assets,
            archive,
        };
        release.checksum();
        release
    }

    fn checksum(&self) {
        let output = Command::new("shasum")
            .args(["-a", "256"])
            .arg(self.archive.file_name().unwrap())
            .current_dir(&self.assets)
            .output()
            .unwrap();
        assert!(output.status.success());
        fs::write(self.assets.join("SHA256SUMS"), output.stdout).unwrap();
    }

    fn run(&self, args: &[&std::ffi::OsStr]) -> Output {
        let home = self.root.path().join("home");
        let temporary = self.root.path().join("downloads");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&temporary).unwrap();
        Command::new("/bin/bash")
            .args(["--noprofile", "--norc"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../install.sh"))
            .args(args)
            .env("HOME", home)
            .env("TMPDIR", temporary)
            .env(
                "CLIMON2_RELEASE_BASE_URL",
                format!("file://{}", self.assets.parent().unwrap().display()),
            )
            .output()
            .unwrap()
    }
}

/// #24.8-13/#39: bash 3.2 installs the pinned payload and preserves --dir with spaces.
#[test]
fn downloaded_script_installs_through_the_real_installer() {
    let release = Release::new();
    let destination = release.root.path().join("executable home with spaces");
    let output = release.run(&["--dir".as_ref(), destination.as_os_str()]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        fs::read_to_string(destination.join("current"))
            .unwrap()
            .trim(),
        env!("CARGO_PKG_VERSION")
    );
    let output = Command::new(destination.join("climon2"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"script-payload-ok\n");
    assert_eq!(
        fs::read_dir(release.root.path().join("downloads"))
            .unwrap()
            .count(),
        0
    );
}

/// #24.13/#39: failed verification cannot execute the installer or leave downloads.
#[test]
fn checksum_mismatch_aborts_before_installing() {
    let release = Release::new();
    fs::OpenOptions::new()
        .append(true)
        .open(&release.archive)
        .unwrap()
        .write_all(b"corrupted")
        .unwrap();
    let destination = release.root.path().join("must not exist");
    let output = release.run(&["--dir".as_ref(), destination.as_os_str()]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("climon2-install.sh: error: archive checksum verification failed")
    );
    assert!(!destination.exists());
    assert_eq!(
        fs::read_dir(release.root.path().join("downloads"))
            .unwrap()
            .count(),
        0
    );
}

/// #24.8/#39: omitted arguments use the real installer's isolated HOME default.
#[test]
fn downloaded_script_uses_the_installer_default_destination() {
    let release = Release::new();
    let output = release.run(&[]);
    assert!(output.status.success(), "{output:?}");
    let destination = release.root.path().join("home/.local/share/climon2");
    assert_eq!(
        fs::read_to_string(destination.join("current"))
            .unwrap()
            .trim(),
        env!("CARGO_PKG_VERSION")
    );
    assert!(destination.join("climon2").is_file());
}
