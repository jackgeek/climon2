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
        verdict.contains(&"    needs: [verify, qualify, qualify-windows]"),
        "{verdict:#?}"
    );
    assert!(
        verdict.iter().any(|line| line.trim_start()
            == "run: cargo xtask qualify verdict --evidence target/evidence --out target/verdict --tag \"$GITHUB_REF_NAME\""),
        "{verdict:#?}"
    );
    let publish = job(&release, "publish");
    assert!(
        publish.contains(&"    needs: [verify, candidate, verdict]"),
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

/// #319: the Windows Dist job builds the zip on `windows-2025`. Unlike the
/// Windows Test and Quality jobs it does not wait for the Windows scope, so
/// `CLIMON2_WINDOWS_TESTS=off` never skips it. Only a Linux-only pull request
/// (#488) does.
#[test]
fn the_windows_dist_job_runs_on_windows_2025_whatever_the_switch_says() {
    let check = workflow("check.yml");
    let dist = job(&check, "windows-dist");
    assert!(dist.contains(&"    runs-on: windows-2025"), "{dist:#?}");
    assert!(dist.contains(&"    needs: platforms"), "{dist:#?}");
    assert!(
        dist.contains(&"    if: needs.platforms.outputs.linux_only != 'true'"),
        "{dist:#?}"
    );
    assert!(dist.contains(&"        run: cargo xtask dist"), "{dist:#?}");
    assert!(
        dist.contains(&"          name: release-dist-windows-x86_64-msvc"),
        "{dist:#?}"
    );
}

/// #488 (owner decision, 2026-11-27): a pull request into the monitor links
/// feature branch, or into a ticket branch stacked on it, runs Linux only.
/// One job decides that, and every job with a macOS or Windows leg asks it.
/// Any other run, release qualification included, is never Linux-only.
#[test]
fn monitor_links_pull_requests_run_linux_only() {
    let check = workflow("check.yml");
    let platforms = job(&check, "platforms");
    assert!(platforms.contains(&"      linux_only: ${{ steps.decide.outputs.linux_only }}"));
    assert!(
        platforms
            .contains(&"        run: echo \"linux_only=$LINUX_ONLY\" | tee -a \"$GITHUB_OUTPUT\""),
        "{platforms:#?}"
    );
    assert!(
        platforms.contains(
            &"          LINUX_ONLY: ${{ github.event_name == 'pull_request' && (github.base_ref == 'feature/monitor-links' || startsWith(github.base_ref, 'monitor-links/')) }}"
        ),
        "{platforms:#?}"
    );
    assert_eq!(
        check.matches("monitor-links").count(),
        2,
        "the platforms job alone names the feature's branches"
    );
    // The Windows Test, Quality and Archive jobs need the scope, so they skip with it.
    for name in ["windows-scope", "windows-dist"] {
        let lines = job(&check, name);
        assert!(
            lines.contains(&"    if: needs.platforms.outputs.linux_only != 'true'"),
            "{name}: {lines:#?}"
        );
    }
    assert!(
        condition(&job(&check, "quality"))
            .starts_with("needs.platforms.outputs.linux_only != 'true' && "),
    );
    let linux = "'[\"ubuntu-24.04\"]'";
    let linux_dist = "'[{\"runner\": \"ubuntu-24.04\", \"platform\": \"linux-x86_64-gnu\"}]'";
    for (name, key, only) in [
        ("macos", "runner", linux),
        ("neutral", "os", linux),
        ("dist", "include", linux_dist),
        ("archive", "include", linux_dist),
    ] {
        let lines = job(&check, name);
        let at = lines
            .iter()
            .position(|line| line.trim_start().starts_with(&format!("{key}:")))
            .unwrap_or_else(|| panic!("{name} has no {key}"));
        let matrix = lines[at..at + 2].join(" ");
        assert!(
            matrix.contains(&format!(
                "fromJSON(needs.platforms.outputs.linux_only == 'true' && {only} || "
            )),
            "{name}: {matrix}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("    needs:") && line.contains("platforms")),
            "{name}: {lines:#?}"
        );
    }
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
        let switch = format!("      && contains(vars.CLIMON2_OWN_RUNNERS, '{name}')");
        assert!(lines.contains(&switch.as_str()), "{name}: {lines:#?}");
        assert!(
            lines.iter().any(|line| line.trim_start().starts_with(gate)),
            "{name}: {lines:#?}"
        );
    }
    for name in ["linux", "windows", "macos"] {
        assert!(
            job(&quality, name).contains(&"        run: rm -rf target/mutation-workers"),
            "{name} must clear leftover mutation workers"
        );
    }
    // The Linux and Windows gates share the owner's PC: four mutation workers on
    // Linux and two on Windows, whose VM's memory and load are the limit (owner
    // decision, 2026-11-23).
    assert!(job(&quality, "linux").contains(&"      CLIMON2_MUTATION_WORKERS: \"4\""));
    assert!(job(&quality, "windows").contains(&"      CLIMON2_NATIVE_MUTATION_WORKERS: \"2\""));
    for name in ["linux", "windows"] {
        // Each worker's Cargo gets 4 build jobs and 4 test threads.
        assert!(job(&quality, name).contains(&"      CLIMON2_MUTATION_WORKER_JOBS: \"4\""));
        // And the rest of the gate, its own baseline included, 4 of each.
        assert!(job(&quality, name).contains(&"      CARGO_BUILD_JOBS: \"4\""));
        assert!(job(&quality, name).contains(&"      RUST_TEST_THREADS: \"4\""));
    }
    // The Mac's gate runs its tests 4 at a time too: CPU-bound timing tests
    // missed their deadlines at its default 16 (#425).
    assert!(job(&quality, "macos").contains(&"      RUST_TEST_THREADS: \"4\""));
    for workflow_name in ["check.yml", "release.yml", "measure.yml"] {
        assert!(
            !workflow(workflow_name).contains("self-hosted"),
            "{workflow_name} must stay on GitHub-hosted runners"
        );
    }
}

