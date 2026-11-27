//! #452: `tools/quality-full-run.py` decides when main's full mutation pass is
//! due, waits for queued gates before it starts, and reports its findings in
//! one tracking issue.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};

const REPO: &str = "owner/repo";
const RUN_URL: &str = "https://github.com/owner/repo/actions/runs/7";
const SHA: &str = "c0d283ea5485b3ec494c9909d1689ec9bfd6cf03";
const ISSUES: &str =
    "repos/owner/repo/issues?state=open&creator=github-actions%5Bbot%5D&per_page=100";

fn python() -> PathBuf {
    env::var_os("CLIMON2_QUALITY_PYTHON")
        .unwrap_or_else(|| if cfg!(windows) { "python" } else { "python3" }.into())
        .into()
}

struct Outcome {
    stdout: String,
    output: String,
    summary: String,
    /// Each API write, one JSON object per line.
    writes: Vec<Value>,
}

/// Runs the script with `args` in `dir`, answering API reads from `responses`.
fn script(dir: &Path, args: &[&str], responses: Option<Value>) -> Outcome {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("output");
    let summary = temp.path().join("summary");
    let record = temp.path().join("record");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/quality-full-run.py");
    let mut command = Command::new(python());
    command.arg(path).args(args).current_dir(dir);
    if let Some(responses) = responses {
        let fixtures = temp.path().join("responses.json");
        fs::write(&fixtures, responses.to_string()).unwrap();
        command
            .arg("--fixtures")
            .arg(&fixtures)
            .arg("--record")
            .arg(&record);
    }
    let result = command
        .env("GITHUB_OUTPUT", &output)
        .env("GITHUB_STEP_SUMMARY", &summary)
        .output()
        .unwrap();
    // Python's text files and standard output end lines with CRLF on Windows.
    let read = |path: &Path| {
        fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n")
    };
    let stdout = String::from_utf8(result.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(
        result.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&result.stderr)
    );
    Outcome {
        output: read(&output),
        summary: read(&summary),
        writes: read(&record)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect(),
        stdout,
    }
}

/// A git repository whose main history is built commit by commit.
struct History {
    dir: tempfile::TempDir,
}

