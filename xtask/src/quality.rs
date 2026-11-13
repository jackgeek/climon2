use crate::coverage::Coverage;
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
/// Full-host mutation workers, and so the suites the mutation-load measurement runs at once;
/// 8 leaves room for another agent's tests on the shared development host.
const MUTATION_WORKERS: &str = "8";
const MUTATION_LOAD_ROUNDS: &str = "5";
/// A mutant's test run times out, counting as killed, after this multiple of the baseline run.
const MUTATION_TIMEOUT_FACTOR: &str = "10";
/// Owner-set maximum CRAP score for every shipped function (#21, amended to 5).
pub(crate) const CRAP_THRESHOLD: &str = "5";
const SHIPPED: &[&str] = &[
    "crates/climon2-launcher/src",
    "crates/climon2-core/src",
    "crates/climon2-platform/src",
    "crates/climon2-cli/src",
    "crates/climon2-daemon/src",
    "crates/climon2-client/src",
    "crates/climon2-install/src",
    "crates/climon2-monitor/src",
    "crates/climon2-vt/src",
];
const TEST_COMMAND: &str = r#""$CLIMON2_MUTATION_PYTHON" tools/test-mutation.py"#;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Scope {
    /// `cargo xtask check`: host source, after the unchanged workspace passes under mutation load.
    Complete,
    /// `cargo xtask mutate`: host source.
    Host,
    /// `cargo xtask mutate --native-only`: only host-exclusive native source.
    Native,
}

impl Scope {
    fn select(self, sources: Vec<PathBuf>) -> Vec<PathBuf> {
        sources
            .into_iter()
            .filter(|path| self != Self::Native || native_source_os(path).is_some())
            .collect()
    }

    fn workers(self) -> io::Result<String> {
        match self {
            Self::Complete | Self::Host => Ok(MUTATION_WORKERS.to_owned()),
            Self::Native => Ok(std::thread::available_parallelism()?
                .get()
                .min(16)
                .to_string()),
        }
    }

    /// Rounds of the mutation-load measurement required before mutation, if any.
    fn load_rounds(self) -> Option<&'static str> {
        (self == Self::Complete).then_some(MUTATION_LOAD_ROUNDS)
    }
}

pub(crate) fn run(command: &mut Command) -> io::Result<()> {
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

pub(crate) fn output(command: &mut Command) -> io::Result<Output> {
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
        .env(
            "CLIMON2_MUTATION_PYTHON",
            tool_root(root).join("venv/bin/python"),
        )
        .args(["--test-command", TEST_COMMAND])
        .args(["--timeout-factor", MUTATION_TIMEOUT_FACTOR]);
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
    source_paths(root, &[changed.stdout, untracked.stdout])
}

fn shipped_sources(root: &Path) -> io::Result<Vec<PathBuf>> {
    let sources = output(
        Command::new("git")
            .current_dir(root)
            .args([
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
                "--",
            ])
            .args(SHIPPED),
    )?;
    source_paths(root, &[sources.stdout])
}

fn native_source_os(path: &Path) -> Option<&'static str> {
    match path.to_str() {
        Some("crates/climon2-platform/src/process/linux.rs")
        | Some("crates/climon2-platform/src/peer/linux.rs")
        | Some("crates/climon2-platform/src/runtime/linux.rs")
        | Some("crates/climon2-install/src/platform/linux.rs") => Some("linux"),
        Some("crates/climon2-platform/src/process/macos.rs")
        | Some("crates/climon2-platform/src/peer/macos.rs")
        | Some("crates/climon2-platform/src/runtime/macos.rs")
        | Some("crates/climon2-install/src/platform/macos.rs") => Some("macos"),
        _ => None,
    }
}

fn native_source_is_active(path: &Path) -> bool {
    native_source_os(path).is_none_or(|os| os == env::consts::OS)
}

fn source_paths(root: &Path, outputs: &[Vec<u8>]) -> io::Result<Vec<PathBuf>> {
    let mut paths = BTreeSet::new();
    for bytes in outputs {
        for path in bytes
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            let path = PathBuf::from(std::str::from_utf8(path).map_err(io::Error::other)?);
            if path.extension().is_some_and(|extension| extension == "rs")
                && root.join(&path).is_file()
                && native_source_is_active(&path)
            {
                paths.insert(path);
            }
        }
    }
    Ok(paths.into_iter().collect())
}

