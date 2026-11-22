//! The PERF-01 measurement workflow's triggers and placement (#121; owner
//! decisions, 2026-11-16): release candidates and manual dispatch on hosted
//! macos-15, macos-15-intel and ubuntu-24.04, never pull requests, and never
//! gating publication; and release qualification (#122): always on all
//! three native runners, between build and publish, blocking publication.

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
    let needs = publish
        .iter()
        .find(|line| line.starts_with("    needs:"))
        .unwrap();
    assert!(!needs.contains("measure"), "{publish:#?}");
}

/// Release qualification (#122; owner decisions, 2026-11-16): the Release
/// workflow qualifies natively on ubuntu-24.04, macos-15 and
/// macos-15-intel, always, whatever CLIMON2_MACOS_TESTS says, each job
/// recording every case with `cargo xtask qualify run` on its platform.
#[test]
fn qualification_always_runs_on_all_three_native_runners() {
    let release = workflow("release.yml");
    let qualify = job(&release, "qualify");
    for (runner, archive, platform) in [
        ("macos-15", "macos-universal", "macos-arm64"),
        ("macos-15-intel", "macos-universal", "macos-x86_64"),
        ("ubuntu-24.04", "linux-x86_64-gnu", "linux-x86_64"),
    ] {
        let at = qualify
            .iter()
            .position(|line| *line == format!("          - runner: {runner}"))
            .unwrap_or_else(|| panic!("{runner} missing: {qualify:#?}"));
        assert_eq!(qualify[at + 1], format!("            archive: {archive}"));
        assert_eq!(qualify[at + 2], format!("            platform: {platform}"));
    }
    assert!(
        !qualify
            .iter()
            .any(|line| line.contains("CLIMON2_MACOS_TESTS")),
        "{qualify:#?}"
    );
    assert!(qualify.contains(&"    runs-on: ${{ matrix.runner }}"));
    assert!(
        qualify.iter().any(|line| line
            .trim_start()
            .starts_with("cargo xtask qualify run --platform \"$PLATFORM\"")),
        "{qualify:#?}"
    );
}

/// Qualification sits between build and publish and blocks publishing:
/// it needs the verified build (and any Measure reports), the verdict needs
/// every qualification job, and publishing needs the verdict, which fails
/// unless every case passed or has the owner's disposition. Measurement
/// alone never gates publishing.
#[test]
fn publishing_waits_for_the_qualification_verdict() {
    let release = workflow("release.yml");
    let qualify = job(&release, "qualify");
    assert!(
        qualify.contains(&"    needs: [verify, measure]"),
        "{qualify:#?}"
    );
    assert!(qualify.contains(&"    if: ${{ !cancelled() && needs.verify.result == 'success' }}"));
    let verdict = job(&release, "verdict");
    assert!(
        verdict.contains(&"    needs: [verify, qualify]"),
        "{verdict:#?}"
    );
    assert!(
        verdict.iter().any(|line| line.trim_start()
            == "run: cargo xtask qualify verdict --evidence target/evidence --out target/verdict"),
        "{verdict:#?}"
    );
    let publish = job(&release, "publish");
    assert!(
        publish.contains(&"    needs: [verify, verdict]"),
        "{publish:#?}"
    );
    assert!(
        !publish.iter().any(|line| line.contains("if:")),
        "{publish:#?}"
    );
}

/// Evidence records go to workflow artifacts and release assets (owner
/// decision on #122).
#[test]
fn evidence_records_are_artifacts_and_release_assets() {
    let release = workflow("release.yml");
    let qualify = job(&release, "qualify");
    assert!(qualify.contains(&"          name: qualification-${{ matrix.platform }}"));
    let verdict = job(&release, "verdict");
    assert!(verdict.contains(&"          name: qualification-verdict"));
    let publish = job(&release, "publish");
    assert!(
        publish.contains(&"          pattern: qualification-*"),
        "{publish:#?}"
    );
    assert!(
        publish.contains(
            &"            \"target/evidence/climon2-$version-qualification.tar.gz\" \"${args[@]}\""
        ),
        "{publish:#?}"
    );
}

/// Pull requests never qualify: the checks workflow has no qualification.
#[test]
fn pull_requests_never_qualify() {
    assert!(!workflow("check.yml").contains("qualify"));
    assert!(!workflow("measure.yml").contains("qualify"));
}

/// #319: the Windows Dist job builds the zip on `windows-2025` on every run.
/// Unlike the Windows Test and Quality jobs it does not wait for the Windows
/// scope, so `CLIMON2_WINDOWS_TESTS=off` never skips it.
#[test]
fn the_windows_dist_job_runs_on_windows_2025_whatever_the_switch_says() {
    let check = workflow("check.yml");
    let dist = job(&check, "windows-dist");
    assert!(dist.contains(&"    runs-on: windows-2025"), "{dist:#?}");
    assert!(
        !dist
            .iter()
            .any(|line| line.starts_with("    needs:") || line.starts_with("    if:")),
        "{dist:#?}"
    );
    assert!(dist.contains(&"        run: cargo xtask dist"), "{dist:#?}");
    assert!(
        dist.contains(&"          name: release-dist-windows-x86_64-msvc"),
        "{dist:#?}"
    );
}