/// A hung gate must not hold its machine, whose one runner every push shares, for GitHub's
/// six hours (#401). Each job fails at a timeout with a clear margin over its platform's
/// longest passing gate: 171 min on Linux and 68 min on Windows by 2026-11-25.
#[test]
fn own_hardware_gates_time_out() {
    let quality = workflow("quality.yml");
    for (name, minutes) in [("linux", 240), ("windows", 120), ("macos", 120)] {
        let line = format!("    timeout-minutes: {minutes}");
        assert!(job(&quality, name).contains(&line.as_str()), "{name}");
    }
}

/// #330: Windows is the fourth qualification platform. Its job qualifies the
/// exact zip on windows-2025 whatever the Windows switches say, records with
/// `--platform windows-x86_64`, takes the latest release candidate's
/// checklist records on a final tag, and uploads its evidence for the
/// verdict.
#[test]
fn windows_qualifies_its_exact_zip_on_windows_2025() {
    let release = workflow("release.yml");
    let windows = job(&release, "qualify-windows");
    for line in [
        "    needs: [verify, measure, candidate]",
        "    if: ${{ !cancelled() && needs.verify.result == 'success' && needs.candidate.result == 'success' }}",
        "    runs-on: windows-2025",
        "          name: release-dist-windows-x86_64-msvc",
        "          pattern: measure-windows-2025-*",
        "          CLIMON2_E2E_REAL_USER_PATH: \"1\"",
        "          CANDIDATE_TAG: ${{ needs.candidate.outputs.tag }}",
        "          CANDIDATE_SHA256: ${{ needs.candidate.outputs.sha256 }}",
        "          name: qualification-windows-x86_64",
    ] {
        assert!(windows.contains(&line), "{line}: {windows:#?}");
    }
    assert!(
        windows.iter().any(|line| line
            .trim_start()
            .starts_with("cargo xtask qualify run --platform windows-x86_64")),
        "{windows:#?}"
    );
    assert!(
        !windows
            .iter()
            .any(|line| line.contains("CLIMON2_WINDOWS_TESTS")
                || line.contains("CLIMON2_WINDOWS_QUALITY")),
        "{windows:#?}"
    );
}

/// #330: a final tag takes the latest release candidate, which must exist
/// and differ from it only in the version stamp and qualification/; the
/// candidate job names it and its zip's SHA-256, and publishing waits for it.
#[test]
fn a_final_tag_takes_its_latest_release_candidate() {
    let release = workflow("release.yml");
    let candidate = job(&release, "candidate");
    assert!(
        candidate.contains(&"          fetch-depth: 0"),
        "{candidate:#?}"
    );
    assert!(
        candidate.iter().any(|line| line
            .contains("cargo xtask qualify candidate --tag \"$GITHUB_REF_NAME\" --releases")),
        "{candidate:#?}"
    );
    assert!(
        candidate
            .iter()
            .any(|line| line.contains("-windows-x86_64-msvc\\.zip$")),
        "{candidate:#?}"
    );
    assert!(
        !candidate.iter().any(|line| line.starts_with("    needs:")),
        "a release candidate's run never waits for it: {candidate:#?}"
    );
}

