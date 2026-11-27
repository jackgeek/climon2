//! #452: `tools/quality-full-run.py` chooses the slice of shipped files each
//! squash-merge to main mutates, and waits for idle gates before a slice starts.
//! `quality_full_report.rs` tests its reports.

#[path = "support/full_run.rs"]
mod full_run;

use std::{fs, process::Command};

use full_run::{REPO, one, script};
use serde_json::{Value, json};

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
        let (slices, _, message) = self.choose(event, requested, before, after);
        (slices, message)
    }

    /// The slices, the commit whose quality.yml run they wait for, and the message.
    fn choose(
        &self,
        event: &str,
        requested: &str,
        before: &str,
        after: &str,
    ) -> (String, String, String) {
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
        let (slices, gate) = outcome
            .output
            .strip_prefix("slices=")
            .and_then(|rest| rest.strip_suffix('\n'))
            .and_then(|rest| rest.split_once("\ngate="))
            .unwrap_or_else(|| panic!("unexpected output {:?}", outcome.output));
        (slices.to_owned(), gate.to_owned(), outcome.stdout)
    }

    /// Commits a change to each of `paths` and returns the commit.
    fn change(&self, paths: &[&str], subject: &str) -> String {
        for path in paths {
            let file = self.dir.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, subject).unwrap();
            self.git(&["add", path]);
        }
        self.commit(subject)
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

/// #457: a force push may lower the count, so when the old head is not in the new
/// history, only the commits the push brought count as added.
#[test]
fn a_force_push_counts_only_the_commits_it_brought() {
    let history = History::new();
    let eighth = history.squash(1..=8);
    let old = history.squash(9..=12);
    history.git(&["reset", "--quiet", "--hard", &eighth]);
    let replaced = history.squash(13..=13);
    let (slices, message) = history.slice("push", "", &old, &replaced);
    assert_eq!(slices, one(9), "{message}");
    assert_eq!(
        message,
        "Full mutation pass mutates slice 9: main has 9 squash-merges, so slice 9 of 10.\n"
    );
    // A force push that brings only a direct push adds none.
    history.git(&["reset", "--quiet", "--hard", &eighth]);
    let direct = history.commit("Fix a typo directly on main");
    let (slices, message) = history.slice("push", "", &old, &direct);
    assert_eq!(slices, "", "{message}");
    assert_eq!(
        message,
        "Full mutation pass skipped: the push adds no squash-merge; main still has 8.\n"
    );
    // A force push back to an older head brings no commit.
    assert_eq!(history.push(&old, &eighth), "");
}

