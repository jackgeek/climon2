//! #452: `tools/quality-full-run.py` chooses the slice of shipped files each
//! squash-merge to main mutates, and reports each slice's findings in its
//! section of one tracking issue.

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

    /// The slice a push from `before` to `after` mutates ("" for none), and what it printed.
    fn slice(&self, event: &str, requested: &str, before: &str, after: &str) -> (String, String) {
        let outcome = script(
            self.dir.path(),
            &[
                "slice",
                "--event",
                event,
                "--requested",
                requested,
                "--before",
                before,
                "--after",
                after,
            ],
            None,
        );
        assert_eq!(outcome.summary, format!("- {}", outcome.stdout));
        let slice = outcome
            .output
            .strip_prefix("slice=")
            .and_then(|rest| rest.strip_suffix('\n'))
            .unwrap_or_else(|| panic!("unexpected output {:?}", outcome.output));
        (slice.to_owned(), outcome.stdout)
    }

    fn push(&self, before: &str, after: &str) -> String {
        self.slice("push", "", before, after).0
    }
}

#[test]
fn each_squash_merge_mutates_the_next_slice_in_turn() {
    let history = History::new();
    let mut before = history.git(&["rev-parse", "HEAD"]);
    for number in 1..=21 {
        let after = history.squash(number..=number);
        assert_eq!(history.push(&before, &after), (number % 10).to_string());
        before = after;
    }
    let (slice, message) = history.slice("push", "", &before, &before);
    assert_eq!(slice, "");
    assert_eq!(
        message,
        "Full mutation pass skipped: the push adds no squash-merge; main still has 21.\n"
    );
    let after = history.squash(22..=22);
    assert_eq!(
        history.slice("push", "", &before, &after).1,
        "Full mutation pass mutates slice 2: main has 22 squash-merges, so slice 2 of 10.\n"
    );
}

#[test]
fn only_squash_merges_on_mains_first_parent_history_count() {
    let history = History::new();
    let start = history.squash(1..=8);
    // A direct push, and a merge commit whose branch holds what look like squash-merges.
    let direct = history.commit("Fix a typo directly on main");
    assert_eq!(history.push(&start, &direct), "");
    history.git(&["checkout", "--quiet", "-b", "topic"]);
    history.squash(100..=105);
    history.git(&["checkout", "--quiet", "main"]);
    history.git(&["merge", "--quiet", "--no-ff", "-m", "Merge topic", "topic"]);
    let merged = history.git(&["rev-parse", "HEAD"]);
    let (slice, message) = history.slice("push", "", &direct, &merged);
    assert_eq!(slice, "", "{message}");
    assert!(message.contains("main still has 8"), "{message}");
    // Nor does a subject that only mentions a number before its end.
    let mention = history.commit("Revert (#3) and explain why");
    assert_eq!(history.push(&merged, &mention), "");
    let ninth = history.squash(9..=9);
    assert_eq!(history.push(&mention, &ninth), "9");
}

#[test]
fn a_push_of_several_merges_mutates_the_last_ones_slice() {
    let history = History::new();
    let before = history.squash(1..=8);
    let after = history.squash(9..=12);
    let (slice, message) = history.slice("push", "", &before, &after);
    assert_eq!(slice, "2");
    assert!(
        message.ends_with(
            "; the push adds 4, and the slices of the others wait for their next turn.\n"
        ),
        "{message}"
    );
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
        assert_eq!(history.push(before, &tenth), "0", "{before:?}");
    }
    let direct = history.commit("Fix a typo directly on main");
    assert_eq!(history.push(&zero, &direct), "");
}