impl History {
    fn new() -> Self {
        let history = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        history.git(&["init", "--quiet", "--initial-branch=main"]);
        history.commit("Initial commit");
        history
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

    /// Commits `subject` on the current branch and returns its id.
    fn commit(&self, subject: &str) -> String {
        self.git(&["commit", "--quiet", "--allow-empty", "-m", subject]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// Squash-merges pull requests `from..=to` and returns the last one's commit.
    fn squash(&self, numbers: std::ops::RangeInclusive<u32>) -> String {
        let mut head = String::new();
        for number in numbers {
            head = self.commit(&format!("Change number {number} (#{number})"));
        }
        head
    }

    /// Whether a push from `before` to `after` is due, and what it printed.
    fn due(&self, event: &str, before: &str, after: &str) -> (bool, String) {
        let outcome = script(
            self.dir.path(),
            &[
                "due", "--event", event, "--before", before, "--after", after,
            ],
            None,
        );
        assert_eq!(outcome.summary, format!("- {}", outcome.stdout));
        let due = match outcome.output.as_str() {
            "due=true\n" => true,
            "due=false\n" => false,
            other => panic!("unexpected output {other:?}"),
        };
        (due, outcome.stdout)
    }
}

#[test]
fn only_the_tenth_squash_merge_since_the_last_full_pass_starts_one() {
    let history = History::new();
    let mut before = history.git(&["rev-parse", "HEAD"]);
    for number in 1..=21 {
        let after = history.squash(number..=number);
        let (due, message) = history.due("push", &before, &after);
        assert_eq!(due, number % 10 == 0, "merge {number}: {message}");
        before = after;
    }
    let (_, message) = history.due("push", &before, &before);
    assert_eq!(
        message,
        "Full mutation pass skipped: main has 21 squash-merges; the last full pass was due \
         at 20 and the next is at 30.\n"
    );
}

#[test]
fn only_squash_merges_on_mains_first_parent_history_count() {
    let history = History::new();
    let start = history.squash(1..=8);
    // A direct push, and a merge commit whose branch holds what look like squash-merges.
    history.commit("Fix a typo directly on main");
    history.git(&["checkout", "--quiet", "-b", "topic"]);
    history.squash(100..=105);
    history.git(&["checkout", "--quiet", "main"]);
    let merged = {
        history.git(&["merge", "--quiet", "--no-ff", "-m", "Merge topic", "topic"]);
        history.git(&["rev-parse", "HEAD"])
    };
    let (due, message) = history.due("push", &start, &merged);
    assert!(!due, "{message}");
    assert!(message.contains("main has 8 squash-merges"), "{message}");
    // Nor does a subject that only mentions a number before its end.
    let mention = history.commit("Revert (#3) and explain why");
    assert!(!history.due("push", &merged, &mention).0);
    let ninth = history.squash(9..=9);
    assert!(!history.due("push", &mention, &ninth).0);
    let tenth = history.squash(10..=10);
    let (due, message) = history.due("push", &ninth, &tenth);
    assert!(due, "{message}");
    assert_eq!(
        message,
        "Full mutation pass runs: main has 10 squash-merges, and the full pass at 10 is due.\n"
    );
}

#[test]
fn a_push_of_several_merges_that_passes_a_tenth_starts_one() {
    let history = History::new();
    let before = history.squash(1..=8);
    let after = history.squash(9..=12);
    assert!(history.due("push", &before, &after).0);
}

#[test]
fn a_push_without_a_known_old_head_counts_only_its_own_commit() {
    let history = History::new();
    history.squash(1..=9);
    let tenth = history.squash(10..=10);
    let zero = "0".repeat(40);
    for before in [
        "",
        zero.as_str(),
        "1111111111111111111111111111111111111111",
    ] {
        assert!(history.due("push", before, &tenth).0, "{before:?}");
    }
    let eleventh = history.squash(11..=11);
    assert!(!history.due("push", &zero, &eleventh).0);
}

#[test]
fn a_manual_run_is_always_a_full_pass() {
    let history = History::new();
    let head = history.squash(1..=3);
    let (due, message) = history.due("workflow_dispatch", "", &head);
    assert!(due);
    assert_eq!(
        message,
        "Full mutation pass runs: a manual run is always a full pass.\n"
    );
}

fn runs_path(status: &str) -> String {
    format!("repos/{REPO}/actions/workflows/quality.yml/runs?status={status}&per_page=1")
}

/// The run list for each unfinished status, `busy` of them in progress.
fn runs(busy: u32) -> Value {
    let mut responses = json!({});
    for status in ["queued", "in_progress", "pending", "waiting", "requested"] {
        let count = if status == "in_progress" { busy } else { 0 };
        responses[runs_path(status)] = json!({"total_count": count, "workflow_runs": []});
    }
    responses
}

fn wait(responses: Value) -> String {
    let dir = tempfile::tempdir().unwrap();
    let args = ["wait", "--repo", REPO, "--interval", "60", "--limit", "0"];
    script(dir.path(), &args, Some(responses)).stdout
}

#[test]
fn a_full_pass_starts_once_no_gate_is_queued_or_in_progress() {
    assert_eq!(
        wait(runs(0)),
        "Full mutation pass starts: no quality.yml run was queued or in progress after 0 min.\n"
    );
    assert_eq!(
        wait(runs(2)),
        "Full mutation pass starts: 2 quality.yml runs still unfinished after 0 min; \
         it starts anyway.\n"
    );
    let failed = wait(json!({}));
    assert!(
        failed.starts_with("Full mutation pass starts: could not look up quality.yml runs"),
        "{failed}"
    );
}

const LINUX_LOG: &str = "\
CRAP Report
===========
Languages: rust

Function                       Namespace                              CC    Cov%     CRAP
-------------------------------------------------------------------------------------------
frame                          climon2_core::screen                    2  100.0%      2.0
parse                          climon2_core::wire                      6   40.0%     14.2
unscored                       climon2_core::wire                      1    N/A       N/A

KILLED    crates/climon2-core/src/screen.rs:10 + -> -
SURVIVED  crates/climon2-core/src/wire.rs:42 == -> !=
UNCOVERED crates/climon2-core/src/wire.rs:57 true -> false
Function Namespace Killed Survived Uncovered Sites Score
";

/// Report logs: Linux with findings, Windows clean, macOS absent.
fn logs(linux_status: Option<&str>) -> tempfile::TempDir {
    let logs = tempfile::tempdir().unwrap();
    let linux = logs.path().join("full-run-linux");
    fs::create_dir(&linux).unwrap();
    fs::write(linux.join("gate.log"), LINUX_LOG.replace('\n', "\r\n")).unwrap();
    if let Some(status) = linux_status {
        fs::write(linux.join("status"), format!("{status}\n")).unwrap();
    }
    let windows = logs.path().join("full-run-windows");
    fs::create_dir(&windows).unwrap();
    fs::write(windows.join("gate.log"), "KILLED    a.rs:1 + -> -\n").unwrap();
    fs::write(windows.join("status"), "0\n").unwrap();
    logs
}

fn report(logs: &Path, responses: Value) -> Outcome {
    let logs = logs.to_str().unwrap();
    let args = [
        "report",
        "--repo",
        REPO,
        "--logs",
        logs,
        "--run-url",
        RUN_URL,
        "--sha",
        SHA,
    ];
    script(Path::new(logs), &args, Some(responses))
}

fn issue(number: u32, title: &str, login: &str) -> Value {
    json!({
        "number": number,
        "title": title,
        "user": {"login": login},
        "html_url": format!("https://github.com/{REPO}/issues/{number}"),
    })
}

#[test]
fn findings_open_the_tracking_issue_when_none_is_open() {
    let logs = logs(Some("3"));
    let outcome = report(
        logs.path(),
        json!({ ISSUES: [
            // Same title, but a person's issue, and a pull request: neither is the tracker.
            issue(5, "Full mutation pass findings", "someone"),
            {
                "number": 6, "title": "Full mutation pass findings",
                "user": {"login": "github-actions[bot]"}, "pull_request": {},
            },
        ]}),
    );
    assert_eq!(outcome.writes.len(), 1, "{:#?}", outcome.writes);
    let write = &outcome.writes[0];
    assert_eq!(write["method"], "POST");
    assert_eq!(write["path"], format!("repos/{REPO}/issues"));
    assert_eq!(write["body"]["title"], "Full mutation pass findings");
    let body = write["body"]["body"].as_str().unwrap();
    assert!(body.contains(SHA) && body.contains(RUN_URL), "{body}");
    let linux = "### Linux: 3 findings, exit 3\n\n```text\n\
                 CRAP parse climon2_core::wire 6 40.0% 14.2\n\
                 SURVIVED  crates/climon2-core/src/wire.rs:42 == -> !=\n\
                 UNCOVERED crates/climon2-core/src/wire.rs:57 true -> false\n```\n";
    assert!(body.contains(linux), "{body}");
    assert!(body.contains("### Windows: no findings\n"), "{body}");
    assert!(body.contains("### macOS\n\nDid not run.\n"), "{body}");
    assert!(!body.contains("KILLED"), "{body}");
    assert_eq!(
        outcome.stdout,
        format!("Full pass findings opened in https://github.com/repos/{REPO}/issues.\n")
    );
}

#[test]
fn findings_update_the_open_tracking_issue() {
    let logs = logs(Some("3"));
    let outcome = report(
        logs.path(),
        json!({ ISSUES: [issue(9, "Full mutation pass findings", "github-actions[bot]")] }),
    );
    let paths: Vec<_> = outcome
        .writes
        .iter()
        .map(|write| format!("{} {}", write["method"], write["path"]))
        .collect();
    assert_eq!(
        paths,
        [
            format!("\"PATCH\" \"repos/{REPO}/issues/9\""),
            format!("\"POST\" \"repos/{REPO}/issues/9/comments\""),
        ]
    );
    assert!(
        outcome.writes[0]["body"]["body"]
            .as_str()
            .unwrap()
            .contains("SURVIVED  crates/climon2-core/src/wire.rs:42")
    );
    assert_eq!(
        outcome.writes[1]["body"]["body"],
        format!("Updated with the findings of {RUN_URL}.")
    );
    assert_eq!(
        outcome.stdout,
        format!("Full pass findings updated in https://github.com/{REPO}/issues/9.\n")
    );
}

#[test]
fn a_failed_or_timed_out_pass_without_findings_is_reported() {
    for (status, outcome) in [
        (Some("2"), "exit 2"),
        (None, "no exit status: the gate timed out or was cancelled"),
    ] {
        let logs = tempfile::tempdir().unwrap();
        let linux = logs.path().join("full-run-linux");
        fs::create_dir(&linux).unwrap();
        fs::write(linux.join("gate.log"), "baseline failed\n").unwrap();
        if let Some(status) = status {
            fs::write(linux.join("status"), status).unwrap();
        }
        let result = report(logs.path(), json!({ ISSUES: [] }));
        let body = result.writes[0]["body"]["body"].as_str().unwrap();
        let expected = format!("### Linux: no findings, {outcome}\n\nSee the run's log.\n");
        assert!(body.contains(&expected), "{body}");
    }
}

#[test]
fn a_clean_pass_comments_on_an_open_tracking_issue_and_opens_none() {
    let logs = tempfile::tempdir().unwrap();
    let linux = logs.path().join("full-run-linux");
    fs::create_dir(&linux).unwrap();
    fs::write(linux.join("gate.log"), "KILLED    a.rs:1 + -> -\n").unwrap();
    fs::write(linux.join("status"), "0").unwrap();
    let clean = report(logs.path(), json!({ ISSUES: [] }));
    assert!(clean.writes.is_empty(), "{:#?}", clean.writes);
    assert_eq!(clean.stdout, "Full pass found nothing.\n");
    let open = report(
        logs.path(),
        json!({ ISSUES: [issue(9, "Full mutation pass findings", "github-actions[bot]")] }),
    );
    assert_eq!(open.writes.len(), 1);
    assert_eq!(
        open.writes[0]["path"],
        format!("repos/{REPO}/issues/9/comments")
    );
    assert_eq!(
        open.writes[0]["body"]["body"],
        format!("{RUN_URL} found nothing on any platform that ran.")
    );
}

#[test]
fn an_api_error_is_reported_without_failing_the_job() {
    let logs = logs(Some("3"));
    let outcome = report(logs.path(), json!({}));
    assert!(outcome.writes.is_empty());
    assert!(
        outcome
            .stdout
            .starts_with("Full pass could not update its tracking issue: no response for "),
        "{}",
        outcome.stdout
    );
}
