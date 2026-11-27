//! #442: `tools/quality-prior-run.py` decides whether a push to main can skip
//! the own-hardware gates. It relies only on a successful push run, on a branch
//! other than main, whose head had the same tree and whose every gate job
//! passed. A partial match, no match or an API error leaves the gates to run.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};

const REPO: &str = "owner/repo";
const TREE: &str = "c0d283ea5485b3ec494c9909d1689ec9bfd6cf03";
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

fn run(id: u64, branch: &str, tree: &str) -> Value {
    json!({
        "id": id,
        "head_branch": branch,
        "head_sha": format!("{id:040}"),
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

/// Runs the script against `responses`, a map of API path to response.
fn decide(responses: Value) -> Outcome {
    let temp = tempfile::tempdir().unwrap();
    let fixtures = temp.path().join("responses.json");
    fs::write(&fixtures, responses.to_string()).unwrap();
    let output = temp.path().join("output");
    let summary = temp.path().join("summary");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/quality-prior-run.py");
    let result = Command::new(python())
        .arg(script)
        .args(["--repo", REPO, "--tree", TREE, "--fixtures"])
        .arg(&fixtures)
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
        "Own-hardware gates skipped: tree {TREE} passed every gate in {url} \
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
    }));
    assert_eq!(outcome.output, "");
    let message =
        format!("Own-hardware gates run: no passed run on another branch has tree {TREE}.\n");
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
    }));
    assert_eq!(outcome.output, "");
    assert_eq!(
        outcome.stdout,
        format!(
            "Own-hardware gates run: tree {TREE} did not pass every gate in \
             https://github.com/{REPO}/actions/runs/3, https://github.com/{REPO}/actions/runs/4.\n"
        )
    );
}

#[test]
fn a_match_missing_a_gate_job_runs_the_gates() {
    let mut partial = jobs(["success", "success", "success"]);
    partial["jobs"].as_array_mut().unwrap().pop();
    let outcome = decide(json!({
        runs_path(): {"workflow_runs": [run(5, "fix/old-workflow", TREE)]},
        jobs_path(5): partial,
    }));
    assert_eq!(outcome.output, "");
    assert!(outcome.stdout.starts_with("Own-hardware gates run: "));
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
    // No response for a matching run's jobs.
    let outcome = decide(json!({
        runs_path(): {"workflow_runs": [run(6, "fix/branch", TREE)]},
    }));
    assert_eq!(outcome.output, "");
    assert!(
        outcome
            .stdout
            .starts_with("Own-hardware gates run: could not look up passed runs: "),
        "{}",
        outcome.stdout
    );
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
