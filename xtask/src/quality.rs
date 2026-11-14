use crate::{
    coverage::Coverage,
    mutation_plan::{ChangedItem, changed_items},
};
use std::{
    collections::BTreeSet,
    env,
    ffi::OsStr,
    fs,
    io::{self, Read},
    ops::RangeInclusive,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const CRAPPER: &str = "6fd92c4c9bcc56efb64a2f8743d5a2993bcfd42d";
const MUTATOR: &str = "a5a0cb803b66e0cdb8bf471b3b08af701d424a41";
const LLVM_COV: &str = "0.9.1";
/// Full-host mutation workers, and so the suites the mutation-load measurement runs at once;
/// 8 leaves room for another agent's tests on the shared development host.
const MUTATION_WORKERS: &str = "8";
/// Rounds of the full gate's mutation-load measurement (`cargo xtask check --full`).
const MUTATION_LOAD_ROUNDS: &str = "5";
/// Rounds of the ticket gate's mutation-load measurement (`cargo xtask check`).
const TICKET_MUTATION_LOAD_ROUNDS: &str = "1";
/// A mutant's test run times out, counting as killed, after this multiple of the baseline run.
const MUTATION_TIMEOUT_FACTOR: &str = "10";
/// Owner-set maximum CRAP score for every shipped function (#21, amended to 5).
pub(crate) const CRAP_THRESHOLD: &str = "5";
const SHIPPED: &[&str] = &[
    "crates/climon2-launcher/src",
    "crates/climon2-core/src",
    "crates/climon2-platform/src",
    "crates/climon2-cli/src",
    "crates/climon2-discovery/src",
    "crates/climon2-daemon/src",
    "crates/climon2-client/src",
    "crates/climon2-install/src",
    "crates/climon2-monitor/src",
    "crates/climon2-vt/src",
];
const TEST_COMMAND: &str = r#""$CLIMON2_MUTATION_PYTHON" tools/test-mutation.py"#;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Scope {
    /// `cargo xtask check`: changed functions of host source, after one round of mutation load.
    Ticket,
    /// `cargo xtask check --full`: changed host source files, after five rounds of mutation load.
    Full,
    /// `cargo xtask mutate`: host source.
    Host,
    /// `cargo xtask mutate --native-only`: only host-exclusive native source.
    Native,
}

impl Scope {
    fn select(self, sources: Vec<PathBuf>) -> Vec<PathBuf> {
        sources
            .into_iter()
            .filter(|path| self.keeps(path))
            .collect()
    }

    fn keeps(self, path: &Path) -> bool {
        self != Self::Native || native_source_os(path).is_some()
    }

    fn workers(self) -> io::Result<String> {
        match self {
            Self::Ticket | Self::Full | Self::Host => Ok(MUTATION_WORKERS.to_owned()),
            Self::Native => Ok(std::thread::available_parallelism()?
                .get()
                .min(16)
                .to_string()),
        }
    }

    /// Rounds of the mutation-load measurement required before mutation, if any.
    fn load_rounds(self) -> Option<&'static str> {
        match self {
            Self::Ticket => Some(TICKET_MUTATION_LOAD_ROUNDS),
            Self::Full => Some(MUTATION_LOAD_ROUNDS),
            Self::Host | Self::Native => None,
        }
    }

    /// Changed sources to mutate without `--mutate-all`, since `CLIMON2_MUTATION_BASE`.
    fn changed_targets(self, root: &Path) -> io::Result<Vec<Target>> {
        // Upstream --changed reads only git status and cannot limit itself to shipped crates.
        let base = env::var_os("CLIMON2_MUTATION_BASE").unwrap_or_else(|| "HEAD".into());
        let mut targets = if self == Self::Ticket {
            function_targets(root, &base)?
        } else {
            Target::whole(changed_sources(root, &base)?)
        };
        targets.retain(|target| self.keeps(&target.path));
        Ok(targets)
    }
}

/// One file's mutation selection: changed items, or the whole file when `items` is `None`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Target {
    path: PathBuf,
    items: Option<Vec<ChangedItem>>,
}

impl Target {
    fn whole(paths: Vec<PathBuf>) -> Vec<Self> {
        paths
            .into_iter()
            .map(|path| Self { path, items: None })
            .collect()
    }