fn verify_native_coverage(root: &Path, report: &Path) -> io::Result<()> {
    for file in fs::read_to_string(report)?
        .lines()
        .filter_map(|line| line.strip_prefix("SF:"))
    {
        let path = Path::new(file);
        let relative = path.strip_prefix(root).unwrap_or(path);
        if !native_source_is_active(relative) {
            return Err(io::Error::other(format!(
                "compiled native source would escape quality gates: {}",
                relative.display()
            )));
        }
    }
    Ok(())
}

pub fn check(root: &Path, all: bool, scope: Scope) -> io::Result<()> {
    let sources = shipped_sources(root)?;
    let scored = scope.select(sources.clone());
    let paths = if all {
        scored.clone()
    } else {
        // Upstream --changed reads only git status and cannot limit itself to shipped crates.
        let base = env::var_os("CLIMON2_MUTATION_BASE").unwrap_or_else(|| "HEAD".into());
        scope.select(changed_sources(root, &base)?)
    };
    if scope == Scope::Native && paths.is_empty() {
        eprintln!("No selected native Rust source to qualify; shared source is checked locally.");
        return Ok(());
    }
    verify_llvm_cov()?;
    let mut mutation = mutation_command(root)?;
    let report = root.join("target/coverage/rust/lcov.info");
    fs::create_dir_all(report.parent().expect("report directory"))?;
    if report.try_exists()? {
        fs::remove_file(&report)?;
    }
    let coverage = Coverage::prepare(root, &root.join("target/coverage-build"))?;
    run(coverage.cargo().args(["test", "--workspace", "--locked"]))?;
    coverage.report(&report)?;
    if fs::metadata(&report)?.len() == 0 {
        return Err(io::Error::other("coverage report is empty"));
    }
    verify_native_coverage(root, &report)?;
    run(tool(root, "crapper")?
        .args(["--use-existing-coverage", "--threshold", CRAP_THRESHOLD])
        .args(&scored))?;
    let workers = scope.workers()?;
    mutation.args([
        "--use-existing-coverage",
        "--mutate-all",
        "--max-workers",
        &workers,
    ]);
    if let Some(rounds) = scope.load_rounds() {
        run(python_script(root, "check-mutation-load.py").args([
            "--workers",
            &workers,
            "--rounds",
            rounds,
            "--timeout-factor",
            MUTATION_TIMEOUT_FACTOR,
        ]))?;
    }
    if paths.is_empty() {
        eprintln!("No changed shipped Rust source to mutate.");
        return Ok(());
    }
    mutation.args(paths);
    verify_worker_baseline(root, &sources)?;
    run(&mut mutation)
}

/// A repository quality script, run by the pinned tools' Python with the workspace and Cargo.
fn python_script(root: &Path, script: &str) -> Command {
    let mut command = Command::new(tool_root(root).join("venv/bin/python"));
    command
        .arg(root.join("tools").join(script))
        .arg(root)
        .arg(env!("CARGO"))
        .current_dir(root);
    command
}