/// #321: the Windows Test job runs the smoke test against the zip the Dist
/// job built, so it waits for that job and names its zip to the tests.
#[test]
fn the_windows_test_job_runs_the_smoke_test_on_the_dist_zip() {
    let check = workflow("check.yml");
    let test = job(&check, "windows-test");
    assert!(
        test.contains(&"    needs: [windows-scope, windows-dist]"),
        "{test:#?}"
    );
    assert!(
        test.contains(&"          name: release-dist-windows-x86_64-msvc"),
        "{test:#?}"
    );
    assert!(
        test.iter()
            .any(|line| line.contains("CLIMON2_E2E_ARCHIVE=") && line.contains(".zip")),
        "{test:#?}"
    );
}

/// #327: the Windows Test job builds and tests the monitor before the CLI's
/// tests start it, and checks the native interface's declarations and
/// examples with MSVC.
#[test]
fn the_windows_test_job_tests_the_monitor_and_checks_the_native_interface_with_msvc() {
    let check = workflow("check.yml");
    let test = job(&check, "windows-test");
    let line = |text: &str| {
        test.iter()
            .position(|line| line.contains(text))
            .unwrap_or_else(|| panic!("no {text:?} in {test:#?}"))
    };
    assert!(
        line("cargo test --locked -p climon2-monitor") < line("-p climon2-launcher -p climon2-cli")
    );
    line("monitor-client/check.py NativeDeclarations.test_msvc_declarations_and_examples");
}

/// #329: the Windows Archive job tests the exact zip the Windows Dist job
/// built, when the Windows scope says the Windows jobs run. Its tests may
/// change the real user PATH, which only that throwaway runner allows.
#[test]
fn the_windows_archive_job_tests_the_windows_dist_jobs_zip() {
    let check = workflow("check.yml");
    let archive = job(&check, "windows-archive");
    for line in [
        "    runs-on: windows-2025",
        "    needs: [windows-scope, windows-dist]",
        "    if: needs.windows-scope.outputs.test == 'true'",
        "          name: release-dist-windows-x86_64-msvc",
        "          CLIMON2_E2E_REAL_USER_PATH: \"1\"",
        "          cargo test --locked -p climon2-install --test archive_windows",
    ] {
        assert!(archive.contains(&line), "{line}: {archive:#?}");
    }
    let dist = job(&check, "windows-dist");
    assert!(
        dist.contains(&"          path: target/dist/*"),
        "the zip, install.ps1 and SHA256SUMS: {dist:#?}"
    );
}

/// Owner decision, 2026-11-22: while Windows support is built, Windows CRAP
/// and mutation run on the development VM, not on every pull request. The
/// Quality job runs only when `CLIMON2_WINDOWS_QUALITY` is "on", and always
/// for release qualification (`mutate_all`) and tags.
#[test]
fn the_windows_quality_job_waits_for_its_switch_except_for_releases() {
    let check = workflow("check.yml");
    let quality = job(&check, "windows-quality");
    let condition = quality
        .iter()
        .skip_while(|line| !line.starts_with("    if: >-"))
        .take_while(|line| !line.starts_with("    runs-on:"))
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(" ");
    for clause in [
        "needs.windows-scope.outputs.test == 'true'",
        "&& (vars.CLIMON2_WINDOWS_QUALITY == 'on' || inputs.mutate_all || startsWith(github.ref, 'refs/tags/'))",
    ] {
        assert!(condition.contains(clause), "{clause} not in {condition}");
    }
    let release = workflow("release.yml");
    assert!(
        release.contains(
            "    uses: ./.github/workflows/check.yml\n    with:\n      mutate_all: true\n"
        )
    );
}

/// The owner's own machines run each platform's CRAP and mutation gate
/// (docs/for-humans/quality-runners.md). The repository is public and a
/// fork's pull request runs its own copy of a workflow, so these
/// self-hosted jobs run only on push and manual dispatch, never on a pull
/// request, and each waits for its platform in `CLIMON2_OWN_RUNNERS`.
#[test]
fn own_hardware_gates_run_on_push_never_on_pull_requests() {
    let quality = workflow("quality.yml");
    assert_eq!(triggers(&quality), ["push", "workflow_dispatch"]);
    for (name, os, gate) in [
        ("linux", "Linux", "cargo xtask mutate ||"),
        ("windows", "Windows", "cargo xtask mutate --native-only ||"),
        ("macos", "macOS", "cargo xtask mutate --native-only ||"),
    ] {
        let lines = job(&quality, name);
        let runs_on = format!("    runs-on: [self-hosted, {os}, X64, climon2-quality]");
        assert!(lines.contains(&runs_on.as_str()), "{name}: {lines:#?}");
        let switch = format!("    if: contains(vars.CLIMON2_OWN_RUNNERS, '{name}')");
        assert!(lines.contains(&switch.as_str()), "{name}: {lines:#?}");
        assert!(
            lines.iter().any(|line| line.trim_start().starts_with(gate)),
            "{name}: {lines:#?}"
        );
    }
    for workflow_name in ["check.yml", "release.yml", "measure.yml"] {
        assert!(
            !workflow(workflow_name).contains("self-hosted"),
            "{workflow_name} must stay on GitHub-hosted runners"
        );
    }
}
