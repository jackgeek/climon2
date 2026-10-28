use std::{
    collections::BTreeSet,
    env,
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const CRAPPER: &str = "6fd92c4c9bcc56efb64a2f8743d5a2993bcfd42d";
const MUTATOR: &str = "a5a0cb803b66e0cdb8bf471b3b08af701d424a41";
const LLVM_COV: &str = "0.9.1";
const SHIPPED: &[&str] = &[
    "crates/climon2-launcher/src",
    "crates/climon2-core/src",
    "crates/climon2-platform/src",
    "crates/climon2-cli/src",
    "crates/climon2-daemon/src",
    "crates/climon2-client/src",
    "crates/climon2-install/src",
];
const TEST_COMMAND: &str = "cargo test --workspace --locked --target-dir target/mutation-tests";

fn run(command: &mut Command) -> io::Result<()> {
    eprintln!("> {command:?}");
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{command:?} failed with {status}"
        )))
    }
}

fn output(command: &mut Command) -> io::Result<Output> {
    let output = command.output()?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(io::Error::other(format!(
            "{command:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn tool_root(root: &Path) -> PathBuf {
    root.join(".metrics/tools")
}

fn verify_checkout(path: &Path, revision: &str) -> io::Result<()> {
    let head = output(
        Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["rev-parse", "HEAD"]),
    )?;
    if String::from_utf8_lossy(&head.stdout).trim() != revision {
        return Err(io::Error::other(format!(
            "{} must be pinned to {revision}",
            path.display()
        )));
    }
    run(Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["diff", "--exit-code", "HEAD"]))
}

pub fn setup(root: &Path) -> io::Result<()> {
    let tools = tool_root(root);
    fs::create_dir_all(&tools)?;
    for (name, revision) in [("crapper", CRAPPER), ("mutator", MUTATOR)] {
        let destination = tools.join(name);
        if !destination.try_exists()? {
            run(Command::new("git")
                .args(["clone", "--quiet", "--no-checkout"])
                .arg(format!("https://github.com/unclebob/{name}.git"))
                .arg(&destination))?;
            run(Command::new("git")
                .arg("-C")
                .arg(&destination)
                .args(["checkout", "--quiet", "--detach", revision]))?;
        }
        verify_checkout(&destination, revision)?;
    }
    let python = env::var_os("CLIMON2_QUALITY_PYTHON").unwrap_or_else(|| "python3".into());
    run(Command::new(&python).args([
        "-c",
        "import sys; assert sys.version_info >= (3, 11), 'quality tools need Python >=3.11; set CLIMON2_QUALITY_PYTHON'",
    ]))?;
    let environment = tools.join("venv");
    if !environment.try_exists()? {
        run(Command::new(python).args(["-m", "venv"]).arg(&environment))?;
    }
    run(Command::new(environment.join("bin/python"))
        .args(["-m", "pip", "install", "--disable-pip-version-check", "-r"])
        .arg(root.join("tools/quality-requirements.txt"))
        .arg(tools.join("crapper"))
        .arg(tools.join("mutator")))?;
    let installed = Command::new(env!("CARGO"))
        .args(["llvm-cov", "--version"])
        .output()?;
    if !installed.status.success() {
        run(Command::new(env!("CARGO")).args([
            "install",
            "cargo-llvm-cov",
            "--version",
            LLVM_COV,
            "--locked",
        ]))?;
    }
    verify_llvm_cov()
}

fn verify_llvm_cov() -> io::Result<()> {
    let version = output(Command::new(env!("CARGO")).args(["llvm-cov", "--version"]))?;
    let expected = format!("cargo-llvm-cov {LLVM_COV}");
    if String::from_utf8_lossy(&version.stdout).trim() != expected {
        return Err(io::Error::other(format!(
            "expected {expected}; install it with cargo install cargo-llvm-cov --version {LLVM_COV} --locked --force"
        )));
    }
    Ok(())
}

fn tool(root: &Path, name: &str) -> io::Result<Command> {
    let tools = tool_root(root);
    let python = tools.join("venv/bin/python");
    if !python.is_file() {
        return Err(io::Error::other(
            "quality tools are missing; run cargo xtask setup-quality",
        ));
    }
    for (package, revision) in [("crapper", CRAPPER), ("mutator", MUTATOR)] {
        verify_checkout(&tools.join(package), revision)?;
    }
    let mut command = Command::new(python);
    command.args(["-m", name]).current_dir(root);
    Ok(command)
}

fn mutation_command(root: &Path) -> io::Result<Command> {
    let mut command = tool(root, "mutator")?;
    // Each overlay owns this relative target; inherited build paths must not re-share it.
    command
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .env_remove("CARGO_BUILD_BUILD_DIR")
        .args(["--test-command", TEST_COMMAND]);
    Ok(command)
}

fn changed_sources(root: &Path, base: &OsStr) -> io::Result<Vec<PathBuf>> {
    let commit = output(
        Command::new("git")
            .current_dir(root)
            .args(["rev-parse", "--verify", "--end-of-options"])
            .arg(format!("{}^{{commit}}", base.to_string_lossy())),
    )?;
    let commit = String::from_utf8(commit.stdout).map_err(io::Error::other)?;
    let changed = output(
        Command::new("git")
            .current_dir(root)
            .args(["diff", "--name-only", "-z", "--diff-filter=ACMR"])
            .arg(commit.trim())
            .arg("--")
            .args(SHIPPED),
    )?;
    let untracked = output(
        Command::new("git")
            .current_dir(root)
            .args(["ls-files", "--others", "--exclude-standard", "-z", "--"])
            .args(SHIPPED),
    )?;
    let mut paths = BTreeSet::new();
    for bytes in [&changed.stdout, &untracked.stdout] {
        for path in bytes
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            let path = PathBuf::from(std::str::from_utf8(path).map_err(io::Error::other)?);
            if path.extension().is_some_and(|extension| extension == "rs")
                && root.join(&path).is_file()
            {
                paths.insert(path);
            }
        }
    }
    Ok(paths.into_iter().collect())
}

