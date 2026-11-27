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

    /// The slices a push from `before` to `after` mutates, as the JSON list it wrote
    /// ("" for none), and what it printed.
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
        let slices = outcome
            .output
            .strip_prefix("slices=")
            .and_then(|rest| rest.strip_suffix('\n'))
            .unwrap_or_else(|| panic!("unexpected output {:?}", outcome.output));
        (slices.to_owned(), outcome.stdout)
    }

    fn push(&self, before: &str, after: &str) -> String {
        self.slice("push", "", before, after).0
    }
}

/// The JSON list of the one slice `index`.
fn one(index: u32) -> String {
    format!("[\"{index}\"]")
}

#[test]
fn each_squash_merge_mutates_the_next_slice_in_turn() {
    let history = History::new();
    let mut before = history.git(&["rev-parse", "HEAD"]);
    for number in 1..=21 {
        let after = history.squash(number..=number);
        assert_eq!(history.push(&before, &after), one(number % 10));
        before = after;
    }
    let (slices, message) = history.slice("push", "", &before, &before);
    assert_eq!(slices, "");
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
    let (slices, message) = history.slice("push", "", &direct, &merged);
    assert_eq!(slices, "", "{message}");
    assert!(message.contains("main still has 8"), "{message}");
    // Nor does a subject that only mentions a number before its end.
    let mention = history.commit("Revert (#3) and explain why");
    assert_eq!(history.push(&merged, &mention), "");
    let ninth = history.squash(9..=9);
    assert_eq!(history.push(&mention, &ninth), one(9));
}

#[test]
fn a_push_of_several_merges_mutates_the_last_ones_slice() {
    let history = History::new();
    let before = history.squash(1..=8);
    let after = history.squash(9..=12);
    let (slices, message) = history.slice("push", "", &before, &after);
    assert_eq!(slices, one(2));
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
        assert_eq!(history.push(before, &tenth), one(0), "{before:?}");
    }
    let direct = history.commit("Fix a typo directly on main");
    assert_eq!(history.push(&zero, &direct), "");
}

#[test]
fn a_manual_run_mutates_the_slice_it_names_or_all_ten() {
    let history = History::new();
    let head = history.squash(1..=3);
    for requested in [0, 7, 9] {
        assert_eq!(
            history
                .slice("workflow_dispatch", &requested.to_string(), "", &head)
                .0,
            one(requested)
        );
    }
    let (slices, message) = history.slice("workflow_dispatch", "all", "", &head);
    assert_eq!(
        slices,
        r#"["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]"#
    );
    assert_eq!(
        message,
        "Full mutation pass mutates slices 0 to 9: a manual run asked for all of them.\n"
    );
    let (slices, message) = history.slice("workflow_dispatch", "10", "", &head);
    assert_eq!(slices, "");
    assert_eq!(
        message,
        "Full mutation pass skipped: a manual run names slice 0 to 9 or all, not '10'.\n"
    );
}

const RUN_ID: u64 = 500;

fn gate_runs(status: &str) -> String {
    format!("repos/{REPO}/actions/workflows/quality.yml/runs?status={status}&per_page=1")
}

fn full_runs(status: &str) -> String {
    format!("repos/{REPO}/actions/workflows/quality-full.yml/runs?status={status}&per_page=100")
}

/// The unfinished runs: `gates` quality.yml runs and these quality-full.yml runs in
/// progress, nothing else.
fn runs(gates: u32, full: &[u64]) -> Value {
    let mut responses = json!({});
    for status in ["queued", "in_progress", "pending", "waiting", "requested"] {
        let busy = status == "in_progress";
        let count = if busy { gates } else { 0 };
        responses[gate_runs(status)] = json!({"total_count": count, "workflow_runs": []});
        let ids: Vec<_> = if busy { full.to_vec() } else { vec![] };
        let listed: Vec<_> = ids.iter().map(|id| json!({ "id": id })).collect();
        responses[full_runs(status)] = json!({"total_count": ids.len(), "workflow_runs": listed});
    }
    responses
}

/// The wait's output and message, with no time to wait.
fn wait(responses: Value) -> (String, String) {
    let dir = tempfile::tempdir().unwrap();
    let id = RUN_ID.to_string();
    let args = [
        "wait",
        "--repo",
        REPO,
        "--run-id",
        &id,
        "--interval",
        "60",
        "--limit",
        "0",
    ];
    let outcome = script(dir.path(), &args, Some(responses));
    assert_eq!(outcome.summary, format!("- {}", outcome.stdout));
    (outcome.output, outcome.stdout)
}

