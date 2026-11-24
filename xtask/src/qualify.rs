//! `cargo xtask qualify`: release qualification (#122) of the exact
//! candidate artifacts against the acceptance matrix
//! (docs/for-humans/behaviour/monitor/acceptance.md).
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

mod candidate;
mod evidence;
mod sources;
mod verdict;

use crate::quality;
use candidate::Stage;
use evidence::{Candidate, Manual, Report, Run, Source};
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

/// A scenario's evidence layer (docs/for-humans/behaviour/monitor/acceptance.md, "Evidence
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

/// A qualified platform: x86-64 GNU/Linux on Ubuntu 24.04, both macOS
/// architectures and x86-64 Windows 11 on `windows-2025` (#330), each on its
/// own native runner.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Platform {
    LinuxX86_64,
    MacosArm64,
    MacosX86_64,
    WindowsX86_64,
}

impl Platform {
    pub(crate) const ALL: [Self; 4] = [
        Self::LinuxX86_64,
        Self::MacosArm64,
        Self::MacosX86_64,
        Self::WindowsX86_64,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::LinuxX86_64 => "linux-x86_64",
            Self::MacosArm64 => "macos-arm64",
            Self::MacosX86_64 => "macos-x86_64",
            Self::WindowsX86_64 => "windows-x86_64",
        }
    }

    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|platform| platform.name() == name)
    }

    pub(crate) fn os(self) -> &'static str {
        ["linux", "macos", "macos", "windows"][self as usize]
    }

    /// The Rust architecture that runs it natively.
    fn arch(self) -> &'static str {
        ["x86_64", "aarch64", "x86_64", "x86_64"][self as usize]
    }

    /// The test runs that make up each source's run on this platform, as
    /// `cargo test` argument lists. Unix runs the packaged tests against the
    /// archive and every workspace test; Windows, where only part of the
    /// workspace builds, runs what its Test and Archive jobs run.
    fn suites(self, source: Source) -> Vec<Vec<&'static str>> {
        let windows = self == Self::WindowsX86_64;
        let lists: Vec<&[&'static str]> = match (source, windows) {
            (Source::Archive, false) => vec![&["-p", "climon2-e2e"]],
            (Source::Workspace, false) => vec![&["--workspace"]],
            (Source::Archive, true) => vec![
                &["-p", "climon2-launcher", "-p", "climon2-cli"],
                &["-p", "climon2-install", "--test", "archive_windows"],
            ],
            (Source::Workspace, true) => quality::WINDOWS_SUITE.to_vec(),
        };
        lists
            .into_iter()
            .map(|packages| {
                let mut args = vec!["test", "--locked", "--no-fail-fast"];
                args.extend_from_slice(packages);
                args
            })
            .collect()
    }

    /// Whether this host is the platform itself: cross-compilation and
    /// another architecture prove nothing at runtime.
    fn is_host(self, os: &str, arch: &str) -> bool {
        self.os() == os && self.arch() == arch
    }
}

/// An evidence state (docs/for-humans/behaviour/monitor/acceptance.md, "Documentation and
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
const MATRIX: &str = "docs/for-humans/behaviour/monitor/acceptance.md";
const WINDOWS_MATRIX: &str = "docs/for-humans/windows/qualification.md";

/// `qualify run`'s options.
#[derive(Debug, Eq, PartialEq)]
struct RunOptions {
    platform: Platform,
    archive: PathBuf,
    out: PathBuf,
    measure: Option<PathBuf>,
    manual: Option<PathBuf>,
    provenance: String,
    /// The latest release candidate, whose manual checklist records a final
    /// tag's Windows run takes; none for a release candidate.
    candidate: Option<Candidate>,
}

/// `qualify verdict`'s options.
#[derive(Debug, Eq, PartialEq)]
struct VerdictOptions {
    evidence: PathBuf,
    out: PathBuf,
    dispositions: Option<PathBuf>,
    stage: Stage,
}

/// `qualify candidate`'s options: the tag being released and a file of every
/// published release's tag, one per line.
#[derive(Debug, Eq, PartialEq)]
struct CandidateOptions {
    tag: String,
    releases: PathBuf,
}

