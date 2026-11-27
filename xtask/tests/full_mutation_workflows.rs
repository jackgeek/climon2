//! #452: the rolling full mutation pass's workflows. `quality-full.yml` chooses
//! the slices on each push to main and calls `quality-slice.yml` once per slice,
//! one at a time; each slice waits on hosted runners for idle gates, runs on the
//! own-hardware machines without ever turning main red, and reports to one
//! tracking issue.

use std::{fs, path::Path};

fn workflow(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.github/workflows")
        .join(name);
    // A Windows checkout may turn each line ending into CRLF.
    fs::read_to_string(path)
        .expect("read workflow")
        .replace("\r\n", "\n")
}

fn full() -> String {
    workflow("quality-full.yml")
}

fn slice() -> String {
    workflow("quality-slice.yml")
}

/// The top-level `on:` block's trigger names.
fn triggers(text: &str) -> Vec<&str> {
    let block = text.split("\non:\n").nth(1).expect("an on: block");
    block
        .lines()
        .take_while(|line| line.is_empty() || line.starts_with(' '))
        .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
        .map(|line| line.trim().trim_end_matches(':'))
        .collect()
}

/// The names of the workflow's jobs, in order.
fn jobs(text: &str) -> Vec<&str> {
    let block = text.split("\njobs:\n").nth(1).expect("a jobs: block");
    block
        .lines()
        .filter(|line| line.starts_with("  ") && !line.starts_with("   ") && line.ends_with(':'))
        .map(|line| line.trim().trim_end_matches(':'))
        .collect()
}

/// The job named `name`'s lines, up to the next job.
fn job<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let start = format!("\n  {name}:\n");
    let body = text.split(&start).nth(1).expect("the job");
    body.lines()
        .take_while(|line| line.is_empty() || line.starts_with("    "))
        .collect()
}

/// The job's `if:`, on one line or folded.
fn condition(lines: &[&str]) -> String {
    let Some(text) = lines.iter().find_map(|line| line.strip_prefix("    if: ")) else {
        return String::new();
    };
    if text != ">-" {
        return text.to_owned();
    }
    lines
        .iter()
        .skip_while(|line| !line.starts_with("    if: >-"))
        .skip(1)
        .take_while(|line| line.starts_with("      "))
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The lines of the step whose first line is `first`, up to the next step.
fn step<'a>(lines: &[&'a str], first: &str) -> Vec<&'a str> {
    let at = lines
        .iter()
        .position(|line| *line == first)
        .unwrap_or_else(|| panic!("no step {first}: {lines:#?}"));
    std::iter::once(lines[at])
        .chain(
            lines[at + 1..]
                .iter()
                .take_while(|line| !line.starts_with("      - "))
                .copied(),
        )
        .collect()
}

/// The pass runs on pushes to main and manual runs only, so a branch never
/// starts one, and the slice workflow only when called. Every push to main
/// chooses its slice, a Markdown-only one too, so no merge's slice is missed,
/// and every job checks the ref again, since a manual run may name a branch.
#[test]
fn the_full_pass_never_runs_on_a_branch() {
    let full = full();
    assert_eq!(triggers(&full), ["push", "workflow_dispatch"]);
    assert!(
        full.contains("\non:\n  push:\n    branches: [main]\n  workflow_dispatch:\n"),
        "only main's pushes"
    );
    assert!(
        !full.contains("paths"),
        "no push to main may skip the count"
    );
    let slice = slice();
    assert_eq!(triggers(&slice), ["workflow_call"]);
    assert_eq!(jobs(&full), ["slice", "pass"]);
    assert_eq!(
        jobs(&slice),
        [
            "wait1", "wait2", "wait3", "wait4", "start", "linux", "windows", "macos", "report"
        ]
    );
    for text in [&full, &slice] {
        for name in jobs(text) {
            let condition = condition(&job(text, name));
            assert!(
                condition.contains("github.ref == 'refs/heads/main'"),
                "{name}: {condition}"
            );
        }
    }
}

