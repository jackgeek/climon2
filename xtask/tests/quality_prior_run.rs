//! #442, #447: `tools/quality-prior-run.py` decides whether a push to main can
//! skip the own-hardware gates. It relies only on a successful push run, on a
//! branch other than main, whose head had the same tree, or differs from it only
//! in paths quality.yml ignores, and in which every gate job that
//! CLIMON2_OWN_RUNNERS lists passed. A partial match, no match or an API error
//! leaves the gates to run.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};

const REPO: &str = "owner/repo";
const TREE: &str = "c0d283ea5485b3ec494c9909d1689ec9bfd6cf03";
const COMMIT: &str = "abcdef0123456789abcdef0123456789abcdef01";
/// CLIMON2_OWN_RUNNERS listing every platform.
const ALL: &str = "linux,windows,macos";
const GATES: [&str; 3] = [
    "Linux CRAP and mutation / own hardware",
    "Windows-only CRAP and mutation / own hardware",
    "macOS-only CRAP and mutation / own hardware",
];

fn python() -> PathBuf {
    env::var_os("CLIMON2_QUALITY_PYTHON")
        .unwrap_or_else(|| if cfg!(windows) { "python" } else { "python3" }.into())
        .into()
}

fn runs_path() -> String {
    format!(
        "repos/{REPO}/actions/workflows/quality.yml/runs?event=push&status=success&per_page=100"
    )
}

fn jobs_path(run: u64) -> String {
    format!("repos/{REPO}/actions/runs/{run}/jobs?filter=latest&per_page=100")
}

fn pulls_path(commit: &str) -> String {
    format!("repos/{REPO}/commits/{commit}/pulls")
}

/// The pull requests GitHub associates with `commit`: the one whose squash-merge made it,
/// from `branch`, after one that merely contains it.
fn merged_from(commit: &str, branch: &str) -> Value {
    json!([
        {"merge_commit_sha": "1".repeat(40), "head": {"ref": "fix/other"}},
        {"merge_commit_sha": commit, "head": {"ref": branch}},
    ])
}

fn run(id: u64, branch: &str, tree: &str) -> Value {
    run_of(id, branch, &format!("{id:040}"), tree)
}

fn run_of(id: u64, branch: &str, sha: &str, tree: &str) -> Value {
    json!({
        "id": id,
        "head_branch": branch,
        "head_sha": sha,
        "event": "push",
        "status": "completed",
        "conclusion": "success",
        "html_url": format!("https://github.com/{REPO}/actions/runs/{id}"),
        "head_commit": {"tree_id": tree},
    })
}

/// The run's jobs: the base job and each gate, with the given conclusions.
fn jobs(conclusions: [&str; 3]) -> Value {
    let mut jobs = vec![json!({
        "name": "Select the changed-source mutation base",
        "conclusion": "success",
    })];
    for (name, conclusion) in GATES.into_iter().zip(conclusions) {
        jobs.push(json!({"name": name, "conclusion": conclusion}));
    }
    json!({ "jobs": jobs })
}

struct Outcome {
    stdout: String,
    output: String,
    summary: String,
}

/// Runs the script against `responses`, a map of API path to response, for a push of
/// COMMIT, whose tree is TREE, while CLIMON2_OWN_RUNNERS lists every platform.
fn decide(responses: Value) -> Outcome {
    let dir = tempfile::tempdir().unwrap();
    decide_in(dir.path(), TREE, COMMIT, ALL, responses)
}

