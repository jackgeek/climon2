//! `cargo xtask qualify`: release qualification (#122) of the exact
//! candidate artifacts against the acceptance matrix
//! (docs/monitor-acceptance.md).
//!
//! `qualify run` makes one platform's evidence records: every scenario's
//! required layers on this host, from the packaged tests against the
//! candidate archive, every workspace test from the candidate's source
//! revision on hook-enabled debug builds, this platform's Measure reports
//! and the manual records. A case's failure is a record, never a failed
//! command; only a harness error fails it.
//!
//! `qualify verdict` judges every platform's records with the owner's
//! recorded dispositions and fails unless the release may be published.

mod evidence;
mod sources;
mod verdict;

use evidence::{Manual, Report, Run, Source};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sources::Scenario;
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

/// A scenario's evidence layer (docs/monitor-acceptance.md, "Evidence
/// policy").
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Layer {
    Contract,
    Library,
    Monitor,
    Fault,
    Measure,
    Docs,
    HostManual,
}

impl Layer {
    pub(crate) const ALL: [Self; 7] = [
        Self::Contract,
        Self::Library,
        Self::Monitor,
        Self::Fault,
        Self::Measure,
        Self::Docs,
        Self::HostManual,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Contract => "Contract",
            Self::Library => "Library",
            Self::Monitor => "Monitor",
            Self::Fault => "Fault",
            Self::Measure => "Measure",
            Self::Docs => "Docs",
            Self::HostManual => "Host manual",
        }
    }

    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|layer| layer.name() == name)
    }
}

/// A qualified platform: x86-64 GNU/Linux on Ubuntu 24.04 and both macOS
/// architectures, each on its own native runner.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Platform {
    LinuxX86_64,
    MacosArm64,
    MacosX86_64,
}

impl Platform {
    pub(crate) const ALL: [Self; 3] = [Self::LinuxX86_64, Self::MacosArm64, Self::MacosX86_64];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::LinuxX86_64 => "linux-x86_64",
            Self::MacosArm64 => "macos-arm64",
            Self::MacosX86_64 => "macos-x86_64",
        }
    }

    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|platform| platform.name() == name)
    }

    pub(crate) fn os(self) -> &'static str {
        ["linux", "macos", "macos"][self as usize]
    }

    /// The Rust architecture that runs it natively.
    fn arch(self) -> &'static str {
        ["x86_64", "aarch64", "x86_64"][self as usize]
    }

    /// Whether this host is the platform itself: cross-compilation and
    /// another architecture prove nothing at runtime.
    fn is_host(self, os: &str, arch: &str) -> bool {
        self.os() == os && self.arch() == arch
    }
}

/// An evidence state (docs/monitor-acceptance.md, "Documentation and
/// evidence records").
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum State {
    NotImplemented,
    NotRun,
    Passed,
    Failed,
    Blocked,
}

impl State {
    const ALL: [Self; 5] = [
        Self::NotImplemented,
        Self::NotRun,
        Self::Passed,
        Self::Failed,
        Self::Blocked,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::NotImplemented => "not implemented",
            Self::NotRun => "not run",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
        }
    }

    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.name() == name)
    }
}

/// Where the owner records dispositions, and manual checks and reviews, in
/// the candidate's source tree.
const DISPOSITIONS: &str = "qualification/dispositions.json";
const MANUAL: &str = "qualification/manual.json";
const MATRIX: &str = "docs/monitor-acceptance.md";

/// `qualify run`'s options.
#[derive(Debug, Eq, PartialEq)]
struct RunOptions {
    platform: Platform,
    archive: PathBuf,
    out: PathBuf,
    measure: Option<PathBuf>,
    manual: Option<PathBuf>,
    provenance: String,
}

/// `qualify verdict`'s options.
#[derive(Debug, Eq, PartialEq)]
struct VerdictOptions {
    evidence: PathBuf,
    out: PathBuf,
    dispositions: Option<PathBuf>,
}

