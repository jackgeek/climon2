#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Layout(std::path::PathBuf);

impl Layout {
    fn new() -> Self {
        let root = std::env::current_dir().unwrap().join(format!(
            "target/launcher tests 雪 {}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_climon2"), root.join("climon2")).unwrap();
        Self(root)
    }

    fn companion(&self, version: &str, body: &str) {
        let directory = self.0.join(version);
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("climon2-cli");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn launch(&self) -> Command {
        let mut command = Command::new(self.0.join("climon2"));
        command.env_clear().env("HOME", &self.0);
        preserve_instrumentation(&mut command);
        command
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

/// ADR 0002 / #33: real exec preserves OS arguments, environment, cwd and stdio.
#[test]
fn frozen_handoff_preserves_bytes_stdio_cwd_and_status() {
    let layout = Layout::new();
    fs::write(layout.0.join("current"), "1.2.3-rc.2+雪").unwrap();
    // The launcher must reject non-ASCII SemVer before handing off.
    assert_eq!(layout.launch().output().unwrap().status.code(), Some(125));
    fs::write(layout.0.join("current"), "1.2.3-rc.2+build.7\r\n").unwrap();
    layout.companion(
        "1.2.3-rc.2+build.7",
        "printf '%s\\000' \"$@\"; printf '%s\\000' \"$RAW\"; /bin/pwd; /bin/cat; printf err >&2; exit 37",
    );
    let raw = std::ffi::OsString::from_vec(vec![b'x', 0xff]);
    let mut child = layout
        .launch()
        .args(["", "--flag", "space 雪"])
        .arg(&raw)
        .env("RAW", &raw)
        .current_dir(&layout.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"input\0bytes")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let mut expected = b"\0--flag\0space \xe9\x9b\xaa\0x\xff\0x\xff\0".to_vec();
    expected.extend_from_slice(
        fs::canonicalize(&layout.0)
            .unwrap()
            .as_os_str()
            .as_encoded_bytes(),
    );
    expected.extend_from_slice(b"\ninput\0bytes");
    assert_eq!(output.stdout, expected);
    assert_eq!(output.stderr, b"err");
    assert_eq!(output.status.code(), Some(37));
}

/// #17 / #33: only a strict one-line SemVer can select a companion.
#[test]
fn malformed_or_missing_current_and_unrunnable_companions_are_own_failures() {
    let layout = Layout::new();
    assert_failure(&layout, "current");
    let invalid = [
        "",
        "\n",
        "1.2",
        "v1.2.3",
        "01.2.3",
        "1.02.3",
        "1.2.03",
        "1.2.3\n\n",
        "1.2.3\r",
        "1.2.3\n4.5.6",
        "../1.2.3",
        "/1.2.3",
        " 1.2.3",
        "1.2.3 ",
        "1.2.3-",
        "1.2.3+",
        "1.2.3-a..b",
        "1.2.3-00",
        "1.2.3-01.a",
        "1.2.3+a..b",
        "1.2.3+a+b",
        "1.2.3-雪",
        "1.2.3_a",
        "+1.2.3",
        "1.2.18446744073709551616",
    ];
    for contents in invalid {
        fs::write(layout.0.join("current"), contents).unwrap();
        assert_failure(&layout, "current");
    }
    fs::write(layout.0.join("current"), [0xff]).unwrap();
    assert_failure(&layout, "current");
    fs::write(layout.0.join("current"), "1.2.3\n").unwrap();
    assert_failure(&layout, "climon2-cli");
    layout.companion("1.2.3", "exit 0");
    fs::set_permissions(
        layout.0.join("1.2.3/climon2-cli"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_failure(&layout, "climon2-cli");
}

fn assert_failure(layout: &Layout, context: &str) {
    let output = layout.launch().output().unwrap();
    assert_eq!(output.status.code(), Some(125));
    assert!(output.stdout.is_empty());
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.starts_with("climon2: error: "));
    assert_eq!(diagnostic.lines().count(), 1);
    assert!(diagnostic.contains(context), "{diagnostic}");
}

/// ADR 0002 / #33: canonical launcher home, never cwd, PATH or CLIMON2_EXE.
#[test]
fn symlinked_launcher_uses_own_canonical_home_and_exec_preserves_signal() {
    use std::os::unix::process::ExitStatusExt;
    let layout = Layout::new();
    layout.companion("0.0.0-a-0.1+000.build-x", "kill -TERM $$");
    fs::write(layout.0.join("current"), "0.0.0-a-0.1+000.build-x").unwrap();
    let elsewhere = layout.0.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    symlink(layout.0.join("climon2"), elsewhere.join("linked")).unwrap();
    let mut command = Command::new(elsewhere.join("linked"));
    command.env_clear();
    preserve_instrumentation(&mut command);
    let output = command
        .env("HOME", &elsewhere)
        .env("CLIMON2_EXE", &elsewhere)
        .env("PATH", &elsewhere)
        .current_dir(&elsewhere)
        .output()
        .unwrap();
    assert_eq!(output.status.signal(), Some(15));
    assert!(output.stderr.is_empty());
    assert!(output.stdout.is_empty());
}

fn preserve_instrumentation(command: &mut Command) {
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
}