pub fn check(root: &Path, all: bool) -> io::Result<()> {
    verify_llvm_cov()?;
    let mut mutation = mutation_command(root)?;
    let report = root.join("target/coverage/rust/lcov.info");
    fs::create_dir_all(report.parent().expect("report directory"))?;
    if report.try_exists()? {
        fs::remove_file(&report)?;
    }
    run(coverage_command(root, &root.join("target/coverage-build"))
        .args(["--workspace", "--locked", "--lcov", "--output-path"])
        .arg(&report))?;
    if fs::metadata(&report)?.len() == 0 {
        return Err(io::Error::other("coverage report is empty"));
    }
    run(tool(root, "crapper")?
        .args(["--use-existing-coverage", "--threshold", "7"])
        .args(SHIPPED))?;
    mutation.args(["--use-existing-coverage", "--mutate-all"]);
    if all {
        mutation.args(SHIPPED);
    } else {
        // Upstream --changed reads only git status and cannot limit itself to shipped crates.
        let base = env::var_os("CLIMON2_MUTATION_BASE").unwrap_or_else(|| "HEAD".into());
        let paths = changed_sources(root, &base)?;
        if paths.is_empty() {
            eprintln!("No changed shipped Rust source to mutate.");
            return Ok(());
        }
        mutation.args(paths);
    }
    run(&mut mutation)
}

pub fn probe(root: &Path) -> io::Result<()> {
    let scratch = probe_workspace(root)?;
    let observations = scratch.path().join("observations");
    let forbidden = scratch.path().join("shared-target-must-not-be-used");
    let mut command = mutation_command(root)?;
    command
        .current_dir(scratch.path())
        .env("CLIMON2_MUTATION_OBSERVATIONS", &observations)
        .env("CARGO_TARGET_DIR", &forbidden)
        .env("CARGO_BUILD_TARGET_DIR", &forbidden)
        .args([
            "--no-coverage",
            "--mutate-all",
            "--max-workers",
            "2",
            "core/src/lib.rs",
        ]);
    run(&mut command)?;
    if forbidden.try_exists()? {
        return Err(io::Error::other("inherited target directory was used"));
    }
    audit_observations(&observations)
}