    /// The changed items' lines, or `None` for a whole file.
    fn lines(&self) -> Option<BTreeSet<usize>> {
        let items = self.items.as_ref()?;
        Some(
            items
                .iter()
                .flat_map(|item| item.lines.iter().cloned().flatten())
                .collect(),
        )
    }
}

fn line_count(root: &Path, path: &Path) -> io::Result<usize> {
    Ok(fs::read_to_string(root.join(path))?.lines().count())
}

/// The single mutator run, so the full-workspace baseline runs once.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RunPlan {
    /// The targets with a selected mutation site.
    targets: Vec<Target>,
    /// `--lines`, or `None` when every target is a whole file.
    lines: Option<BTreeSet<usize>>,
}

/// `--lines` applies to every file in a run, so it is the union of the lines that hold each
/// target's own selected sites (`sites`), and a target with none is dropped. A file may also
/// mutate a site at a line number another file selected: a safe superset, never less.
fn run_plan(
    targets: Vec<Target>,
    mut sites: impl FnMut(&Target) -> io::Result<BTreeSet<usize>>,
) -> io::Result<RunPlan> {
    if targets.iter().all(|target| target.items.is_none()) {
        return Ok(RunPlan {
            targets,
            lines: None,
        });
    }
    let mut lines = BTreeSet::new();
    let mut kept = Vec::new();
    for target in targets {
        let found = sites(&target)?;
        if !found.is_empty() {
            lines.extend(found);
            kept.push(target);
        }
    }
    Ok(RunPlan {
        targets: kept,
        lines: Some(lines),
    })
}

/// The lines of `target`'s selected mutation sites, from the mutator's own scan, which runs no tests.
fn scanned_sites(root: &Path, target: &Target) -> io::Result<BTreeSet<usize>> {
    let mut scan = tool(root, "mutator");
    scan.args(["--scan", "--no-coverage"]);
    if let Some(lines) = target.lines() {
        scan.args(["--lines", &joined(&lines)]);
    }
    let scanned = output(scan.arg(&target.path))?;
    let prefix = format!("  {}:", target.path.display());
    String::from_utf8_lossy(&scanned.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .map(|rest| {
            rest.split(' ')
                .next()
                .unwrap_or_default()
                .parse()
                .map_err(|error| io::Error::other(format!("mutator scan: {rest}: {error}")))
        })
        .collect()
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
    checked(command, output)
}

/// How long one inspection or compiler run of a release check (nm, readelf,
/// codesign, lipo, clang or cc on one small file) may take before it is killed.
pub(crate) const TOOL_TIMEOUT: Duration = Duration::from_secs(60);

/// Like `output`, with null stdin, but a command still running at `timeout` is
/// killed and reaped, and fails naming its process, so no tool hangs a check.
pub(crate) fn output_within(command: &mut Command, timeout: Duration) -> io::Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = drain(child.stdout.take().expect("piped stdout"));
    let stderr = drain(child.stderr.take().expect("piped stderr"));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            child.wait()?;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "{command:?} (pid {}) did not exit within {timeout:?}; killed and reaped",
                    child.id()
                ),
            ));
        }
        thread::sleep(Duration::from_millis(10));
    };
    let output = Output {
        status,
        stdout: stdout.join().expect("stdout reader")?,
        stderr: stderr.join().expect("stderr reader")?,
    };
    checked(command, output)
}

fn drain(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes).map(|_| bytes)
    })
}

