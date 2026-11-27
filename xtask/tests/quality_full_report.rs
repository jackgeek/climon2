//! #452, #457: `tools/quality-full-run.py` reports each slice's findings in that
//! slice's own comment on one tracking issue, so reports of different slices
//! never rewrite the same text.

#[path = "support/full_run.rs"]
mod full_run;

use std::{fs, path::Path};

use full_run::{Outcome, REPO, one, script};
use serde_json::{Value, json};

const RUN_URL: &str = "https://github.com/owner/repo/actions/runs/7";
const SHA: &str = "c0d283ea5485b3ec494c9909d1689ec9bfd6cf03";
const ISSUES: &str =
    "repos/owner/repo/issues?state=open&creator=github-actions%5Bbot%5D&per_page=100&page=1";
const BOT: &str = "github-actions[bot]";
/// The open tracking issue's number in the tests.
const TRACKER: u32 = 9;
/// The number the fixtures give an issue the report opens.
const OPENED: u32 = 1;

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
    report_for(OWN, logs, slices, gates, responses)
}

/// A report while CLIMON2_OWN_RUNNERS is `own`.
fn report_for(own: &str, logs: &Path, slices: &str, gates: &str, responses: Value) -> Outcome {
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
        own,
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

fn issue(number: u32, title: &str, login: &str) -> Value {
    json!({
        "number": number,
        "title": title,
        "user": {"login": login},
        "body": "A fixed header.",
        "html_url": format!("https://github.com/{REPO}/issues/{number}"),
    })
}

fn comments_path(page: u32) -> String {
    format!("repos/{REPO}/issues/{TRACKER}/comments?per_page=100&page={page}")
}

fn comment(id: u64, login: &str, body: &str) -> Value {
    json!({"id": id, "user": {"login": login}, "body": body})
}

/// The open tracking issue, with `comments` on it.
fn tracker(comments: Vec<Value>) -> Value {
    json!({
        ISSUES: [issue(TRACKER, "Full mutation pass findings", BOT)],
        comments_path(1): comments,
    })
}

/// No tracking issue is open.
fn untracked() -> Value {
    json!({ ISSUES: [] })
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

/// The body of write `index`.
fn written(outcome: &Outcome, index: usize) -> &str {
    outcome.writes[index]["body"]["body"].as_str().unwrap()
}

/// Each write as "METHOD path".
fn calls(outcome: &Outcome) -> Vec<String> {
    outcome
        .writes
        .iter()
        .map(|write| {
            format!(
                "{} {}",
                write["method"].as_str().unwrap(),
                write["path"].as_str().unwrap()
            )
        })
        .collect()
}

fn patch(id: u64) -> String {
    format!("PATCH repos/{REPO}/issues/comments/{id}")
}

fn post_comment(number: u32) -> String {
    format!("POST repos/{REPO}/issues/{number}/comments")
}

fn open_issue() -> String {
    format!("POST repos/{REPO}/issues")
}

#[test]
fn findings_open_the_tracking_issue_with_a_header_and_the_slices_comment() {
    let logs = logs("4");
    let outcome = report(
        logs.path(),
        &one(4),
        "idle",
        json!({ ISSUES: [
            // Same title, but a person's issue, and a pull request: neither is the tracker.
            issue(5, "Full mutation pass findings", "someone"),
            {
                "number": 6, "title": "Full mutation pass findings",
                "user": {"login": BOT}, "pull_request": {},
            },
        ]}),
    );
    assert_eq!(calls(&outcome), [open_issue(), post_comment(OPENED)]);
    assert_eq!(
        outcome.writes[0]["body"]["title"],
        "Full mutation pass findings"
    );
    // The issue's body is a fixed header; the findings are in the slice's comment.
    let header = written(&outcome, 0);
    assert!(
        header.starts_with("Each squash-merge to main mutates one slice"),
        "{header}"
    );
    assert!(!header.contains("<!--"), "{header}");
    let expected = format!(
        "<!-- slice 4 -->\n## Slice 4 of 10, at {SHA}\n\n{RUN_URL}\n\n\
         <!-- platform linux -->\n### Linux: 3 findings, exit 3\n\n```text\n\
         CRAP parse climon2_core::wire 6 40.0% 14.2\n\
         SURVIVED  crates/climon2-core/src/wire.rs:42 == -> !=\n\
         UNCOVERED crates/climon2-core/src/wire.rs:57 true -> false\n```\n\
         <!-- end platform linux -->\n\n\
         <!-- platform windows -->\n### Windows: no findings\n<!-- end platform windows -->\n\n\
         <!-- platform macos -->\n\
         > Did not run on {DATE}: it is not in CLIMON2_OWN_RUNNERS.\n\n### macOS\n\
         <!-- end platform macos -->\n<!-- end slice 4 -->\n"
    );
    assert_eq!(written(&outcome, 1), expected);
    assert_eq!(
        outcome.stdout,
        format!("Findings in slice 4; https://github.com/repos/{REPO}/issues is opened.\n")
    );
}

/// A slice's comment as GitHub returns it, with `line_end` ends, and with or without its
/// final newline, as a comment edited in the browser may come back.
fn old_comment(slice: &str, content: &str, line_end: &str, final_newline: bool) -> String {
    let body = format!("<!-- slice {slice} -->\n{content}\n<!-- end slice {slice} -->\n");
    let body = if final_newline {
        &body
    } else {
        body.trim_end()
    };
    body.replace('\n', line_end)
}

/// Each slice owns one comment: a report rewrites only its own slice's comment,
/// whatever the line ends, and never another slice's, a person's comment, or the
/// issue's body. Findings add a short comment, which notifies the owner.
#[test]
fn findings_rewrite_only_their_slices_comment() {
    for (line_end, final_newline) in [("\n", true), ("\r\n", true), ("\r\n", false)] {
        let logs = logs("2");
        let outcome = report(
            logs.path(),
            &one(2),
            "idle",
            tracker(vec![
                comment(21, BOT, &old_comment("9", "nine", line_end, final_newline)),
                comment(
                    23,
                    "someone",
                    "<!-- slice 2 -->\nquoted\n<!-- end slice 2 -->\n",
                ),
                comment(
                    22,
                    BOT,
                    &old_comment("2", "stale two", line_end, final_newline),
                ),
                comment(24, BOT, &format!("Findings in slice 2: {RUN_URL}")),
            ]),
        );
        assert_eq!(
            calls(&outcome),
            [patch(22), post_comment(TRACKER)],
            "{line_end:?} {final_newline}"
        );
        let body = written(&outcome, 0);
        assert!(!body.contains("stale two"), "{line_end:?}: {body}");
        assert_eq!(block(body, "slice", "2"), body);
        assert!(body.contains("SURVIVED  crates/climon2-core/src/wire.rs:42"));
        assert_eq!(
            written(&outcome, 1),
            format!("Findings in slice 2: {RUN_URL}")
        );
        assert_eq!(
            outcome.stdout,
            format!(
                "Findings in slice 2; https://github.com/{REPO}/issues/{TRACKER} is updated.\n"
            )
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
        let result = report(logs.path(), &one(1), "idle", untracked());
        assert_eq!(
            calls(&result),
            [open_issue(), post_comment(OPENED)],
            "{status:?}"
        );
        let expected = format!("### Linux: no findings, {outcome}\n\nSee the run's log.\n");
        assert!(
            written(&result, 1).contains(&expected),
            "{}",
            written(&result, 1)
        );
    }
}

/// The comment of slice 3 from an earlier pass: Linux and macOS had findings.
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

/// The tracking issue with slice 3's earlier comment, number 30, on it.
fn tracked_three(body: &str) -> Value {
    tracker(vec![comment(30, BOT, body)])
}

#[test]
fn a_platform_that_did_not_run_keeps_its_earlier_result_with_a_dated_note() {
    // Linux ran clean; Windows is listed but left no log; macOS is not listed.
    let logs = tempfile::tempdir().unwrap();
    platform(logs.path(), "linux", "3", KILLED, Some("0"));
    let outcome = report(logs.path(), &one(3), "idle", tracked_three(&earlier()));
    // A listed platform that left no log needs the owner, so it comments.
    assert_eq!(calls(&outcome), [patch(30), post_comment(TRACKER)]);
    let section = written(&outcome, 0);
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
    assert_eq!(
        written(&outcome, 1),
        format!("Findings in slice 3: {RUN_URL}")
    );
}

#[test]
fn a_skipped_slice_keeps_its_earlier_findings_with_a_dated_note() {
    let empty = tempfile::tempdir().unwrap();
    let busy = report(empty.path(), &one(3), "busy", tracked_three(&earlier()));
    assert_eq!(calls(&busy), [patch(30)], "no notice");
    let note = format!("> Skipped on {DATE}: gates were busy ({RUN_URL}).\n\n");
    assert_eq!(
        written(&busy, 0),
        format!(
            "<!-- slice 3 -->\n{note}{}",
            &earlier()["<!-- slice 3 -->\n".len()..]
        )
    );
    assert_eq!(
        busy.stdout,
        format!(
            "Slice 3 skipped: gates were busy; https://github.com/{REPO}/issues/{TRACKER} is updated.\n"
        )
    );
    // Skipped again: the newer note replaces the older one.
    let again = report(
        empty.path(),
        &one(3),
        "busy",
        tracked_three(written(&busy, 0)),
    );
    assert_eq!(written(&again, 0).matches("> Skipped on ").count(), 1);
    // With no issue open, a busy skip opens none.
    let none = report(empty.path(), &one(6), "busy", untracked());
    assert!(none.writes.is_empty(), "{:#?}", none.writes);
    assert_eq!(none.stdout, "Slice 6 skipped: gates were busy.\n");
}

#[test]
fn a_failed_wait_is_told_apart_from_busy_gates_and_reported() {
    let empty = tempfile::tempdir().unwrap();
    let failed = report(empty.path(), &one(6), "failed", untracked());
    assert_eq!(calls(&failed), [open_issue(), post_comment(OPENED)]);
    assert_eq!(
        written(&failed, 1),
        format!(
            "<!-- slice 6 -->\n\
             > Skipped on {DATE}: a job that waits for the gates failed ({RUN_URL}).\n\n\
             ## Slice 6 of 10\n<!-- end slice 6 -->\n"
        )
    );
    // Skipped again for busy gates: the note comes first, so the newer one replaces it.
    let again = report(
        empty.path(),
        &one(6),
        "busy",
        tracker(vec![comment(60, BOT, written(&failed, 1))]),
    );
    assert_eq!(calls(&again), [patch(60)]);
    assert_eq!(
        written(&again, 0),
        format!(
            "<!-- slice 6 -->\n> Skipped on {DATE}: gates were busy ({RUN_URL}).\n\n\
             ## Slice 6 of 10\n<!-- end slice 6 -->\n"
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

/// A clean pass opens no issue; on an open one it rewrites its slice's comment
/// quietly, or adds it when the slice has none yet, with no notice.
#[test]
fn a_clean_slice_clears_its_comment_quietly_and_opens_no_issue() {
    let clean = report(clean_logs("3").path(), &one(3), "idle", untracked());
    assert!(clean.writes.is_empty(), "{:#?}", clean.writes);
    assert_eq!(clean.stdout, "No findings in slice 3.\n");
    let open = report(
        clean_logs("3").path(),
        &one(3),
        "idle",
        tracked_three(&earlier()),
    );
    assert_eq!(calls(&open), [patch(30)]);
    assert!(
        !written(&open, 0).contains("old.rs"),
        "{}",
        written(&open, 0)
    );
    assert_eq!(
        open.stdout,
        format!("No findings in slice 3; https://github.com/{REPO}/issues/{TRACKER} is updated.\n")
    );
    let first = report(clean_logs("3").path(), &one(3), "idle", tracker(vec![]));
    assert_eq!(calls(&first), [post_comment(TRACKER)]);
    assert!(written(&first, 0).starts_with("<!-- slice 3 -->\n"));
}

#[test]
fn a_run_of_all_slices_writes_each_slices_comment() {
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
    let outcome = report(logs.path(), all, "idle", untracked());
    assert_eq!(outcome.writes.len(), 11, "{:#?}", calls(&outcome));
    assert_eq!(calls(&outcome)[0], open_issue());
    for slice in 0..10 {
        let text = written(&outcome, slice + 1);
        assert_eq!(block(text, "slice", &slice.to_string()), text);
        assert_eq!(text.contains("SURVIVED"), slice == 5, "{text}");
    }
    assert_eq!(
        outcome.stdout,
        format!("Findings in slice 5; https://github.com/repos/{REPO}/issues is opened.\n")
    );
}

#[test]
fn a_long_list_is_cut_short_inside_its_comment() {
    let logs = clean_logs("8");
    fs::remove_dir_all(logs.path().join("full-run-linux-8")).unwrap();
    let log: String = (0..4000)
        .map(|line| format!("SURVIVED  crates/climon2-core/src/wire.rs:{line} == -> !=\n"))
        .collect();
    platform(logs.path(), "linux", "8", &log, Some("3"));
    let outcome = report(logs.path(), &one(8), "idle", untracked());
    let text = written(&outcome, 1);
    // It fits in GitHub's 65,536-character limit for a comment.
    assert!(text.len() < 65_536, "{}", text.len());
    // The text block is closed before the note of what was left out.
    let note = text
        .find("\n```\n\nAnd ")
        .expect("a closed block, then the note");
    assert!(
        text[note..].contains(&format!(" more in {RUN_URL}.\n")),
        "{text}"
    );
    // #457: every fence is balanced, and the block is cut at a line boundary.
    assert_eq!(text.matches("```").count() % 2, 0, "{text}");
    let block = &text[text.find("```text\n").unwrap() + "```text\n".len()..note];
    assert!(
        block
            .lines()
            .all(|line| line.starts_with("SURVIVED  ") && line.ends_with(" == -> !=")),
        "{block}"
    );
}

/// #457: when CLIMON2_OWN_RUNNERS lists no machine, no pass ran, so the report
/// says so rather than that it found nothing.
#[test]
fn a_report_with_no_machine_listed_says_no_pass_ran() {
    let empty = tempfile::tempdir().unwrap();
    let outcome = report_for("", empty.path(), &one(3), "idle", tracked_three(&earlier()));
    assert_eq!(
        outcome.stdout,
        format!(
            "No pass of slice 3 ran: CLIMON2_OWN_RUNNERS lists no machine; \
             https://github.com/{REPO}/issues/{TRACKER} is updated.\n"
        )
    );
    // Each machine keeps its earlier result, with a note of why it did not run.
    assert_eq!(calls(&outcome), [patch(30)], "no notice");
    let section = written(&outcome, 0);
    assert_eq!(
        section.matches("it is not in CLIMON2_OWN_RUNNERS.").count(),
        3,
        "{section}"
    );
    assert!(section.contains("SURVIVED  old.rs:1"), "{section}");
    let none = report_for("", empty.path(), &one(3), "idle", untracked());
    assert!(none.writes.is_empty(), "{:#?}", none.writes);
    assert_eq!(
        none.stdout,
        "No pass of slice 3 ran: CLIMON2_OWN_RUNNERS lists no machine.\n"
    );
}

/// #457: the tracking issue, and a slice's comment, are found on any page.
#[test]
fn the_issue_and_the_slices_comment_are_found_past_the_first_page() {
    let page = |number: u32| ISSUES.replace("&page=1", &format!("&page={number}"));
    let others: Vec<_> = (100..200)
        .map(|number| issue(number, "Another bot issue", BOT))
        .collect();
    let notices: Vec<_> = (100..200)
        .map(|id| comment(id, BOT, &format!("Findings in slice 3: {RUN_URL}")))
        .collect();
    let tracked = json!({
        page(1): others.clone(),
        page(2): [issue(TRACKER, "Full mutation pass findings", BOT)],
        comments_path(1): notices,
        comments_path(2): [comment(30, BOT, &earlier())],
    });
    let outcome = report(clean_logs("3").path(), &one(3), "idle", tracked);
    assert_eq!(calls(&outcome), [patch(30)]);
    // A full page asks for the next; a short one is the last, so none after it is asked for.
    let full_page = json!({ page(1): others, page(2): [] });
    let outcome = report(logs("3").path(), &one(3), "idle", full_page);
    assert_eq!(calls(&outcome), [open_issue(), post_comment(OPENED)]);
}

#[test]
fn an_api_error_is_reported_without_failing_the_job() {
    let logs = logs("5");
    for responses in [
        json!({}),
        json!({ ISSUES: [issue(TRACKER, "Full mutation pass findings", BOT)] }),
    ] {
        let outcome = report(logs.path(), &one(5), "idle", responses);
        assert!(outcome.writes.is_empty());
        assert!(
            outcome.stdout.starts_with(
                "Findings in slice 5, but could not update the tracking issue: no response for "
            ),
            "{}",
            outcome.stdout
        );
    }
}
