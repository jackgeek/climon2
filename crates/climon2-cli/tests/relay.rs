//! #499 (spec #488, RLY-01, X1-2): `c2 relay` and `c2 r` hand off to the
//! relay executable of the CLI's own version, with the arguments they were
//! given. The relay here is a script that records how it was started.
#![cfg(unix)]

use climon2_test_term::copy_executable;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const VERSION: &str = "1.2.3";

/// An executable home holding this CLI in a version folder, and nothing else.
fn executable_home(root: &Path) -> PathBuf {
    let folder = root.join("executable home").join(VERSION);
    fs::create_dir_all(&folder).unwrap();
    let cli = folder.join("climon2-cli");
    copy_executable(Path::new(env!("CARGO_BIN_EXE_climon2-cli")), &cli);
    cli
}

/// A relay that writes its arguments, one per line, and exits 7.
fn recording_relay(cli: &Path, record: &Path) {
    let relay = cli.with_file_name("climon2-relay");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$0\" \"$@\" > '{}'\nexit 7\n",
        record.display()
    );
    // Written beside it and copied by `cp`, so this process never holds the
    // script open for writing while another test's child is forked.
    let source = relay.with_extension("source");
    fs::write(&source, script).unwrap();
    copy_executable(&source, &relay);
    fs::set_permissions(&relay, fs::Permissions::from_mode(0o755)).unwrap();
}

fn run(cli: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(cli)
        .args(args)
        .env_clear()
        .env("CLIMON2_HOME", home)
        .output()
        .unwrap()
}

#[test]
fn relay_and_r_run_the_relay_of_the_clis_own_version_with_their_arguments() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let cli = executable_home(&root_path);
    let record = root_path.join("started");
    recording_relay(&cli, &record);
    let relay = cli.with_file_name("climon2-relay");
    for (args, passed) in [
        (&["relay"][..], &[][..]),
        (&["r"], &[]),
        (
            &["relay", "--listen", "0.0.0.0:0"],
            &["--listen", "0.0.0.0:0"],
        ),
        (
            &["r", "--listen", "[::1]:7300"],
            &["--listen", "[::1]:7300"],
        ),
    ] {
        let _ = fs::remove_file(&record);
        let output = run(&cli, &root_path.join("home"), args);
        // The relay's own status is the command's: the CLI was replaced.
        assert_eq!(output.status.code(), Some(7), "{args:?}: {output:?}");
        let started = fs::read_to_string(&record).unwrap();
        let mut expected = vec![relay.to_str().unwrap()];
        expected.extend(passed);
        assert_eq!(started.lines().collect::<Vec<_>>(), expected, "{args:?}");
    }
}

#[test]
fn a_version_without_a_relay_fails_naming_the_version() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let cli = executable_home(&root_path);
    let output = run(&cli, &root_path.join("home"), &["relay"]);
    assert_eq!(output.status.code(), Some(125), "{output:?}");
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.starts_with("climon2: error: cannot start the climon2 1.2.3 relay: "),
        "{stderr}"
    );
    assert!(stderr.contains("(logs: "), "{stderr}");
}

/// CM5: an unknown flag or a malformed `--listen` is a usage error, answered
/// before the home is touched and before any relay starts.
#[test]
fn a_relay_usage_error_starts_nothing_and_touches_no_home() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let cli = executable_home(&root_path);
    let record = root_path.join("started");
    recording_relay(&cli, &record);
    let home = root_path.join("home");
    for args in [
        &["relay", "--port", "1"][..],
        &["r", "--listen", "localhost:7222"],
        &["relay", "--listen"],
    ] {
        let output = run(&cli, &home, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.starts_with("climon2: error: usage: c2 "), "{stderr}");
        assert!(
            stderr.contains("relay|r [--listen <ip>:<port>]"),
            "{stderr}"
        );
    }
    assert!(!record.exists());
    assert!(!home.exists());
}