/// A finished command's output, or its failure with its diagnostics.
fn checked(command: &Command, output: Output) -> io::Result<Output> {
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

/// Fails unless the pinned quality tools are installed; run once before using [`tool`].
fn verify_tools(root: &Path) -> io::Result<()> {
    let tools = tool_root(root);
    if !tools.join("venv/bin/python").is_file() {
        return Err(io::Error::other(
            "quality tools are missing; run cargo xtask setup-quality",
        ));
    }
    for (package, revision) in [("crapper", CRAPPER), ("mutator", MUTATOR)] {
        verify_checkout(&tools.join(package), revision)?;
    }
    Ok(())
}

fn tool(root: &Path, name: &str) -> Command {
    let mut command = Command::new(tool_root(root).join("venv/bin/python"));
    command.args(["-m", name]).current_dir(root);
    command
}

fn mutation_command(root: &Path) -> Command {
    let mut command = tool(root, "mutator");
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
    command
}

fn changed_sources(root: &Path, base: &OsStr) -> io::Result<Vec<PathBuf>> {
    let (_, files) = changed_files(root, base)?;
    let paths: BTreeSet<_> = files.into_iter().map(|(path, _)| path).collect();
    Ok(paths.into_iter().collect())
}

/// A changed source and its path at the base, which is `None` when the file is new.
type ChangedFile = (PathBuf, Option<PathBuf>);

/// Changed shipped sources since the merge base of `base` and `HEAD`, each with its base
/// path unless git reports it as new.
/// Renames compare against their old path; added, copied and untracked files are new.
fn changed_files(root: &Path, base: &OsStr) -> io::Result<(String, Vec<ChangedFile>)> {
    let commit = output(
        Command::new("git")
            .current_dir(root)
            .args(["rev-parse", "--verify", "--end-of-options"])
            .arg(format!("{}^{{commit}}", base.to_string_lossy())),
    )?;
    let commit = String::from_utf8(commit.stdout).map_err(io::Error::other)?;
    // A base that moved on after the branch point must not bring in its own changes.
    let merge_base =
        output(
            Command::new("git")
                .current_dir(root)
                .args(["merge-base", commit.trim(), "HEAD"]),
        )?;
    let commit = String::from_utf8(merge_base.stdout)
        .map_err(io::Error::other)?
        .trim()
        .to_owned();
    let changed = output(
        Command::new("git")
            .current_dir(root)
            .args(["diff", "--name-status", "-z", "-M", "--diff-filter=ACMR"])
            .arg(&commit)
            .arg("--")
            .args(SHIPPED),
    )?;
    let untracked = output(
        Command::new("git")
            .current_dir(root)
            .args(["ls-files", "--others", "--exclude-standard", "-z", "--"])
            .args(SHIPPED),
    )?;
    let mut files = name_status(&changed.stdout)?;
    for path in nul_separated(&untracked.stdout)? {
        files.push((path, None));
    }
    files.retain(|(path, _)| is_active_source(root, path));
    files.sort();
    Ok((commit, files))
}

/// `git diff --name-status -z` records as (path, base path).
fn name_status(bytes: &[u8]) -> io::Result<Vec<ChangedFile>> {
    let mut fields = nul_separated(bytes)?.into_iter();
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        let status = status.to_string_lossy().into_owned();
        let first = fields
            .next()
            .ok_or_else(|| io::Error::other("truncated git diff"))?;
        let file = match status.chars().next() {
            Some('M') => (first.clone(), Some(first)),
            Some('R') => {
                let new = fields
                    .next()
                    .ok_or_else(|| io::Error::other("truncated rename"))?;
                (new, Some(first))
            }
            Some('C') => (
                fields
                    .next()
                    .ok_or_else(|| io::Error::other("truncated copy"))?,
                None,
            ),
            _ => (first, None),
        };
        files.push(file);
    }
    Ok(files)
}

fn nul_separated(bytes: &[u8]) -> io::Result<Vec<PathBuf>> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            Ok(PathBuf::from(
                std::str::from_utf8(path).map_err(io::Error::other)?,
            ))
        })
        .collect()
}

/// The ticket gate's selection: each changed file's changed items; new files whole.
fn function_targets(root: &Path, base: &OsStr) -> io::Result<Vec<Target>> {
    let (commit, files) = changed_files(root, base)?;
    let mut targets = Vec::new();
    for (path, base_path) in files {
        let items = match base_path {
            None => None,
            Some(base_path) => Some(file_changes(root, &commit, &base_path, &path)?),
        };
        if items.as_ref().is_none_or(|items| !items.is_empty()) {
            targets.push(Target { path, items });
        }
    }
    Ok(targets)
}

fn file_changes(
    root: &Path,
    commit: &str,
    base_path: &Path,
    path: &Path,
) -> io::Result<Vec<ChangedItem>> {
    let base = output(
        Command::new("git")
            .current_dir(root)
            .args(["cat-file", "-p"])
            .arg(format!("{commit}:{}", base_path.display())),
    )?;
    changed_items(
        &String::from_utf8_lossy(&base.stdout),
        &fs::read_to_string(root.join(path))?,
    )
    .map_err(|error| io::Error::other(format!("{}: {error}", path.display())))
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
        | Some("crates/climon2-install/src/platform/linux.rs")
        | Some("crates/climon2-client/src/library/linux.rs") => Some("linux"),
        Some("crates/climon2-platform/src/process/macos.rs")
        | Some("crates/climon2-platform/src/peer/macos.rs")
        | Some("crates/climon2-platform/src/runtime/macos.rs")
        | Some("crates/climon2-install/src/platform/macos.rs")
        | Some("crates/climon2-client/src/library/macos.rs") => Some("macos"),
        _ => None,
    }
}