fn probe_workspace(root: &Path) -> io::Result<tempfile::TempDir> {
    fs::create_dir_all(root.join("target"))?;
    let scratch = tempfile::Builder::new()
        .prefix("mutation-isolation-")
        .tempdir_in(root.join("target"))?;
    for relative in [
        "Cargo.toml",
        "Cargo.lock",
        "core/Cargo.toml",
        "core/src/lib.rs",
        "cli/Cargo.toml",
        "cli/src/main.rs",
        "cli/tests/binary.rs",
    ] {
        let destination = scratch.path().join(relative);
        fs::create_dir_all(destination.parent().expect("fixture parent"))?;
        fs::copy(
            root.join("xtask/tests/fixtures/mutation-isolation")
                .join(relative),
            destination,
        )?;
    }
    let observations = scratch.path().join("observations");
    fs::create_dir(&observations)?;
    Ok(scratch)
}

fn audit_observations(observations: &Path) -> io::Result<()> {
    let mut variants = BTreeSet::new();
    for entry in fs::read_dir(observations)? {
        let text = fs::read_to_string(entry?.path())?;
        variants.insert(verify_observation(&text)?);
    }
    if variants.len() != 7 {
        return Err(io::Error::other(format!(
            "expected baseline and six observed variants, got {variants:?}"
        )));
    }
    eprintln!("Verified baseline and all six real-binary variants with isolated worker targets.");
    Ok(())
}

fn coverage_command(root: &Path, build: &Path) -> Command {
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(root)
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .env_remove("CARGO_BUILD_BUILD_DIR")
        .env("CARGO_LLVM_COV_TARGET_DIR", build)
        .env("CARGO_LLVM_COV_BUILD_DIR", build)
        .arg("llvm-cov");
    command
}

fn staged_count(report: &Path) -> io::Result<u64> {
    let text = fs::read_to_string(report)?;
    let rows: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("FNDA:") && line.contains("staged_probe"))
        .collect();
    if rows.is_empty() {
        return Err(io::Error::other(
            "staged probe function missing from coverage",
        ));
    }
    rows.into_iter()
        .map(|row| {
            row.trim_start_matches("FNDA:")
                .split(',')
                .next()
                .unwrap()
                .parse::<u64>()
                .map_err(io::Error::other)
        })
        .sum()
}

/// #31/#33: installer-built binaries contribute coverage and the pinned gates reject defects.
pub fn tooling_probe(root: &Path) -> io::Result<()> {
    verify_llvm_cov()?;
    let scratch = probe_workspace(root)?;
    let workspace = scratch.path();
    let build = workspace.join("target/instrumented");
    let observations = workspace.join("observations");
    run(coverage_command(workspace, &build)
        .env("CLIMON2_MUTATION_OBSERVATIONS", &observations)
        .args(["--workspace", "--locked", "--no-report"]))?;
    let report = workspace.join("target/coverage/rust/lcov.info");
    fs::create_dir_all(report.parent().unwrap())?;
    let export = || {
        run(coverage_command(workspace, &build)
            .args(["report", "--lcov", "--output-path"])
            .arg(&report))
    };
    export()?;
    if staged_count(&report)? != 0 {
        return Err(io::Error::other(
            "staged-only function unexpectedly covered before staged execution",
        ));
    }
    let staged = install_instrumented_probe(workspace, &build)?;
    let environment = output(coverage_command(workspace, &build).arg("show-env"))?;
    let environment = String::from_utf8(environment.stdout).map_err(io::Error::other)?;
    let profile = environment
        .lines()
        .find_map(|line| {
            line.strip_prefix("LLVM_PROFILE_FILE='")
                .and_then(|value| value.strip_suffix('\''))
        })
        .ok_or_else(|| io::Error::other("llvm-cov did not supply its profile filename"))?;
    let result = output(
        Command::new(staged.join(env!("CARGO_PKG_VERSION")).join("climon2-cli"))
            .arg("--staged-probe")
            .env("LLVM_PROFILE_FILE", profile),
    )?;
    if result.stdout != b"42\n" {
        return Err(io::Error::other(
            "staged binary returned the wrong observation",
        ));
    }
    export()?;
    let count = staged_count(&report)?;
    if count != 1 {
        return Err(io::Error::other(format!(
            "staged execution must contribute exactly one coverage hit, got {count}"
        )));
    }
    prove_crapper(root, workspace)?;
    prove_covered_mutations(root, workspace, &observations)?;
    eprintln!(
        "Verified installer-built coverage, ? complexity, threshold 7 rejection, and changed/full covered mutations."
    );
    Ok(())
}

