use crate::{
    coverage::Coverage,
    mutation_plan::{ChangedItem, Changes, changed_items},
    mutation_slice::Selection,
};
use std::{
    collections::{BTreeSet, HashMap},
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read},
    ops::RangeInclusive,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const CRAPPER: &str = "6fd92c4c9bcc56efb64a2f8743d5a2993bcfd42d";
/// The owner's fork of unclebob/mutator: upstream a5a0cb80 plus its Windows fixes. A typed
/// test command's `$NAME` is expanded past cmd.exe, a timed-out command's tree is killed,
/// worker trees past the 260-character path limit are deleted, and a mutant of a CRLF
/// source is written byte for byte.
const MUTATOR: &str = "7dc09821d8ab5d077a90fbc4e9ad8c3613a9ea49";
/// Each pinned quality tool, the repository it is cloned from, and its revision.
const TOOLS: [(&str, &str, &str); 2] = [
    (
        "crapper",
        "https://github.com/unclebob/crapper.git",
        CRAPPER,
    ),
    (
        "mutator",
        "https://github.com/jackgeek/mutator.git",
        MUTATOR,
    ),
];
const LLVM_COV: &str = "0.9.1";
/// Full-host mutation workers, and so the suites the mutation-load measurement runs at once;
/// 8 leaves room for another agent's tests on the shared development host.
const MUTATION_WORKERS: &str = "8";
/// Caps the native gate's workers, which otherwise number the CPUs up to 16. Windows-only
/// CRAP and mutation run on the development VM while Windows support is built
/// (docs/for-humans/windows/quality.md, "Running the gate locally"), and 16 workers'
/// builds do not fit in its memory.
const NATIVE_WORKERS_VARIABLE: &str = "CLIMON2_NATIVE_MUTATION_WORKERS";
/// Caps the host gates' workers (`cargo xtask check` and `cargo xtask mutate`), which
/// otherwise number `MUTATION_WORKERS`. The owner's Linux runner shares its PC with the
/// Windows VM (docs/for-humans/quality-runners.md, "Sharing the PC").
const HOST_WORKERS_VARIABLE: &str = "CLIMON2_MUTATION_WORKERS";
/// Limits each mutation worker's Cargo to this many build jobs and test threads, which
/// otherwise number the CPUs. It applies to the stage-timeout measurement too, so the
/// timeouts fit the workers. The owner's runners set it so that two gates sharing a PC
/// do not starve each other's timing tests (docs/for-humans/quality-runners.md,
/// "Sharing the PC").
const WORKER_JOBS_VARIABLE: &str = "CLIMON2_MUTATION_WORKER_JOBS";
/// Rounds of the full gate's mutation-load measurement (`cargo xtask check --full`).
const MUTATION_LOAD_ROUNDS: &str = "5";
/// Rounds of the ticket gate's mutation-load measurement (`cargo xtask check`).
const TICKET_MUTATION_LOAD_ROUNDS: &str = "1";
/// A mutant's test run times out, counting as killed, after this multiple of the baseline run;
/// each earlier test stage likewise after this multiple of its own baseline run.
/// `tools/test-mutation.py` ends a mutant's whole run after this multiple of the suite's
/// unloaded run, so that it, not the mutator, kills a hung run with every process the run
/// started (#401).
const MUTATION_TIMEOUT_FACTOR: &str = "10";
/// Each earlier test stage's timeout, and the suite's, measured before the load rounds that
/// rehearse the stages and read by the mutation workers.
const STAGE_TIMEOUTS: &str = "target/mutation-stage-timeouts.json";
/// Owner-set maximum CRAP score for every shipped function (#21, amended to 5).
pub(crate) const CRAP_THRESHOLD: &str = "5";
pub(crate) const SHIPPED: &[&str] = &[
    "crates/climon2-launcher/src",
    "crates/climon2-core/src",
    "crates/climon2-platform/src",
    "crates/climon2-cli/src",
    "crates/climon2-discovery/src",
    "crates/climon2-daemon/src",
    "crates/climon2-client/src",
    "crates/climon2-install/src",
    "crates/climon2-monitor/src",
    "crates/climon2-embed/src",
    "crates/climon2-vt/src",
    "crates/climon2-link/src",
];
/// The test runs that make up the workspace suite on Windows, where only part of the
/// workspace builds so far: as the Windows Test job runs them.
pub(crate) const WINDOWS_SUITE: &[&[&str]] = &[
    // First: it takes seconds, so a mutant of the installer dies before the long runs.
    &["-p", "climon2-install"],
    &["-p", "climon2-core", "-p", "climon2-vt"],
    &[
        "-p",
        "climon2-platform",
        "--lib",
        "--test",
        "home_windows",
        "--test",
        "private_windows",
        "--test",
        "cross_account_windows",
        "--test",
        "presentation_windows",
        "--test",
        "connection_windows",
        "--test",
        "runtime_windows",
        "--test",
        "process_windows",
        "--test",
        "pty_windows",
        "--test",
        "command_windows",
        "--test",
        "notifier_windows",
        "--test",
        "native_path_windows",
        "--test",
        "terminal_windows",
        "--test",
        "handoff_windows",
    ],
    // Builds the daemon binary, which the CLI's exec tests start.
    &["-p", "climon2-daemon", "--test", "windows"],
    // Builds the client binary and library, which the CLI's attach, kill and
    // smoke tests start.
    &["-p", "climon2-client", "--test", "library_windows"],
    // Builds the monitor binary, which the CLI's monitor tests start; client
    // embedding's own tests run with it.
    &["-p", "climon2-monitor", "-p", "climon2-embed"],
    &["-p", "climon2-launcher"],
    // The CLI's tests but the end-to-end scenarios #328 ported
    // (docs/for-humans/windows/qualification.md), which the Windows Test job
    // runs: none is the only test covering a registered Windows file, so they
    // stay out of each mutant's run (owner decision, 2026-11-22).
    &[
        "-p",
        "climon2-cli",
        "--lib",
        "--bins",
        "--test",
        "attach_deadline_windows",
        "--test",
        "exec_windows",
        "--test",
        "host_models_windows",
        "--test",
        "list_windows",
        "--test",
        "monitor_windows",
        "--test",
        "packaged_windows",
    ],
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Scope {
    /// `cargo xtask check`: changed functions of host source, after one round of mutation load.
    Ticket,
    /// `cargo xtask check --full`: changed host source files, after five rounds of mutation load.
    Full,
    /// `cargo xtask mutate`: changed functions of host source.
    Host,
    /// `cargo xtask mutate --native-only`: changed functions of only host-exclusive native source.
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
        self.workers_capped(env::var_os(self.workers_variable()))
    }

    /// The variable that caps this gate's mutation workers.
    fn workers_variable(self) -> &'static str {
        match self {
            Self::Native => NATIVE_WORKERS_VARIABLE,
            Self::Ticket | Self::Full | Self::Host => HOST_WORKERS_VARIABLE,
        }
    }

    /// The gate's workers: `cap` when it is given; otherwise 8 for the host gates and the
    /// CPUs up to 16 for the native gate.
    fn workers_capped(self, cap: Option<OsString>) -> io::Result<String> {
        match (self, cap) {
            (_, Some(cap)) => cap
                .to_str()
                .and_then(|text| text.parse::<u16>().ok())
                .filter(|&count| count > 0)
                .map(|count| count.to_string())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "{} must be a positive whole number, not {cap:?}",
                            self.workers_variable()
                        ),
                    )
                }),
            (Self::Ticket | Self::Full | Self::Host, None) => Ok(MUTATION_WORKERS.to_owned()),
            (Self::Native, None) => Ok(std::thread::available_parallelism()?
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
        let base = mutation_base();
        // Only the full gate mutates whole changed files (#445). In the own-hardware gates a
        // changed test also selects the functions it names, so a weakened test cannot pass (#449).
        let mut targets = if self == Self::Full {
            Target::whole(changed_sources(root, &base)?)
        } else {
            function_targets(root, &base, self != Self::Ticket)?
        };
        targets.retain(|target| self.keeps(&target.path));
        Ok(targets)
    }

    /// Whether the native gate has a selected native source to qualify: one it mutates, or,
    /// when it selects changes, any changed one, which CRAP scores even when its changes
    /// select no mutant (#449).
    fn qualifies(self, root: &Path, selection: Selection, targets: &[Target]) -> io::Result<bool> {
        Ok(!targets.is_empty()
            || (!selection.whole_files()
                && !self
                    .select(changed_sources(root, &mutation_base())?)
                    .is_empty()))
    }
}