#[test]
fn a_slice_starts_only_once_no_gate_or_older_full_run_is_unfinished() {
    // This run itself and newer full runs never hold it back, so two never wait on each other.
    assert_eq!(
        wait(runs(0, &[RUN_ID, RUN_ID + 1])),
        (
            "idle=true\n".to_owned(),
            "Gates idle: no quality.yml or older full run was unfinished after 0 min.\n".to_owned()
        )
    );
    assert_eq!(
        wait(runs(2, &[])),
        (
            "idle=false\n".to_owned(),
            "Gates still busy: 2 quality.yml and 0 older quality-full.yml runs unfinished \
             after 0 min.\n"
                .to_owned()
        )
    );
    // An older full run, such as a manual run of all slices, holds the machines.
    assert_eq!(
        wait(runs(0, &[RUN_ID - 1, RUN_ID])),
        (
            "idle=false\n".to_owned(),
            "Gates still busy: 0 quality.yml and 1 older quality-full.yml runs unfinished \
             after 0 min.\n"
                .to_owned()
        )
    );
    // An API that fails counts as busy, so a slice never starts while gates may wait.
    let (output, message) = wait(json!({}));
    assert_eq!(output, "idle=false\n");
    assert!(
        message.starts_with("Gates still busy: could not look up unfinished runs: "),
        "{message}"
    );
}

/// The start job's state and message for the wait jobs' `needs` context.
fn start(needs: Value) -> (String, String) {
    let dir = tempfile::tempdir().unwrap();
    let needs = needs.to_string();
    let outcome = script(dir.path(), &["start", "--needs", &needs], None);
    (outcome.output, outcome.stdout)
}

fn waited(results: [(&str, &str); 4]) -> Value {
    let mut needs = json!({ "slice": {"result": "success", "outputs": {"slices": "[\"3\"]"}} });
    for (index, (result, idle)) in results.into_iter().enumerate() {
        let outputs = if idle.is_empty() {
            json!({})
        } else {
            json!({ "idle": idle })
        };
        needs[format!("wait{}", index + 1)] = json!({"result": result, "outputs": outputs});
    }
    needs
}

#[test]
fn the_start_job_tells_idle_gates_from_busy_ones_and_failed_waits() {
    let skipped = ("skipped", "");
    assert_eq!(
        start(waited([("success", "true"), skipped, skipped, skipped])),
        (
            "state=idle\n".to_owned(),
            "Gates idle: the gates went idle.\n".to_owned()
        )
    );
    assert_eq!(
        start(waited([
            ("success", "false"),
            ("success", "false"),
            ("success", "true"),
            skipped
        ]))
        .0,
        "state=idle\n"
    );
    assert_eq!(
        start(waited([("success", "false"); 4])),
        (
            "state=busy\n".to_owned(),
            "Gates busy: the gates stayed busy through 4 waits.\n".to_owned()
        )
    );
    let (state, message) = start(waited([
        ("success", "false"),
        ("failure", ""),
        skipped,
        skipped,
    ]));
    assert_eq!(state, "state=failed\n");
    assert_eq!(
        message,
        "Gates failed: a wait job did not finish: wait1 success, wait2 failure, \
         wait3 skipped, wait4 skipped.\n"
    );
    assert_eq!(start(waited([skipped; 4])).0, "state=failed\n");
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
const KILLED: &str = "KILLED    a.rs:1 + -> -\n";
const DATE: &str = "2026-11-28";
/// CLIMON2_OWN_RUNNERS in the tests: macOS is not listed.
const OWN: &str = "linux,windows";

/// Writes a platform's log, and its status unless it has none, for slice `slice`.
fn platform(logs: &Path, key: &str, slice: &str, log: &str, status: Option<&str>) {
    let folder = logs.join(format!("full-run-{key}-{slice}"));
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("gate.log"), log).unwrap();
    if let Some(status) = status {
        fs::write(folder.join("status"), format!("{status}\n")).unwrap();
    }
}

/// Report logs for `slice`: Linux with findings, Windows clean, macOS absent.
fn logs(slice: &str) -> tempfile::TempDir {
    let logs = tempfile::tempdir().unwrap();
    let crlf = LINUX_LOG.replace('\n', "\r\n");
    platform(logs.path(), "linux", slice, &crlf, Some("3"));
    platform(logs.path(), "windows", slice, KILLED, Some("0"));
    logs
}

/// Logs for `slice` in which both listed platforms found nothing.
fn clean_logs(slice: &str) -> tempfile::TempDir {
    let logs = tempfile::tempdir().unwrap();
    platform(logs.path(), "linux", slice, KILLED, Some("0"));
    platform(logs.path(), "windows", slice, KILLED, Some("0"));
    logs
}