fn verify_worker_baseline(root: &Path, sources: &[PathBuf]) -> io::Result<()> {
    run(python_script(root, "check-mutation-workspace.py")
        .args(sources)
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .env_remove("CARGO_BUILD_BUILD_DIR"))
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
    fs::create_dir(scratch.path().join("tools"))?;
    fs::copy(
        root.join("tools/test-mutation.py"),
        scratch.path().join("tools/test-mutation.py"),
    )?;
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

pub(crate) fn coverage_command(root: &Path, build: &Path) -> Command {
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
    let coverage = Coverage::prepare(workspace, &build)?;
    run(coverage
        .cargo()
        .env("CLIMON2_MUTATION_OBSERVATIONS", &observations)
        .args(["test", "--workspace", "--locked"]))?;
    let report = workspace.join("target/coverage/rust/lcov.info");
    fs::create_dir_all(report.parent().unwrap())?;
    let export = || coverage.report(&report);
    export()?;
    if staged_count(&report)? != 0 {
        return Err(io::Error::other(
            "staged-only function unexpectedly covered before staged execution",
        ));
    }
    let staged = install_instrumented_probe(workspace, &build)?;
    let profile = coverage.profile();
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
    let exec = output(
        Command::new(staged.join(env!("CARGO_PKG_VERSION")).join("climon2-cli"))
            .arg("--staged-exec")
            .env("LLVM_PROFILE_FILE", profile),
    )?;
    if exec.stdout != b"42\n" {
        return Err(io::Error::other(
            "exec-replaced staged binary returned the wrong observation",
        ));
    }
    prove_killed_profile(&staged, profile)?;
    export()?;
    let count = staged_count(&report)?;
    if count != 3 {
        return Err(io::Error::other(format!(
            "normal, exec-replaced and killed staged runs must contribute three coverage hits, got {count}"
        )));
    }
    prove_crapper(root, workspace)?;
    prove_covered_mutations(root, workspace, &observations)?;
    eprintln!(
        "Verified installer-built coverage, ? complexity, threshold {CRAP_THRESHOLD} rejection, and changed/full covered mutations."
    );
    Ok(())
}

fn prove_killed_profile(staged: &Path, profile: &str) -> io::Result<()> {
    use std::{
        io::{BufRead, BufReader},
        process::Stdio,
    };
    let mut child = Command::new(staged.join(env!("CARGO_PKG_VERSION")).join("climon2-cli"))
        .arg("--staged-kill")
        .env("LLVM_PROFILE_FILE", profile)
        .stdout(Stdio::piped())
        .spawn()?;
    let mut line = String::new();
    let observation =
        BufReader::new(child.stdout.take().expect("piped output")).read_line(&mut line);
    child.kill()?;
    child.wait()?;
    observation?;
    if line != "42\n" {
        return Err(io::Error::other(
            "killed staged binary returned the wrong observation",
        ));
    }
    Ok(())
}

fn install_instrumented_probe(workspace: &Path, build: &Path) -> io::Result<PathBuf> {
    let payload = workspace.join("payload");
    fs::create_dir(&payload)?;
    fs::copy(build.join("debug/isolation-cli"), payload.join("climon2"))?;
    for name in climon2_install::version_files() {
        let path = climon2_install::cargo_output(&payload, &name);
        fs::create_dir_all(path.parent().expect("payload file directory"))?;
        fs::copy(build.join("debug/isolation-cli"), path)?;
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
        CRAP_THRESHOLD,
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
    for (fixture, name, score) in [
        (
            "over-limit.rs",
            "over_limit",
            ":complexity 3, :coverage 0.0, :crap 12.0",
        ),
        (
            "between-limits.rs",
            "between_limits",
            ":complexity 2, :coverage 0.0, :crap 6.0",
        ),
    ] {
        prove_rejection(root, workspace, fixture, name, score)?;
    }
    Ok(())
}

fn prove_rejection(
    root: &Path,
    workspace: &Path,
    fixture: &str,
    name: &str,
    score: &str,
) -> io::Result<()> {
    let excessive = workspace.join(fixture);
    fs::copy(root.join("xtask/tests/fixtures").join(fixture), &excessive)?;
    let rejected = tool(root, "crapper")?
        .current_dir(workspace)
        .args(["--use-existing-coverage", "--threshold", CRAP_THRESHOLD])
        .arg(&excessive)
        .output()?;
    if rejected.status.code() != Some(2) {
        return Err(io::Error::other(format!(
            "threshold {CRAP_THRESHOLD} accepted {name} ({score}): {rejected:?}"
        )));
    }
    let metrics = fs::read_to_string(workspace.join(".metrics/crap.edn"))?;
    if !metrics
        .lines()
        .any(|line| line.contains(&format!(":name \"{name}\"")) && line.contains(score))
    {
        return Err(io::Error::other(format!(
            "threshold failure did not score {name} as {score}: {metrics}"
        )));
    }
    fs::remove_file(excessive)
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

    #[cfg(unix)]
    #[test]
    #[ignore = "owned subprocess fixture"]
    fn owned_load_gate_fixture() {
        let root = env::var_os("CLIMON2_LOAD_GATE_FIXTURE").unwrap();
        let scope = match env::var("LOAD_GATE_SCOPE").unwrap().as_str() {
            "complete" => Scope::Complete,
            "host" => Scope::Host,
            "native" => Scope::Native,
            other => panic!("unknown load-gate scope {other}"),
        };
        check(Path::new(&root), false, scope).unwrap();
    }

    #[cfg(unix)]
    fn executable(path: &Path, text: &str) {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A `quality::check` run whose git, coverage and Python quality tools are controlled.
    #[cfg(unix)]
    struct Gate {
        scope: &'static str,
        load_fails: bool,
        changed: bool,
    }

    #[cfg(unix)]
    struct GateRun {
        success: bool,
        output: String,
        /// Each quality tool started, in order, with its arguments.
        events: Vec<String>,
    }

    #[cfg(unix)]
    impl GateRun {
        fn started(&self, tool: &str) -> Option<usize> {
            self.events
                .iter()
                .position(|event| event.split(' ').next() == Some(tool))
        }

        /// The value a quality tool was given for `option`.
        fn option(&self, tool: &str, option: &str) -> &str {
            let event = &self.events[self.started(tool).unwrap()];
            let mut arguments = event.split(' ');
            arguments.find(|argument| *argument == option).unwrap();
            arguments.next().unwrap()
        }
    }

    #[cfg(unix)]
    fn run_gate(gate: Gate) -> GateRun {
        use std::{
            process::Stdio,
            thread,
            time::{Duration, Instant},
        };
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        let source = if gate.scope == "native" {
            format!("crates/climon2-platform/src/process/{}.rs", env::consts::OS)
        } else {
            "crates/climon2-core/src/lib.rs".to_owned()
        };
        fs::create_dir_all(root.join(&source).parent().unwrap()).unwrap();
        fs::write(root.join(&source), "").unwrap();
        fs::write(
            root.join("Cargo.toml"),
            format!(
                "[package]\nname = \"gate-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
             [lib]\npath = \"{source}\"\n[workspace]\n"
            ),
        )
        .unwrap();
        fs::write(
            root.join("Cargo.lock"),
            "version = 4\n[[package]]\nname = \"gate-fixture\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();
        executable(
            &root.join("bin/git"),
            &format!(
                "#!/bin/sh\ncase \"$*\" in\n\
             *crapper*rev-parse*) echo {CRAPPER} ;;\n\
             *mutator*rev-parse*) echo {MUTATOR} ;;\n\
             *'rev-parse --verify'*) echo fixture ;;\n\
             *'ls-files --cached'*) printf '%s\\0' \"$LOAD_GATE_SOURCE\" ;;\n\
             *'diff --name-only'*) if [ \"$LOAD_GATE_CHANGED\" = yes ]; then printf '%s\\0' \"$LOAD_GATE_SOURCE\"; fi ;;\n\
             esac\n"
            ),
        );
        executable(
            &root.join("bin/cargo-llvm-cov"),
            r#"#!/bin/sh
case "$2" in
    --version) echo 'cargo-llvm-cov 0.9.1' ;;
    show-env) echo "LLVM_PROFILE_FILE='$CLIMON2_LOAD_GATE_FIXTURE/target/profile.profraw'" ;;
    report)
        for last do :; done
        printf 'SF:%s\nDA:1,1\n' "$LOAD_GATE_SOURCE" > "$last"
        ;;
esac
"#,
        );
        executable(
            &root.join(".metrics/tools/venv/bin/python"),
            r#"#!/bin/sh
events="$CLIMON2_LOAD_GATE_FIXTURE/events"
case "$*" in
    *check-mutation-load.py*)
        shift
        printf 'load %s\n' "$*" >> "$events"
        if [ "$LOAD_GATE_FAIL" = yes ]; then
            echo 'test flaky_fixture ... FAILED' >&2
            exit 1
        fi
        ;;
    *check-mutation-workspace.py*) echo baseline >> "$events" ;;
    *'-m mutator'*) shift 2; printf 'mutation %s\n' "$*" >> "$events" ;;
esac
"#,
        );
        let output = root.join("output");
        let file = fs::File::create(&output).unwrap();
        let mut path = vec![root.join("bin")];
        path.extend(env::split_paths(&env::var_os("PATH").unwrap()));
        let mut child = Command::new(env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "quality::tests::owned_load_gate_fixture",
                "--nocapture",
            ])
            .env("CLIMON2_LOAD_GATE_FIXTURE", root)
            .env("LOAD_GATE_SCOPE", gate.scope)
            .env("LOAD_GATE_SOURCE", &source)
            .env("LOAD_GATE_CHANGED", if gate.changed { "yes" } else { "no" })
            .env("LOAD_GATE_FAIL", if gate.load_fails { "yes" } else { "no" })
            .env("PATH", env::join_paths(path).unwrap())
            .env_remove("CLIMON2_MUTATION_BASE")
            .stdout(Stdio::from(file.try_clone().unwrap()))
            .stderr(Stdio::from(file))
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!(
                    "gate fixture timed out: {}",
                    fs::read_to_string(&output).unwrap()
                );
            }
            thread::sleep(Duration::from_millis(20));
        };
        let events = fs::read_to_string(root.join("events")).unwrap_or_default();
        GateRun {
            success: status.success(),
            output: fs::read_to_string(output).unwrap(),
            events: events.lines().map(str::to_owned).collect(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn complete_qualification_measures_load_with_mutations_workers_before_mutating() {
        let run = run_gate(Gate {
            scope: "complete",
            load_fails: false,
            changed: true,
        });
        assert!(run.success, "{}", run.output);
        let load = run.started("load").unwrap();
        assert!(load < run.started("mutation").unwrap(), "{:?}", run.events);
        assert_eq!(
            run.option("load", "--workers"),
            run.option("mutation", "--max-workers")
        );
        assert_eq!(
            run.option("load", "--timeout-factor"),
            run.option("mutation", "--timeout-factor")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_load_measurement_prevents_mutation() {
        let run = run_gate(Gate {
            scope: "complete",
            load_fails: true,
            changed: true,
        });
        assert!(!run.success, "{}", run.output);
        assert!(run.output.contains("flaky_fixture"), "{}", run.output);
        assert_eq!(run.started("mutation"), None, "{:?}", run.events);
    }

    #[cfg(unix)]
    #[test]
    fn complete_qualification_measures_load_even_without_mutation_candidates() {
        let run = run_gate(Gate {
            scope: "complete",
            load_fails: true,
            changed: false,
        });
        assert!(!run.success, "{}", run.output);
        assert!(run.output.contains("flaky_fixture"), "{}", run.output);
        let run = run_gate(Gate {
            scope: "complete",
            load_fails: false,
            changed: false,
        });
        assert!(run.success, "{}", run.output);
        assert!(run.started("load").is_some(), "{:?}", run.events);
        assert_eq!(run.started("mutation"), None, "{:?}", run.events);
    }

    /// #135 owner decision: only `cargo xtask check` measures load; `mutate` stays incremental.
    #[cfg(unix)]
    #[test]
    fn mutate_does_not_measure_load() {
        for scope in ["host", "native"] {
            let run = run_gate(Gate {
                scope,
                load_fails: true,
                changed: true,
            });
            assert!(run.success, "{scope}: {}", run.output);
            assert_eq!(run.started("load"), None, "{scope}: {:?}", run.events);
            assert!(
                run.started("mutation").is_some(),
                "{scope}: {:?}",
                run.events
            );
        }
    }

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

    #[test]
    fn native_selection_keeps_uncovered_shared_sources_and_only_the_current_os() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        run(Command::new("git")
            .current_dir(root)
            .args(["init", "--quiet"]))
        .unwrap();
        let sources = [
            ("crates/climon2-platform/src/process.rs", true),
            (
                "crates/climon2-platform/src/process/linux.rs",
                cfg!(target_os = "linux"),
            ),
            (
                "crates/climon2-platform/src/process/macos.rs",
                cfg!(target_os = "macos"),
            ),
            (
                "crates/climon2-platform/src/peer/linux.rs",
                cfg!(target_os = "linux"),
            ),
            (
                "crates/climon2-platform/src/peer/macos.rs",
                cfg!(target_os = "macos"),
            ),
            (
                "crates/climon2-platform/src/runtime/linux.rs",
                cfg!(target_os = "linux"),
            ),
            (
                "crates/climon2-platform/src/runtime/macos.rs",
                cfg!(target_os = "macos"),
            ),
            (
                "crates/climon2-install/src/platform/linux.rs",
                cfg!(target_os = "linux"),
            ),
            (
                "crates/climon2-install/src/platform/macos.rs",
                cfg!(target_os = "macos"),
            ),
        ];
        for (path, _) in sources {
            fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            fs::write(root.join(path), "pub fn never_called() { if true {} }").unwrap();
        }
        let selected = shipped_sources(root).unwrap();
        for (path, active) in sources {
            assert_eq!(selected.contains(&PathBuf::from(path)), active, "{path}");
        }
        let native = Scope::Native.select(selected.clone());
        assert_eq!(
            native,
            selected
                .iter()
                .filter(|path| path.as_path() != Path::new("crates/climon2-platform/src/process.rs"))
                .cloned()
                .collect::<Vec<_>>()
        );
        assert_eq!(Scope::Host.select(selected.clone()), selected);
        commit(root);
        for (path, _) in sources {
            fs::write(root.join(path), "pub fn never_called() { if false {} }").unwrap();
        }
        assert_eq!(changed_sources(root, OsStr::new("HEAD")).unwrap(), selected);
        fs::write(
            root.join("crates/climon2-platform/src/process.rs"),
            "shared-only committed change",
        )
        .unwrap();
        commit(root);
        fs::write(
            root.join("crates/climon2-platform/src/process.rs"),
            "another shared-only change",
        )
        .unwrap();
        assert!(
            Scope::Native
                .select(changed_sources(root, OsStr::new("HEAD")).unwrap())
                .is_empty()
        );
        let report = root.join("coverage.info");
        for (path, active) in sources {
            fs::write(
                &report,
                format!("SF:{}\nDA:1,0\n", root.join(path).display()),
            )
            .unwrap();
            assert_eq!(
                verify_native_coverage(root, &report).is_ok(),
                active,
                "{path}"
            );
        }
    }
}