#[test]
fn a_manual_run_mutates_the_slice_it_names_or_all() {
    let history = History::new();
    let head = history.squash(1..=3);
    for requested in ["0", "7", "9", "all"] {
        assert_eq!(
            history.slice("workflow_dispatch", requested, "", &head).0,
            requested
        );
    }
    assert_eq!(
        history.slice("workflow_dispatch", "all", "", &head).1,
        "Full mutation pass mutates all slices: a manual run asked for slice all.\n"
    );
    let (slice, message) = history.slice("workflow_dispatch", "10", "", &head);
    assert_eq!(slice, "");
    assert_eq!(
        message,
        "Full mutation pass skipped: a manual run names slice 0 to 9 or all, not '10'.\n"
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

/// Logs in which every platform that ran found nothing.
fn clean_logs() -> tempfile::TempDir {
    let logs = tempfile::tempdir().unwrap();
    let linux = logs.path().join("full-run-linux");
    fs::create_dir(&linux).unwrap();
    fs::write(linux.join("gate.log"), "KILLED    a.rs:1 + -> -\n").unwrap();
    fs::write(linux.join("status"), "0").unwrap();
    logs
}

fn report(logs: &Path, slice: &str, responses: Value) -> Outcome {
    let logs = logs.to_str().unwrap();
    let args = [
        "report",
        "--repo",
        REPO,
        "--slice",
        slice,
        "--logs",
        logs,
        "--run-url",
        RUN_URL,
        "--sha",
        SHA,
    ];
    script(Path::new(logs), &args, Some(responses))
}

fn issue(number: u32, title: &str, login: &str, body: &str) -> Value {
    json!({
        "number": number,
        "title": title,
        "user": {"login": login},
        "body": body,
        "html_url": format!("https://github.com/{REPO}/issues/{number}"),
    })
}

fn tracker(body: &str) -> Value {
    json!({ ISSUES: [issue(9, "Full mutation pass findings", "github-actions[bot]", body)] })
}

/// The section a slice's report writes, between its markers.
fn section<'a>(body: &'a str, slice: &str) -> &'a str {
    let start = format!("<!-- slice {slice} -->\n");
    let end = format!("<!-- end slice {slice} -->\n");
    let from = body
        .find(&start)
        .unwrap_or_else(|| panic!("no {slice}: {body}"));
    let to = body[from..].find(&end).unwrap() + from + end.len();
    &body[from..to]
}