/// The base that changes are measured from. Upstream --changed reads only git status and
/// cannot limit itself to shipped crates.
fn mutation_base() -> OsString {
    env::var_os("CLIMON2_MUTATION_BASE").unwrap_or_else(|| "HEAD".into())
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

/// A target of the single run and its mutants, before coverage drops the uncovered ones.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Planned {
    target: Target,
    /// Mutants on the target's own selected lines.
    selected: usize,
    /// Mutants the run makes in this file: `selected`, and any on a line another file selected.
    mutants: usize,
}

/// The single mutator run, so the full-workspace baseline runs once.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RunPlan {
    /// The targets with a selected mutation site.
    targets: Vec<Planned>,
    /// `--lines`, or `None` when every target is a whole file.
    lines: Option<BTreeSet<usize>>,
}

/// `sites` gives the line of each of a file's mutation sites. `--lines` applies to every file
/// in a run, so it is the union of the lines that hold each target's own selected sites, every
/// site of a whole file included, and a target of changed items with none is dropped. A
/// whole file is kept even without one, so that no scan can drop it unseen. A file
/// therefore also mutates any site on a line number another file selected: a safe superset,
/// never less, that a mutator taking each file's own lines would remove (#445 follow-up). The
/// owner keeps it so that the full-workspace baseline still runs once (2026-11-27).
fn run_plan(
    targets: Vec<Target>,
    mut sites: impl FnMut(&Target) -> io::Result<Vec<usize>>,
) -> io::Result<RunPlan> {
    let mut union = BTreeSet::new();
    let mut kept = Vec::new();
    for target in targets {
        let found = sites(&target)?;
        let own: Vec<usize> = match target.lines() {
            None => found.clone(),
            Some(lines) => found
                .iter()
                .copied()
                .filter(|line| lines.contains(line))
                .collect(),
        };
        if own.is_empty() && target.items.is_some() {
            continue;
        }
        union.extend(&own);
        kept.push((target, found, own.len()));
    }
    let lines = kept
        .iter()
        .any(|(target, ..)| target.items.is_some())
        .then_some(union);
    let targets = kept
        .into_iter()
        .map(|(target, found, selected)| Planned {
            mutants: found
                .iter()
                .filter(|line| lines.as_ref().is_none_or(|lines| lines.contains(*line)))
                .count(),
            target,
            selected,
        })
        .collect();
    Ok(RunPlan { targets, lines })
}

/// The line of each mutation site in each target's whole file, from one run of the mutator's
/// own scan, which runs no tests.
fn scanned_sites(root: &Path, targets: &[Target]) -> io::Result<HashMap<PathBuf, Vec<usize>>> {
    if targets.is_empty() {
        return Ok(HashMap::new());
    }
    let scanned = output(
        tool(root, "mutator")
            .args(["--scan", "--no-coverage"])
            .args(targets.iter().map(|target| &target.path)),
    )?;
    let scan = String::from_utf8_lossy(&scanned.stdout);
    targets
        .iter()
        .map(|target| Ok((target.path.clone(), scan_lines(&scan, &target.path)?)))
        .collect()
}

/// A run plan for `targets`, from one scan of their files.
fn scanned_plan(root: &Path, targets: Vec<Target>) -> io::Result<RunPlan> {
    let mut sites = scanned_sites(root, &targets)?;
    run_plan(targets, |target| {
        Ok(sites.remove(&target.path).unwrap_or_default())
    })
}

/// Each site's line in a mutator scan of `path`. A site line starts with a mark, `*` when its
/// function changed since the mutator's last snapshot, otherwise a space. The scan's header
/// names the file as the mutator does, with either separator, and relative or absolute
/// (#449 review 2); its site lines name it the same way. Fails unless the scan has a header
/// for `path` and as many of its site lines as the header counts, so that a path the mutator
/// names differently cannot pass as a file without sites.
fn scan_lines(scan: &str, path: &Path) -> io::Result<Vec<usize>> {
    let path = path.display().to_string();
    // The pinned mutator writes "Scan: N mutation sites in <path>" for every file it scans,
    // even for 0 or 1 site (`format_scan` in its `report.py`); the singular is accepted too.
    let headers: Vec<(&str, &str)> = scan
        .lines()
        .filter_map(|line| {
            let header = line.strip_prefix("Scan: ")?;
            header
                .split_once(" mutation sites in ")
                .or_else(|| header.split_once(" mutation site in "))
        })
        .filter(|(_, scanned)| same_file(scanned, &path))
        .collect();
    // Prefer the header that spells the path exactly, should another only end with it.
    let Some((count, scanned)) = headers
        .iter()
        .find(|(_, scanned)| *scanned == path)
        .or(headers.first())
    else {
        return Err(io::Error::other(format!(
            "mutator scan has no \"Scan: N mutation sites in {path}\" header; \
             it names the file differently from the gate"
        )));
    };
    let prefix = format!(" {scanned}:");
    let lines = scan
        .lines()
        .filter_map(|line| {
            line.strip_prefix([' ', '*'])
                .and_then(|line| line.strip_prefix(&prefix))
        })
        .map(|rest| {
            rest.split(' ')
                .next()
                .unwrap_or_default()
                .parse()
                .map_err(|error| io::Error::other(format!("mutator scan: {rest}: {error}")))
        })
        .collect::<io::Result<Vec<usize>>>()?;
    match count.parse::<usize>() {
        Ok(count) if count == lines.len() => Ok(lines),
        _ => Err(io::Error::other(format!(
            "mutator scan counts {count} mutation sites in {path} but lists {}",
            lines.len()
        ))),
    }
}