/// #330: the release gate requires the Windows Test and Archive jobs and
/// the native-only gate over every registered Windows file, after its
/// probes: Release calls the checks with `mutate_all`, which runs the
/// Windows Quality job whatever `CLIMON2_WINDOWS_QUALITY` says, and a tag
/// always runs the Windows jobs; publishing needs the checks to pass.
#[test]
fn the_release_gate_needs_windows_test_archive_and_mutate_all() {
    let check = workflow("check.yml");
    let scope = job(&check, "windows-scope");
    assert!(
        scope
            .iter()
            .any(|line| line.contains("\"$REF_TYPE\" == tag")),
        "{scope:#?}"
    );
    for name in ["windows-test", "windows-archive", "windows-quality"] {
        assert!(
            job(&check, name)
                .iter()
                .any(|line| line.contains("needs.windows-scope.outputs.test == 'true'")),
            "{name}"
        );
    }
    let quality = job(&check, "windows-quality");
    assert!(
        quality
            .iter()
            .any(|line| line.trim() == "flags=(--native-only)"),
        "{quality:#?}"
    );
    assert!(
        quality.iter().any(|line| line
            .trim()
            .starts_with("if [[ \"$MUTATE_ALL\" == true ]]; then flags+=(--mutate-all); fi")),
        "{quality:#?}"
    );
    let release = workflow("release.yml");
    assert!(
        release.contains(
            "    uses: ./.github/workflows/check.yml\n    with:\n      mutate_all: true\n"
        )
    );
    let publish = job(&release, "publish");
    assert!(
        publish
            .iter()
            .any(|line| line.starts_with("    needs: [verify,"))
    );
}

/// #330: one release publishes every platform: the Windows zip and the
/// stamped install.ps1 beside the Unix archives and install.sh, one
/// SHA256SUMS over all of them, and attestations for every file. A missing
/// platform's artifact fails publishing as a whole.
#[test]
fn publication_includes_the_windows_zip_and_install_ps1() {
    let release = workflow("release.yml");
    let publish = job(&release, "publish");
    for line in [
        "          windows=target/downloaded/release-dist-windows-x86_64-msvc",
        "            \"$windows/climon2-$version-windows-x86_64-msvc.zip\" \\",
        "            \"$macos/install.sh\" \"$windows/install.ps1\"; do",
        "            test -f \"$file\"",
        "          cat \"$macos/SHA256SUMS\" \"$linux/SHA256SUMS\" \"$windows/SHA256SUMS\" | sort -u > target/dist/SHA256SUMS",
        "          subject-path: target/dist/*",
        "            \"target/dist/climon2-$version-windows-x86_64-msvc.zip\" \\",
        "            target/dist/install.sh target/dist/install.ps1 target/dist/SHA256SUMS \\",
    ] {
        assert!(publish.contains(&line), "{line}: {publish:#?}");
    }
}

/// #158: where the daemon tests run on macOS, a failed readiness wait's
/// evidence (the test's wait timestamps and daemon.log) is kept and uploaded
/// when the job fails.
#[test]
fn macos_test_jobs_upload_failed_tests_evidence() {
    for file in ["check.yml", "quality.yml"] {
        let text = workflow(file);
        let lines = job(&text, "macos");
        let evidence = "          CLIMON2_TEST_EVIDENCE: ${{ runner.temp }}/test-evidence";
        assert!(lines.contains(&evidence), "{file}: {lines:#?}");
        let upload = lines
            .iter()
            .position(|line| line.trim() == "- name: Upload failed tests' evidence")
            .unwrap_or_else(|| panic!("{file}: no evidence upload"));
        let step: Vec<_> = lines[upload + 1..]
            .iter()
            .take_while(|line| !line.trim_start().starts_with("- "))
            .map(|line| line.trim())
            .collect();
        assert!(step.contains(&"if: failure()"), "{file}: {step:#?}");
        assert!(
            step.contains(&"uses: actions/upload-artifact@v4"),
            "{file}: {step:#?}"
        );
        assert!(
            step.contains(&"path: ${{ runner.temp }}/test-evidence"),
            "{file}: {step:#?}"
        );
    }
}

