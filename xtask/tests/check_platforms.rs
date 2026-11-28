//! #488 and #544: `tools/check-platforms.sh` decides, for one run of
//! check.yml, whether its macOS and Windows jobs run. A pull request into the
//! monitor links feature branch, or into one of its ticket branches, skips
//! macOS (owner decision, 2026-11-27). It runs every Windows job when its head
//! is a Stack W ticket branch (`monitor-links/win-*`), or when it changes
//! Windows-only code, as `cargo xtask windows-changed` decides against its
//! base, and none otherwise (owner decision, 2026-11-28). Any other run leaves
//! both to the repository variables.
#![cfg(unix)]

use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output},
};

const BASE: &str = "0123456789abcdef0123456789abcdef01234567";

/// What the script printed and every argument list it gave Cargo.
struct Run {
    macos: String,
    windows: String,
    cargo: Vec<String>,
}

/// Runs the script for a pull request from `head` into `base_ref` (both empty
/// for any other event), with a `cargo` that records its arguments, prints
/// `answer` and exits with `status`. Returns the script's output and Cargo's
/// argument lists.
fn script(
    event: &str,
    base_ref: &str,
    head: &str,
    answer: &str,
    status: i32,
) -> (Output, Vec<String>) {
    let fake = tempfile::tempdir().unwrap();
    let calls = fake.path().join("calls");
    let cargo = fake.path().join("cargo");
    let body = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\necho {answer}\nexit {status}\n",
        calls.display()
    );
    fs::write(&cargo, body).unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", fake.path().display(), env::var("PATH").unwrap());
    let output = Command::new("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/check-platforms.sh"))
        .env("PATH", path)
        .env("EVENT", event)
        .env("BASE_REF", base_ref)
        .env("HEAD_REF", head)
        .env("BASE", BASE)
        .output()
        .unwrap();
    let cargo = fs::read_to_string(&calls)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    (output, cargo)
}

/// The script's answer, with a `cargo` that answers `answer` and succeeds.
fn platforms(event: &str, base_ref: &str, head: &str, answer: &str) -> Run {
    let (output, cargo) = script(event, base_ref, head, answer, 0);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = |key: &str| {
        stdout
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("no {key} in {stdout}"))
            .to_owned()
    };
    Run {
        macos: value("macos"),
        windows: value("windows"),
        cargo,
    }
}

fn decided(run: &Run) -> (&str, &str) {
    (run.macos.as_str(), run.windows.as_str())
}

#[test]
fn a_run_that_is_not_a_feature_pull_request_leaves_both_to_the_switches() {
    for (event, base_ref, head) in [
        ("push", "", ""),
        ("workflow_call", "", ""),
        ("pull_request", "main", "fix/other"),
        ("pull_request", "main", "monitor-links/win-544-windows-ci"),
        ("pull_request", "main", "feature/monitor-links"),
        // A name that only resembles the feature's is not one of its branches.
        (
            "pull_request",
            "feature/monitor-links-2",
            "monitor-links/503-x",
        ),
        ("pull_request", "x/monitor-links/1", "monitor-links/503-x"),
    ] {
        let run = platforms(event, base_ref, head, "true");
        assert_eq!(
            decided(&run),
            ("switch", "switch"),
            "{event} {base_ref} {head}"
        );
        assert!(run.cargo.is_empty(), "{event} {base_ref} {head}");
    }
}

#[test]
fn a_stack_w_pull_request_runs_every_windows_job_and_no_macos_job() {
    for base_ref in ["feature/monitor-links", "monitor-links/win-546-key-storage"] {
        let run = platforms(
            "pull_request",
            base_ref,
            "monitor-links/win-547-sockets",
            "false",
        );
        assert_eq!(decided(&run), ("off", "on"), "{base_ref}");
        assert!(run.cargo.is_empty(), "{base_ref}");
    }
}

#[test]
fn another_feature_pull_request_runs_windows_only_when_it_changes_windows_only_code() {
    for head in [
        "monitor-links/503-relay-pairing",
        "monitor-links/window-503",
        "x/monitor-links/win-503",
    ] {
        for base_ref in ["feature/monitor-links", "monitor-links/502-delete"] {
            for (answer, windows) in [("true", "on"), ("false", "off")] {
                let run = platforms("pull_request", base_ref, head, answer);
                assert_eq!(
                    decided(&run),
                    ("off", windows),
                    "{base_ref} {head} {answer}"
                );
                assert_eq!(run.cargo, [format!("xtask windows-changed {BASE}")]);
            }
        }
    }
}

#[test]
fn a_failed_windows_changed_fails_the_decision() {
    let (output, cargo) = script(
        "pull_request",
        "feature/monitor-links",
        "monitor-links/503-relay-pairing",
        "false",
        1,
    );
    assert!(!output.status.success());
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("windows="),
        "no answer is given"
    );
    assert_eq!(cargo.len(), 1);
}
