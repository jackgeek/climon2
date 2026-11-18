//! The PERF-01 measurement workflow's triggers and placement (#121; owner
//! decisions, 2026-11-16): release candidates and manual dispatch on hosted
//! macos-15, macos-15-intel and ubuntu-24.04, never pull requests, and never
//! gating publication.

use std::{fs, path::Path};

fn workflow(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.github/workflows")
        .join(name);
    fs::read_to_string(path).expect("read workflow")
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

/// The job named `name`'s lines, up to the next job.
fn job<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let start = format!("\n  {name}:\n");
    let body = text.split(&start).nth(1).expect("the job");
    body.lines()
        .take_while(|line| line.is_empty() || line.starts_with("    "))
        .collect()
}

#[test]
fn measurement_runs_only_on_dispatch_and_release_candidate_calls() {
    let measure = workflow("measure.yml");
    assert_eq!(triggers(&measure), ["workflow_dispatch", "workflow_call"]);
    let check = workflow("check.yml");
    assert!(
        !check.contains("measure"),
        "checks on pull requests never measure"
    );
}

#[test]
fn measurement_covers_three_hosted_runners_and_every_geometry() {
    let measure = workflow("measure.yml");
    let jobs = job(&measure, "measure");
    assert!(jobs.contains(&"        runner: [macos-15, macos-15-intel, ubuntu-24.04]"));
    assert!(jobs.contains(&"        geometry: [\"80x24\", \"200x60\", \"500x200\"]"));
    assert!(
        jobs.iter().any(|line| line
            .trim_start()
            .starts_with("cargo xtask measure --archive")),
        "{jobs:#?}"
    );
}

#[test]
fn release_candidates_are_measured_without_gating_publication() {
    let release = workflow("release.yml");
    let measure = job(&release, "measure");
    assert!(measure.contains(&"    if: contains(github.ref_name, '-rc.')"));
    assert!(measure.contains(&"    uses: ./.github/workflows/measure.yml"));
    assert!(measure.contains(&"      candidate_built: true"));
    let publish = job(&release, "publish");
    assert!(publish.contains(&"    needs: verify"), "{publish:#?}");
}