/// The lines of the job `name`'s folded `if: >-` condition, trimmed and joined.
fn condition(lines: &[&str]) -> String {
    lines
        .iter()
        .skip_while(|line| !line.starts_with("    if: >-"))
        .skip(1)
        .take_while(|line| line.starts_with("      "))
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

/// #442: a push to main looks for a passed run of its tree first, on a
/// GitHub-hosted runner, so the look-up never queues behind an own-hardware
/// gate. Only a push to main looks: any other push or a manual dispatch skips
/// the look-up, which leaves its output empty. #447: it knows which platforms
/// CLIMON2_OWN_RUNNERS lists and the pushed commit, and a failed look-up never
/// turns main's run red.
#[test]
fn only_a_push_to_main_looks_for_a_passed_run_of_its_tree_on_a_hosted_runner() {
    let quality = workflow("quality.yml");
    let prior = job(&quality, "prior");
    for line in [
        "    if: github.event_name == 'push' && github.ref == 'refs/heads/main'",
        "    runs-on: ubuntu-24.04",
        "    continue-on-error: true",
        "      actions: read",
        "      run: ${{ steps.find.outputs.run }}",
        "          GH_TOKEN: ${{ github.token }}",
        "          OWN_RUNNERS: ${{ vars.CLIMON2_OWN_RUNNERS }}",
        "        run: python3 tools/quality-prior-run.py --repo \"$GITHUB_REPOSITORY\" --tree \"$(git rev-parse \"$GITHUB_SHA^{tree}\")\" --commit \"$GITHUB_SHA\" --own-runners \"$OWN_RUNNERS\"",
    ] {
        assert!(prior.contains(&line), "{line}: {prior:#?}");
    }
    assert!(
        !prior.iter().any(|line| line.contains("self-hosted")),
        "{prior:#?}"
    );
}

/// #442: each gate waits for the look-up and skips only when it found a run,
/// which it does only on a push to main. It runs whatever the look-up's own
/// result, so a failed look-up still runs the gates, as does a branch push,
/// whose look-up is skipped.
#[test]
fn gates_skip_only_when_main_found_a_passed_run_of_its_tree() {
    let quality = workflow("quality.yml");
    for name in ["linux", "windows", "macos"] {
        let lines = job(&quality, name);
        assert!(
            lines.contains(&"    needs: [base, prior]"),
            "{name}: {lines:#?}"
        );
        assert_eq!(
            condition(&lines),
            format!(
                "!cancelled() && needs.base.result == 'success' \
                 && contains(needs.base.outputs.gates, '{name}') \
                 && contains(vars.CLIMON2_OWN_RUNNERS, '{name}') \
                 && needs.prior.outputs.run == ''"
            ),
            "{name}"
        );
    }
}

/// #488: one script decides which gates a run has and what they mutate
/// against (xtask/tests/quality_scope.rs), and the base job hands its
/// answers to the gates.
#[test]
fn the_base_job_asks_the_scope_script() {
    let quality = workflow("quality.yml");
    let base = job(&quality, "base");
    for line in [
        "      sha: ${{ steps.base.outputs.sha }}",
        "      gates: ${{ steps.base.outputs.gates }}",
        "          EVENT: ${{ github.event_name }}",
        "          REF: ${{ github.ref }}",
        "          BEFORE: ${{ github.event.before }}",
        "        run: bash tools/quality-scope.sh >> \"$GITHUB_OUTPUT\"",
    ] {
        assert!(base.contains(&line), "{line}: {base:#?}");
    }
    assert!(
        !quality.contains("monitor-links"),
        "the script alone names the feature's branches"
    );
}

/// #442: the look-up requires every gate job by the name quality.yml gives it,
/// and #447 by the platform key that job's condition looks for.
#[test]
fn the_look_up_requires_every_gate_by_its_job_name() {
    let quality = workflow("quality.yml");
    let script = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/quality-prior-run.py"),
    )
    .unwrap()
    .replace("\r\n", "\n");
    for name in ["linux", "windows", "macos"] {
        let title = job(&quality, name)[0]
            .strip_prefix("    name: ")
            .unwrap_or_else(|| panic!("{name} has no name first"));
        assert!(
            script.contains(&format!("    (\"{name}\", \"{title}\"),\n")),
            "{title} missing from GATES"
        );
    }
}