/// Runs the script in `dir` for a push of `commit`, whose tree is `tree`.
fn decide_in(dir: &Path, tree: &str, commit: &str, own: &str, responses: Value) -> Outcome {
    let temp = tempfile::tempdir().unwrap();
    let fixtures = temp.path().join("responses.json");
    fs::write(&fixtures, responses.to_string()).unwrap();
    let output = temp.path().join("output");
    let summary = temp.path().join("summary");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/quality-prior-run.py");
    let result = Command::new(python())
        .arg(script)
        .args(["--repo", REPO, "--tree", tree, "--commit", commit])
        .args(["--own-runners", own, "--fixtures"])
        .arg(&fixtures)
        .current_dir(dir)
        .env("GITHUB_OUTPUT", &output)
        .env("GITHUB_STEP_SUMMARY", &summary)
        .output()
        .unwrap();
    // Python's text files and standard output end lines with CRLF on Windows.
    let stdout = String::from_utf8(result.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(
        result.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let read = |path: &Path| {
        fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n")
    };
    Outcome {
        output: read(&output),
        summary: read(&summary),
        stdout,
    }
}

#[test]
fn a_branch_run_of_the_same_tree_that_passed_every_gate_skips_the_gates() {
    let outcome = decide(json!({
        runs_path(): {"workflow_runs": [
            run(1, "fix/other", "0000000000000000000000000000000000000000"),
            run(2, "fix/422-short-localappdata", TREE),
        ]},
        jobs_path(2): jobs(["success", "success", "success"]),
    }));
    let url = format!("https://github.com/{REPO}/actions/runs/2");
    assert_eq!(outcome.output, format!("run={url}\n"));
    let message = format!(
        "Own-hardware gates skipped: tree {TREE} passed every listed gate in {url} \
         (fix/422-short-localappdata at {:040}).\n",
        2
    );
    assert_eq!(outcome.stdout, message);
    assert_eq!(outcome.summary, format!("- {message}"));
}

#[test]
fn no_branch_run_of_the_same_tree_runs_the_gates() {
    let outcome = decide(json!({
        runs_path(): {"workflow_runs": [
            run(1, "fix/other", "0000000000000000000000000000000000000000"),
            // main's own earlier run of the tree is no record of a branch's gate.
            run(2, "main", TREE),
        ]},
        jobs_path(2): jobs(["success", "success", "success"]),
        // A direct push merged no pull request.
        pulls_path(COMMIT): [],
    }));
    assert_eq!(outcome.output, "");
    let message = format!(
        "Own-hardware gates run: no passed run on another branch has tree {TREE}, \
         or one that differs from it only in paths quality.yml ignores.\n"
    );
    assert_eq!(outcome.stdout, message);
    assert_eq!(outcome.summary, format!("- {message}"));
}

#[test]
fn a_match_whose_gates_did_not_all_pass_runs_the_gates() {
    let outcome = decide(json!({
        runs_path(): {"workflow_runs": [run(3, "fix/partial", TREE), run(4, "fix/skipped", TREE)]},
        // A failed gate never makes a successful run, but a skipped one does.
        jobs_path(3): jobs(["success", "failure", "success"]),
        jobs_path(4): jobs(["success", "success", "skipped"]),
        pulls_path(COMMIT): [],
    }));
    assert_eq!(outcome.output, "");
    assert_eq!(
        outcome.stdout,
        format!(
            "Own-hardware gates run: tree {TREE} did not pass every listed gate in \
             https://github.com/{REPO}/actions/runs/3, https://github.com/{REPO}/actions/runs/4.\n"
        )
    );
}

/// #447: a platform CLIMON2_OWN_RUNNERS does not list runs no gate on the branch,
/// and would run none on main, so its skipped gate never stops a match. Every
/// listed one must still have passed.
#[test]
fn only_the_gates_of_listed_platforms_must_have_passed() {
    let responses = json!({
        runs_path(): {"workflow_runs": [run(4, "fix/no-mac", TREE)]},
        jobs_path(4): jobs(["success", "success", "skipped"]),
        pulls_path(COMMIT): [],
    });
    let dir = tempfile::tempdir().unwrap();
    for own in ["linux,windows", "Linux, Windows", "linux"] {
        let outcome = decide_in(dir.path(), TREE, COMMIT, own, responses.clone());
        assert_eq!(
            outcome.output,
            format!("run=https://github.com/{REPO}/actions/runs/4\n"),
            "{own}: {}",
            outcome.stdout
        );
    }
    for own in [ALL, "macos"] {
        let outcome = decide_in(dir.path(), TREE, COMMIT, own, responses.clone());
        assert_eq!(outcome.output, "", "{own}");
    }
    // A missing job counts as not passed when its platform is listed.
    let mut partial = jobs(["success", "success", "success"]);
    partial["jobs"].as_array_mut().unwrap().pop();
    let missing = json!({
        runs_path(): {"workflow_runs": [run(5, "fix/old-workflow", TREE)]},
        jobs_path(5): partial,
        pulls_path(COMMIT): [],
    });
    let outcome = decide_in(dir.path(), TREE, COMMIT, "linux,windows", missing.clone());
    assert_eq!(
        outcome.output,
        format!("run=https://github.com/{REPO}/actions/runs/5\n")
    );
    let outcome = decide(missing);
    assert_eq!(outcome.output, "");
    assert!(outcome.stdout.starts_with("Own-hardware gates run: "));
}

/// A git repository whose commits the script compares.
struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    fn new() -> Self {
        let repo = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        repo.git(&["init", "--quiet", "--initial-branch=main"]);
        repo
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(self.dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    /// Commits `paths` with new content and returns (the commit, its tree).
    fn change(&self, paths: &[&str], subject: &str) -> (String, String) {
        for path in paths {
            let file = self.dir.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, subject).unwrap();
            self.git(&["add", path]);
        }
        self.git(&["commit", "--quiet", "-m", subject]);
        (
            self.git(&["rev-parse", "HEAD"]),
            self.git(&["rev-parse", "HEAD^{tree}"]),
        )
    }
}

/// #447: a branch whose last push changed only paths quality.yml ignores has no
/// run of the merged tree. The merged pull request's branch's latest passed run
/// counts when its commit differs from the pushed one only in such paths.
#[test]
fn a_docs_only_difference_from_the_merged_branchs_latest_passed_run_skips_the_gates() {
    let repo = Repo::new();
    let (gated, gated_tree) = repo.change(&["crates/a/src/lib.rs", "README.md"], "Code");
    let (pushed, tree) = repo.change(&["README.md", "docs/a/b.md", "LICENSE"], "Docs (#7)");
    let branch = "fix/docs-last";
    let responses = |latest: Value| {
        json!({
            runs_path(): {"workflow_runs": [
                latest,
                run_of(10, branch, &"2".repeat(40), "0".repeat(40).as_str()),
                run_of(11, "fix/other", &gated, &gated_tree),
            ]},
            jobs_path(9): jobs(["success", "success", "skipped"]),
            jobs_path(11): jobs(["success", "success", "success"]),
            pulls_path(&pushed): merged_from(&pushed, branch),
        })
    };
    let latest = run_of(9, branch, &gated, &gated_tree);
    let outcome = decide_in(
        repo.dir.path(),
        &tree,
        &pushed,
        "linux,windows",
        responses(latest),
    );
    let url = format!("https://github.com/{REPO}/actions/runs/9");
    assert_eq!(outcome.output, format!("run={url}\n"), "{}", outcome.stdout);
    assert_eq!(
        outcome.stdout,
        format!(
            "Own-hardware gates skipped: tree {tree} differs only in paths quality.yml \
             ignores (LICENSE, README.md, docs/a/b.md) from the one that passed every \
             listed gate in {url} ({branch} at {gated}).\n"
        )
    );
    // Its listed gates must have passed too.
    let latest = run_of(9, branch, &gated, &gated_tree);
    let outcome = decide_in(repo.dir.path(), &tree, &pushed, ALL, responses(latest));
    assert_eq!(outcome.output, "", "{}", outcome.stdout);
    // Only the branch's latest passed run counts, not another branch's run.
    let other = run_of(9, "fix/third", &gated, &gated_tree);
    let outcome = decide_in(repo.dir.path(), &tree, &pushed, ALL, responses(other));
    assert_eq!(outcome.output, "", "{}", outcome.stdout);
}

#[test]
fn a_difference_in_gated_paths_or_an_unknown_commit_runs_the_gates() {
    let repo = Repo::new();
    let (gated, gated_tree) = repo.change(&["crates/a/src/lib.rs"], "Code");
    // The protocol's Markdown is gated.
    for path in ["crates/a/src/lib.rs", "docs/for-humans/protocol/x/y.md"] {
        let (pushed, tree) = repo.change(&[path, "README.md"], &format!("Change {path}"));
        let outcome = decide_in(
            repo.dir.path(),
            &tree,
            &pushed,
            ALL,
            json!({
                runs_path(): {"workflow_runs": [run_of(9, "fix/code", &gated, &gated_tree)]},
                jobs_path(9): jobs(["success", "success", "success"]),
                pulls_path(&pushed): merged_from(&pushed, "fix/code"),
            }),
        );
        assert_eq!(outcome.output, "", "{path}: {}", outcome.stdout);
        assert!(
            outcome
                .stdout
                .starts_with("Own-hardware gates run: no passed run on another branch"),
            "{path}: {}",
            outcome.stdout
        );
    }
    // A run whose commit the clone lacks and cannot fetch.
    let (pushed, tree) = repo.change(&["README.md"], "Docs");
    let missing = "3".repeat(40);
    let outcome = decide_in(
        repo.dir.path(),
        &tree,
        &pushed,
        ALL,
        json!({
            runs_path(): {"workflow_runs": [run_of(9, "fix/gone", &missing, &gated_tree)]},
            jobs_path(9): jobs(["success", "success", "success"]),
            pulls_path(&pushed): merged_from(&pushed, "fix/gone"),
        }),
    );
    assert_eq!(outcome.output, "", "{}", outcome.stdout);
}

/// A file moved from a gated path to an ignored one removes gated source, so the
/// move counts by both its names, not only the ignored new one.
#[test]
fn a_rename_from_a_gated_path_to_an_ignored_one_runs_the_gates() {
    let repo = Repo::new();
    let (gated, gated_tree) = repo.change(&["crates/a/src/lib.rs", "README.md"], "Code");
    repo.git(&["mv", "crates/a/src/lib.rs", "notes.md"]);
    repo.git(&["commit", "--quiet", "-m", "Move (#8)"]);
    let pushed = repo.git(&["rev-parse", "HEAD"]);
    let tree = repo.git(&["rev-parse", "HEAD^{tree}"]);
    let outcome = decide_in(
        repo.dir.path(),
        &tree,
        &pushed,
        ALL,
        json!({
            runs_path(): {"workflow_runs": [run_of(9, "fix/move", &gated, &gated_tree)]},
            jobs_path(9): jobs(["success", "success", "success"]),
            pulls_path(&pushed): merged_from(&pushed, "fix/move"),
        }),
    );
    assert_eq!(outcome.output, "", "{}", outcome.stdout);
}

/// When the docs-only fallback's look-up fails after a partial match, the partial
/// match's message is kept, as the more useful one.
#[test]
fn a_failed_fallback_keeps_the_partial_matchs_message() {
    let outcome = decide(json!({
        runs_path(): {"workflow_runs": [run(3, "fix/partial", TREE)]},
        jobs_path(3): jobs(["success", "failure", "success"]),
    }));
    assert_eq!(outcome.output, "");
    assert_eq!(
        outcome.stdout,
        format!(
            "Own-hardware gates run: tree {TREE} did not pass every listed gate in \
             https://github.com/{REPO}/actions/runs/3.\n"
        )
    );
}

#[test]
fn an_api_error_runs_the_gates() {
    // No response for the run list.
    let outcome = decide(json!({}));
    assert_eq!(outcome.output, "");
    assert_eq!(
        outcome.stdout,
        format!(
            "Own-hardware gates run: could not look up passed runs: no response for {}.\n",
            runs_path()
        )
    );
    // No response for a matching run's jobs, or for the merged pull request.
    for responses in [
        json!({ runs_path(): {"workflow_runs": [run(6, "fix/branch", TREE)]} }),
        json!({ runs_path(): {"workflow_runs": []} }),
    ] {
        let outcome = decide(responses);
        assert_eq!(outcome.output, "");
        assert!(
            outcome
                .stdout
                .starts_with("Own-hardware gates run: could not look up passed runs: "),
            "{}",
            outcome.stdout
        );
    }
    // A response of the wrong shape.
    let outcome = decide(json!({ runs_path(): {"message": "Bad credentials"} }));
    assert_eq!(outcome.output, "");
    assert!(
        outcome
            .stdout
            .starts_with("Own-hardware gates run: could not look up passed runs: "),
        "{}",
        outcome.stdout
    );
}
