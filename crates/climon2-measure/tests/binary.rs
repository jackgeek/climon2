//! The `climon2-measure` binary at its boundary (#121): its command line,
//! and the measured sessions' workload as a session daemon runs it.

use std::{
    io::Write,
    process::{Command, Stdio},
};

const BINARY: &str = env!("CARGO_BIN_EXE_climon2-measure");

/// Arguments that are not a command fail with status 2 and the usage,
/// before anything is installed or measured.
#[test]
fn invalid_arguments_fail_with_the_usage() {
    for args in [
        &[][..],
        &["walk"],
        &["run", "--out", "o"],
        &["run", "--seconds", "0"],
    ] {
        let output = Command::new(BINARY).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Usage: climon2-measure run --archive"),
            "{stderr}"
        );
    }
}

/// The workload titles itself, prints its start line, reports working,
/// discards what is typed, and ends when its input does.
#[test]
fn the_workload_opens_titled_and_working_and_ends_with_its_input() {
    let mut workload = Command::new(BINARY)
        .args(["workload", "--title", "s001", "--rate", "0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    workload.stdin.take().unwrap().write_all(b"QABC\r").unwrap();
    let output = workload.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"\x1b]2;s001\x07started s001\n\x1b]7501;state=working:app=climon2-measure\x1b\\"
    );
}