#[derive(Debug, Eq, PartialEq)]
enum Options {
    Run(RunOptions),
    Verdict(VerdictOptions),
}

/// Each `--flag value` pair; none for an unknown flag, a missing value or a
/// repeated flag.
fn flags<'a>(args: &'a [OsString], known: &[&str]) -> Option<Vec<(&'a str, &'a OsString)>> {
    let mut pairs: Vec<(&str, &OsString)> = Vec::new();
    for pair in args.chunks(2) {
        let [flag, value] = pair else { return None };
        let flag = flag.to_str().filter(|flag| known.contains(flag))?;
        if pairs.iter().any(|(seen, _)| *seen == flag) || value.is_empty() {
            return None;
        }
        pairs.push((flag, value));
    }
    Some(pairs)
}

fn value<'a>(pairs: &[(&str, &'a OsString)], flag: &str) -> Option<&'a OsString> {
    pairs
        .iter()
        .find(|(name, _)| *name == flag)
        .map(|(_, value)| *value)
}

fn parse(args: &[OsString]) -> Option<Options> {
    let (command, rest) = args.split_first()?;
    match command.to_str()? {
        "run" => {
            let known = [
                "--platform",
                "--archive",
                "--out",
                "--measure",
                "--manual",
                "--provenance",
            ];
            let pairs = flags(rest, &known)?;
            Some(Options::Run(RunOptions {
                platform: Platform::parse(value(&pairs, "--platform")?.to_str()?)?,
                archive: value(&pairs, "--archive")?.into(),
                out: value(&pairs, "--out")?.into(),
                measure: value(&pairs, "--measure").map(PathBuf::from),
                manual: value(&pairs, "--manual").map(PathBuf::from),
                provenance: value(&pairs, "--provenance")
                    .map_or("local", |text| text.to_str().unwrap_or("local"))
                    .to_owned(),
            }))
        }
        "verdict" => {
            let pairs = flags(rest, &["--evidence", "--out", "--dispositions"])?;
            Some(Options::Verdict(VerdictOptions {
                evidence: value(&pairs, "--evidence")?.into(),
                out: value(&pairs, "--out")?.into(),
                dispositions: value(&pairs, "--dispositions").map(PathBuf::from),
            }))
        }
        _ => None,
    }
}

pub(crate) const USAGE: &str = "qualify run --platform <linux-x86_64|macos-arm64|macos-x86_64> \
--archive <archive> --out <directory> [--measure <directory>] [--manual <file>] [--provenance <text>] \
| qualify verdict --evidence <directory> --out <directory> [--dispositions <file>]";

pub(crate) fn run(root: &Path, args: &[OsString]) -> io::Result<()> {
    match parse(args) {
        Some(Options::Run(options)) => record(root, &options),
        Some(Options::Verdict(options)) => judge(root, &options),
        None => Err(io::Error::other(format!("usage: cargo xtask {USAGE}"))),
    }
}