/// GitHub gives a job without a status function an implied success(), which
/// fails whenever a job it needs was skipped, as the later waits usually are.
/// So every job that needs another starts its condition with !cancelled() and
/// checks the results it relies on itself.
#[test]
fn every_job_that_needs_another_states_its_own_status_check() {
    for text in [full(), slice()] {
        for name in jobs(&text) {
            let lines = job(&text, name);
            if !lines.iter().any(|line| line.starts_with("    needs:")) {
                continue;
            }
            let condition = condition(&lines);
            assert!(
                condition.starts_with("!cancelled() && "),
                "{name}: {condition}"
            );
        }
    }
    let slice = slice();
    for name in ["linux", "windows", "macos"] {
        assert_eq!(
            condition(&job(&slice, name)),
            format!(
                "!cancelled() && github.ref == 'refs/heads/main' \
                 && needs.start.result == 'success' && needs.start.outputs.state == 'idle' \
                 && contains(vars.CLIMON2_OWN_RUNNERS, '{name}')"
            ),
            "{name}"
        );
    }
    assert_eq!(
        condition(&job(&full(), "pass")),
        "!cancelled() && github.ref == 'refs/heads/main' \
         && needs.slice.result == 'success' && needs.slice.outputs.slices != ''"
    );
}

/// The hosted `slice` job chooses the slices from main's squash-merges with
/// tools/quality-full-run.py, or takes a manual run's choice of one slice or
/// all ten, and names the pushed commit whose own quality.yml run each slice
/// waits for. The slice workflow is called once per slice, one at a time, so
/// each slice of a manual run of all of them waits for idle gates first.
#[test]
fn each_chosen_slice_is_its_own_call_one_at_a_time() {
    let full = full();
    let inputs = [
        "        type: choice",
        "        options: [\"all\", \"0\", \"1\", \"2\", \"3\", \"4\", \"5\", \"6\", \"7\", \"8\", \"9\"]",
    ];
    for line in inputs {
        assert!(full.contains(&format!("\n{line}\n")), "{line}");
    }
    let chooser = job(&full, "slice");
    for line in [
        "    runs-on: ubuntu-24.04",
        "      slices: ${{ steps.slice.outputs.slices }}",
        "      gate: ${{ steps.slice.outputs.gate }}",
        "          fetch-depth: 0",
        "          REQUESTED: ${{ inputs.slice }}",
        "        run: python3 tools/quality-full-run.py slice --event \"$EVENT\" --requested \"$REQUESTED\" --before \"$BEFORE\" --after \"$GITHUB_SHA\"",
    ] {
        assert!(chooser.contains(&line), "{line}: {chooser:#?}");
    }
    let pass = job(&full, "pass");
    for line in [
        "    needs: slice",
        "      fail-fast: false",
        "      max-parallel: 1",
        "        slice: ${{ fromJSON(needs.slice.outputs.slices) }}",
        "    uses: ./.github/workflows/quality-slice.yml",
        "      slice: ${{ matrix.slice }}",
        "      gate: ${{ needs.slice.outputs.gate }}",
        "      actions: read",
        "      issues: write",
    ] {
        assert!(pass.contains(&line), "{line}: {pass:#?}");
    }
    let slice = slice();
    assert!(slice.contains(
        "\nenv:\n  CARGO_TARGET_DIR: target/quality-run\n  SLICE: ${{ inputs.slice }}\n"
    ));
    for (name, native) in [
        ("linux", ""),
        ("windows", "--native-only "),
        ("macos", "--native-only "),
    ] {
        let lines = job(&slice, name);
        assert!(lines.contains(&"    needs: start"), "{name}");
        let gate = format!("cargo xtask mutate {native}--mutate-slice \"$SLICE\" 2>&1 | tee ");
        assert!(
            lines
                .iter()
                .any(|line| line.trim_start().starts_with(&gate)),
            "{name}: {lines:#?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("--mutate-all")),
            "{name}"
        );
    }
}

/// tools/quality_paths.py, which quality-full-run.py and quality-prior-run.py
/// share, knows which pushes run quality.yml by that workflow's `paths` filter;
/// if the filter changes, the module must change with it.
#[test]
fn the_gate_paths_the_script_knows_are_quality_ymls() {
    let quality = workflow("quality.yml");
    assert!(quality.contains(
        "    paths:\n      - \"**\"\n      - \"!**/*.md\"\n      - \"!LICENSE\"\n      \
         - \"docs/for-humans/protocol/**/*.md\"\n  workflow_dispatch:\n"
    ));
}

