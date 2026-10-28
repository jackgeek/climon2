use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const GOLDEN: &str = "crates/climon2-core/tests/goldens/metadata-v1.json";

fn git(repository: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repository)
        .env("GIT_AUTHOR_NAME", "Fixture test")
        .env("GIT_AUTHOR_EMAIL", "fixtures@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture test")
        .env("GIT_COMMITTER_EMAIL", "fixtures@example.invalid")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
}

/// A private clone with the repository's whole history; `extra` adds clone options.
fn clone(directory: &Path, extra: &[&str]) -> PathBuf {
    let source = format!(
        "file://{}",
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .display()
    );
    let target = directory.join("clone");
    let mut args = vec!["clone", "--quiet", "--no-local"];
    args.extend(extra);
    args.extend([source.as_str(), target.to_str().unwrap()]);
    git(directory, &args);
    target
}

fn check(repository: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("fixtures-check")
        .env("GIT_DIR", repository.join(".git"))
        .env("GIT_WORK_TREE", repository)
        .env_remove("CLIMON2_MUTATION_BASE")
        .output()
        .unwrap()
}

fn assert_rejected(output: &Output) {
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("metadata fixtures are append-only"),
        "{output:?}"
    );
}

/// #26/ADR 0005/#39: adding a golden is allowed; history and working-tree checks
/// need no comparison base, so a new release tag cannot hide an edited golden.
#[test]
fn metadata_fixture_history_allows_additions_only() {
    let temporary = tempfile::tempdir().unwrap();
    let repository = clone(temporary.path(), &[]);
    let golden = repository.join(GOLDEN);
    fs::write(golden.with_file_name("metadata-v2.json"), b"{}\n").unwrap();
    git(&repository, &["add", "-A"]);
    git(&repository, &["commit", "--quiet", "-m", "append fixture"]);
    assert!(check(&repository).status.success());

    fs::write(&golden, b"{\"rewritten\":true}\n").unwrap();
    assert_rejected(&check(&repository));
    git(
        &repository,
        &["commit", "--quiet", "-am", "rewrite fixture"],
    );
    assert_rejected(&check(&repository));
}

/// #26/ADR 0005: deleting an existing golden anywhere in history is rejected.
#[test]
fn metadata_fixture_history_rejects_deletion() {
    let temporary = tempfile::tempdir().unwrap();
    let repository = clone(temporary.path(), &[]);
    git(&repository, &["rm", "--quiet", GOLDEN]);
    git(&repository, &["commit", "--quiet", "-m", "delete fixture"]);
    assert_rejected(&check(&repository));
}

/// #26/#39: without full history the check cannot be proven, so it fails closed.
#[test]
fn metadata_fixture_check_fails_closed_on_shallow_history() {
    let temporary = tempfile::tempdir().unwrap();
    let repository = clone(temporary.path(), &["--depth", "1"]);
    let output = check(&repository);
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("full git history"),
        "{output:?}"
    );
}