#[derive(Debug, Eq, PartialEq)]
enum Options {
    Run(RunOptions),
    Verdict(VerdictOptions),
    Candidate(CandidateOptions),
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
                "--candidate",
                "--candidate-sha256",
            ];
            let pairs = flags(rest, &known)?;
            let candidate = match (
                value(&pairs, "--candidate"),
                value(&pairs, "--candidate-sha256"),
            ) {
                (Some(version), Some(sha256)) => Some(Candidate {
                    version: version.to_str()?.to_owned(),
                    sha256: sha256.to_str()?.to_owned(),
                }),
                (None, None) => None,
                _ => return None,
            };
            Some(Options::Run(RunOptions {
                candidate,
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
            let pairs = flags(rest, &["--evidence", "--out", "--dispositions", "--tag"])?;
            Some(Options::Verdict(VerdictOptions {
                evidence: value(&pairs, "--evidence")?.into(),
                out: value(&pairs, "--out")?.into(),
                dispositions: value(&pairs, "--dispositions").map(PathBuf::from),
                stage: Stage::of(value(&pairs, "--tag").map_or(Some(""), |tag| tag.to_str())?),
            }))
        }
        "candidate" => {
            let pairs = flags(rest, &["--tag", "--releases"])?;
            Some(Options::Candidate(CandidateOptions {
                tag: value(&pairs, "--tag")?.to_str()?.to_owned(),
                releases: value(&pairs, "--releases")?.into(),
            }))
        }
        _ => None,
    }
}

pub(crate) const USAGE: &str = "qualify run \
--platform <linux-x86_64|macos-arm64|macos-x86_64|windows-x86_64> \
--archive <archive> --out <directory> [--measure <directory>] [--manual <file>] [--provenance <text>] \
[--candidate <version> --candidate-sha256 <sha256>] \
| qualify verdict --evidence <directory> --out <directory> [--dispositions <file>] [--tag <tag>] \
| qualify candidate --tag <tag> --releases <file>";

pub(crate) fn run(root: &Path, args: &[OsString]) -> io::Result<()> {
    match parse(args) {
        Some(Options::Run(options)) => record(root, &options),
        Some(Options::Verdict(options)) => judge(root, &options),
        Some(Options::Candidate(options)) => candidate(root, &options),
        None => Err(io::Error::other(format!("usage: cargo xtask {USAGE}"))),
    }
}