/// A slice never holds a machine a gate or an earlier slice is waiting for.
/// Hosted jobs wait, four in turn for up to 5 hours each, until no quality.yml
/// run and no older full run is unfinished and main's own quality.yml run for
/// the pushed commit has appeared; a start job tells idle gates from busy ones
/// and from a failed wait; and only on idle is a machine's job requested, so it
/// never waits in a machine's queue for GitHub's 24 hours.
#[test]
fn a_slice_waits_on_hosted_runners_until_the_gates_are_idle() {
    let slice = slice();
    assert_eq!(
        condition(&job(&slice, "wait1")),
        "github.ref == 'refs/heads/main'"
    );
    for (index, previous) in [(2, 1), (3, 2), (4, 3)] {
        let lines = job(&slice, &format!("wait{index}"));
        let needs = format!("    needs: wait{previous}");
        assert!(lines.contains(&needs.as_str()), "wait{index}");
        assert_eq!(
            condition(&lines),
            format!(
                "!cancelled() && github.ref == 'refs/heads/main' \
                 && needs.wait{previous}.result == 'success' \
                 && needs.wait{previous}.outputs.idle == 'false'"
            )
        );
    }
    for index in 1..=4 {
        let name = format!("wait{index}");
        let lines = job(&slice, &name);
        for line in [
            "    runs-on: ubuntu-24.04",
            "    timeout-minutes: 330",
            "      actions: read",
            "      idle: ${{ steps.wait.outputs.idle }}",
            "          GATE: ${{ inputs.gate }}",
            "        run: python3 tools/quality-full-run.py wait --repo \"$GITHUB_REPOSITORY\" --run-id \"$GITHUB_RUN_ID\" --gate \"$GATE\"",
        ] {
            assert!(lines.contains(&line), "{name}: {line}: {lines:#?}");
        }
    }
    // The script's default limit is 5 hours, under the job's 330 minutes.
    let script = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/quality-full-run.py"),
    )
    .unwrap();
    assert!(script.contains("add_argument(\"--limit\", type=int, default=5 * 60 * 60)"));
    let start = job(&slice, "start");
    for line in [
        "    needs: [wait1, wait2, wait3, wait4]",
        "    runs-on: ubuntu-24.04",
        "      state: ${{ steps.start.outputs.state }}",
        "          NEEDS: ${{ toJSON(needs) }}",
        "        run: python3 tools/quality-full-run.py start --needs \"$NEEDS\"",
    ] {
        assert!(start.contains(&line), "{line}: {start:#?}");
    }
    assert!(job(&slice, "report").contains(
        &"          GATES: ${{ needs.start.result == 'success' && needs.start.outputs.state || 'failed' }}"
    ));
}

/// A failed set-up never turns main red and is always reported. Each machine's
/// job first records that set-up has not finished, every set-up step continues
/// on error, and the pass runs only if all of them succeeded; it records that it
/// started, then its exit status. The log is uploaded whatever happened, and a
/// failed upload is reported as a missing log, not a red job.
#[test]
fn a_failed_set_up_is_reported_not_red() {
    let slice = slice();
    for (name, ids) in [
        (
            "linux",
            &["checkout", "toolchain", "bash", "workers", "tools"][..],
        ),
        ("windows", &["checkout", "toolchain", "workers", "tools"]),
        (
            "macos",
            &[
                "checkout",
                "toolchain",
                "targets",
                "bash",
                "workers",
                "tools",
            ],
        ),
    ] {
        let lines = job(&slice, name);
        let steps = lines.iter().position(|line| *line == "    steps:").unwrap();
        assert_eq!(
            lines[steps + 1..steps + 5],
            [
                "      - name: Record that set-up has started",
                "        run: |",
                "          mkdir -p \"$RUNNER_TEMP/full-run\"",
                "          echo setup-failed > \"$RUNNER_TEMP/full-run/status\"",
            ],
            "{name}"
        );
        for id in ids {
            let set_up = step(&lines, &format!("      - id: {id}"));
            assert!(
                set_up.contains(&"        continue-on-error: true"),
                "{name}: {id}: {set_up:#?}"
            );
        }
        let first = lines
            .iter()
            .find(|line| line.starts_with("      - name: Mutate the slice's"))
            .unwrap();
        let mutate: Vec<_> = step(&lines, first).iter().map(|line| line.trim()).collect();
        let condition = mutate
            .iter()
            .skip_while(|line| **line != "if: >-")
            .skip(1)
            .take_while(|line| line.starts_with("!cancelled()") || line.starts_with("&&"))
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        let expected = std::iter::once("!cancelled()".to_owned())
            .chain(
                ids.iter()
                    .map(|id| format!("&& steps.{id}.outcome == 'success'")),
            )
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(condition, expected, "{name}");
        for line in [
            "continue-on-error: true",
            "echo started > \"$RUNNER_TEMP/full-run/status\"",
            "echo \"$status\" > \"$RUNNER_TEMP/full-run/status\"",
        ] {
            assert!(mutate.contains(&line), "{name}: {line}");
        }
        let upload = step(&lines, "      - name: Upload the pass's log");
        for line in [
            "        if: ${{ !cancelled() }}",
            "        continue-on-error: true",
            "          path: ${{ runner.temp }}/full-run",
        ] {
            assert!(upload.contains(&line), "{name}: {line}: {upload:#?}");
        }
        let artifact = format!("          name: full-run-{name}-${{{{ inputs.slice }}}}");
        assert!(upload.contains(&artifact.as_str()), "{name}");
    }
    // A daemon test's evidence (#158) is kept in the uploaded folder.
    let macos = job(&slice, "macos");
    assert!(
        macos.contains(
            &"          CLIMON2_TEST_EVIDENCE: ${{ runner.temp }}/full-run/test-evidence"
        )
    );
}