fn install_instrumented_probe(workspace: &Path, build: &Path) -> io::Result<PathBuf> {
    let payload = workspace.join("payload");
    fs::create_dir(&payload)?;
    for binary in ["climon2", "climon2-cli", "climon2-daemon", "climon2-client"] {
        fs::copy(build.join("debug/isolation-cli"), payload.join(binary))?;
    }
    let payload = climon2_install::Payload::from_siblings(&payload, env!("CARGO_PKG_VERSION"))?;
    let destination = workspace.join("installed");
    climon2_install::install(&payload, &destination)?;
    Ok(destination)
}

fn prove_crapper(root: &Path, workspace: &Path) -> io::Result<()> {
    run(tool(root, "crapper")?.current_dir(workspace).args([
        "--use-existing-coverage",
        "--threshold",
        "7",
        "core/src/lib.rs",
    ]))?;
    let metrics = fs::read_to_string(workspace.join(".metrics/crap.edn"))?;
    let question = metrics
        .lines()
        .find(|line| line.contains(":name \"question_probe\""))
        .ok_or_else(|| io::Error::other("question probe absent from CRAP metrics"))?;
    if !question.contains(":complexity 2, :coverage 100.0, :crap 2.0") {
        return Err(io::Error::other(format!(
            "? must count as a covered branch: {question}"
        )));
    }
    let excessive = workspace.join("over-limit.rs");
    fs::copy(root.join("xtask/tests/fixtures/over-limit.rs"), &excessive)?;
    let rejected = tool(root, "crapper")?
        .current_dir(workspace)
        .args(["--use-existing-coverage", "--threshold", "7"])
        .arg(&excessive)
        .output()?;
    if rejected.status.code() != Some(2) {
        return Err(io::Error::other(format!(
            "threshold 7 accepted uncovered complexity 3: {rejected:?}"
        )));
    }
    let metrics = fs::read_to_string(workspace.join(".metrics/crap.edn"))?;
    if !metrics.lines().any(|line| {
        line.contains(":name \"over_limit\"")
            && line.contains(":complexity 3, :coverage 0.0, :crap 12.0")
    }) {
        return Err(io::Error::other(format!(
            "threshold failure did not score the expected uncovered function: {metrics}"
        )));
    }
    fs::remove_file(excessive)?;
    Ok(())
}