fn invalid(error: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

/// The matrix's scenarios.
fn matrix(root: &Path) -> io::Result<Vec<Scenario>> {
    sources::scenarios(&fs::read_to_string(root.join(MATRIX))?).map_err(invalid)
}

/// Every crate README's claims of the matrix's scenarios.
fn claims(root: &Path, scenarios: &[Scenario]) -> io::Result<Vec<sources::Claim>> {
    let ids: BTreeSet<String> = scenarios
        .iter()
        .map(|scenario| scenario.id.clone())
        .collect();
    let mut claims = Vec::new();
    for readme in readmes(root)? {
        let source = readme
            .strip_prefix(root)
            .unwrap_or(&readme)
            .display()
            .to_string();
        claims.extend(sources::claims(
            &fs::read_to_string(&readme)?,
            &source,
            &ids,
        ));
    }
    Ok(claims)
}

fn readmes(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut readmes: Vec<PathBuf> = fs::read_dir(root.join("crates"))?
        .map(|entry| entry.map(|entry| entry.path().join("README.md")))
        .collect::<io::Result<_>>()?;
    readmes.retain(|readme| readme.is_file());
    readmes.sort();
    Ok(readmes)
}

/// The packaged tests that return at once against an archive.
fn hooked(root: &Path) -> io::Result<BTreeSet<String>> {
    let mut hooked = BTreeSet::new();
    for entry in fs::read_dir(root.join("crates/climon2-e2e/tests"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            let found = sources::hooked_only(&fs::read_to_string(&path)?)
                .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
            hooked.extend(found);
        }
    }
    Ok(hooked)
}

fn record(root: &Path, options: &RunOptions) -> io::Result<()> {
    let platform = options.platform;
    if !platform.is_host(std::env::consts::OS, std::env::consts::ARCH) {
        return Err(io::Error::other(format!(
            "{} evidence needs a native {} host: cross-compilation proves nothing at runtime",
            platform.name(),
            platform.name()
        )));
    }
    let scenarios = matrix(root)?;
    let claims = claims(root, &scenarios)?;
    let hooked = hooked(root)?;
    let archive = fs::canonicalize(&options.archive)?;
    let sha256 = format!("{:x}", Sha256::digest(fs::read(&archive)?));
    let logs = options.out.join("logs");
    fs::create_dir_all(&logs)?;
    let runs = [
        test_run(root, Source::Archive, Some(&archive), &logs)?,
        test_run(root, Source::Workspace, None, &logs)?,
    ];
    let reports = match &options.measure {
        Some(directory) => measure_reports(directory)?,
        None => Vec::new(),
    };
    let manual = manual_records(&options.manual.clone().unwrap_or_else(|| root.join(MANUAL)))?;
    let common = json!({
        "source_revision": revision(root)?,
        "artifact": {
            "name": archive.file_name().map(|name| name.to_string_lossy().into_owned()),
            "sha256": sha256,
            "provenance": options.provenance,
        },
        "environment": environment(platform),
        "configuration": "each test's own configuration in an isolated home; defaults otherwise",
        "commands": runs.iter().map(run_json).collect::<Vec<_>>(),
    });
    let records = evidence::records(&evidence::Inputs {
        platform,
        scenarios: &scenarios,
        claims: &claims,
        hooked: &hooked,
        runs: &runs,
        reports: &reports,
        manual: &manual,
        sha256: &sha256,
        common: &common,
    });
    let document = json!({
        "format": "climon2-qualification-evidence/1",
        "platform": platform.name(),
        "records": records,
    });
    fs::write(
        options.out.join("evidence.json"),
        serde_json::to_string_pretty(&document)?,
    )?;
    let counts = State::ALL.map(|state| {
        let count = records
            .iter()
            .filter(|record| record["state"] == state.name())
            .count();
        format!("{count} {}", state.name())
    });
    eprintln!(
        "{} cases on {}: {}",
        records.len(),
        platform.name(),
        counts.join(", ")
    );
    Ok(())
}

/// Runs one set of tests, without stopping at the first failing binary,
/// and reads every result from its log.
fn test_run(root: &Path, source: Source, archive: Option<&Path>, logs: &Path) -> io::Result<Run> {
    let mut command = Command::new(env!("CARGO"));
    command.current_dir(root).args(test_args(source));
    match archive {
        Some(archive) => command.env("CLIMON2_E2E_ARCHIVE", archive),
        None => command.env_remove("CLIMON2_E2E_ARCHIVE"),
    };
    let log = logs.join(format!("{}.log", source.name()));
    let started = Instant::now();
    eprintln!(
        "> cargo {} ({} run)",
        test_args(source).join(" "),
        source.name()
    );
    let output = command.output()?;
    let text = [output.stdout, output.stderr].concat();
    fs::write(&log, &text)?;
    let text = String::from_utf8_lossy(&text);
    Ok(Run {
        source,
        command: format!("cargo {}", test_args(source).join(" ")),
        status: output.status.to_string(),
        seconds: started.elapsed().as_secs_f64(),
        log: log.display().to_string(),
        outcomes: evidence::outcomes(&text),
    })
}

/// The packaged tests alone run against the archive; the workspace run is
/// every test.
fn test_args(source: Source) -> Vec<&'static str> {
    let packages: &[&str] = match source {
        Source::Archive => &["-p", "climon2-e2e"],
        Source::Workspace => &["--workspace"],
    };
    let mut args = vec!["test", "--locked", "--no-fail-fast"];
    args.extend_from_slice(packages);
    args
}