fn native_source_is_active(path: &Path) -> bool {
    native_source_os(path).is_none_or(|os| os == env::consts::OS)
}

fn source_paths(root: &Path, outputs: &[Vec<u8>]) -> io::Result<Vec<PathBuf>> {
    let mut paths = BTreeSet::new();
    for bytes in outputs {
        for path in nul_separated(bytes)? {
            if is_active_source(root, &path) {
                paths.insert(path);
            }
        }
    }
    Ok(paths.into_iter().collect())
}

fn is_active_source(root: &Path, path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "rs")
        && root.join(path).is_file()
        && native_source_is_active(path)
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

fn mutation_targets(root: &Path, all: bool, scope: Scope) -> io::Result<Vec<Target>> {
    if all {
        Ok(Target::whole(scope.select(shipped_sources(root)?)))
    } else {
        scope.changed_targets(root)
    }
}

/// What the gate would mutate: each target, its line counts, and the single run's lines.
fn plan_text(root: &Path, targets: &[Target], run: &RunPlan) -> io::Result<String> {
    if targets.is_empty() {
        return Ok("No changed shipped Rust source to mutate.\n".to_owned());
    }
    let mut text = String::new();
    for target in targets {
        let total = line_count(root, &target.path)?;
        let path = target.path.display();
        let (Some(items), Some(lines)) = (&target.items, target.lines()) else {
            text += &format!("{path}: whole file, {total} lines\n");
            continue;
        };
        text += &format!(
            "{path}: {} changed items, {} of {total} lines\n",
            items.len(),
            lines.len()
        );
        for item in items {
            let spans: Vec<_> = item.lines.iter().map(span).collect();
            text += &format!("  {}  lines {}\n", item.name, spans.join(","));
        }
    }
    let files = run.targets.len();
    text += &match &run.lines {
        None => format!("One mutator run over {files} files, whole files.\n"),
        Some(_) if files == 0 => {
            "No selected line holds a mutation site; nothing to mutate.\n".to_owned()
        }
        Some(lines) => format!(
            "One mutator run over {files} files, --lines {} \
             (the union of the lines holding each file's selected mutation sites).\n",
            joined(lines)
        ),
    };
    Ok(text)
}

fn span(lines: &RangeInclusive<usize>) -> String {
    if lines.start() == lines.end() {
        lines.start().to_string()
    } else {
        format!("{}-{}", lines.start(), lines.end())
    }
}

/// Prints what the gate would mutate, without running anything else.
pub fn plan(root: &Path, all: bool, scope: Scope) -> io::Result<()> {
    let targets = mutation_targets(root, all, scope)?;
    if targets.iter().any(|target| target.items.is_some()) {
        verify_tools(root)?;
    }
    let run = run_plan(targets.clone(), |target| scanned_sites(root, target))?;
    print!("{}", plan_text(root, &targets, &run)?);
    Ok(())
}

pub fn check(root: &Path, all: bool, scope: Scope) -> io::Result<()> {
    let sources = shipped_sources(root)?;
    let scored = scope.select(sources.clone());
    let targets = mutation_targets(root, all, scope)?;
    if scope == Scope::Native && targets.is_empty() {
        eprintln!("No selected native Rust source to qualify; shared source is checked locally.");
        return Ok(());
    }
    verify_llvm_cov()?;
    verify_tools(root)?;
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
    run(tool(root, "crapper")
        .args(["--use-existing-coverage", "--threshold", CRAP_THRESHOLD])
        .args(&scored))?;
    let workers = scope.workers()?;
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
    if targets.is_empty() {
        eprintln!("No changed shipped Rust source to mutate.");
        return Ok(());
    }
    let run = run_plan(targets, |target| scanned_sites(root, target))?;
    if run.targets.is_empty() {
        eprintln!("No selected line holds a mutation site; nothing to mutate.");
        return Ok(());
    }
    verify_worker_baseline(root, &sources)?;
    mutate(root, &workers, run)
}