/// A slice runs on its machine's own runner with quality.yml's worker limits
/// and a time limit of a few hours. No workflow or job, the report included, has
/// a concurrency group, so a later merge never cancels or replaces a slice or its
/// report (#457). The job's limit leaves over 45 minutes beyond the pass's for
/// checkout, set-up and the log's upload.
#[test]
fn a_slice_never_reddens_main_or_gets_cancelled() {
    let slice = slice();
    let quality = workflow("quality.yml");
    for text in [full(), slice.clone()] {
        assert!(
            !text.contains("concurrency:"),
            "a slice or its report must never be dropped"
        );
    }
    for (name, os, job_minutes, step_minutes) in [
        ("linux", "Linux", 300, 225),
        ("windows", "Windows", 150, 105),
        ("macos", "macOS", 150, 105),
    ] {
        assert!(job_minutes - step_minutes >= 45, "{name}");
        let lines = job(&slice, name);
        for line in [
            format!("    runs-on: [self-hosted, {os}, X64, climon2-quality]"),
            format!("    timeout-minutes: {job_minutes}"),
            format!("        timeout-minutes: {step_minutes}"),
            "        run: rm -rf target/mutation-workers".to_owned(),
        ] {
            assert!(lines.contains(&line.as_str()), "{name}: {line}: {lines:#?}");
        }
        assert!(
            !lines.iter().any(|line| line.contains("if: failure()")),
            "{name}: a continued step never fails its job"
        );
        // Every worker limit the platform's gate sets, the same.
        for line in job(&quality, name).iter().filter(|line| {
            line.starts_with("      CLIMON2_") && !line.contains("MUTATION_BASE")
                || line.starts_with("      CARGO_BUILD_JOBS:")
                || line.starts_with("      RUST_TEST_THREADS:")
        }) {
            assert!(lines.contains(line), "{name}: {line}");
        }
    }
}

/// Every step runs in bash with pipefail, so a failed pass piped through tee
/// records its own exit status, not tee's. A step without an explicit shell runs
/// `bash -e` without pipefail, so each workflow names bash for every step and no
/// job or step picks another shell.
#[test]
fn a_piped_pass_keeps_its_exit_status() {
    for text in [full(), slice()] {
        assert!(
            text.contains("\ndefaults:\n  run:\n    shell: bash\n"),
            "the workflow's steps run in bash with pipefail"
        );
        assert_eq!(text.matches("shell:").count(), 1, "nothing overrides it");
    }
}

/// The report job puts the slice's findings into one tracking issue with the
/// workflow's own token, the only write either workflow has; neither uses a
/// secret. It knows which machines should have run, and the date for its notes.
/// #457: each slice's findings go in its own comment (quality_full_report.rs),
/// so reports need no concurrency group.
#[test]
fn each_slice_reports_its_findings_in_one_issue() {
    let full = full();
    assert!(full.contains("\npermissions:\n  contents: read\n\n"));
    for text in [full.clone(), slice()] {
        assert!(!text.contains("secrets."));
        let writes: Vec<_> = text
            .lines()
            .filter(|line| line.trim_end().ends_with(": write"))
            .collect();
        assert_eq!(writes, ["      issues: write"]);
    }
    let slice = slice();
    let report = job(&slice, "report");
    for line in [
        "    needs: [start, linux, windows, macos]",
        "    runs-on: ubuntu-24.04",
        "      issues: write",
        "          pattern: full-run-*-${{ inputs.slice }}",
        "          GH_TOKEN: ${{ github.token }}",
        "          OWN_RUNNERS: ${{ vars.CLIMON2_OWN_RUNNERS }}",
        "        run: python3 tools/quality-full-run.py report --repo \"$GITHUB_REPOSITORY\" --slices \"[\\\"$SLICE\\\"]\" --gates \"$GATES\" --own-runners \"$OWN_RUNNERS\" --date \"$(date -u +%Y-%m-%d)\" --logs \"$RUNNER_TEMP/full-run\" --run-url \"$RUN_URL\" --sha \"$GITHUB_SHA\"",
    ] {
        assert!(report.contains(&line), "{line}: {report:#?}");
    }
}