fn run_json(run: &Run) -> Value {
    json!({
        "run": run.source.name(),
        "command": run.command,
        "result": run.status,
        "seconds": run.seconds,
        "log": run.log,
        "tests": run.outcomes.len(),
    })
}

/// Every `report.json` one level below `directory`, as the Measure
/// workflow's artifacts unpack.
fn measure_reports(directory: &Path) -> io::Result<Vec<Report>> {
    let mut reports = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path().join("report.json");
        if path.is_file() {
            let json = serde_json::from_str(&fs::read_to_string(&path)?)
                .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
            reports.push(Report {
                path: path.display().to_string(),
                json,
            });
        }
    }
    reports.sort_by(|one, other| one.path.cmp(&other.path));
    Ok(reports)
}

/// The manual records file: Host manual checks and documentation reviews.
fn manual_records(path: &Path) -> io::Result<Vec<Manual>> {
    let json: Value = serde_json::from_str(&fs::read_to_string(path)?)
        .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
    manual(&json).map_err(|error| invalid(format!("{}: {error}", path.display())))
}

fn manual(json: &Value) -> Result<Vec<Manual>, String> {
    let entries = json["records"].as_array().ok_or("no \"records\" array")?;
    entries
        .iter()
        .map(|entry| {
            let text = |name: &str| entry[name].as_str().unwrap_or_default().to_owned();
            let layer = Layer::parse(&text("layer"))
                .ok_or_else(|| format!("{}: unknown layer", text("scenario")))?;
            let passed = match text("result").as_str() {
                "passed" => true,
                "limitation" | "failed" => false,
                other => return Err(format!("{}: unknown result {other:?}", text("scenario"))),
            };
            Ok(Manual {
                scenario: text("scenario"),
                layer,
                os: text("os"),
                host: text("host"),
                gesture: text("gesture"),
                observed: text("observed"),
                passed,
                limitation: text("limitation"),
                recorded: text("recorded"),
            })
        })
        .collect()
}