/// One mutator run for every target, so the full-workspace baseline runs once.
fn mutate(root: &Path, workers: &str, run_plan: RunPlan) -> io::Result<()> {
    clear_mutation_history(root)?;
    let mut mutation = mutation_command(root);
    mutation.args([
        "--use-existing-coverage",
        "--mutate-all",
        "--max-workers",
        workers,
    ]);
    if let Some(lines) = &run_plan.lines {
        mutation.args(["--lines", &joined(lines)]);
    }
    run(mutation.args(run_plan.targets.into_iter().map(|target| target.path)))
}

/// Removes the mutator's per-file history, so no outcome of an earlier run is carried forward and
/// the result depends only on this run.
fn clear_mutation_history(root: &Path) -> io::Result<()> {
    match fs::remove_dir_all(root.join(".metrics/mutate")) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn joined(lines: &BTreeSet<usize>) -> String {
    lines
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(",")
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
    verify_tools(root)?;
    let mut command = mutation_command(root);
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
    verify_tools(root)?;
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
    run(tool(root, "crapper").current_dir(workspace).args([
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
    let rejected = tool(root, "crapper")
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
        run(mutation_command(root)
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
            "ticket" => Scope::Ticket,
            "full" => Scope::Full,
            "host" => Scope::Host,
            "native" => Scope::Native,
            other => panic!("unknown load-gate scope {other}"),
        };
        check(Path::new(&root), false, scope).unwrap();
    }

    /// Only an external `cp` ever opens the script for writing. A write
    /// descriptor held here could be inherited by a concurrent test's fork
    /// until its exec, and executing the script would then fail with ETXTBSY.
    #[cfg(unix)]
    fn executable(path: &Path, text: &str) {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let staged = path.with_extension("staged");
        fs::write(&staged, text).unwrap();
        let copied = Command::new("cp").arg(&staged).arg(path).status().unwrap();
        assert!(copied.success(), "cp {}: {copied}", path.display());
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A `quality::check` run whose git, coverage and Python quality tools are controlled.
    #[cfg(unix)]
    #[derive(Default)]
    struct Gate {
        scope: &'static str,
        load_fails: bool,
        changed: bool,
        /// A three-line shipped source the diff adds, mutated whole beside the changed one.
        added: bool,
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
        fs::write(root.join(&source), "pub fn changed() -> u8 { 1 }\n").unwrap();
        let added = if gate.added {
            let added = "crates/climon2-core/src/added.rs";
            fs::write(root.join(added), "pub fn added() -> u8 {\n    1\n}\n").unwrap();
            added
        } else {
            ""
        };
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
             *crapper*rev-parse*) echo {CRAPPER}; echo verify >> \"$CLIMON2_LOAD_GATE_FIXTURE/events\" ;;\n\
             *mutator*rev-parse*) echo {MUTATOR} ;;\n\
             *'rev-parse --verify'*) echo fixture ;;\n\
             *merge-base*) echo fixture ;;\n\
             *'ls-files --cached'*) printf '%s\\0' \"$LOAD_GATE_SOURCE\" $LOAD_GATE_ADDED ;;\n\
             *'diff --name-status'*) if [ \"$LOAD_GATE_CHANGED\" = yes ]; then printf 'M\\0%s\\0' \"$LOAD_GATE_SOURCE\"; fi; \
             if [ -n \"$LOAD_GATE_ADDED\" ]; then printf 'A\\0%s\\0' \"$LOAD_GATE_ADDED\"; fi ;;\n\
             *'cat-file'*) echo 'pub fn changed() -> u8 {{ 0 }}' ;;\n\
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
    *'-m mutator --scan'*)
        lines=1,3
        previous=
        for last do
            if [ "$previous" = --lines ]; then lines=$last; fi
            previous=$last
        done
        printf 'scan %s\n' "$last" >> "$events"
        for site in 1 3; do
            case ",$lines," in *",$site,"*) printf '  %s:%s + -> -  [defn/f]\n' "$last" "$site" ;; esac
        done
        ;;
    *'-m mutator'*)
        if [ -e .metrics/mutate ]; then echo stale-history >> "$events"; fi
        shift 2
        printf 'mutation %s\n' "$*" >> "$events"
        ;;
esac
"#,
        );
        // An earlier run's outcomes, which the mutator would carry forward into this one.
        fs::create_dir_all(root.join(".metrics/mutate")).unwrap();
        fs::write(root.join(".metrics/mutate/history.edn"), "{}\n").unwrap();
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
            .env("LOAD_GATE_ADDED", added)
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
            scope: "full",
            load_fails: false,
            changed: true,
            added: false,
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
            scope: "full",
            load_fails: true,
            changed: true,
            added: false,
        });
        assert!(!run.success, "{}", run.output);
        assert!(run.output.contains("flaky_fixture"), "{}", run.output);
        assert_eq!(run.started("mutation"), None, "{:?}", run.events);
    }

    #[cfg(unix)]
    #[test]
    fn complete_qualification_measures_load_even_without_mutation_candidates() {
        let run = run_gate(Gate {
            scope: "full",
            load_fails: true,
            changed: false,
            added: false,
        });
        assert!(!run.success, "{}", run.output);
        assert!(run.output.contains("flaky_fixture"), "{}", run.output);
        let run = run_gate(Gate {
            scope: "full",
            load_fails: false,
            changed: false,
            added: false,
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
                added: false,
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

    /// 2026-11-13 owner decision: the full gate keeps five load rounds and whole-file mutation.
    #[cfg(unix)]
    #[test]
    fn the_full_gate_measures_five_rounds_and_mutates_whole_changed_files() {
        let run = run_gate(Gate {
            scope: "full",
            load_fails: false,
            changed: true,
            added: false,
        });
        assert!(run.success, "{}", run.output);
        assert_eq!(run.option("load", "--rounds"), "5");
        let mutation = &run.events[run.started("mutation").unwrap()];
        assert!(!mutation.contains("--lines"), "{mutation}");
        assert!(
            mutation.ends_with(" crates/climon2-core/src/lib.rs"),
            "{mutation}"
        );
    }

    /// 2026-11-13 owner decision: the ticket gate measures one round and mutates changed functions.
    #[cfg(unix)]
    #[test]
    fn the_ticket_gate_measures_one_round_and_mutates_only_changed_function_lines() {
        let run = run_gate(Gate {
            scope: "ticket",
            load_fails: false,
            changed: true,
            added: false,
        });
        assert!(run.success, "{}", run.output);
        assert_eq!(run.option("load", "--rounds"), "1");
        assert!(
            run.started("load") < run.started("mutation"),
            "{:?}",
            run.events
        );
        assert_eq!(run.option("mutation", "--lines"), "1");
        let run = run_gate(Gate {
            scope: "ticket",
            load_fails: false,
            changed: false,
            added: false,
        });
        assert!(run.success, "{}", run.output);
        assert_eq!(run.option("load", "--rounds"), "1");
        assert_eq!(run.started("mutation"), None, "{:?}", run.events);
    }

    /// Review 7 and 8: one mutator run (one workspace baseline) takes every target, with the union
    /// of the lines holding each target's selected sites; the quality tools are verified once.
    #[cfg(unix)]
    #[test]
    fn the_ticket_gate_mutates_every_target_in_one_run_after_one_tool_check() {
        let run = run_gate(Gate {
            scope: "ticket",
            changed: true,
            added: true,
            ..Gate::default()
        });
        assert!(run.success, "{}", run.output);
        let checks = run.events.iter().filter(|event| *event == "verify").count();
        assert_eq!(checks, 1, "{:?}", run.events);
        let mutations: Vec<_> = run
            .events
            .iter()
            .filter(|event| event.starts_with("mutation "))
            .collect();
        assert_eq!(mutations.len(), 1, "{:?}", run.events);
        assert_eq!(run.option("mutation", "--lines"), "1,3");
        assert!(
            mutations[0]
                .ends_with(" crates/climon2-core/src/added.rs crates/climon2-core/src/lib.rs"),
            "{}",
            mutations[0]
        );
    }

    /// #185 follow-up: gate results depend only on the current run, so every mutation run starts
    /// without the history an earlier run in the same worktree left behind.
    #[cfg(unix)]
    #[test]
    fn every_mutation_run_starts_without_an_earlier_runs_history() {
        for scope in ["ticket", "full", "host", "native"] {
            let run = run_gate(Gate {
                scope,
                changed: true,
                ..Gate::default()
            });
            assert!(run.success, "{scope}: {}", run.output);
            assert!(
                run.started("mutation").is_some(),
                "{scope}: {:?}",
                run.events
            );
            assert_eq!(
                run.started("stale-history"),
                None,
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

    /// #75 bounded tool runs: a tool still running at its deadline is killed
    /// and reaped, and the run fails naming its process instead of hanging.
    #[cfg(unix)]
    #[test]
    fn output_within_kills_and_reaps_a_tool_that_outlives_its_deadline() {
        let started = Instant::now();
        let error = output_within(Command::new("sleep").arg("600"), Duration::from_secs(1))
            .unwrap_err()
            .to_string();
        assert!(started.elapsed() < Duration::from_secs(60), "{error}");
        assert!(
            error.ends_with("did not exit within 1s; killed and reaped"),
            "{error}"
        );
        let pid: libc::pid_t = error
            .split_once("(pid ")
            .and_then(|(_, rest)| rest.split_once(')'))
            .map(|(pid, _)| pid.parse().unwrap())
            .unwrap();
        // SAFETY: signal 0 only asks whether the process still exists.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "the tool survives");
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }

    /// #75 bounded tool runs: a tool that finishes in time yields its output,
    /// and one that fails reports its diagnostics.
    #[cfg(unix)]
    #[test]
    fn output_within_returns_a_finished_tools_output_or_its_failure() {
        let timeout = Duration::from_secs(60);
        let output = output_within(Command::new("echo").arg("listed"), timeout).unwrap();
        assert_eq!(output.stdout, b"listed\n");
        let error = output_within(
            Command::new("sh").args(["-c", "echo diagnosis >&2; exit 3"]),
            timeout,
        )
        .unwrap_err();
        assert!(error.to_string().ends_with(": diagnosis\n"), "{error}");
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

    fn git(root: &Path, args: &[&str]) {
        run(Command::new("git").current_dir(root).args(args)).unwrap();
    }

    /// Ticket gate: functions changed since the base, compared through renames; new files whole.
    #[test]
    fn ticket_selection_scopes_changed_functions_through_renames_and_keeps_new_files_whole() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        git(root, &["init", "--quiet"]);
        let edited = "crates/climon2-cli/src/lib.rs";
        let renamed = "crates/climon2-core/src/old.rs";
        let comment = "crates/climon2-vt/src/lib.rs";
        let base_source = "pub fn kept() -> u8 {\n    1\n}\n\npub fn edited() -> u8 {\n    2\n}\n";
        for path in [edited, renamed, comment] {
            fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            fs::write(root.join(path), base_source).unwrap();
        }
        commit(root);
        let changed = base_source.replace("    2", "    3");
        fs::write(root.join(edited), &changed).unwrap();
        git(root, &["mv", renamed, "crates/climon2-core/src/new.rs"]);
        fs::write(root.join("crates/climon2-core/src/new.rs"), &changed).unwrap();
        fs::write(
            root.join(comment),
            format!("// Only a comment.\n{base_source}"),
        )
        .unwrap();
        commit(root);
        let untracked = "crates/climon2-daemon/src/lib.rs";
        fs::create_dir_all(root.join(untracked).parent().unwrap()).unwrap();
        fs::write(root.join(untracked), "pub fn fresh() {}\n").unwrap();

        let edited_function = Some(vec![ChangedItem {
            name: "edited".to_owned(),
            lines: vec![5..=7],
        }]);
        assert_eq!(
            function_targets(root, OsStr::new("HEAD~1")).unwrap(),
            vec![
                Target {
                    path: edited.into(),
                    items: edited_function.clone(),
                },
                Target {
                    path: "crates/climon2-core/src/new.rs".into(),
                    items: edited_function,
                },
                Target {
                    path: untracked.into(),
                    items: None,
                },
            ]
        );
        assert_eq!(
            function_targets(root, OsStr::new("HEAD"))
                .unwrap()
                .iter()
                .map(|target| target.items.clone())
                .collect::<Vec<_>>(),
            vec![None]
        );
    }

    /// Review 6: a base that moved on after the branch point does not bring in its own changes.
    #[test]
    fn changes_are_measured_from_the_merge_base() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        git(root, &["init", "--quiet"]);
        let cli = "crates/climon2-cli/src/lib.rs";
        let core = "crates/climon2-core/src/lib.rs";
        for path in [cli, core] {
            fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            fs::write(root.join(path), "pub fn f() -> u8 {\n    1\n}\n").unwrap();
        }
        commit(root);
        git(root, &["branch", "branch-point"]);
        fs::write(root.join(core), "pub fn f() -> u8 {\n    2\n}\n").unwrap();
        commit(root);
        git(root, &["branch", "moved-on"]);
        git(root, &["reset", "--quiet", "--hard", "branch-point"]);
        fs::write(root.join(cli), "pub fn f() -> u8 {\n    3\n}\n").unwrap();
        commit(root);

        assert_eq!(
            changed_sources(root, OsStr::new("moved-on")).unwrap(),
            vec![PathBuf::from(cli)]
        );
        assert_eq!(
            function_targets(root, OsStr::new("moved-on"))
                .unwrap()
                .into_iter()
                .map(|target| target.path)
                .collect::<Vec<_>>(),
            vec![PathBuf::from(cli)]
        );
    }

    /// Review 7: the run's lines are the union of the lines holding each target's own selected
    /// sites; a target without any is dropped, and whole-file-only runs need no `--lines`.
    #[test]
    fn the_single_run_unions_site_lines_and_drops_targets_without_sites() {
        let scoped = |path: &str| Target {
            path: path.into(),
            items: Some(vec![ChangedItem {
                name: "f".to_owned(),
                lines: vec![1..=9],
            }]),
        };
        let whole = Target {
            path: "new.rs".into(),
            items: None,
        };
        let sites = |target: &Target| -> io::Result<BTreeSet<usize>> {
            Ok(match target.path.to_str().unwrap() {
                "a.rs" => BTreeSet::from([2, 7]),
                "b.rs" => BTreeSet::new(),
                _ => BTreeSet::from([4]),
            })
        };
        let plan = run_plan(vec![scoped("a.rs"), scoped("b.rs"), whole.clone()], sites).unwrap();
        assert_eq!(plan.lines, Some(BTreeSet::from([2, 4, 7])));
        assert_eq!(
            plan.targets
                .iter()
                .map(|target| target.path.clone())
                .collect::<Vec<_>>(),
            vec![PathBuf::from("a.rs"), PathBuf::from("new.rs")]
        );
        let unscanned =
            |_: &Target| -> io::Result<BTreeSet<usize>> { panic!("whole files need no scan") };
        let plan = run_plan(vec![whole.clone()], unscanned).unwrap();
        assert_eq!(plan.lines, None);
        assert_eq!(plan.targets, vec![whole]);
        assert!(
            run_plan(vec![scoped("b.rs")], sites)
                .unwrap()
                .targets
                .is_empty()
        );
    }

    /// Review 9: the printed plan names whole-file and item targets, their line counts and the run.
    #[test]
    fn the_plan_lists_each_target_and_the_single_runs_lines() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        fs::write(root.join("new.rs"), "a\nb\nc\n").unwrap();
        fs::write(root.join("old.rs"), "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n").unwrap();
        let targets = vec![
            Target {
                path: "new.rs".into(),
                items: None,
            },
            Target {
                path: "old.rs".into(),
                items: Some(vec![
                    ChangedItem {
                        name: "W::f".to_owned(),
                        lines: vec![2..=3, 5..=5],
                    },
                    ChangedItem {
                        name: "LIMIT".to_owned(),
                        lines: vec![9..=9],
                    },
                ]),
            },
        ];
        let run = RunPlan {
            targets: targets.clone(),
            lines: Some(BTreeSet::from([1, 3, 9])),
        };
        assert_eq!(
            plan_text(root, &targets, &run).unwrap(),
            "\
new.rs: whole file, 3 lines
old.rs: 2 changed items, 4 of 10 lines
  W::f  lines 2-3,5
  LIMIT  lines 9
One mutator run over 2 files, --lines 1,3,9 \
(the union of the lines holding each file's selected mutation sites).
"
        );
        let whole = RunPlan {
            targets: targets[..1].to_vec(),
            lines: None,
        };
        assert_eq!(
            plan_text(root, &targets[..1], &whole).unwrap(),
            "new.rs: whole file, 3 lines\nOne mutator run over 1 files, whole files.\n"
        );
        let nothing = RunPlan {
            targets: Vec::new(),
            lines: Some(BTreeSet::new()),
        };
        assert_eq!(
            plan_text(root, &targets[1..], &nothing).unwrap(),
            "\
old.rs: 2 changed items, 4 of 10 lines
  W::f  lines 2-3,5
  LIMIT  lines 9
No selected line holds a mutation site; nothing to mutate.
"
        );
        assert_eq!(
            plan_text(root, &[], &nothing).unwrap(),
            "No changed shipped Rust source to mutate.\n"
        );
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
            (
                "crates/climon2-client/src/library/linux.rs",
                cfg!(target_os = "linux"),
            ),
            (
                "crates/climon2-client/src/library/macos.rs",
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
