//! #452: `tools/quality-full-run.py` reports each slice's findings in its section
//! of one tracking issue.

#[path = "support/full_run.rs"]
mod full_run;

use std::{fs, path::Path};

use full_run::{Outcome, REPO, one, script};
use serde_json::{Value, json};

const RUN_URL: &str = "https://github.com/owner/repo/actions/runs/7";
const SHA: &str = "c0d283ea5485b3ec494c9909d1689ec9bfd6cf03";
const ISSUES: &str =
    "repos/owner/repo/issues?state=open&creator=github-actions%5Bbot%5D&per_page=100&page=1";

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
         <!-- platform macos -->\n\
         > Did not run on {DATE}: it is not in CLIMON2_OWN_RUNNERS.\n\n### macOS\n\
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
            "<!-- slice 6 -->\n\
             > Skipped on {DATE}: a job that waits for the gates failed ({RUN_URL}).\n\n\
             ## Slice 6 of 10\n<!-- end slice 6 -->\n"
        )
    );
    // Skipped again for busy gates: the note comes first, so the newer one replaces it.
    let again = report(empty.path(), &one(6), "busy", tracker(written(&failed, 0)));
    assert_eq!(
        section(written(&again, 0), "6"),
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
    let outcome = report_for("", empty.path(), &one(3), "idle", tracker(&earlier()));
    assert_eq!(
        outcome.stdout,
        format!(
            "No pass of slice 3 ran: CLIMON2_OWN_RUNNERS lists no machine; \
             https://github.com/{REPO}/issues/9 is updated.\n"
        )
    );
    // Each machine keeps its earlier result, with a note of why it did not run.
    let section = section(written(&outcome, 0), "3");
    assert_eq!(
        section_count(section, "it is not in CLIMON2_OWN_RUNNERS."),
        3,
        "{section}"
    );
    assert!(section.contains("SURVIVED  old.rs:1"), "{section}");
    assert_eq!(outcome.writes.len(), 1, "no comment: {:#?}", outcome.writes);
    let none = report_for("", empty.path(), &one(3), "idle", json!({ ISSUES: [] }));
    assert!(none.writes.is_empty(), "{:#?}", none.writes);
    assert_eq!(
        none.stdout,
        "No pass of slice 3 ran: CLIMON2_OWN_RUNNERS lists no machine.\n"
    );
}

/// #457: the tracking issue is found on any page of the bot's open issues.
#[test]
fn the_tracking_issue_is_found_past_the_first_page() {
    let page = |number: u32| ISSUES.replace("&page=1", &format!("&page={number}"));
    let others: Vec<_> = (100..200)
        .map(|number| issue(number, "Another bot issue", "github-actions[bot]", ""))
        .collect();
    let tracked = json!({
        page(1): others.clone(),
        page(2): [issue(9, "Full mutation pass findings", "github-actions[bot]", &earlier())],
    });
    let outcome = report(clean_logs("3").path(), &one(3), "idle", tracked);
    assert_eq!(outcome.writes.len(), 1, "{:#?}", outcome.writes);
    assert_eq!(outcome.writes[0]["method"], "PATCH");
    assert_eq!(outcome.writes[0]["path"], format!("repos/{REPO}/issues/9"));
    // A full page asks for the next; a short one is the last, so none after it is asked for.
    let full_page = json!({ page(1): others, page(2): [] });
    let outcome = report(logs("3").path(), &one(3), "idle", full_page);
    assert_eq!(outcome.writes.len(), 1, "{:#?}", outcome.writes);
    assert_eq!(outcome.writes[0]["method"], "POST");
    assert_eq!(outcome.writes[0]["path"], format!("repos/{REPO}/issues"));
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
