//! #488: `tools/quality-scope.sh` decides, for one run of quality.yml, which
//! own-hardware gates run and what they mutate against. On the monitor links
//! feature's branches (`feature/monitor-links` and its ticket branches,
//! `monitor-links/*`) a push runs no gate. A manual run on a ticket branch
//! runs the Linux gate alone against the feature branch; a manual run on the
//! feature branch runs every gate against main (owner decisions, 2026-11-27).
//! A manual run on a Stack W ticket branch (`monitor-links/win-*`) runs the
//! Linux gate and then, once it has finished, the Windows gate, both against
//! the feature branch (owner decision, 2026-11-28).
#![cfg(unix)]

use std::{fs, path::Path, process::Command};

const ALL: &str = "linux,windows,macos";

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn commit(dir: &Path, name: &str) -> String {
    fs::write(dir.join(name), name).unwrap();
    git(dir, &["add", name]);
    git(dir, &["commit", "-q", "-m", name]);
    git(dir, &["rev-parse", "HEAD"])
}

/// A checkout whose `origin/main` is `main`, whose `origin/feature/monitor-links`
/// is `feature`, one commit on, and whose `ticket` is one commit on from that.
struct Repo {
    dir: tempfile::TempDir,
    first: String,
    main: String,
    feature: String,
    ticket: String,
}

fn repo() -> Repo {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    git(path, &["init", "-q"]);
    let first = commit(path, "first");
    let main = commit(path, "main");
    git(path, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    let feature = commit(path, "feature");
    git(
        path,
        &[
            "update-ref",
            "refs/remotes/origin/feature/monitor-links",
            "HEAD",
        ],
    );
    let ticket = commit(path, "ticket");
    Repo {
        dir,
        first,
        main,
        feature,
        ticket,
    }
}

/// The script's `sha`, `gates` and `windows_after_linux` outputs with `head`
/// checked out.
fn scope(
    repo: &Repo,
    head: &str,
    event: &str,
    branch: &str,
    before: &str,
) -> (String, String, String) {
    git(repo.dir.path(), &["checkout", "-q", "--detach", head]);
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/quality-scope.sh");
    let output = Command::new("bash")
        .arg(script)
        .current_dir(repo.dir.path())
        .env("EVENT", event)
        .env("REF", format!("refs/heads/{branch}"))
        .env("BEFORE", before)
        .output()
        .unwrap();
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
    (value("sha"), value("gates"), value("windows_after_linux"))
}

/// Outputs for a run whose gates run at once, or not at all.
fn parallel(sha: &str, gates: &str) -> (String, String, String) {
    (sha.to_owned(), gates.to_owned(), "false".to_owned())
}

#[test]
fn a_push_to_the_features_branches_runs_no_gate() {
    let repo = repo();
    for (head, branch) in [
        (&repo.ticket, "monitor-links/489-link-crate"),
        (&repo.ticket, "monitor-links/win-544-windows-ci"),
        (&repo.feature, "feature/monitor-links"),
    ] {
        let (_, gates, _) = scope(&repo, head, "push", branch, &repo.main);
        assert_eq!(gates, "", "{branch}");
    }
}

#[test]
fn a_manual_run_on_a_ticket_branch_runs_linux_against_the_feature_branch() {
    let repo = repo();
    let outcome = scope(
        &repo,
        &repo.ticket,
        "workflow_dispatch",
        "monitor-links/489-link-crate",
        "",
    );
    assert_eq!(outcome, parallel(&repo.feature, "linux"));
}

#[test]
fn a_manual_run_on_a_stack_w_branch_runs_linux_then_windows_against_the_feature_branch() {
    let repo = repo();
    let outcome = scope(
        &repo,
        &repo.ticket,
        "workflow_dispatch",
        "monitor-links/win-544-windows-ci",
        "",
    );
    let expected = (
        repo.feature.clone(),
        "linux,windows".to_owned(),
        "true".to_owned(),
    );
    assert_eq!(outcome, expected);
}

#[test]
fn a_manual_run_on_the_feature_branch_runs_every_gate_against_main() {
    let repo = repo();
    let outcome = scope(
        &repo,
        &repo.feature,
        "workflow_dispatch",
        "feature/monitor-links",
        "",
    );
    assert_eq!(outcome, parallel(&repo.main, ALL));
}

#[test]
fn any_other_branch_runs_every_gate_against_main() {
    let repo = repo();
    for event in ["push", "workflow_dispatch"] {
        // A name that only resembles the feature's is not one of its branches.
        for branch in [
            "fix/other",
            "feature/monitor-links-2",
            "x/monitor-links/1",
            "win-544",
            "x/monitor-links/win-544",
        ] {
            let outcome = scope(&repo, &repo.ticket, event, branch, &repo.feature);
            assert_eq!(outcome, parallel(&repo.main, ALL), "{event} {branch}");
        }
    }
}

#[test]
fn main_runs_every_gate_against_the_commit_before_the_push() {
    let repo = repo();
    let outcome = scope(&repo, &repo.main, "push", "main", &repo.first);
    assert_eq!(outcome, parallel(&repo.first, ALL));
    // A push that created the branch, or a manual run, names no commit before it.
    for before in ["", "0000000000000000000000000000000000000000"] {
        let outcome = scope(&repo, &repo.main, "push", "main", before);
        assert_eq!(outcome, parallel(&repo.first, ALL), "{before:?}");
    }
}