/// Whether the mutator's `scanned` path is the gate's relative `path`: the same once both
/// use `/`, and on Windows either case, or an absolute form ending with it.
fn same_file(scanned: &str, path: &str) -> bool {
    let normal = |text: &str| {
        let text = text.replace('\\', "/");
        if cfg!(windows) {
            text.to_lowercase()
        } else {
            text
        }
    };
    let (scanned, path) = (normal(scanned), normal(path));
    scanned == path || scanned.ends_with(&format!("/{path}"))
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
    for (name, repository, revision) in TOOLS {
        let destination = tools.join(name);
        if !destination.try_exists()? {
            run(Command::new("git")
                .args(["clone", "--quiet", "--no-checkout", repository])
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
    run(Command::new(venv_python(root))
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
    if !venv_python(root).is_file() {
        return Err(io::Error::other(
            "quality tools are missing; run cargo xtask setup-quality",
        ));
    }
    for (package, _, revision) in TOOLS {
        verify_checkout(&tools.join(package), revision)?;
    }
    Ok(())
}

/// The pinned tools' Python, in the virtual environment's layout on this platform.
fn venv_python(root: &Path) -> PathBuf {
    let environment = tool_root(root).join("venv");
    if cfg!(windows) {
        environment.join("Scripts").join("python.exe")
    } else {
        environment.join("bin").join("python")
    }
}

fn tool(root: &Path, name: &str) -> Command {
    let mut command = Command::new(venv_python(root));
    command.args(["-m", name]).current_dir(root);
    command
}

/// Each mutation build owns its relative target; inherited build paths must not re-share it.
fn own_targets(command: &mut Command) -> &mut Command {
    command
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .env_remove("CARGO_BUILD_BUILD_DIR")
}

fn mutation_command(root: &Path) -> io::Result<Command> {
    let mut command = tool(root, "mutator");
    own_targets(&mut command)
        .args(["--test-command", &test_command(&venv_python(root))?])
        .args(["--timeout-factor", MUTATION_TIMEOUT_FACTOR]);
    Ok(command)
}

/// The mutator's typed test command, with the Python path already in it, so no shell
/// (sh, or cmd.exe on Windows) is left to expand a variable. Both shells keep a
/// double-quoted path whole unless it holds a character they expand there.
fn test_command(python: &Path) -> io::Result<String> {
    let python = python.to_string_lossy();
    if python.contains(['"', '$', '`', '%']) {
        return Err(io::Error::other(format!(
            "the quality tools' Python path cannot be quoted for a shell: {python}"
        )));
    }
    Ok(format!(r#""{python}" tools/test-mutation.py"#))
}

/// The suite the gate runs, as `cargo test` argument lists: the whole workspace, except on
/// Windows.
const SUITE: &[&[&str]] = if cfg!(windows) {
    WINDOWS_SUITE
} else {
    &[&["--workspace"]]
};

/// Names the suite to the quality scripts when it is only part of the workspace; without
/// it they test the whole workspace. It also applies the worker job limit.
fn suite_environment(command: &mut Command) -> &mut Command {
    if SUITE != [["--workspace"]] {
        command.env(
            "CLIMON2_MUTATION_SUITE",
            serde_json::to_string(SUITE).expect("the suite serializes"),
        );
    }
    worker_limits(command, env::var_os(WORKER_JOBS_VARIABLE))
}

/// Cargo and libtest read their limits from the environment, which the mutator passes on
/// to every worker. Workers build without debug info: each has its own target folder,
/// which with debug info nears 8.4 GB on Windows, and the gate's workers together filled
/// the owner's VM's disk.
/// Workers keep no failed tests' evidence (#158): a mutant that fails a
/// test is the gate working, and its evidence would bury a real failure's.
fn worker_limits(command: &mut Command, jobs: Option<OsString>) -> &mut Command {
    command
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .env_remove("CLIMON2_TEST_EVIDENCE");
    if let Some(jobs) = jobs {
        command
            .env("CARGO_BUILD_JOBS", &jobs)
            .env("RUST_TEST_THREADS", &jobs);
    }
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

/// The selection of the ticket gate and the own-hardware gates: each changed file's changed
/// items. New files are whole, and so is a file whose changes `changed_items` marks whole.
/// With `tests_select`, a file's changed tests also select the functions they name.
fn function_targets(root: &Path, base: &OsStr, tests_select: bool) -> io::Result<Vec<Target>> {
    let (commit, files) = changed_files(root, base)?;
    let mut targets = Vec::new();
    for (path, base_path) in files {
        let items = match base_path {
            None => None,
            Some(base_path) => select(
                file_changes(root, &commit, &base_path, &path)?,
                tests_select,
            ),
        };
        if items.as_ref().is_none_or(|items| !items.is_empty()) {
            targets.push(Target { path, items });
        }
    }
    Ok(targets)
}

/// One file's selection, `None` for the whole file. With `tests_select`, changed tests also
/// select the functions they name, and the whole file when they name none (#449).
fn select(changes: Changes, tests_select: bool) -> Option<Vec<ChangedItem>> {
    if changes.whole {
        return None;
    }
    let mut items = changes.items;
    if tests_select && let Some(tested) = changes.tested {
        if tested.is_empty() {
            return None;
        }
        for item in tested {
            if !items.contains(&item) {
                items.push(item);
            }
        }
        items.sort_by_key(|item| item.lines.first().map(|lines| *lines.start()));
    }
    Some(items)
}

fn file_changes(root: &Path, commit: &str, base_path: &Path, path: &Path) -> io::Result<Changes> {
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

pub(crate) fn native_source_os(path: &Path) -> Option<&'static str> {
    match path.to_str() {
        Some("crates/climon2-platform/src/process/linux.rs")
        | Some("crates/climon2-platform/src/peer/linux.rs")
        | Some("crates/climon2-platform/src/runtime/linux.rs")
        | Some("crates/climon2-platform/src/library/linux.rs")
        | Some("crates/climon2-platform/src/notifier/linux.rs")
        | Some("crates/climon2-platform/src/connection/linux.rs")
        | Some("crates/climon2-install/src/platform/linux.rs")
        | Some("crates/climon2-client/src/library/linux.rs") => Some("linux"),
        Some("crates/climon2-platform/src/process/macos.rs")
        | Some("crates/climon2-platform/src/peer/macos.rs")
        | Some("crates/climon2-platform/src/runtime/macos.rs")
        | Some("crates/climon2-platform/src/library/macos.rs")
        | Some("crates/climon2-platform/src/notifier/macos.rs")
        | Some("crates/climon2-platform/src/connection/macos.rs")
        | Some("crates/climon2-install/src/platform/macos.rs") => Some("macos"),
        Some("crates/climon2-platform/src/home/windows.rs")
        | Some("crates/climon2-platform/src/private/windows.rs")
        | Some("crates/climon2-launcher/src/handoff/windows.rs")
        | Some("crates/climon2-cli/src/windows.rs")
        | Some("crates/climon2-platform/src/process/windows.rs")
        | Some("crates/climon2-platform/src/runtime/windows.rs")
        | Some("crates/climon2-platform/src/waiting/windows.rs")
        | Some("crates/climon2-platform/src/presentation/windows.rs")
        | Some("crates/climon2-platform/src/peer/windows.rs")
        | Some("crates/climon2-platform/src/terminal/windows.rs")
        | Some("crates/climon2-platform/src/logging/windows.rs")
        | Some("crates/climon2-platform/src/daemon/windows.rs")
        | Some("crates/climon2-platform/src/pty/windows.rs")
        | Some("crates/climon2-platform/src/command/windows.rs")
        | Some("crates/climon2-daemon/src/pty/windows.rs")
        | Some("crates/climon2-platform/src/library/windows.rs")
        | Some("crates/climon2-platform/src/notifier/windows.rs")
        | Some("crates/climon2-platform/src/connection/windows.rs")
        | Some("crates/climon2-platform/src/native_path/windows.rs")
        | Some("crates/climon2-platform/src/handoff/windows.rs")
        | Some("crates/climon2-install/src/windows.rs")
        | Some("crates/climon2-install/src/user_path/windows.rs") => Some("windows"),
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

fn mutation_targets(root: &Path, selection: Selection, scope: Scope) -> io::Result<Vec<Target>> {
    if !selection.whole_files() {
        return scope.changed_targets(root);
    }
    let sources = scope.select(shipped_sources(root)?);
    // The shipped sources are workspace-relative paths.
    Ok(Target::whole(
        sources
            .into_iter()
            .filter(|path| selection.keeps(path))
            .collect(),
    ))
}

/// What the gate would mutate: each target, its line counts and mutants, and the single run.
/// Mutants are counted before coverage, which drops those on lines no test runs.
fn plan_text(
    root: &Path,
    selection: Selection,
    targets: &[Target],
    run: &RunPlan,
) -> io::Result<String> {
    if targets.is_empty() {
        return Ok(format!("{}\n", selection.nothing_to_mutate()));
    }
    let mut text = String::new();
    for target in targets {
        let total = line_count(root, &target.path)?;
        let path = target.path.display();
        let planned = run
            .targets
            .iter()
            .find(|planned| planned.target.path == target.path);
        let mutants = match planned {
            None => "no mutants".to_owned(),
            Some(planned) if planned.mutants == planned.selected => {
                format!("{} mutants", planned.mutants)
            }
            Some(planned) => format!(
                "{} mutants, {} of them on lines another file selected",
                planned.mutants,
                planned.mutants - planned.selected
            ),
        };
        let (Some(items), Some(lines)) = (&target.items, target.lines()) else {
            text += &format!("{path}: whole file, {total} lines, {mutants}\n");
            continue;
        };
        text += &format!(
            "{path}: {} changed items, {} of {total} lines, {mutants}\n",
            items.len(),
            lines.len()
        );
        for item in items {
            let spans: Vec<_> = item.lines.iter().map(span).collect();
            text += &format!("  {}  lines {}\n", item.name, spans.join(","));
        }
    }
    let files = run.targets.len();
    let mutants: usize = run.targets.iter().map(|planned| planned.mutants).sum();
    text += &match &run.lines {
        _ if files == 0 => {
            "No selected line holds a mutation site; nothing to mutate.\n".to_owned()
        }
        None => format!("One mutator run over {files} files, {mutants} mutants, whole files.\n"),
        Some(lines) => format!(
            "One mutator run over {files} files, {mutants} mutants, --lines {} \
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
pub fn plan(root: &Path, selection: Selection, scope: Scope) -> io::Result<()> {
    let targets = mutation_targets(root, selection, scope)?;
    if !targets.is_empty() {
        verify_tools(root)?;
    }
    let run = scanned_plan(root, targets.clone())?;
    print!("{}", plan_text(root, selection, &targets, &run)?);
    Ok(())
}

pub fn check(root: &Path, selection: Selection, scope: Scope) -> io::Result<()> {
    let sources = shipped_sources(root)?;
    let scored: Vec<_> = scope
        .select(sources.clone())
        .into_iter()
        .filter(|path| selection.scores(path))
        .collect();
    let targets = mutation_targets(root, selection, scope)?;
    if scope == Scope::Native && !scope.qualifies(root, selection, &targets)? {
        eprintln!("No selected native Rust source to qualify; shared source is checked locally.");
        return Ok(());
    }
    verify_llvm_cov()?;
    verify_tools(root)?;
    if scope == Scope::Native && cfg!(windows) {
        // The Windows tooling has no history behind it (docs/for-humans/windows/quality.md): prove
        // mutants really run and killed processes keep coverage before trusting either.
        sentinel(root)?;
        killed_coverage_probe(root)?;
    }
    let report = root.join("target/coverage/rust/lcov.info");
    fs::create_dir_all(report.parent().expect("report directory"))?;
    if report.try_exists()? {
        fs::remove_file(&report)?;
    }
    let coverage = Coverage::prepare(root, &root.join("target/coverage-build"))?;
    for selection in SUITE {
        run(coverage
            .cargo()
            .arg("test")
            .args(*selection)
            .arg("--locked"))?;
    }
    coverage.report(&report)?;
    if fs::metadata(&report)?.len() == 0 {
        return Err(io::Error::other("coverage report is empty"));
    }
    verify_native_coverage(root, &report)?;
    run(tool(root, "crapper")
        .args(["--use-existing-coverage", "--threshold", CRAP_THRESHOLD])
        .args(&scored))?;
    let workers = scope.workers()?;
    // Unloaded and warm: before the load rounds that rehearse them and any mutation worker.
    let stage_timeouts = root.join(STAGE_TIMEOUTS);
    let paths: Vec<_> = targets.iter().map(|target| &target.path).collect();
    run(
        suite_environment(own_targets(&mut python_script(root, "test-mutation.py")))
            .arg(MUTATION_TIMEOUT_FACTOR)
            .arg(&stage_timeouts)
            .args(paths),
    )?;
    if let Some(rounds) = scope.load_rounds() {
        // Its stage timeouts cover each stage's build too, so it builds as the workers do.
        run(suite_environment(own_targets(&mut python_script(
            root,
            "check-mutation-load.py",
        )))
        .args([
            "--workers",
            &workers,
            "--rounds",
            rounds,
            "--timeout-factor",
            MUTATION_TIMEOUT_FACTOR,
            "--stage-timeouts",
        ])
        .arg(&stage_timeouts))?;
    }
    if targets.is_empty() {
        eprintln!("{}", selection.nothing_to_mutate());
        return Ok(());
    }
    let run = scanned_plan(root, targets.clone())?;
    eprint!("{}", plan_text(root, selection, &targets, &run)?);
    if run.targets.is_empty() {
        return Ok(());
    }
    verify_worker_baseline(root, &sources)?;
    mutate(root, &workers, run, &stage_timeouts)
}

/// One mutator run for every target, so the full-workspace baseline runs once.
/// Its workers apply `stage_timeouts`, measured for at least every target.
fn mutate(root: &Path, workers: &str, run_plan: RunPlan, stage_timeouts: &Path) -> io::Result<()> {
    clear_mutation_history(root)?;
    let mut mutation = mutation_command(root)?;
    suite_environment(&mut mutation)
        .env("CLIMON2_MUTATION_STAGE_TIMEOUTS", stage_timeouts)
        .args([
            "--use-existing-coverage",
            "--mutate-all",
            "--max-workers",
            workers,
        ]);
    if let Some(lines) = &run_plan.lines {
        mutation.args(["--lines", &joined(lines)]);
    }
    run(mutation.args(
        run_plan
            .targets
            .into_iter()
            .map(|planned| planned.target.path),
    ))
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
    let mut command = Command::new(venv_python(root));
    command
        .arg(root.join("tools").join(script))
        .arg(root)
        .arg(env!("CARGO"))
        .current_dir(root);
    command
}

fn verify_worker_baseline(root: &Path, sources: &[PathBuf]) -> io::Result<()> {
    run(suite_environment(own_targets(&mut python_script(
        root,
        "check-mutation-workspace.py",
    )))
    .args(sources))
}

pub fn probe(root: &Path) -> io::Result<()> {
    let scratch = probe_workspace(root)?;
    let observations = scratch.path().join("observations");
    let forbidden = scratch.path().join("shared-target-must-not-be-used");
    verify_tools(root)?;
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
    let scratch = fixture_workspace(
        root,
        "mutation-isolation",
        &[
            "Cargo.toml",
            "Cargo.lock",
            "core/Cargo.toml",
            "core/src/lib.rs",
            "cli/Cargo.toml",
            "cli/src/main.rs",
            "cli/tests/binary.rs",
        ],
    )?;
    fs::create_dir(scratch.path().join("observations"))?;
    Ok(scratch)
}

/// A scratch copy of `files` of the fixture workspace `fixture`, with the mutation test
/// runner, under `target/` (out of `tests/`, where the mutator would skip every source).
fn fixture_workspace(root: &Path, fixture: &str, files: &[&str]) -> io::Result<tempfile::TempDir> {
    fs::create_dir_all(root.join("target"))?;
    let scratch = tempfile::Builder::new()
        .prefix(&format!("{fixture}-"))
        .tempdir_in(root.join("target"))?;
    for relative in files {
        let destination = scratch.path().join(relative);
        fs::create_dir_all(destination.parent().expect("fixture parent"))?;
        fs::copy(
            root.join("xtask/tests/fixtures")
                .join(fixture)
                .join(relative),
            destination,
        )?;
    }
    fs::create_dir(scratch.path().join("tools"))?;
    fs::copy(
        root.join("tools/test-mutation.py"),
        scratch.path().join("tools/test-mutation.py"),
    )?;
    Ok(scratch)
}

/// `cargo xtask mutation-probe`: the sentinel, then real-binary worker isolation.
pub fn mutation_probe(root: &Path) -> io::Result<()> {
    sentinel(root)?;
    probe(root)
}

/// The sentinel's site log: its checked mutant killed and its unchecked one surviving.
const SENTINEL_OUTCOMES: [&str; 2] = [
    "KILLED    src/lib.rs:6 + -> -",
    "SURVIVED  src/lib.rs:10 + -> -",
];

/// Fails unless the mutator, through the gate's own test command, kills the one mutant the
/// sentinel fixture's test checks and lets the unchecked one survive. A mutant that cannot
/// start counts as killed, so a gate whose tests never ran would otherwise pass.
pub fn sentinel(root: &Path) -> io::Result<()> {
    verify_tools(root)?;
    let scratch = fixture_workspace(
        root,
        "mutation-sentinel",
        &[
            "Cargo.toml",
            "Cargo.lock",
            "src/lib.rs",
            "tests/sentinel.rs",
        ],
    )?;
    let mut command = mutation_command(root)?;
    let output = command
        .current_dir(scratch.path())
        .args([
            "--no-coverage",
            "--mutate-all",
            "--max-workers",
            "2",
            "src/lib.rs",
        ])
        .output()?;
    sentinel_verdict(
        output.status.code(),
        &String::from_utf8_lossy(&output.stdout),
    )
    .map_err(|error| {
        io::Error::other(format!(
            "{error}\n{command:?}\n{}",
            String::from_utf8_lossy(&output.stderr)
        ))
    })?;
    eprintln!(
        "Verified the sentinel: its checked mutant was killed and its unchecked one survived."
    );
    Ok(())
}

/// The mutator reports a survivor with status 3; the site log must name exactly the
/// sentinel's two outcomes.
fn sentinel_verdict(status: Option<i32>, log: &str) -> io::Result<()> {
    let outcomes: Vec<_> = log
        .lines()
        .filter(|line| line.contains("src/lib.rs:"))
        .collect();
    if status == Some(3) && outcomes == SENTINEL_OUTCOMES {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "the sentinel must end with status 3 and {SENTINEL_OUTCOMES:?}; got status {status:?} and {outcomes:?}"
    )))
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

/// Fails unless a process killed mid-run (`TerminateProcess` on Windows, SIGKILL elsewhere)
/// keeps its coverage counters: the mutation-isolation fixture's instrumented CLI counts its
/// probe once and is killed, and that count must reach the report. Tests often end the
/// daemon that way.
pub fn killed_coverage_probe(root: &Path) -> io::Result<()> {
    let scratch = probe_workspace(root)?;
    let workspace = scratch.path();
    let build = workspace.join("target/instrumented");
    let coverage = Coverage::prepare(workspace, &build)?;
    run(coverage
        .cargo()
        .args(["build", "--locked", "--package", "isolation-cli"]))?;
    let binary = build
        .join("debug")
        .join(format!("isolation-cli{}", env::consts::EXE_SUFFIX));
    prove_killed_profile(&binary, coverage.profile())?;
    let report = workspace.join("target/coverage/rust/lcov.info");
    fs::create_dir_all(report.parent().expect("report directory"))?;
    coverage.report(&report)?;
    let count = staged_count(&report)?;
    if count != 1 {
        return Err(io::Error::other(format!(
            "a killed instrumented process must keep its one coverage hit, got {count}"
        )));
    }
    eprintln!("Verified that a killed process keeps its coverage counters.");
    Ok(())
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
#[cfg(unix)]
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
    prove_killed_profile(
        &staged.join(env!("CARGO_PKG_VERSION")).join("climon2-cli"),
        profile,
    )?;
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

/// Runs the instrumented probe `binary` until it has counted its probe, then kills it
/// (SIGKILL; `TerminateProcess` on Windows), so its counters reach `profile` only if they
/// were written as they changed.
fn prove_killed_profile(binary: &Path, profile: &str) -> io::Result<()> {
    use std::{
        io::{BufRead, BufReader},
        process::Stdio,
    };
    let mut child = Command::new(binary)
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

#[cfg(unix)]
fn install_instrumented_probe(workspace: &Path, build: &Path) -> io::Result<PathBuf> {
    let payload = workspace.join("payload");
    fs::create_dir(&payload)?;
    fs::copy(build.join("debug/isolation-cli"), payload.join("c2"))?;
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

    /// The mutator's site log and summary for the sentinel, as it prints them.
    fn sentinel_log(checked: &str, unchecked: &str) -> String {
        format!(
            "{checked:<9} src/lib.rs:6 + -> -\n{unchecked:<9} src/lib.rs:10 + -> -\n\
             Function  Namespace          Killed  Survived\n"
        )
    }

    #[test]
    fn the_sentinel_passes_only_with_its_checked_mutant_killed_and_its_unchecked_one_alive() {
        sentinel_verdict(Some(3), &sentinel_log("KILLED", "SURVIVED")).unwrap();
        // Every mutant failing to start scores as killed, and the run as a pass.
        let error = sentinel_verdict(Some(0), &sentinel_log("KILLED", "KILLED")).unwrap_err();
        assert!(error.to_string().contains("got status Some(0)"), "{error}");
        sentinel_verdict(Some(3), &sentinel_log("SURVIVED", "SURVIVED")).unwrap_err();
        sentinel_verdict(Some(3), "SURVIVED  src/lib.rs:10 + -> -\n").unwrap_err();
        sentinel_verdict(Some(2), "").unwrap_err();
    }

    #[test]
    fn the_test_command_names_the_python_path_itself() {
        assert_eq!(
            test_command(Path::new("/work/.metrics/tools/venv/bin/python")).unwrap(),
            r#""/work/.metrics/tools/venv/bin/python" tools/test-mutation.py"#
        );
        assert_eq!(
            test_command(Path::new(
                r"D:\a\climon2\.metrics\tools\venv\Scripts\python.exe"
            ))
            .unwrap(),
            r#""D:\a\climon2\.metrics\tools\venv\Scripts\python.exe" tools/test-mutation.py"#
        );
        for path in [
            "/a$HOME/python",
            "/a\"b/python",
            r"C:\%TEMP%\python.exe",
            "/a`x`/python",
        ] {
            test_command(Path::new(path)).unwrap_err();
        }
    }

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
        check(Path::new(&root), Selection::Changed, scope).unwrap();
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
        /// The changed source's base text; by default its one function returns another value.
        base: Option<&'static str>,
    }

    #[cfg(unix)]
    struct GateRun {
        success: bool,
        output: String,
        /// Each quality tool started, in order, with its arguments.
        events: Vec<String>,
        /// Each test-building tool started, with the build environment it was given.
        environments: Vec<String>,
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
             *'cat-file'*) printf '%s\\n' \"$LOAD_GATE_BASE\" ;;\n\
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
# The build environment each tool starts its Cargo runs in.
build_environment() {
    printf '%s debug=%s jobs=%s threads=%s evidence=%s target=%s\n' "$1" \
        "${CARGO_PROFILE_DEV_DEBUG-unset}" "${CARGO_BUILD_JOBS-unset}" \
        "${RUST_TEST_THREADS-unset}" "${CLIMON2_TEST_EVIDENCE-unset}" \
        "${CARGO_TARGET_DIR-unset}" >> "$CLIMON2_LOAD_GATE_FIXTURE/environments"
}
case "$*" in
    *check-mutation-load.py*)
        build_environment load
        shift
        printf 'load %s\n' "$*" >> "$events"
        if [ "$LOAD_GATE_FAIL" = yes ]; then
            echo 'test flaky_fixture ... FAILED' >&2
            exit 1
        fi
        ;;
    *check-mutation-workspace.py*) echo baseline >> "$events" ;;
    *'-m crapper'*)
        shift 2
        printf 'crapper %s\n' "$*" >> "$events"
        ;;
    *'-m mutator --scan'*)
        for path do
            case "$path" in
                *.rs)
                    printf 'scan %s\n' "$path" >> "$events"
                    printf 'Scan: 2 mutation sites in %s\n' "$path"
                    for site in 1 3; do
                        printf '  %s:%s + -> -  [defn/f]\n' "$path" "$site"
                    done
                    ;;
            esac
        done
        ;;
    *'-m mutator'*)
        build_environment mutation
        if [ -e .metrics/mutate ]; then echo stale-history >> "$events"; fi
        printf 'mutation-stage-timeouts %s\n' "$CLIMON2_MUTATION_STAGE_TIMEOUTS" >> "$events"
        shift 2
        printf 'mutation %s\n' "$*" >> "$events"
        ;;
    *test-mutation.py*)
        build_environment stages
        shift 3
        printf 'stages %s\n' "$*" >> "$events"
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
            .env(
                "LOAD_GATE_BASE",
                gate.base.unwrap_or("pub fn changed() -> u8 { 0 }"),
            )
            .env("LOAD_GATE_CHANGED", if gate.changed { "yes" } else { "no" })
            .env("LOAD_GATE_FAIL", if gate.load_fails { "yes" } else { "no" })
            .env("PATH", env::join_paths(path).unwrap())
            .env_remove("CLIMON2_MUTATION_BASE")
            .env("CLIMON2_MUTATION_WORKER_JOBS", "3")
            .env("CLIMON2_TEST_EVIDENCE", root.join("evidence"))
            .env("CARGO_TARGET_DIR", root.join("shared-target"))
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
        let lines = |name| {
            fs::read_to_string(root.join(name))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        };
        GateRun {
            success: status.success(),
            output: fs::read_to_string(output).unwrap(),
            events: lines("events"),
            environments: lines("environments"),
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
            base: None,
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

    /// The load rounds time each stage's whole `cargo test`, build included, so they hold
    /// only when the stages build what the measurement and the workers built. Built with
    /// debug info, unlike the workers, the warm build's proc-macros shared xtask's debug
    /// build of syn, so `--lib`, which leaves xtask out, rebuilt the whole graph in each
    /// load worker within its stage timeout.
    #[cfg(unix)]
    #[test]
    fn the_load_rounds_build_in_the_mutation_workers_environment() {
        let run = run_gate(Gate {
            scope: "ticket",
            changed: true,
            ..Gate::default()
        });
        assert!(run.success, "{}", run.output);
        let worker = "debug=0 jobs=3 threads=3 evidence=unset target=unset";
        assert_eq!(
            run.environments,
            [
                format!("stages {worker}"),
                format!("load {worker}"),
                format!("mutation {worker}"),
            ],
            "{:?}",
            run.events
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
            base: None,
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
            base: None,
        });
        assert!(!run.success, "{}", run.output);
        assert!(run.output.contains("flaky_fixture"), "{}", run.output);
        let run = run_gate(Gate {
            scope: "full",
            load_fails: false,
            changed: false,
            added: false,
            base: None,
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
                base: None,
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
            base: None,
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
            base: None,
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
            base: None,
        });
        assert!(run.success, "{}", run.output);
        assert_eq!(run.option("load", "--rounds"), "1");
        assert_eq!(run.started("mutation"), None, "{:?}", run.events);
    }

    /// Review 7 and 8: one mutator run (one workspace baseline) takes every target, with the union
    /// of the lines holding each target's selected sites; the quality tools are verified once.
    /// #445: `cargo xtask mutate` selects as the ticket gate does, mutates a new file whole,
    /// and prints each file's selection and mutants.
    #[cfg(unix)]
    #[test]
    fn the_ticket_and_host_gates_mutate_every_target_in_one_run_after_one_tool_check() {
        for scope in ["ticket", "host"] {
            let run = run_gate(Gate {
                scope,
                changed: true,
                added: true,
                ..Gate::default()
            });
            assert!(run.success, "{scope}: {}", run.output);
            let checks = run.events.iter().filter(|event| *event == "verify").count();
            assert_eq!(checks, 1, "{scope}: {:?}", run.events);
            let mutations: Vec<_> = run
                .events
                .iter()
                .filter(|event| event.starts_with("mutation "))
                .collect();
            assert_eq!(mutations.len(), 1, "{scope}: {:?}", run.events);
            assert_eq!(run.option("mutation", "--lines"), "1,3", "{scope}");
            assert!(
                mutations[0]
                    .ends_with(" crates/climon2-core/src/added.rs crates/climon2-core/src/lib.rs"),
                "{scope}: {}",
                mutations[0]
            );
            assert!(
                run.output.contains(
                    "crates/climon2-core/src/added.rs: whole file, 3 lines, 2 mutants\n\
                     crates/climon2-core/src/lib.rs: 1 changed items, 1 of 1 lines, \
                     2 mutants, 1 of them on lines another file selected\n  changed  lines 1\n"
                ),
                "{scope}: {}",
                run.output
            );
        }
    }

    /// #449 review: the native gate qualifies a changed native file whose changes select no
    /// mutant: CRAP still scores it, and nothing is mutated.
    #[cfg(unix)]
    #[test]
    fn the_native_gate_scores_a_changed_file_that_selects_no_mutant() {
        let run = run_gate(Gate {
            scope: "native",
            changed: true,
            base: Some("// Only a comment differs.\npub fn changed() -> u8 { 1 }"),
            ..Gate::default()
        });
        assert!(run.success, "{}", run.output);
        let crapper = &run.events[run.started("crapper").unwrap()];
        assert!(crapper.ends_with(".rs"), "{crapper}");
        assert_eq!(run.started("mutation"), None, "{:?}", run.events);
        let run = run_gate(Gate {
            scope: "native",
            ..Gate::default()
        });
        assert!(run.success, "{}", run.output);
        assert_eq!(run.started("crapper"), None, "{:?}", run.events);
    }

    /// #445 owner decision: the own-hardware gates mutate only the changed functions' lines.
    #[cfg(unix)]
    #[test]
    fn the_host_and_native_gates_mutate_only_changed_function_lines() {
        for scope in ["host", "native"] {
            let run = run_gate(Gate {
                scope,
                changed: true,
                ..Gate::default()
            });
            assert!(run.success, "{scope}: {}", run.output);
            assert_eq!(run.option("mutation", "--lines"), "1", "{scope}");
        }
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

    /// Gate speed-up (#73, 2026-11-16): each target's earlier test stages are timed once with
    /// the mutator's timeout factor, after coverage and before anything loads the host; the load
    /// rounds rehearse those stage timeouts and the mutator's workers apply them.
    #[cfg(unix)]
    #[test]
    fn stage_timeouts_are_measured_once_before_load_and_used_by_load_and_mutation() {
        for scope in ["ticket", "full", "host", "native"] {
            let run = run_gate(Gate {
                scope,
                changed: true,
                added: scope == "ticket",
                ..Gate::default()
            });
            assert!(run.success, "{scope}: {}", run.output);
            let stages = run.started("stages").unwrap();
            let measurements = run
                .events
                .iter()
                .filter(|event| event.starts_with("stages "));
            assert_eq!(measurements.count(), 1, "{:?}", run.events);
            // `stages TIMEOUT_FACTOR STAGE_TIMEOUTS SOURCE...`
            let measured: Vec<_> = run.events[stages].split(' ').skip(1).collect();
            assert_eq!(measured[0], run.option("mutation", "--timeout-factor"));
            let file = measured[1];
            assert!(
                file.ends_with("target/mutation-stage-timeouts.json"),
                "{file}"
            );
            if let Some(load) = run.started("load") {
                assert!(stages < load, "{:?}", run.events);
                assert_eq!(run.option("load", "--stage-timeouts"), file);
            } else {
                assert!(matches!(scope, "host" | "native"), "{scope}");
            }
            assert!(
                stages < run.started("baseline").unwrap(),
                "{:?}",
                run.events
            );
            let mutation = run.started("mutation").unwrap();
            assert_eq!(
                run.events[mutation - 1],
                format!("mutation-stage-timeouts {file}")
            );
            let mutated: Vec<_> = run.events[mutation]
                .split(' ')
                .skip_while(|word| !word.starts_with("crates/"))
                .collect();
            assert_eq!(measured[2..], mutated[..], "{:?}", run.events);
        }
        // Without a target, load still rehearses the full workspace with no earlier stage.
        let run = run_gate(Gate {
            scope: "ticket",
            ..Gate::default()
        });
        assert!(run.success, "{}", run.output);
        let stages = run.started("stages").unwrap();
        assert_eq!(run.events[stages].split(' ').count(), 3, "{:?}", run.events);
        assert!(stages < run.started("load").unwrap(), "{:?}", run.events);
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
            function_targets(root, OsStr::new("HEAD~1"), false).unwrap(),
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
            function_targets(root, OsStr::new("HEAD"), false)
                .unwrap()
                .iter()
                .map(|target| target.items.clone())
                .collect::<Vec<_>>(),
            vec![None]
        );
    }

    /// #449 owner decision: in the own-hardware gates a changed test selects the functions its
    /// body names, or the whole file when it names none. Every gate mutates whole a file with a
    /// changed glob import or a base that is not Rust.
    #[test]
    fn changed_tests_globs_and_unparsable_bases_select_as_the_gate_requires() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        git(root, &["init", "--quiet"]);
        let tested = "crates/climon2-cli/src/lib.rs";
        let imports = "crates/climon2-core/src/lib.rs";
        let both = "crates/climon2-daemon/src/lib.rs";
        let silent = "crates/climon2-install/src/lib.rs";
        let broken = "crates/climon2-vt/src/lib.rs";
        let source = "pub fn f() -> u8 {\n    1\n}\npub fn g() -> u8 {\n    2\n}\n";
        let test = |body: &str| {
            format!("#[cfg(test)]\nmod tests {{\n    #[test]\n    fn t() {{ {body} }}\n}}\n")
        };
        let base = format!("{source}{}", test("assert_eq!(super::f(), 1);"));
        for (path, text) in [
            (tested, base.clone()),
            (imports, source.to_owned()),
            (both, base.clone()),
            (silent, format!("{source}{}", test("assert_eq!(1, 2);"))),
            (broken, "pub fn f( {".to_owned()),
        ] {
            fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            fs::write(root.join(path), text).unwrap();
        }
        commit(root);
        let changed = |body: &str| format!("{source}{}", test(body));
        fs::write(root.join(tested), changed("assert_eq!(super::f(), 3);")).unwrap();
        fs::write(root.join(imports), format!("use std::fmt::*;\n{source}")).unwrap();
        fs::write(
            root.join(both),
            changed("assert_eq!(super::g(), 2);").replace("    1\n", "    5\n"),
        )
        .unwrap();
        fs::write(root.join(silent), changed("assert_eq!(1, 1);")).unwrap();
        fs::write(root.join(broken), source).unwrap();
        commit(root);

        let target = |path: &str, items: Option<&[(&str, RangeInclusive<usize>)]>| Target {
            path: path.into(),
            items: items.map(|items| {
                items
                    .iter()
                    .map(|(name, lines)| ChangedItem {
                        name: (*name).to_owned(),
                        lines: vec![lines.clone()],
                    })
                    .collect()
            }),
        };
        assert_eq!(
            function_targets(root, OsStr::new("HEAD~1"), true).unwrap(),
            vec![
                target(tested, Some(&[("f", 1..=3)])),
                target(imports, None),
                target(both, Some(&[("f", 1..=3), ("g", 4..=6)])),
                target(silent, None),
                target(broken, None),
            ]
        );
        assert_eq!(
            function_targets(root, OsStr::new("HEAD~1"), false).unwrap(),
            vec![
                target(imports, None),
                target(both, Some(&[("f", 1..=3)])),
                target(broken, None),
            ]
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
            function_targets(root, OsStr::new("moved-on"), false)
                .unwrap()
                .into_iter()
                .map(|target| target.path)
                .collect::<Vec<_>>(),
            vec![PathBuf::from(cli)]
        );
    }

    fn scoped(path: &str, lines: RangeInclusive<usize>) -> Target {
        Target {
            path: path.into(),
            items: Some(vec![ChangedItem {
                name: "f".to_owned(),
                lines: vec![lines],
            }]),
        }
    }

    /// Review 7, and #445 owner decision (2026-11-27): the run's lines are the union of the
    /// lines holding each target's own selected sites, every site of a whole file included; a
    /// target of changed items without any is dropped, and whole-file-only runs need no
    /// `--lines`. Each file counts its own selected mutants apart from those on a line only
    /// another file selected, which the union adds.
    #[test]
    fn the_single_run_unions_site_lines_and_counts_each_files_own_mutants() {
        let whole = Target {
            path: "new.rs".into(),
            items: None,
        };
        let sites = |target: &Target| -> io::Result<Vec<usize>> {
            Ok(match target.path.to_str().unwrap() {
                "a.rs" => vec![2, 7, 7, 12],
                "b.rs" => vec![2, 11],
                "c.rs" => vec![20],
                _ => vec![4],
            })
        };
        let plan = run_plan(
            vec![
                scoped("a.rs", 1..=9),
                scoped("b.rs", 10..=12),
                scoped("c.rs", 1..=9),
                whole.clone(),
            ],
            sites,
        )
        .unwrap();
        assert_eq!(plan.lines, Some(BTreeSet::from([2, 4, 7, 11])));
        assert_eq!(
            plan.targets,
            vec![
                Planned {
                    target: scoped("a.rs", 1..=9),
                    selected: 3,
                    mutants: 3,
                },
                // Line 2 is a's, not b's: the union adds it to b's run.
                Planned {
                    target: scoped("b.rs", 10..=12),
                    selected: 1,
                    mutants: 2,
                },
                Planned {
                    target: whole.clone(),
                    selected: 1,
                    mutants: 1,
                },
            ]
        );
        // #449 review: a whole file is kept even when its scan finds no site.
        let empty = run_plan(vec![whole.clone()], |_| Ok(Vec::new())).unwrap();
        assert_eq!(
            empty.targets,
            vec![Planned {
                target: whole.clone(),
                selected: 0,
                mutants: 0,
            }]
        );
        let plan = run_plan(vec![whole.clone()], |_| Ok(vec![1, 1, 5])).unwrap();
        assert_eq!(plan.lines, None);
        assert_eq!(
            plan.targets,
            vec![Planned {
                target: whole,
                selected: 3,
                mutants: 3,
            }]
        );
        assert!(
            run_plan(vec![scoped("c.rs", 1..=9)], sites)
                .unwrap()
                .targets
                .is_empty()
        );
    }

    /// #449 review: a scan that does not name a target as the gate does, or lists fewer
    /// sites than it counts, fails rather than passing as a file without sites.
    #[test]
    fn a_scan_without_the_targets_header_or_with_missing_sites_fails() {
        let scan = "Scan: 1 mutation sites in C:/work/ba.rs\n  C:/work/ba.rs:4 + -> -  [defn/f]\n";
        let error = scan_lines(scan, Path::new("a.rs")).unwrap_err().to_string();
        assert!(
            error.contains("no \"Scan: N mutation sites in a.rs\" header"),
            "{error}"
        );
        let scan = "Scan: 2 mutation sites in a.rs\n  a.rs:4 + -> -  [defn/f]\n";
        let error = scan_lines(scan, Path::new("a.rs")).unwrap_err().to_string();
        assert!(
            error.contains("counts 2 mutation sites in a.rs but lists 1"),
            "{error}"
        );
        assert_eq!(
            scan_lines("Scan: 0 mutation sites in a.rs\n", Path::new("a.rs")).unwrap(),
            Vec::<usize>::new()
        );
    }

    /// #449 review 3: a header for no or one site is read, in the plural the pinned mutator
    /// writes or in the singular.
    #[test]
    fn a_scan_header_for_no_or_one_site_is_read() {
        for scan in [
            "Scan: 1 mutation sites in a.rs\n  a.rs:3 + -> -  [defn/f]\n",
            "Scan: 1 mutation site in a.rs\n  a.rs:3 + -> -  [defn/f]\n",
        ] {
            assert_eq!(
                scan_lines(scan, Path::new("a.rs")).unwrap(),
                vec![3],
                "{scan}"
            );
        }
        assert_eq!(
            scan_lines("Scan: 0 mutation sites in a.rs\n", Path::new("a.rs")).unwrap(),
            Vec::<usize>::new()
        );
        let error = scan_lines("Scan: 0 mutation sites in b.rs\n", Path::new("a.rs"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("no \"Scan: N mutation sites in a.rs\" header"),
            "{error}"
        );
    }

    /// #449 review 2: the mutator may name a target with backslashes, or absolute.
    #[test]
    fn a_scan_naming_the_file_with_backslashes_or_absolute_is_read() {
        let scan = r"Scan: 2 mutation sites in C:\work\crates\core\src\lib.rs
  C:\work\crates\core\src\lib.rs:4 + -> -  [defn/f]
* C:\work\crates\core\src\lib.rs:7 < -> <=  [defn/g]
Scan: 1 mutation sites in crates\core\src\other.rs
  crates\core\src\other.rs:2 + -> -  [defn/h]
";
        assert_eq!(
            scan_lines(scan, Path::new("crates/core/src/lib.rs")).unwrap(),
            vec![4, 7]
        );
        assert_eq!(
            scan_lines(scan, Path::new("crates/core/src/other.rs")).unwrap(),
            vec![2]
        );
    }

    /// #445: a scan's site lines are read for its own file whether or not the mutator marks
    /// the site's function as changed since its last snapshot.
    #[test]
    fn a_scan_gives_each_site_line_of_its_file() {
        let scan = "\
Scan: 3 mutation sites in a.rs
  a.rs:4 + -> -  [defn/f]
Scan: 1 mutation sites in ab.rs
* a.rs:9 < -> <=  [defn/g]
  a.rs:9 1 -> 0  [defn/g]
  ab.rs:5 + -> -  [defn/h]
* marks a function whose text changed since the last snapshot.
";
        assert_eq!(scan_lines(scan, Path::new("a.rs")).unwrap(), vec![4, 9, 9]);
    }

    /// Review 9 and #445: the printed plan names whole-file and item targets, their line
    /// counts, each file's mutants, and the run.
    #[test]
    fn the_plan_lists_each_target_its_mutants_and_the_single_runs_lines() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        fs::write(root.join("new.rs"), "a\nb\nc\n").unwrap();
        fs::write(root.join("old.rs"), "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n").unwrap();
        fs::write(root.join("dry.rs"), "1\n2\n").unwrap();
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
            scoped("dry.rs", 2..=2),
        ];
        let run = RunPlan {
            targets: vec![
                Planned {
                    target: targets[0].clone(),
                    selected: 2,
                    mutants: 2,
                },
                Planned {
                    target: targets[1].clone(),
                    selected: 2,
                    mutants: 3,
                },
            ],
            lines: Some(BTreeSet::from([1, 3, 9])),
        };
        assert_eq!(
            plan_text(root, Selection::Changed, &targets, &run).unwrap(),
            "\
new.rs: whole file, 3 lines, 2 mutants
old.rs: 2 changed items, 4 of 10 lines, 3 mutants, 1 of them on lines another file selected
  W::f  lines 2-3,5
  LIMIT  lines 9
dry.rs: 1 changed items, 1 of 2 lines, no mutants
  f  lines 2
One mutator run over 2 files, 5 mutants, --lines 1,3,9 \
(the union of the lines holding each file's selected mutation sites).
"
        );
        let whole = RunPlan {
            targets: run.targets[..1].to_vec(),
            lines: None,
        };
        assert_eq!(
            plan_text(root, Selection::All, &targets[..1], &whole).unwrap(),
            "new.rs: whole file, 3 lines, 2 mutants\n\
             One mutator run over 1 files, 2 mutants, whole files.\n"
        );
        let nothing = RunPlan {
            targets: Vec::new(),
            lines: None,
        };
        assert_eq!(
            plan_text(root, Selection::Changed, &targets[2..], &nothing).unwrap(),
            "\
dry.rs: 1 changed items, 1 of 2 lines, no mutants
  f  lines 2
No selected line holds a mutation site; nothing to mutate.
"
        );
        assert_eq!(
            plan_text(root, Selection::Changed, &[], &nothing).unwrap(),
            "No changed shipped Rust source to mutate.\n"
        );
    }

    #[test]
    fn workers_build_without_debug_info_and_a_job_limit_caps_their_cargo() {
        let limits = |jobs: Option<&str>| {
            let mut command = Command::new("cargo");
            worker_limits(&mut command, jobs.map(OsString::from));
            command
                .get_envs()
                .map(|(name, value)| (name.to_owned(), value.map(OsStr::to_owned)))
                .collect::<Vec<_>>()
        };
        let set = |name: &str, value: &str| (OsString::from(name), Some(OsString::from(value)));
        let unset = |name: &str| (OsString::from(name), None);
        assert_eq!(
            limits(None),
            [
                set("CARGO_PROFILE_DEV_DEBUG", "0"),
                unset("CLIMON2_TEST_EVIDENCE"),
            ]
        );
        assert_eq!(
            limits(Some("2")),
            [
                set("CARGO_BUILD_JOBS", "2"),
                set("CARGO_PROFILE_DEV_DEBUG", "0"),
                unset("CLIMON2_TEST_EVIDENCE"),
                set("RUST_TEST_THREADS", "2"),
            ]
        );
    }

    #[test]
    fn each_gate_takes_its_own_worker_cap_and_it_must_be_a_positive_number() {
        let cap = |text: &str| Some(OsString::from(text));
        let cpus = std::thread::available_parallelism().unwrap().get().min(16);
        assert_eq!(
            Scope::Native.workers_capped(None).unwrap(),
            cpus.to_string()
        );
        assert_eq!(Scope::Native.workers_variable(), NATIVE_WORKERS_VARIABLE);
        for scope in [Scope::Ticket, Scope::Full, Scope::Host] {
            assert_eq!(scope.workers_capped(None).unwrap(), MUTATION_WORKERS);
            assert_eq!(scope.workers_variable(), HOST_WORKERS_VARIABLE);
        }
        for scope in [Scope::Ticket, Scope::Full, Scope::Host, Scope::Native] {
            assert_eq!(scope.workers_capped(cap("6")).unwrap(), "6");
            assert_eq!(scope.workers_capped(cap("40")).unwrap(), "40");
            for bad in ["0", "-1", "two", "", " 2"] {
                let error = scope.workers_capped(cap(bad)).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
                assert!(
                    error.to_string().contains(scope.workers_variable()),
                    "{error}"
                );
            }
        }
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
                "crates/climon2-platform/src/library/linux.rs",
                cfg!(target_os = "linux"),
            ),
            (
                "crates/climon2-platform/src/library/macos.rs",
                cfg!(target_os = "macos"),
            ),
            (
                "crates/climon2-platform/src/notifier/linux.rs",
                cfg!(target_os = "linux"),
            ),
            (
                "crates/climon2-platform/src/notifier/macos.rs",
                cfg!(target_os = "macos"),
            ),
            (
                "crates/climon2-platform/src/connection/linux.rs",
                cfg!(target_os = "linux"),
            ),
            (
                "crates/climon2-platform/src/connection/macos.rs",
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
            ("crates/climon2-platform/src/home/windows.rs", cfg!(windows)),
            (
                "crates/climon2-platform/src/private/windows.rs",
                cfg!(windows),
            ),
            (
                "crates/climon2-launcher/src/handoff/windows.rs",
                cfg!(windows),
            ),
            ("crates/climon2-cli/src/windows.rs", cfg!(windows)),
            (
                "crates/climon2-platform/src/presentation/windows.rs",
                cfg!(windows),
            ),
            (
                "crates/climon2-platform/src/waiting/windows.rs",
                cfg!(windows),
            ),
            (
                "crates/climon2-platform/src/runtime/windows.rs",
                cfg!(windows),
            ),
            (
                "crates/climon2-platform/src/process/windows.rs",
                cfg!(windows),
            ),
            ("crates/climon2-platform/src/peer/windows.rs", cfg!(windows)),
            (
                "crates/climon2-platform/src/terminal/windows.rs",
                cfg!(windows),
            ),
            (
                "crates/climon2-platform/src/logging/windows.rs",
                cfg!(windows),
            ),
            (
                "crates/climon2-platform/src/daemon/windows.rs",
                cfg!(windows),
            ),
            ("crates/climon2-platform/src/pty/windows.rs", cfg!(windows)),
            (
                "crates/climon2-platform/src/command/windows.rs",
                cfg!(windows),
            ),
            ("crates/climon2-daemon/src/pty/windows.rs", cfg!(windows)),
            (
                "crates/climon2-platform/src/library/windows.rs",
                cfg!(windows),
            ),
            (
                "crates/climon2-platform/src/notifier/windows.rs",
                cfg!(windows),
            ),
            (
                "crates/climon2-platform/src/connection/windows.rs",
                cfg!(windows),
            ),
            (
                "crates/climon2-platform/src/native_path/windows.rs",
                cfg!(windows),
            ),
            (
                "crates/climon2-platform/src/handoff/windows.rs",
                cfg!(windows),
            ),
            ("crates/climon2-install/src/windows.rs", cfg!(windows)),
            (
                "crates/climon2-install/src/user_path/windows.rs",
                cfg!(windows),
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