/// #457: the count goes by subject alone, so a direct push whose subject ends
/// with a number in brackets, as a squash-merge's does, counts as one.
#[test]
fn a_direct_push_whose_subject_ends_like_a_squash_merge_counts() {
    let history = History::new();
    let before = history.squash(1..=4);
    let after = history.commit("Fix the typo that #12 left (#12)");
    assert_eq!(history.push(&before, &after), one(5));
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

#[test]
fn a_slice_waits_for_mains_own_gate_only_when_the_push_runs_one() {
    let history = History::new();
    let before = history.git(&["rev-parse", "HEAD"]);
    let code = history.change(&["crates/a/src/lib.rs"], "Change code (#1)");
    let (_, gate, _) = history.choose("push", "", &before, &code);
    assert_eq!(gate, code);
    // quality.yml ignores Markdown, except the protocol's, and LICENSE.
    let docs = history.change(&["README.md", "docs/a/b.md", "LICENSE"], "Docs (#2)");
    assert_eq!(history.choose("push", "", &code, &docs).1, "");
    let protocol = history.change(&["docs/for-humans/protocol/x/y.md"], "Protocol (#3)");
    assert_eq!(history.choose("push", "", &docs, &protocol).1, protocol);
    // An unknown old head compares with the pushed commit's parent.
    let zero = "0".repeat(40);
    let readme = history.change(&["README.md"], "Readme (#4)");
    assert_eq!(history.choose("push", "", &zero, &readme).1, "");
    // A manual run has no push of its own to wait for, and no slice none.
    assert_eq!(history.choose("workflow_dispatch", "3", "", &readme).1, "");
    let direct = history.change(&["crates/a/src/lib.rs"], "Direct push");
    assert_eq!(
        history.choose("push", "", &readme, &direct),
        (
            String::new(),
            String::new(),
            "Full mutation pass skipped: the push adds no squash-merge; main still has 4.\n"
                .to_owned()
        )
    );
}

const RUN_ID: u64 = 500;
const GATE: &str = "c0ffee0000000000000000000000000000000000";

fn gate_runs() -> String {
    format!("repos/{REPO}/actions/workflows/quality.yml/runs?per_page=100")
}

fn full_runs() -> String {
    format!("repos/{REPO}/actions/workflows/quality-full.yml/runs?per_page=100")
}

/// The latest runs: quality.yml runs as (status, commit), and quality-full.yml runs as
/// (id, status). Without `gates`, a look-up of quality.yml fails.
fn runs(gates: Option<&[(&str, &str)]>, full: &[(u64, &str)]) -> Value {
    let full: Vec<_> = full
        .iter()
        .map(|(id, status)| json!({"id": id, "status": status}))
        .collect();
    let mut responses = json!({ full_runs(): {"workflow_runs": full} });
    if let Some(gates) = gates {
        let gates: Vec<_> = gates
            .iter()
            .map(|(status, sha)| json!({"status": status, "head_sha": sha}))
            .collect();
        responses[gate_runs()] = json!({ "workflow_runs": gates });
    }
    responses
}

/// The wait's output and message, with no time to wait, for main's own run of `gate`.
fn wait(gate: &str, responses: Value) -> (String, String) {
    let dir = tempfile::tempdir().unwrap();
    let id = RUN_ID.to_string();
    let args = [
        "wait",
        "--repo",
        REPO,
        "--run-id",
        &id,
        "--gate",
        gate,
        "--interval",
        "60",
        "--limit",
        "0",
    ];
    let outcome = script(dir.path(), &args, Some(responses));
    assert_eq!(outcome.summary, format!("- {}", outcome.stdout));
    (outcome.output, outcome.stdout)
}

fn idle() -> (String, String) {
    (
        "idle=true\n".to_owned(),
        "Gates idle: no quality.yml or older full run was unfinished after 0 min.\n".to_owned(),
    )
}

fn still_busy(why: &str, every: u32) -> (String, String) {
    (
        "idle=false\n".to_owned(),
        format!("Gates still busy: {why} after 0 min, checking every {every} min.\n"),
    )
}

#[test]
fn a_slice_starts_only_once_no_gate_or_older_full_run_is_unfinished() {
    let done = [("completed", GATE)];
    // This run itself, newer full runs and finished ones never hold it back, so two
    // full runs never wait on each other.
    let full = [
        (RUN_ID, "in_progress"),
        (RUN_ID + 1, "queued"),
        (RUN_ID - 2, "completed"),
    ];
    assert_eq!(wait(GATE, runs(Some(&done), &full)), idle());
    // An unfinished gate holds it; each other full run waiting too stretches the checks.
    assert_eq!(
        wait(
            GATE,
            runs(Some(&[("in_progress", "aaaa"), ("completed", GATE)]), &full)
        ),
        still_busy("1 quality.yml runs unfinished", 2)
    );
    // An older full run, such as a manual run of all slices, holds the machines, and
    // then quality.yml is not even looked up.
    assert_eq!(
        wait(
            GATE,
            runs(None, &[(RUN_ID - 1, "waiting"), (RUN_ID, "in_progress")])
        ),
        still_busy("1 older quality-full.yml runs unfinished", 2)
    );
    // An API that fails counts as busy, so a slice never starts while gates may wait.
    let (output, message) = wait(GATE, json!({}));
    assert_eq!(output, "idle=false\n");
    assert!(
        message.starts_with("Gates still busy: could not look up unfinished runs: "),
        "{message}"
    );
}

#[test]
fn a_slice_waits_until_mains_own_gate_run_has_appeared() {
    // Right after the merge, main's own quality.yml run may not be queued yet.
    let other = [("completed", "bbbb")];
    assert_eq!(
        wait(GATE, runs(Some(&other), &[])),
        still_busy(
            &format!("main's own quality.yml run for {GATE} has not appeared yet"),
            1
        )
    );
    // Once it has run, or when the push runs none, the gates are idle.
    assert_eq!(wait(GATE, runs(Some(&[("completed", GATE)]), &[])), idle());
    assert_eq!(wait("", runs(Some(&other), &[])), idle());
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