fn prove_covered_mutations(root: &Path, workspace: &Path, observations: &Path) -> io::Result<()> {
    run(Command::new("git")
        .current_dir(workspace)
        .args(["init", "--quiet"]))?;
    run(Command::new("git").current_dir(workspace).args([
        "add",
        "Cargo.toml",
        "Cargo.lock",
        "core",
        "cli",
    ]))?;
    run(Command::new("git").current_dir(workspace).args([
        "-c",
        "user.name=Quality fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--quiet",
        "--no-verify",
        "-m",
        "quality baseline",
    ]))?;
    let source = workspace.join("core/src/lib.rs");
    let mut text = fs::read_to_string(&source)?;
    text.push('\n');
    fs::write(source, text)?;
    for mode in ["--changed", "--mutate-all"] {
        run(mutation_command(root)?
            .current_dir(workspace)
            .env("CLIMON2_MUTATION_OBSERVATIONS", observations)
            .args([
                "--use-existing-coverage",
                mode,
                "--max-workers",
                "2",
                "core/src/lib.rs",
            ]))?;
        audit_observations(observations)?;
        for entry in fs::read_dir(observations)? {
            fs::remove_file(entry?.path())?;
        }
    }
    Ok(())
}
fn verify_observation(text: &str) -> io::Result<&'static str> {
    let parts: Vec<_> = text.split("\n---\n").collect();
    if parts.len() != 3 {
        return Err(io::Error::other("malformed mutation observation"));
    }
    let source = parts[0];
    let (variant, expected, code) = if source.contains("value >= 0") {
        (
            ">=",
            "[true, true, true, false] [None, Some(1), Some(2)]\n",
            "125",
        )
    } else if source.contains("value > 1") {
        (
            "zero-to-one",
            "[false, false, true, false] [None, Some(1), Some(2)]\n",
            "125",
        )
    } else if source.contains(" || ") {
        (
            "or",
            "[true, true, true, true] [None, Some(1), Some(2)]\n",
            "125",
        )
    } else if source.contains("value <= 10") {
        (
            "<=",
            "[false, true, true, true] [None, Some(1), Some(2)]\n",
            "125",
        )
    } else if source.contains("value? - 1") {
        ("subtract", "", "101")
    } else if source.contains("value? + 0") {
        (
            "one-to-zero",
            "[false, true, true, false] [None, Some(0), Some(1)]\n",
            "125",
        )
    } else if source.contains("value > 0 && value < 10") && source.contains("value? + 1") {
        (
            "baseline",
            "[false, true, true, false] [None, Some(1), Some(2)]\n",
            "125",
        )
    } else {
        return Err(io::Error::other("unknown probe source variant"));
    };
    if parts[1] != expected || parts[2] != code {
        return Err(io::Error::other(format!(
            "worker executed the wrong binary for {variant}: output={:?}, exit={}",
            parts[1], parts[2]
        )));
    }
    Ok(variant)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #21 isolation amendment: a reported kill cannot stand in for the selected binary.
    #[test]
    fn observation_audit_rejects_a_paired_workers_executable() {
        let source = "value > 1 && value < 10; Some(value? + 1)";
        let wrong = format!(
            "{source}\n---\n[true, true, true, false] [None, Some(1), Some(2)]\n\n---\n125"
        );
        assert!(
            verify_observation(&wrong)
                .unwrap_err()
                .to_string()
                .contains("wrong binary")
        );
    }

    /// #31: audit both observable output and status; incomplete evidence is a failure.
    #[test]
    fn observation_audit_accepts_the_actual_variant_and_rejects_bad_records() {
        let correct = "value > 1 && value < 10; Some(value? + 1)\n---\n[false, false, true, false] [None, Some(1), Some(2)]\n\n---\n125";
        assert_eq!(verify_observation(correct).unwrap(), "zero-to-one");
        assert!(verify_observation("truncated").is_err());
        assert!(verify_observation("unknown\n---\n\n---\n0").is_err());
    }

    fn commit(root: &Path) {
        run(Command::new("git").current_dir(root).args(["add", "."])).unwrap();
        run(Command::new("git").current_dir(root).args([
            "-c",
            "user.name=Mutation selection fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--no-verify",
            "--quiet",
            "-m",
            "selection fixture",
        ]))
        .unwrap();
    }

    /// #21 PR gate: committed changes count, and test-only sources never enter shipped gates.
    #[test]
    fn changed_selection_covers_committed_diff_and_untracked_shipped_source_only() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        run(Command::new("git")
            .current_dir(root)
            .args(["init", "--quiet"]))
        .unwrap();
        let cli = Path::new("crates/climon2-cli/src/lib.rs");
        let core = Path::new("crates/climon2-core/src/lib.rs");
        let test_only = Path::new("xtask/src/main.rs");
        for path in [cli, core, test_only] {
            fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
        }
        fs::write(root.join(cli), "baseline").unwrap();
        fs::write(root.join(test_only), "baseline").unwrap();
        commit(root);
        let base = output(
            Command::new("git")
                .current_dir(root)
                .args(["rev-parse", "HEAD"]),
        )
        .unwrap();
        let base = String::from_utf8(base.stdout).unwrap();
        fs::write(root.join(cli), "committed change").unwrap();
        fs::write(root.join(test_only), "test-only change").unwrap();
        commit(root);
        fs::write(root.join(core), "untracked shipped source").unwrap();

        assert_eq!(
            changed_sources(root, OsStr::new(base.trim())).unwrap(),
            vec![cli.to_owned(), core.to_owned()]
        );
        assert_eq!(
            changed_sources(root, OsStr::new("HEAD")).unwrap(),
            vec![core.to_owned()]
        );
        assert!(changed_sources(root, OsStr::new("missing-base")).is_err());
    }
}