fn report(logs: &Path, slices: &str, gates: &str, responses: Value) -> Outcome {
    let logs = logs.to_str().unwrap();
    let args = [
        "report",
        "--repo",
        REPO,
        "--slices",
        slices,
        "--gates",
        gates,
        "--own-runners",
        OWN,
        "--date",
        DATE,
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

/// The block between `<!-- {kind} {key} -->` and its end marker, markers included.
fn block<'a>(text: &'a str, kind: &str, key: &str) -> &'a str {
    let start = format!("<!-- {kind} {key} -->\n");
    let end = format!("<!-- end {kind} {key} -->\n");
    let from = text
        .find(&start)
        .unwrap_or_else(|| panic!("no {kind} {key}: {text}"));
    let to = text[from..].find(&end).unwrap() + from + end.len();
    &text[from..to]
}

fn section<'a>(body: &'a str, slice: &str) -> &'a str {
    block(body, "slice", slice)
}

/// The body of the issue write `index`.
fn written(outcome: &Outcome, index: usize) -> &str {
    outcome.writes[index]["body"]["body"].as_str().unwrap()
}

#[test]
fn findings_open_the_tracking_issue_when_none_is_open() {
    let logs = logs("4");
    let outcome = report(
        logs.path(),
        &one(4),
        "idle",
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
    let body = written(&outcome, 0);
    let expected = format!(
        "<!-- slice 4 -->\n## Slice 4 of 10, at {SHA}\n\n{RUN_URL}\n\n\
         <!-- platform linux -->\n### Linux: 3 findings, exit 3\n\n```text\n\
         CRAP parse climon2_core::wire 6 40.0% 14.2\n\
         SURVIVED  crates/climon2-core/src/wire.rs:42 == -> !=\n\
         UNCOVERED crates/climon2-core/src/wire.rs:57 true -> false\n```\n\
         <!-- end platform linux -->\n\n\
         <!-- platform windows -->\n### Windows: no findings\n<!-- end platform windows -->\n\n\
         <!-- platform macos -->\n### macOS\n\n\
         > Did not run on {DATE}: it is not in CLIMON2_OWN_RUNNERS.\n\
         <!-- end platform macos -->\n<!-- end slice 4 -->\n"
    );
    assert_eq!(section(body, "4"), expected);
    assert!(
        body.starts_with("Each squash-merge to main mutates one slice"),
        "{body}"
    );
    assert_eq!(
        outcome.stdout,
        format!("Findings in slice 4; https://github.com/repos/{REPO}/issues is opened.\n")
    );
}

/// An issue body as GitHub returns it, with `line_end` ends, and with or without its
/// final newline, as a body edited in the browser may come back.
fn old_body(line_end: &str, final_newline: bool) -> String {
    let body = "Old header\n\n<!-- slice 9 -->\nnine\n<!-- end slice 9 -->\n\n\
                <!-- slice 7 -->\nseven\n<!-- end slice 7 -->\n\n\
                <!-- slice 2 -->\nstale two\n<!-- end slice 2 -->\n";
    let body = if final_newline { body } else { body.trim_end() };
    body.replace('\n', line_end)
}

#[test]
fn findings_replace_only_their_slices_section_in_the_open_issue() {
    for (line_end, final_newline) in [("\n", true), ("\r\n", true), ("\r\n", false)] {
        let logs = logs("2");
        let old = old_body(line_end, final_newline);
        let outcome = report(logs.path(), &one(2), "idle", tracker(&old));
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
        let body = written(&outcome, 0);
        // The other slices' sections stay, in slice order, and the new one replaces slice 2's.
        assert!(!body.contains("stale two"), "{line_end:?}: {body}");
        assert!(section(body, "2").contains("SURVIVED  crates/climon2-core/src/wire.rs:42"));
        assert_eq!(
            section(body, "7"),
            "<!-- slice 7 -->\nseven\n<!-- end slice 7 -->\n",
            "{line_end:?}"
        );
        assert_eq!(
            section(body, "9"),
            "<!-- slice 9 -->\nnine\n<!-- end slice 9 -->\n",
            "{line_end:?} {final_newline}"
        );
        let two = body.find("<!-- slice 2 -->").unwrap();
        let seven = body.find("<!-- slice 7 -->").unwrap();
        let nine = body.find("<!-- slice 9 -->").unwrap();
        assert!(two < seven && seven < nine, "{body}");
        assert_eq!(
            outcome.writes[1]["body"]["body"],
            format!("Findings in slice 2: {RUN_URL}")
        );
    }
}

#[test]
fn a_failed_timed_out_or_unset_up_pass_is_reported() {
    for (status, outcome) in [
        (Some("2"), "exit 2"),
        (
            Some("started"),
            "no exit status: the slice timed out or was cancelled",
        ),
        (None, "no exit status: the slice timed out or was cancelled"),
        (Some("setup-failed"), "its set-up failed"),
    ] {
        let logs = clean_logs("1");
        fs::remove_dir_all(logs.path().join("full-run-linux-1")).unwrap();
        platform(logs.path(), "linux", "1", "baseline failed\n", status);
        let result = report(logs.path(), &one(1), "idle", json!({ ISSUES: [] }));
        assert_eq!(result.writes.len(), 1, "{status:?}: {}", result.stdout);
        let expected = format!("### Linux: no findings, {outcome}\n\nSee the run's log.\n");
        assert!(
            section(written(&result, 0), "1").contains(&expected),
            "{}",
            written(&result, 0)
        );
    }
}

/// The section of slice 3 from an earlier pass: Linux and macOS had findings.
fn earlier() -> String {
    "<!-- slice 3 -->\n## Slice 3 of 10, at 1111\n\nold-run\n\n\
         <!-- platform linux -->\n### Linux: 1 findings, exit 3\n\n\
         ```text\nSURVIVED  old.rs:1 + -> -\n```\n<!-- end platform linux -->\n\n\
         <!-- platform windows -->\n### Windows: no findings\n<!-- end platform windows -->\n\n\
         <!-- platform macos -->\n\
         > Did not run on 2026-11-01: it is not in CLIMON2_OWN_RUNNERS.\n\n\
         ### macOS: 1 findings, exit 3\n\n\
         ```text\nSURVIVED  mac.rs:2 + -> -\n```\n<!-- end platform macos -->\n\
         <!-- end slice 3 -->\n"
        .to_owned()
}

#[test]
fn a_platform_that_did_not_run_keeps_its_earlier_result_with_a_dated_note() {
    // Linux ran clean; Windows is listed but left no log; macOS is not listed.
    let logs = tempfile::tempdir().unwrap();
    platform(logs.path(), "linux", "3", KILLED, Some("0"));
    let outcome = report(logs.path(), &one(3), "idle", tracker(&earlier()));
    let body = written(&outcome, 0);
    let section = section(body, "3");
    assert!(section.starts_with(&format!("<!-- slice 3 -->\n## Slice 3 of 10, at {SHA}\n")));
    assert_eq!(
        block(section, "platform", "linux"),
        "<!-- platform linux -->\n### Linux: no findings\n<!-- end platform linux -->\n"
    );
    assert_eq!(
        block(section, "platform", "windows"),
        format!(
            "<!-- platform windows -->\n\
             > Did not run on {DATE}: its job left no log ({RUN_URL}).\n\n\
             ### Windows: no findings\n<!-- end platform windows -->\n"
        )
    );
    // The earlier note is replaced, not stacked.
    assert_eq!(
        block(section, "platform", "macos"),
        format!(
            "<!-- platform macos -->\n\
             > Did not run on {DATE}: it is not in CLIMON2_OWN_RUNNERS.\n\n\
             ### macOS: 1 findings, exit 3\n\n\
             ```text\nSURVIVED  mac.rs:2 + -> -\n```\n<!-- end platform macos -->\n"
        )
    );
    // A listed platform that left no log needs the owner, so it comments.
    assert_eq!(outcome.writes.len(), 2, "{:#?}", outcome.writes);
    assert_eq!(
        outcome.writes[1]["body"]["body"],
        format!("Findings in slice 3: {RUN_URL}")
    );
}

#[test]
fn a_skipped_slice_keeps_its_earlier_findings_with_a_dated_note() {
    let empty = tempfile::tempdir().unwrap();
    let busy = report(empty.path(), &one(3), "busy", tracker(&earlier()));
    assert_eq!(busy.writes.len(), 1, "no comment: {:#?}", busy.writes);
    let section = section(written(&busy, 0), "3");
    let note = format!("> Skipped on {DATE}: gates were busy ({RUN_URL}).\n\n");
    assert_eq!(
        section,
        format!(
            "<!-- slice 3 -->\n{note}{}",
            &earlier()["<!-- slice 3 -->\n".len()..]
        )
    );
    assert!(section.contains("SURVIVED  old.rs:1"), "{section}");
    assert_eq!(
        busy.stdout,
        format!(
            "Slice 3 skipped: gates were busy; https://github.com/{REPO}/issues/9 is updated.\n"
        )
    );
    // Skipped again: the newer note replaces the older one.
    let again = report(empty.path(), &one(3), "busy", tracker(written(&busy, 0)));
    assert_eq!(section_count(written(&again, 0), "> Skipped on "), 1);
    // With no issue open, a busy skip opens none.
    let none = report(empty.path(), &one(6), "busy", json!({ ISSUES: [] }));
    assert!(none.writes.is_empty(), "{:#?}", none.writes);
    assert_eq!(none.stdout, "Slice 6 skipped: gates were busy.\n");
}

fn section_count(text: &str, needle: &str) -> usize {
    text.matches(needle).count()
}

#[test]
fn a_failed_wait_is_told_apart_from_busy_gates_and_reported() {
    let empty = tempfile::tempdir().unwrap();
    let failed = report(empty.path(), &one(6), "failed", json!({ ISSUES: [] }));
    assert_eq!(failed.writes.len(), 1, "{:#?}", failed.writes);
    assert_eq!(
        section(written(&failed, 0), "6"),
        format!(
            "<!-- slice 6 -->\n## Slice 6 of 10\n\n\
             > Skipped on {DATE}: a job that waits for the gates failed ({RUN_URL}).\n\
             <!-- end slice 6 -->\n"
        )
    );
    assert_eq!(
        failed.stdout,
        format!(
            "Slice 6 skipped: a job that waits for the gates failed; \
             https://github.com/repos/{REPO}/issues is opened.\n"
        )
    );
}

#[test]
fn a_clean_slice_clears_its_section_without_a_comment_and_opens_no_issue() {
    let clean = report(
        clean_logs("3").path(),
        &one(3),
        "idle",
        json!({ ISSUES: [] }),
    );
    assert!(clean.writes.is_empty(), "{:#?}", clean.writes);
    assert_eq!(clean.stdout, "No findings in slice 3.\n");
    let open = report(clean_logs("3").path(), &one(3), "idle", tracker(&earlier()));
    assert_eq!(open.writes.len(), 1, "{:#?}", open.writes);
    assert_eq!(open.writes[0]["method"], "PATCH");
    let body = written(&open, 0);
    assert!(!body.contains("old.rs"), "{body}");
    assert_eq!(
        open.stdout,
        format!("No findings in slice 3; https://github.com/{REPO}/issues/9 is updated.\n")
    );
}

#[test]
fn a_run_of_all_slices_writes_each_slices_section() {
    let logs = tempfile::tempdir().unwrap();
    for slice in 0..10 {
        let slice = slice.to_string();
        let log = if slice == "5" {
            "SURVIVED  crates/climon2-core/src/wire.rs:42 == -> !=\n"
        } else {
            KILLED
        };
        let status = if slice == "5" { "3" } else { "0" };
        platform(logs.path(), "linux", &slice, log, Some(status));
        platform(logs.path(), "windows", &slice, KILLED, Some("0"));
    }
    let all = r#"["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]"#;
    let outcome = report(logs.path(), all, "idle", json!({ ISSUES: [] }));
    let body = written(&outcome, 0);
    for slice in 0..10 {
        let text = section(body, &slice.to_string());
        assert_eq!(text.contains("SURVIVED"), slice == 5, "{text}");
    }
    assert_eq!(
        outcome.stdout,
        format!("Findings in slice 5; https://github.com/repos/{REPO}/issues is opened.\n")
    );
}

#[test]
fn a_long_list_is_cut_short_inside_its_section() {
    let logs = clean_logs("8");
    fs::remove_dir_all(logs.path().join("full-run-linux-8")).unwrap();
    let log: String = (0..2000)
        .map(|line| format!("SURVIVED  crates/climon2-core/src/wire.rs:{line} == -> !=\n"))
        .collect();
    platform(logs.path(), "linux", "8", &log, Some("3"));
    let outcome = report(logs.path(), &one(8), "idle", json!({ ISSUES: [] }));
    let text = section(written(&outcome, 0), "8");
    // Ten such sections fit in GitHub's 65,536-character limit.
    assert!(text.len() < 6000, "{}", text.len());
    // The text block is closed before the note of what was left out.
    let note = text
        .find("\n```\n\nAnd ")
        .expect("a closed block, then the note");
    assert!(
        text[note..].contains(&format!(" more in {RUN_URL}.\n")),
        "{text}"
    );
}

#[test]
fn an_api_error_is_reported_without_failing_the_job() {
    let logs = logs("5");
    let outcome = report(logs.path(), &one(5), "idle", json!({}));
    assert!(outcome.writes.is_empty());
    assert!(
        outcome.stdout.starts_with(
            "Findings in slice 5, but could not update the tracking issue: no response for "
        ),
        "{}",
        outcome.stdout
    );
}