fn invalid(error: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

/// The matrix's scenarios, on every platform, then the Windows-only
/// scenarios and manual checklist (#330), on Windows alone.
fn matrix(root: &Path) -> io::Result<Vec<Scenario>> {
    let mut scenarios = Vec::new();
    for (document, platforms) in [
        (MATRIX, &Platform::ALL[..]),
        (WINDOWS_MATRIX, &[Platform::WindowsX86_64][..]),
    ] {
        sources::scenarios(
            &fs::read_to_string(root.join(document))?,
            platforms,
            &mut scenarios,
        )
        .map_err(|error| invalid(format!("{document}: {error}")))?;
    }
    Ok(scenarios)
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
    let mut scenarios = matrix(root)?;
    let claims = claims(root, &scenarios)?;
    scenarios.retain(|scenario| scenario.platforms.contains(&platform));
    let hooked = hooked(root)?;
    // Absolute, not canonical: Windows' `tar` cannot read a verbatim
    // `\\?\` path.
    let archive = std::path::absolute(&options.archive)?;
    let sha256 = format!("{:x}", Sha256::digest(fs::read(&archive)?));
    let logs = options.out.join("logs");
    fs::create_dir_all(&logs)?;
    // The workspace run first: on Windows it builds the binaries the
    // archive run's tests start beside the installed product.
    let runs = [
        test_run(root, platform, Source::Workspace, None, &logs)?,
        test_run(root, platform, Source::Archive, Some(&archive), &logs)?,
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
        candidate: options.candidate.as_ref(),
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
fn test_run(
    root: &Path,
    platform: Platform,
    source: Source,
    archive: Option<&Path>,
    logs: &Path,
) -> io::Result<Run> {
    let log = logs.join(format!("{}.log", source.name()));
    let started = Instant::now();
    let mut text = Vec::new();
    let mut commands = Vec::new();
    let mut statuses = Vec::new();
    for args in platform.suites(source) {
        let mut command = Command::new(env!("CARGO"));
        command.current_dir(root).args(&args);
        match archive {
            Some(archive) => command.env("CLIMON2_E2E_ARCHIVE", archive),
            None => command.env_remove("CLIMON2_E2E_ARCHIVE"),
        };
        let line = format!("cargo {}", args.join(" "));
        eprintln!("> {line} ({} run)", source.name());
        let output = command.output()?;
        text.extend([output.stdout, output.stderr].concat());
        commands.push(line);
        statuses.push(output.status.to_string());
    }
    fs::write(&log, &text)?;
    let text = String::from_utf8_lossy(&text);
    Ok(Run {
        source,
        command: commands.join(" && "),
        status: statuses.join("; "),
        seconds: started.elapsed().as_secs_f64(),
        log: log.display().to_string(),
        outcomes: evidence::outcomes(&text),
    })
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
                candidate: Candidate {
                    version: text("candidate"),
                    sha256: text("sha256"),
                },
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
    let variable = |name: &str| std::env::var(name).unwrap_or_default();
    let runner = ["RUNNER_NAME", "ImageOS", "ImageVersion", "GITHUB_RUN_ID"]
        .map(|name| format!("{name}={}", variable(name)))
        .join(" ");
    let mut environment = json!({
        "platform": platform.name(),
        "os": uname,
        "architecture": std::env::consts::ARCH,
        "runner": runner,
        "runner_image": runner_image(&variable("ImageOS"), &variable("ImageVersion")),
    });
    if platform == Platform::WindowsX86_64 {
        // `ver` is a cmd.exe built-in: "Microsoft Windows [Version 10.0.26100.6899]".
        let ver = Command::new("cmd")
            .args(["/d", "/c", "ver"])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default();
        environment["windows_build"] = json!(windows_build(&ver));
    }
    environment
}

/// The hosted runner's image and its version, such as `win25 20251110.1`;
/// empty off a hosted runner.
fn runner_image(os: &str, version: &str) -> String {
    if os.is_empty() || version.is_empty() {
        return String::new();
    }
    format!("{os} {version}")
}

/// The Windows build number in `ver`'s output, such as `10.0.26100.6899`;
/// empty when it names none.
fn windows_build(ver: &str) -> String {
    ver.split_once("[Version ")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map_or_else(String::new, |(build, _)| build.trim().to_owned())
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
    let verdict =
        verdict::verdict(&scenarios, &records, &dispositions, options.stage).map_err(invalid)?;
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

/// `qualify candidate`: for a final tag, prints the latest published release
/// candidate of its version, after checking that `HEAD` differs from it only
/// in the version stamp and the owner's records; fails when there is none
/// or anything else changed. For a release candidate it prints nothing.
fn candidate(root: &Path, options: &CandidateOptions) -> io::Result<()> {
    if Stage::of(&options.tag) == Stage::Candidate {
        return Ok(());
    }
    let releases = fs::read_to_string(&options.releases)?;
    let latest = candidate::latest(&options.tag, &releases).ok_or_else(|| {
        io::Error::other(format!(
            "no release candidate of {} is published: the final tag takes the manual checklist of its latest candidate",
            options.tag
        ))
    })?;
    let versions = (
        latest.trim_start_matches('v'),
        options.tag.trim_start_matches('v'),
    );
    let git = |args: &[&str]| quality::output(Command::new("git").current_dir(root).args(args));
    let names = git(&["diff", "--name-only", "--no-renames", &latest, "HEAD"])?;
    let show = |revision: &str, path: &str| {
        git(&["show", &format!("{revision}:{path}")])
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let changed: Vec<String> = String::from_utf8_lossy(&names.stdout)
        .lines()
        .filter(|path| {
            !candidate::stamp_only(
                path,
                show(&latest, path).as_deref(),
                show("HEAD", path).as_deref(),
                versions,
            )
        })
        .map(str::to_owned)
        .collect();
    if !changed.is_empty() {
        return Err(io::Error::other(format!(
            "{} differs from {latest} beyond the version stamp and qualification/: {}",
            options.tag,
            changed.join(", ")
        )));
    }
    println!("{latest}");
    Ok(())
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
                candidate: None,
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
                "--candidate-sha256",
                "feed",
                "--candidate",
                "1.2.0-rc.2",
            ])),
            Some(Options::Run(RunOptions {
                platform: Platform::MacosX86_64,
                archive: "a".into(),
                out: "o".into(),
                measure: Some("m".into()),
                manual: Some("r.json".into()),
                provenance: "run 7".into(),
                candidate: Some(Candidate {
                    version: "1.2.0-rc.2".into(),
                    sha256: "feed".into(),
                }),
            }))
        );
        assert_eq!(
            parse(&args(&["verdict", "--evidence", "e", "--out", "o"])),
            Some(Options::Verdict(VerdictOptions {
                evidence: "e".into(),
                out: "o".into(),
                dispositions: None,
                stage: Stage::Final,
            }))
        );
        assert_eq!(
            parse(&args(&[
                "verdict",
                "--tag",
                "v1.2.0-rc.2",
                "--evidence",
                "e",
                "--out",
                "o"
            ])),
            Some(Options::Verdict(VerdictOptions {
                evidence: "e".into(),
                out: "o".into(),
                dispositions: None,
                stage: Stage::Candidate,
            }))
        );
        assert_eq!(
            parse(&args(&[
                "candidate",
                "--tag",
                "v1.2.0",
                "--releases",
                "r.txt"
            ])),
            Some(Options::Candidate(CandidateOptions {
                tag: "v1.2.0".into(),
                releases: "r.txt".into(),
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
            &[
                "run",
                "--platform",
                "windows-x86_64",
                "--archive",
                "a",
                "--out",
                "o",
                "--candidate",
                "1.2.0-rc.2",
            ],
            &["candidate", "--tag", "v1.2.0"],
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
        assert!(Platform::WindowsX86_64.is_host("windows", "x86_64"));
        assert!(!Platform::WindowsX86_64.is_host("linux", "x86_64"));
        assert!(!Platform::WindowsX86_64.is_host("windows", "aarch64"));
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

    /// On Unix the archive run is the packaged tests alone and the
    /// workspace run every test. On Windows the archive run is the CLI's
    /// packaged tests and the installer's archive tests, as the Test and
    /// Archive jobs run them, and the workspace run the Windows suite. No
    /// run stops at a failing binary.
    #[test]
    fn test_runs_never_stop_at_the_first_failure() {
        let unix = Platform::LinuxX86_64;
        assert_eq!(
            unix.suites(Source::Archive),
            [["test", "--locked", "--no-fail-fast", "-p", "climon2-e2e"]]
        );
        assert_eq!(
            unix.suites(Source::Workspace),
            [["test", "--locked", "--no-fail-fast", "--workspace"]]
        );
        assert_eq!(
            Platform::MacosArm64.suites(Source::Archive),
            unix.suites(Source::Archive)
        );
        let windows = Platform::WindowsX86_64;
        assert_eq!(
            windows.suites(Source::Archive),
            [
                vec![
                    "test",
                    "--locked",
                    "--no-fail-fast",
                    "-p",
                    "climon2-launcher",
                    "-p",
                    "climon2-cli"
                ],
                vec![
                    "test",
                    "--locked",
                    "--no-fail-fast",
                    "-p",
                    "climon2-install",
                    "--test",
                    "archive_windows"
                ],
            ]
        );
        let suite = windows.suites(Source::Workspace);
        assert_eq!(suite.len(), quality::WINDOWS_SUITE.len());
        for (run, packages) in suite.iter().zip(quality::WINDOWS_SUITE) {
            assert_eq!(run[..3], ["test", "--locked", "--no-fail-fast"]);
            assert_eq!(&run[3..], *packages);
        }
    }

    /// Every record names the hosted runner's image, and a Windows record
    /// its Windows build, read from `ver` (#330); off a hosted runner, or
    /// from output naming no version, they are empty.
    #[test]
    fn records_name_the_runner_image_and_windows_build() {
        assert_eq!(runner_image("win25", "20251110.1"), "win25 20251110.1");
        assert_eq!(runner_image("", "20251110.1"), "");
        assert_eq!(runner_image("win25", ""), "");
        assert_eq!(
            windows_build("\r\nMicrosoft Windows [Version 10.0.26100.6899]\r\n"),
            "10.0.26100.6899"
        );
        assert_eq!(windows_build("Microsoft Windows"), "");
        assert_eq!(windows_build("[Version 10.0"), "");
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
                candidate: Candidate::default(),
            }]
        );
        let mut passed = records.clone();
        passed["records"][0]["result"] = json!("passed");
        assert!(manual(&passed).unwrap()[0].passed);
        let mut checklist = passed.clone();
        checklist["records"][0]["candidate"] = json!("1.2.0-rc.2");
        checklist["records"][0]["sha256"] = json!("feed");
        assert_eq!(
            manual(&checklist).unwrap()[0].candidate,
            Candidate {
                version: "1.2.0-rc.2".into(),
                sha256: "feed".into(),
            }
        );
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
    /// 73-scenario matrix with every layer named, followed by the Windows
    /// scenarios (#330), WIN-01 to WIN-11 in the Contract layer and the
    /// checklist, WINM-01 to WINM-16 in the Host manual layer, each for both
    /// hosts but WINM-12, VS Code's alone. Every scenario-to-test row in a
    /// crate README names tests, and each WIN scenario is claimed; the
    /// committed dispositions and manual records are valid against it.
    #[test]
    fn the_matrix_readmes_and_owner_records_are_consistent() {
        let scenarios = matrix(root()).unwrap();
        assert_eq!(scenarios.len(), 100);
        let (shared, windows) = scenarios.split_at(73);
        assert!(
            shared
                .iter()
                .all(|scenario| scenario.platforms == Platform::ALL)
        );
        let cases: usize = shared.iter().map(|scenario| scenario.layers.len()).sum();
        assert_eq!(cases, 174);
        let hosts = ["Windows Terminal".to_owned(), "VS Code".to_owned()];
        for (index, scenario) in windows.iter().enumerate() {
            assert_eq!(scenario.platforms, [Platform::WindowsX86_64]);
            let (id, layer, hosts) = match index {
                0..11 => (
                    format!("WIN-{:02}", index + 1),
                    Layer::Contract,
                    &hosts[..0],
                ),
                22 => ("WINM-12".to_owned(), Layer::HostManual, &hosts[1..]),
                _ => (
                    format!("WINM-{:02}", index - 10),
                    Layer::HostManual,
                    &hosts[..],
                ),
            };
            assert_eq!(
                (&scenario.id, &scenario.layers[..], &scenario.hosts[..]),
                (&id, &[layer][..], hosts)
            );
        }
        let claims = claims(root(), &scenarios).unwrap();
        assert!(claims.iter().all(|claim| !claim.tests.is_empty()));
        let claimed: BTreeSet<&str> = claims.iter().map(|claim| claim.id.as_str()).collect();
        assert!(
            claimed.contains("DEL-01") && claimed.contains("PERF-01"),
            "{claimed:?}"
        );
        for scenario in &windows[..11] {
            let contract = claims.iter().any(|claim| {
                claim.id == scenario.id
                    && claim.layers.contains(&Layer::Contract)
                    && !claim.supplement
            });
            assert!(contract, "{} has no Contract claim", scenario.id);
        }
        let dispositions = fs::read_to_string(root().join(DISPOSITIONS)).unwrap();
        verdict::dispositions(&dispositions, &scenarios).unwrap();
        manual_records(&root().join(MANUAL)).unwrap();
        assert!(!hooked(root()).unwrap().is_empty());
    }
}