fn revision(root: &Path) -> io::Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("git rev-parse HEAD failed"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn environment(platform: Platform) -> Value {
    let uname = Command::new("uname")
        .arg("-srvm")
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default();
    let runner = ["RUNNER_NAME", "ImageOS", "ImageVersion", "GITHUB_RUN_ID"]
        .map(|name| format!("{name}={}", std::env::var(name).unwrap_or_default()))
        .join(" ");
    json!({
        "platform": platform.name(),
        "os": uname,
        "architecture": std::env::consts::ARCH,
        "runner": runner,
    })
}

fn judge(root: &Path, options: &VerdictOptions) -> io::Result<()> {
    let scenarios = matrix(root)?;
    let path = options
        .dispositions
        .clone()
        .unwrap_or_else(|| root.join(DISPOSITIONS));
    let dispositions = verdict::dispositions(&fs::read_to_string(&path)?, &scenarios)
        .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
    let mut records = Vec::new();
    for file in evidence_files(&options.evidence)? {
        let document: Value = serde_json::from_str(&fs::read_to_string(&file)?)
            .map_err(|error| invalid(format!("{}: {error}", file.display())))?;
        records.extend(
            document["records"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned(),
        );
    }
    let verdict = verdict::verdict(&scenarios, &records, &dispositions).map_err(invalid)?;
    fs::create_dir_all(&options.out)?;
    let document = json!({
        "format": "climon2-qualification-verdict/1",
        "conformance": verdict.conformance.name(),
        "publishable": verdict.conformance.publishable(),
        "records": verdict.records,
    });
    fs::write(
        options.out.join("qualification.json"),
        serde_json::to_string_pretty(&document)?,
    )?;
    let summary = verdict::summary(&verdict);
    fs::write(options.out.join("qualification.md"), &summary)?;
    eprintln!("{}", verdict.conformance.name());
    if verdict.conformance.publishable() {
        Ok(())
    } else {
        Err(io::Error::other(
            "not qualified: a case neither passed nor has the owner's disposition; see qualification.md",
        ))
    }
}

/// Every `evidence.json` at or below `directory`, as the qualification
/// artifacts unpack.
fn evidence_files(directory: &Path) -> io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![directory.to_owned()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.file_name().is_some_and(|name| name == "evidence.json") {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    /// `qualify run` needs a platform, an archive and an output directory;
    /// Measure reports, the manual records and provenance are optional.
    /// Unknown, repeated or valueless flags and unknown platforms are
    /// refused.
    #[test]
    fn qualify_takes_its_flags_once_each() {
        assert_eq!(
            parse(&args(&[
                "run",
                "--archive",
                "a.tar.gz",
                "--platform",
                "linux-x86_64",
                "--out",
                "o",
            ])),
            Some(Options::Run(RunOptions {
                platform: Platform::LinuxX86_64,
                archive: "a.tar.gz".into(),
                out: "o".into(),
                measure: None,
                manual: None,
                provenance: "local".into(),
            }))
        );
        assert_eq!(
            parse(&args(&[
                "run",
                "--platform",
                "macos-x86_64",
                "--archive",
                "a",
                "--out",
                "o",
                "--measure",
                "m",
                "--manual",
                "r.json",
                "--provenance",
                "run 7",
            ])),
            Some(Options::Run(RunOptions {
                platform: Platform::MacosX86_64,
                archive: "a".into(),
                out: "o".into(),
                measure: Some("m".into()),
                manual: Some("r.json".into()),
                provenance: "run 7".into(),
            }))
        );
        assert_eq!(
            parse(&args(&["verdict", "--evidence", "e", "--out", "o"])),
            Some(Options::Verdict(VerdictOptions {
                evidence: "e".into(),
                out: "o".into(),
                dispositions: None,
            }))
        );
        for refused in [
            &["run", "--platform", "linux-x86_64", "--archive", "a"][..],
            &[
                "run",
                "--platform",
                "windows",
                "--archive",
                "a",
                "--out",
                "o",
            ],
            &[
                "run",
                "--platform",
                "linux-x86_64",
                "--archive",
                "a",
                "--out",
                "o",
                "--out",
                "p",
            ],
            &[
                "run",
                "--platform",
                "linux-x86_64",
                "--archive",
                "",
                "--out",
                "o",
            ],
            &[
                "run",
                "--platform",
                "linux-x86_64",
                "--archive",
                "a",
                "--out",
                "o",
                "--fast",
                "x",
            ],
            &[
                "run",
                "--platform",
                "linux-x86_64",
                "--archive",
                "a",
                "--out",
            ],
            &["verdict", "--evidence", "e"],
            &["publish"],
            &[],
        ] {
            assert_eq!(parse(&args(refused)), None, "{refused:?}");
        }
    }

    /// Each platform is only its own native host.
    #[test]
    fn each_platform_is_only_its_native_host() {
        assert!(Platform::LinuxX86_64.is_host("linux", "x86_64"));
        assert!(Platform::MacosArm64.is_host("macos", "aarch64"));
        assert!(Platform::MacosX86_64.is_host("macos", "x86_64"));
        assert!(!Platform::LinuxX86_64.is_host("linux", "aarch64"));
        assert!(!Platform::MacosArm64.is_host("macos", "x86_64"));
        assert!(!Platform::MacosX86_64.is_host("linux", "x86_64"));
        for platform in Platform::ALL {
            assert_eq!(Platform::parse(platform.name()), Some(platform));
        }
        for layer in Layer::ALL {
            assert_eq!(Layer::parse(layer.name()), Some(layer));
        }
        for state in State::ALL {
            assert_eq!(State::parse(state.name()), Some(state));
        }
    }

    /// The archive run is the packaged tests alone; the workspace run is
    /// every test; neither stops at a failing binary.
    #[test]
    fn test_runs_never_stop_at_the_first_failure() {
        assert_eq!(
            test_args(Source::Archive),
            ["test", "--locked", "--no-fail-fast", "-p", "climon2-e2e"]
        );
        assert_eq!(
            test_args(Source::Workspace),
            ["test", "--locked", "--no-fail-fast", "--workspace"]
        );
    }

    /// Manual records: a limitation or failed result does not pass; an
    /// unknown layer or result is refused.
    #[test]
    fn manual_records_are_read_strictly() {
        let records = json!({"records": [{
            "scenario": "FOC-02", "layer": "Host manual", "os": "linux", "host": "Konsole",
            "gesture": "Shift+drag", "observed": "copied", "result": "limitation",
            "limitation": "visible page only", "recorded": "owner",
        }]});
        let read = manual(&records).unwrap();
        assert_eq!(
            read,
            [Manual {
                scenario: "FOC-02".into(),
                layer: Layer::HostManual,
                os: "linux".into(),
                host: "Konsole".into(),
                gesture: "Shift+drag".into(),
                observed: "copied".into(),
                passed: false,
                limitation: "visible page only".into(),
                recorded: "owner".into(),
            }]
        );
        let mut passed = records.clone();
        passed["records"][0]["result"] = json!("passed");
        assert!(manual(&passed).unwrap()[0].passed);
        let mut unknown = records.clone();
        unknown["records"][0]["result"] = json!("fine");
        assert_eq!(
            manual(&unknown).unwrap_err(),
            "FOC-02: unknown result \"fine\""
        );
        let mut layer = records;
        layer["records"][0]["layer"] = json!("Manual");
        assert_eq!(manual(&layer).unwrap_err(), "FOC-02: unknown layer");
        assert_eq!(manual(&json!({})).unwrap_err(), "no \"records\" array");
    }

    fn root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
    }

    /// DOC-02 (Docs, traceability): the acceptance matrix is the complete
    /// 73-scenario matrix with every layer named, and every scenario-to-test
    /// row in a crate README names tests; the committed dispositions and
    /// manual records are valid against it.
    #[test]
    fn the_matrix_readmes_and_owner_records_are_consistent() {
        let scenarios = matrix(root()).unwrap();
        assert_eq!(scenarios.len(), 73);
        let cases: usize = scenarios.iter().map(|scenario| scenario.layers.len()).sum();
        assert_eq!(cases, 174);
        let claims = claims(root(), &scenarios).unwrap();
        assert!(claims.iter().all(|claim| !claim.tests.is_empty()));
        let claimed: BTreeSet<&str> = claims.iter().map(|claim| claim.id.as_str()).collect();
        assert!(
            claimed.contains("DEL-01") && claimed.contains("PERF-01"),
            "{claimed:?}"
        );
        let dispositions = fs::read_to_string(root().join(DISPOSITIONS)).unwrap();
        verdict::dispositions(&dispositions, &scenarios).unwrap();
        manual_records(&root().join(MANUAL)).unwrap();
        assert!(!hooked(root()).unwrap().is_empty());
    }
}