#[test]
fn findings_open_the_tracking_issue_when_none_is_open() {
    let logs = logs(Some("3"));
    let outcome = report(
        logs.path(),
        "4",
        json!({ ISSUES: [
            // Same title, but a person's issue, and a pull request: neither is the tracker.
            issue(5, "Full mutation pass findings", "someone", ""),
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
    let expected = format!(
        "<!-- slice 4 -->\n## Slice 4 of 10, at {SHA}\n\n{RUN_URL}\n\n\
         ### Linux: 3 findings, exit 3\n\n```text\n\
         CRAP parse climon2_core::wire 6 40.0% 14.2\n\
         SURVIVED  crates/climon2-core/src/wire.rs:42 == -> !=\n\
         UNCOVERED crates/climon2-core/src/wire.rs:57 true -> false\n```\n\n\
         ### Windows: no findings\n\n\
         ### macOS\n\nDid not run.\n<!-- end slice 4 -->\n"
    );
    assert_eq!(section(body, "4"), expected);
    assert!(
        body.starts_with("Each squash-merge to main mutates one slice"),
        "{body}"
    );
    assert_eq!(
        outcome.stdout,
        format!("Findings in slice 4 opened in https://github.com/repos/{REPO}/issues.\n")
    );
}

#[test]
fn findings_replace_only_their_slices_section_in_the_open_issue() {
    let old = "Old header\n\n<!-- slice all -->\nall\n<!-- end slice all -->\n\n\
               <!-- slice 7 -->\nseven\n<!-- end slice 7 -->\n\n\
               <!-- slice 2 -->\nstale two\n<!-- end slice 2 -->\n";
    let logs = logs(Some("3"));
    let outcome = report(logs.path(), "2", tracker(old));
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
    let body = outcome.writes[0]["body"]["body"].as_str().unwrap();
    // The other slices' sections stay, in slice order, and the new one replaces slice 2's.
    assert!(!body.contains("stale two"), "{body}");
    assert!(section(body, "2").contains("SURVIVED  crates/climon2-core/src/wire.rs:42"));
    assert_eq!(
        section(body, "7"),
        "<!-- slice 7 -->\nseven\n<!-- end slice 7 -->\n"
    );
    let two = body.find("<!-- slice 2 -->").unwrap();
    let seven = body.find("<!-- slice 7 -->").unwrap();
    let all = body.find("<!-- slice all -->").unwrap();
    assert!(two < seven && seven < all, "{body}");
    assert_eq!(
        outcome.writes[1]["body"]["body"],
        format!("New findings in slice 2: {RUN_URL}")
    );
    assert_eq!(
        outcome.stdout,
        format!("Findings in slice 2 updated in https://github.com/{REPO}/issues/9.\n")
    );
}

#[test]
fn a_failed_or_timed_out_slice_without_findings_is_reported() {
    for (status, outcome) in [
        (Some("2"), "exit 2"),
        (None, "no exit status: the slice timed out or was cancelled"),
    ] {
        let logs = tempfile::tempdir().unwrap();
        let linux = logs.path().join("full-run-linux");
        fs::create_dir(&linux).unwrap();
        fs::write(linux.join("gate.log"), "baseline failed\n").unwrap();
        if let Some(status) = status {
            fs::write(linux.join("status"), status).unwrap();
        }
        let result = report(logs.path(), "1", json!({ ISSUES: [] }));
        let body = result.writes[0]["body"]["body"].as_str().unwrap();
        let expected = format!("### Linux: no findings, {outcome}\n\nSee the run's log.\n");
        assert!(section(body, "1").contains(&expected), "{body}");
    }
}

#[test]
fn a_clean_slice_clears_its_section_without_a_comment_and_opens_no_issue() {
    let clean = report(clean_logs().path(), "3", json!({ ISSUES: [] }));
    assert!(clean.writes.is_empty(), "{:#?}", clean.writes);
    assert_eq!(clean.stdout, "No findings in slice 3.\n");
    let old = "<!-- slice 3 -->\nSURVIVED old\n<!-- end slice 3 -->\n";
    let open = report(clean_logs().path(), "3", tracker(old));
    assert_eq!(open.writes.len(), 1, "{:#?}", open.writes);
    assert_eq!(open.writes[0]["method"], "PATCH");
    let body = open.writes[0]["body"]["body"].as_str().unwrap();
    assert!(
        section(body, "3").contains("### Linux: no findings\n"),
        "{body}"
    );
    assert!(!body.contains("SURVIVED old"), "{body}");
    assert_eq!(
        open.stdout,
        format!(
            "No findings in slice 3; its section in https://github.com/{REPO}/issues/9 \
             is cleared.\n"
        )
    );
}

#[test]
fn a_long_list_is_cut_short_inside_its_section() {
    let logs = tempfile::tempdir().unwrap();
    let linux = logs.path().join("full-run-linux");
    fs::create_dir(&linux).unwrap();
    let log: String = (0..2000)
        .map(|line| format!("SURVIVED  crates/climon2-core/src/wire.rs:{line} == -> !=\n"))
        .collect();
    fs::write(linux.join("gate.log"), log).unwrap();
    fs::write(linux.join("status"), "3").unwrap();
    let outcome = report(logs.path(), "all", json!({ ISSUES: [] }));
    let body = outcome.writes[0]["body"]["body"].as_str().unwrap();
    let all = section(body, "all");
    assert!(
        all.starts_with("<!-- slice all -->\n## All slices, at "),
        "{all}"
    );
    assert!(all.len() < 5500, "{}", all.len());
    // The text block is closed before the note of what was left out.
    let note = all
        .find("\n```\n\nAnd ")
        .expect("a closed block, then the note");
    assert!(
        all[note..].contains(&format!(" more in {RUN_URL}.\n")),
        "{all}"
    );
}

#[test]
fn an_api_error_is_reported_without_failing_the_job() {
    let logs = logs(Some("3"));
    let outcome = report(logs.path(), "5", json!({}));
    assert!(outcome.writes.is_empty());
    assert!(
        outcome
            .stdout
            .starts_with("Could not update the tracking issue for slice 5: no response for "),
        "{}",
        outcome.stdout
    );
}
