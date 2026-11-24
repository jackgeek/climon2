//! Evidence records (#122): what each test pass, Measure report and manual
//! record shows of every scenario case on one platform. Pure: the runner
//! collects the inputs and writes the records.

use super::{Layer, Platform, State, sources::Claim, sources::Scenario};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// One test's outcome as libtest printed it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Passed,
    Failed,
    Ignored,
}

impl Outcome {
    fn name(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Ignored => "ignored",
        }
    }
}

/// Every `test <name> ... <result>` line of libtest's output, in order.
pub(crate) fn outcomes(log: &str) -> Vec<(String, Outcome)> {
    log.lines()
        .filter_map(|line| {
            let (name, result) = line.strip_prefix("test ")?.split_once(" ... ")?;
            let outcome = match result.split(',').next()? {
                "ok" => Outcome::Passed,
                "FAILED" => Outcome::Failed,
                "ignored" => Outcome::Ignored,
                _ => return None,
            };
            Some((name.to_owned(), outcome))
        })
        .collect()
}

/// Whether the test libtest names `name` is the one `selector` names: the
/// same path or a suffix of it, or any test under a `path::*` glob.
pub(crate) fn selects(selector: &str, name: &str) -> bool {
    match selector.strip_suffix("::*") {
        Some(prefix) => {
            name.starts_with(&format!("{prefix}::")) || name.contains(&format!("::{prefix}::"))
        }
        None => name == selector || name.ends_with(&format!("::{selector}")),
    }
}

/// Which run of the tests a result comes from.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Source {
    /// Packaged tests against the exact candidate archive.
    Archive,
    /// Every workspace test from the candidate's source revision, on
    /// hook-enabled debug builds.
    Workspace,
}

impl Source {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Workspace => "workspace",
        }
    }

    /// The runs whose results prove `layer`: only the candidate archive
    /// shows actual packaged libraries and the actual packaged monitor;
    /// contracts, faults and documentation checks hold from either.
    fn proves(self, layer: Layer) -> bool {
        match layer {
            Layer::Library | Layer::Monitor => self == Self::Archive,
            Layer::Contract | Layer::Fault | Layer::Docs => true,
            Layer::Measure | Layer::HostManual => false,
        }
    }
}

/// One test run and what it showed.
#[derive(Clone, Debug)]
pub(crate) struct Run {
    pub(crate) source: Source,
    pub(crate) command: String,
    pub(crate) status: String,
    pub(crate) seconds: f64,
    pub(crate) log: String,
    pub(crate) outcomes: Vec<(String, Outcome)>,
}

/// One Measure report (`climon2-measure`'s `report.json`), where it was
/// found.
#[derive(Clone, Debug)]
pub(crate) struct Report {
    pub(crate) path: String,
    pub(crate) json: Value,
}

/// A published release candidate: its version and its Windows zip's
/// SHA-256 (#330).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Candidate {
    pub(crate) version: String,
    pub(crate) sha256: String,
}

/// A manual record: a Host manual check (FOC-02), a Windows manual checklist
/// item (WINM-01 to WINM-16) or a documentation review.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Manual {
    pub(crate) scenario: String,
    pub(crate) layer: Layer,
    /// `linux`, `macos`, or `any`.
    pub(crate) os: String,
    pub(crate) host: String,
    pub(crate) gesture: String,
    pub(crate) observed: String,
    /// Whether it worked; a host that did not is a recorded limitation.
    pub(crate) passed: bool,
    pub(crate) limitation: String,
    pub(crate) recorded: String,
    /// The release candidate a checklist record was run on; empty for any
    /// other record.
    pub(crate) candidate: Candidate,
}

/// Every input one platform's records are made from.
pub(crate) struct Inputs<'a> {
    pub(crate) platform: Platform,
    pub(crate) scenarios: &'a [Scenario],
    pub(crate) claims: &'a [Claim],
    /// Tests that return at once against an archive.
    pub(crate) hooked: &'a BTreeSet<String>,
    pub(crate) runs: &'a [Run],
    pub(crate) reports: &'a [Report],
    pub(crate) manual: &'a [Manual],
    /// The latest release candidate, whose checklist records count; none
    /// for a release candidate's own run.
    pub(crate) candidate: Option<&'a Candidate>,
    /// The candidate archive's SHA-256.
    pub(crate) sha256: &'a str,
    /// What every record shares: revision, artifact, environment.
    pub(crate) common: &'a Value,
}

/// A case's state, what was observed, and any limitation.
struct Finding {
    state: State,
    observed: String,
    limitation: String,
    evidence: Value,
}

/// Every scenario case's record on this platform: one per required layer.
pub(crate) fn records(inputs: &Inputs<'_>) -> Vec<Value> {
    let mut records = Vec::new();
    for scenario in inputs.scenarios {
        for &layer in &scenario.layers {
            let finding = finding(inputs, scenario, layer);
            let mut record = inputs.common.clone();
            let fields = json!({
                "scenario": scenario.id,
                "layer": layer.name(),
                "platform": inputs.platform.name(),
                "state": finding.state.name(),
                "required_layers": scenario.layers.iter().map(|layer| layer.name()).collect::<Vec<_>>(),
                "setup": scenario.setup,
                "expected": scenario.expected,
                "observed": finding.observed,
                "limitation": finding.limitation,
                "evidence": finding.evidence,
            });
            merge(&mut record, fields);
            records.push(record);
        }
    }
    records
}

fn merge(record: &mut Value, fields: Value) {
    if let (Some(record), Value::Object(fields)) = (record.as_object_mut(), fields) {
        record.extend(fields);
    }
}

fn finding(inputs: &Inputs<'_>, scenario: &Scenario, layer: Layer) -> Finding {
    match layer {
        Layer::Measure => measured(inputs),
        Layer::HostManual if scenario.is_checklist() => checklist(inputs, scenario, layer),
        Layer::HostManual => manual(inputs, scenario, layer),
        layer => tested(inputs, scenario, layer),
    }
}

/// A tested layer: every claimed test that ran in a run proving `layer`
/// (a hooked-only test never counts from the archive), and, for Docs, any
/// passed review record. No claim is not implemented; claims that never
/// ran are not run; a failure fails; a test skipped is blocked.
fn tested(inputs: &Inputs<'_>, scenario: &Scenario, layer: Layer) -> Finding {
    let claims: Vec<_> = inputs
        .claims
        .iter()
        .filter(|claim| claim.id == scenario.id)
        .collect();
    let proving: Vec<_> = claims
        .iter()
        .filter(|claim| !claim.supplement && claim.layers.contains(&layer))
        .collect();
    let reviews = reviews(inputs, scenario, layer);
    let mut results = Vec::new();
    for claim in &proving {
        for selector in &claim.tests {
            results.extend(ran(inputs, selector, layer));
        }
    }
    let selectors: Vec<Value> = proving
        .iter()
        .flat_map(|claim| {
            claim
                .tests
                .iter()
                .map(|test| json!({"test": test, "claimed_in": claim.source}))
        })
        .collect();
    let supplements: Vec<&String> = claims
        .iter()
        .filter(|claim| claim.supplement)
        .flat_map(|claim| &claim.tests)
        .collect();
    let state = tested_state(proving.is_empty() && reviews.is_empty(), &results, &reviews);
    Finding {
        state,
        observed: observed(&results, &reviews),
        limitation: String::new(),
        evidence: json!({
            "test_selectors": selectors,
            "results": results.iter().map(|(source, name, outcome)| json!({
                "run": source.name(), "test": name, "result": outcome.name(),
            })).collect::<Vec<_>>(),
            "supplements": supplements,
            "reviews": reviews.iter().map(|review| manual_json(review)).collect::<Vec<_>>(),
        }),
    }
}

/// Every result of `selector` in the runs that prove `layer`, leaving out a
/// hooked-only test's trivial pass against the archive.
fn ran(inputs: &Inputs<'_>, selector: &str, layer: Layer) -> Vec<(Source, String, Outcome)> {
    let hooked = |source: Source, name: &str| {
        source == Source::Archive && inputs.hooked.iter().any(|test| selects(test, name))
    };
    let mut results = Vec::new();
    for run in inputs.runs.iter().filter(|run| run.source.proves(layer)) {
        for (name, outcome) in &run.outcomes {
            if selects(selector, name) && !hooked(run.source, name) {
                results.push((run.source, name.clone(), *outcome));
            }
        }
    }
    results
}

fn tested_state(
    unclaimed: bool,
    results: &[(Source, String, Outcome)],
    reviews: &[&Manual],
) -> State {
    let outcomes: Vec<Outcome> = results.iter().map(|(_, _, outcome)| *outcome).collect();
    let reviewed = reviews
        .iter()
        .map(|review| [Outcome::Failed, Outcome::Passed][usize::from(review.passed)]);
    let all: Vec<Outcome> = outcomes.into_iter().chain(reviewed).collect();
    if unclaimed {
        State::NotImplemented
    } else if all.contains(&Outcome::Failed) {
        State::Failed
    } else if all.contains(&Outcome::Passed) {
        [State::Passed, State::Blocked][usize::from(all.contains(&Outcome::Ignored))]
    } else {
        [State::NotRun, State::Blocked][usize::from(all.contains(&Outcome::Ignored))]
    }
}

fn observed(results: &[(Source, String, Outcome)], reviews: &[&Manual]) -> String {
    let mut counts: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for (source, _, outcome) in results {
        *counts.entry((source.name(), outcome.name())).or_default() += 1;
    }
    let mut parts: Vec<String> = counts
        .iter()
        .map(|((run, result), count)| format!("{count} {result} in the {run} run"))
        .collect();
    parts.extend(
        reviews
            .iter()
            .map(|review| format!("reviewed: {}", review.observed)),
    );
    if parts.is_empty() {
        return "no claimed test ran".to_owned();
    }
    parts.join("; ")
}

/// The review records of `scenario`'s `layer` on this platform's OS.
fn reviews<'a>(inputs: &'a Inputs<'_>, scenario: &Scenario, layer: Layer) -> Vec<&'a Manual> {
    let os = inputs.platform.os();
    inputs
        .manual
        .iter()
        .filter(|record| record.scenario == scenario.id && record.layer == layer)
        .filter(|record| record.os == os || record.os == "any")
        .collect()
}

/// The Host manual check: three host terminals on this platform's OS
/// (owner decision on #122). A host that failed is a recorded limitation,
/// not a failure; fewer than three hosts recorded is not run.
fn manual(inputs: &Inputs<'_>, scenario: &Scenario, layer: Layer) -> Finding {
    let records = reviews(inputs, scenario, layer);
    let hosts: BTreeSet<&str> = records.iter().map(|record| record.host.as_str()).collect();
    let limitations: Vec<String> = records
        .iter()
        .filter(|record| !record.passed)
        .map(|record| format!("{}: {}", record.host, record.limitation))
        .collect();
    let state = [State::NotRun, State::Passed][usize::from(hosts.len() >= MANUAL_HOSTS)];
    Finding {
        state,
        observed: format!(
            "{} of {MANUAL_HOSTS} host terminals recorded on {}",
            hosts.len(),
            inputs.platform.os()
        ),
        limitation: limitations.join("; "),
        evidence: json!({"manual": records.iter().map(|record| manual_json(record)).collect::<Vec<_>>()}),
    }
}

/// A Windows manual checklist item (#330): it passes with a passed record
/// for each of its hosts run on the latest release candidate, and fails
/// with any failed or limited one. Records of another candidate never
/// count, and with no candidate named (a release candidate's own run) it is
/// not run.
fn checklist(inputs: &Inputs<'_>, scenario: &Scenario, layer: Layer) -> Finding {
    let Some(candidate) = inputs.candidate else {
        return Finding {
            state: State::NotRun,
            observed: "no release candidate named: the checklist runs on a published candidate"
                .to_owned(),
            limitation: String::new(),
            evidence: json!({"manual": []}),
        };
    };
    let records: Vec<&Manual> = reviews(inputs, scenario, layer)
        .into_iter()
        .filter(|record| record.candidate == *candidate && scenario.hosts.contains(&record.host))
        .collect();
    let passed = scenario
        .hosts
        .iter()
        .filter(|host| {
            records
                .iter()
                .any(|record| record.host == **host && record.passed)
        })
        .count();
    let failures: Vec<String> = records
        .iter()
        .filter(|record| !record.passed)
        .map(|record| format!("{}: {}", record.host, record.observed))
        .collect();
    let state = if !failures.is_empty() {
        State::Failed
    } else if passed == scenario.hosts.len() {
        State::Passed
    } else {
        State::NotRun
    };
    Finding {
        state,
        observed: format!(
            "{passed} of {} hosts passed on {}",
            scenario.hosts.len(),
            candidate.version
        ),
        limitation: failures.join("; "),
        evidence: json!({"manual": records.iter().map(|record| manual_json(record)).collect::<Vec<_>>()}),
    }
}

/// Host terminals each OS's FOC-02 check covers (owner decision on #122).
pub(crate) const MANUAL_HOSTS: usize = 3;

fn result(passed: bool) -> &'static str {
    ["limitation", "passed"][usize::from(passed)]
}

fn manual_json(record: &Manual) -> Value {
    json!({
        "host": record.host,
        "gesture": record.gesture,
        "observed": record.observed,
        "result": result(record.passed),
        "limitation": record.limitation,
        "recorded": record.recorded,
        "candidate": record.candidate.version,
        "sha256": record.candidate.sha256,
    })
}

/// Geometries the Measure workflow runs, one report each.
const GEOMETRIES: [&str; 3] = ["80x24", "200x60", "500x200"];

/// A Measure case on this platform: its reports must cover every geometry
/// of this exact archive over the decided window. A case or requirement
/// that missed is failed (it needs the owner's written disposition), one
/// not run is not run, and a report of another artifact or a reduced
/// window is blocked.
fn measured(inputs: &Inputs<'_>) -> Finding {
    let reports = inputs.reports;
    let cases: Vec<&Value> = reports
        .iter()
        .flat_map(|report| report.json["cases"].as_array().into_iter().flatten())
        .collect();
    let id = |case: &&Value| case["id"].as_str().unwrap_or_default().to_owned();
    let result = |case: &&Value| {
        case["outcome"]["result"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    let failed: Vec<String> = cases
        .iter()
        .filter(|case| result(case) == "failed")
        .map(id)
        .collect();
    let not_run: Vec<String> = cases
        .iter()
        .filter(|case| result(case) == "not-run")
        .map(id)
        .collect();
    let short: Vec<String> = reports
        .iter()
        .flat_map(|report| {
            report.json["qualification"]["findings"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .filter_map(|finding| finding.as_str().map(str::to_owned))
        .collect();
    let missing: Vec<&str> = GEOMETRIES
        .into_iter()
        .filter(|geometry| {
            !cases
                .iter()
                .any(|case| id(case).contains(&format!("/{geometry}/")))
        })
        .collect();
    let foreign: Vec<&str> = reports
        .iter()
        .filter(|report| !measures(&report.json, inputs.sha256))
        .map(|report| report.path.as_str())
        .collect();
    let state = measure_state(&foreign, &failed, &short, &missing, &not_run);
    Finding {
        state,
        observed: format!(
            "{} Measure cases in {} reports: {} failed, {} not run; {} requirements short",
            cases.len(),
            reports.len(),
            failed.len(),
            not_run.len(),
            short.len()
        ),
        limitation: String::new(),
        evidence: json!({
            "reports": reports.iter().map(|report| report.path.as_str()).collect::<Vec<_>>(),
            "failed_cases": failed,
            "not_run_cases": not_run,
            "requirement_findings": short,
            "geometries_missing": missing,
            "reports_not_of_this_candidate": foreign,
        }),
    }
}

/// Whether a report measured the archive with SHA-256 `sha256` over the
/// decided window.
fn measures(report: &Value, sha256: &str) -> bool {
    let environment = &report["environment"];
    let candidate = environment["candidate"].as_str().unwrap_or_default();
    let duration = environment["duration"].as_str().unwrap_or("REDUCED");
    candidate.ends_with(&format!("(sha256 {sha256})")) && !duration.contains("REDUCED")
}

fn measure_state(
    foreign: &[&str],
    failed: &[String],
    short: &[String],
    missing: &[&str],
    not_run: &[String],
) -> State {
    if !foreign.is_empty() {
        State::Blocked
    } else if !failed.is_empty() || !short.is_empty() {
        State::Failed
    } else if !missing.is_empty() || !not_run.is_empty() {
        State::NotRun
    } else {
        State::Passed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario(id: &str, layers: &[Layer]) -> Scenario {
        Scenario {
            id: id.to_owned(),
            setup: format!("{id} setup"),
            expected: format!("{id} expected"),
            layers: layers.to_vec(),
            platforms: Platform::ALL.to_vec(),
            hosts: Vec::new(),
        }
    }

    fn claim(id: &str, layers: &[Layer], supplement: bool, tests: &[&str]) -> Claim {
        Claim {
            id: id.to_owned(),
            layers: layers.to_vec(),
            supplement,
            tests: tests.iter().map(|test| (*test).to_owned()).collect(),
            source: "crates/x/README.md".to_owned(),
        }
    }

    fn run(source: Source, outcomes: &[(&str, Outcome)]) -> Run {
        Run {
            source,
            command: "cargo test".to_owned(),
            status: "exit status: 0".to_owned(),
            seconds: 1.0,
            log: "logs/run.log".to_owned(),
            outcomes: outcomes
                .iter()
                .map(|(name, outcome)| ((*name).to_owned(), *outcome))
                .collect(),
        }
    }

    struct Fixture {
        scenarios: Vec<Scenario>,
        claims: Vec<Claim>,
        hooked: BTreeSet<String>,
        runs: Vec<Run>,
        reports: Vec<Report>,
        manual: Vec<Manual>,
        candidate: Option<Candidate>,
        common: Value,
    }

    impl Fixture {
        fn new(scenarios: Vec<Scenario>, claims: Vec<Claim>, runs: Vec<Run>) -> Self {
            Self {
                scenarios,
                claims,
                hooked: BTreeSet::new(),
                runs,
                reports: Vec::new(),
                manual: Vec::new(),
                candidate: None,
                common: json!({"source_revision": "abc123"}),
            }
        }

        fn records(&self, platform: Platform) -> Vec<Value> {
            records(&Inputs {
                platform,
                scenarios: &self.scenarios,
                claims: &self.claims,
                hooked: &self.hooked,
                runs: &self.runs,
                reports: &self.reports,
                manual: &self.manual,
                candidate: self.candidate.as_ref(),
                sha256: "feed",
                common: &self.common,
            })
        }

        /// Each record's scenario, layer and state.
        fn states(&self, platform: Platform) -> Vec<(String, String, String)> {
            self.records(platform)
                .iter()
                .map(|record| {
                    let text = |name: &str| record[name].as_str().unwrap().to_owned();
                    (text("scenario"), text("layer"), text("state"))
                })
                .collect()
        }
    }

    fn states(list: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
        list.iter()
            .map(|(id, layer, state)| ((*id).to_owned(), (*layer).to_owned(), (*state).to_owned()))
            .collect()
    }

    /// libtest's result lines, an ignored test with a reason included, are
    /// read in order; every other line is not.
    #[test]
    fn libtest_result_lines_are_read() {
        let log = "\
     Running tests/monitor.rs (target/debug/deps/monitor-1)
running 4 tests
test a_test ... ok
test module::tests::b_test ... FAILED
test c_test ... ignored
test d_test ... ignored, needs a host
test e_test has been running for over 60 seconds
test result: FAILED. 1 passed; 1 failed; 2 ignored
";
        assert_eq!(
            outcomes(log),
            [
                ("a_test".to_owned(), Outcome::Passed),
                ("module::tests::b_test".to_owned(), Outcome::Failed),
                ("c_test".to_owned(), Outcome::Ignored),
                ("d_test".to_owned(), Outcome::Ignored),
            ]
        );
    }

    /// A selector names a test by its whole path or a suffix of it, or a
    /// module's tests by a glob; never a longer or shorter name.
    #[test]
    fn selectors_name_whole_paths_suffixes_or_globs() {
        assert!(selects("a_test", "a_test"));
        assert!(selects("a_test", "supervisor::tests::a_test"));
        assert!(selects("tests::a_test", "supervisor::tests::a_test"));
        assert!(!selects("a_test", "a_test_more"));
        assert!(!selects("a_test", "not_a_test"));
        assert!(selects("workload::tests::*", "workload::tests::rates"));
        assert!(selects("tests::*", "workload::tests::rates"));
        assert!(!selects("workload::tests::*", "workload::testsuite::rates"));
        assert!(!selects(
            "workload::tests::*",
            "other::workload_tests::rates"
        ));
    }

    /// Conformance states (#122): an unclaimed layer is not implemented;
    /// claims whose tests never ran are not run; any failure fails; a
    /// skipped (ignored) test blocks; supplements prove no layer; Monitor
    /// and Library hold only from the archive run, contracts and faults from
    /// either.
    #[test]
    fn each_case_takes_the_state_its_claimed_tests_show() {
        use Layer::*;
        let fixture = Fixture::new(
            vec![
                scenario("ENT-01", &[Monitor]),
                scenario("DSC-01", &[Contract, Monitor]),
                scenario("DSC-02", &[Monitor, Fault]),
                scenario("DSC-03", &[Contract]),
                scenario("DSC-04", &[Library]),
                scenario("DSC-05", &[Fault]),
            ],
            vec![
                claim("ENT-01", &[Monitor], false, &["opens"]),
                claim("DSC-01", &[Contract], false, &["orders"]),
                claim("DSC-01", &[Monitor], false, &["workspace_only"]),
                claim("DSC-02", &[Monitor], false, &["fails_once", "passes"]),
                claim("DSC-02", &[Fault], false, &["skipped"]),
                claim("DSC-03", &[], true, &["pure"]),
                claim("DSC-04", &[Library], false, &["never_built"]),
                claim("DSC-05", &[Fault], false, &["module::tests::*"]),
            ],
            vec![
                run(
                    Source::Archive,
                    &[
                        ("opens", Outcome::Passed),
                        ("fails_once", Outcome::Failed),
                        ("passes", Outcome::Passed),
                    ],
                ),
                run(
                    Source::Workspace,
                    &[
                        ("orders", Outcome::Passed),
                        ("workspace_only", Outcome::Passed),
                        ("fails_once", Outcome::Passed),
                        ("skipped", Outcome::Ignored),
                        ("pure", Outcome::Passed),
                        ("module::tests::one", Outcome::Passed),
                        ("module::tests::two", Outcome::Passed),
                    ],
                ),
            ],
        );
        assert_eq!(
            fixture.states(Platform::LinuxX86_64),
            states(&[
                ("ENT-01", "Monitor", "passed"),
                ("DSC-01", "Contract", "passed"),
                ("DSC-01", "Monitor", "not run"),
                ("DSC-02", "Monitor", "failed"),
                ("DSC-02", "Fault", "blocked"),
                ("DSC-03", "Contract", "not implemented"),
                ("DSC-04", "Library", "not run"),
                ("DSC-05", "Fault", "passed"),
            ])
        );
    }

    /// A test skipped beside one that passed still blocks its case; a
    /// failed one beside a skipped one fails it.
    #[test]
    fn a_skip_blocks_and_a_failure_wins() {
        use Layer::*;
        let fixture = Fixture::new(
            vec![
                scenario("ENT-01", &[Contract]),
                scenario("DSC-01", &[Contract]),
            ],
            vec![
                claim("ENT-01", &[Contract], false, &["ran", "skipped"]),
                claim("DSC-01", &[Contract], false, &["broke", "skipped"]),
            ],
            vec![run(
                Source::Workspace,
                &[
                    ("ran", Outcome::Passed),
                    ("skipped", Outcome::Ignored),
                    ("broke", Outcome::Failed),
                ],
            )],
        );
        assert_eq!(
            fixture.states(Platform::MacosArm64),
            states(&[
                ("ENT-01", "Contract", "blocked"),
                ("DSC-01", "Contract", "failed")
            ])
        );
    }

    /// A hooked-only test's pass against the archive proves nothing: its
    /// Monitor claim is not run there, while its Fault claim holds from the
    /// workspace run.
    #[test]
    fn a_hooked_only_test_never_counts_from_the_archive() {
        use Layer::*;
        let mut fixture = Fixture::new(
            vec![scenario("REC-01", &[Monitor, Fault])],
            vec![claim(
                "REC-01",
                &[Monitor, Fault],
                false,
                &["contained_panic"],
            )],
            vec![
                run(Source::Archive, &[("contained_panic", Outcome::Passed)]),
                run(Source::Workspace, &[("contained_panic", Outcome::Passed)]),
            ],
        );
        fixture.hooked.insert("contained_panic".to_owned());
        assert_eq!(
            fixture.states(Platform::LinuxX86_64),
            states(&[
                ("REC-01", "Monitor", "not run"),
                ("REC-01", "Fault", "passed")
            ])
        );
        fixture.hooked.clear();
        assert_eq!(
            fixture.states(Platform::LinuxX86_64),
            states(&[
                ("REC-01", "Monitor", "passed"),
                ("REC-01", "Fault", "passed")
            ])
        );
    }

    /// Each record carries the common fields, the scenario's setup, expected
    /// result and required layers, the selectors with the README claiming
    /// them, each result and the supplements.
    #[test]
    fn records_carry_everything_a_case_needs() {
        use Layer::*;
        let fixture = Fixture::new(
            vec![scenario("ENT-01", &[Contract, Monitor])],
            vec![
                claim("ENT-01", &[Contract], false, &["opens"]),
                claim("ENT-01", &[], true, &["pure"]),
            ],
            vec![run(Source::Workspace, &[("tests::opens", Outcome::Passed)])],
        );
        let records = fixture.records(Platform::MacosX86_64);
        assert_eq!(
            records[0],
            json!({
                "source_revision": "abc123",
                "scenario": "ENT-01",
                "layer": "Contract",
                "platform": "macos-x86_64",
                "state": "passed",
                "required_layers": ["Contract", "Monitor"],
                "setup": "ENT-01 setup",
                "expected": "ENT-01 expected",
                "observed": "1 passed in the workspace run",
                "limitation": "",
                "evidence": {
                    "test_selectors": [{"test": "opens", "claimed_in": "crates/x/README.md"}],
                    "results": [{"run": "workspace", "test": "tests::opens", "result": "passed"}],
                    "supplements": ["pure"],
                    "reviews": [],
                },
            })
        );
        assert_eq!(records[1]["observed"], "no claimed test ran");
    }

    fn manual_record(os: &str, host: &str, passed: bool) -> Manual {
        Manual {
            scenario: "FOC-02".to_owned(),
            layer: Layer::HostManual,
            os: os.to_owned(),
            host: host.to_owned(),
            gesture: "Shift+drag, then copy".to_owned(),
            observed: "the visible text was copied".to_owned(),
            passed,
            limitation: ["only the visible page", ""][usize::from(passed)].to_owned(),
            recorded: "owner, 2026-11-20".to_owned(),
            candidate: Candidate::default(),
        }
    }

    /// FOC-02's Host manual check needs three host terminals on the
    /// platform's own OS (owner decision on #122): with two it is not run;
    /// with three it passes, a failing host recorded as a limitation, not a
    /// failure. Another OS's records never count.
    #[test]
    fn the_manual_copy_check_needs_three_hosts_and_records_limitations() {
        let mut fixture = Fixture::new(
            vec![scenario("FOC-02", &[Layer::HostManual])],
            vec![],
            vec![],
        );
        fixture.manual = vec![
            manual_record("macos", "Terminal", true),
            manual_record("macos", "iTerm2", false),
            manual_record("linux", "GNOME Terminal", true),
            manual_record("linux", "Konsole", true),
            manual_record("linux", "Konsole", true),
        ];
        let linux = fixture.records(Platform::LinuxX86_64);
        assert_eq!(linux[0]["state"], "not run");
        assert_eq!(
            linux[0]["observed"],
            "2 of 3 host terminals recorded on linux"
        );
        fixture.manual.push(manual_record("macos", "Ghostty", true));
        let macos = fixture.records(Platform::MacosArm64);
        assert_eq!(macos[0]["state"], "passed");
        assert_eq!(macos[0]["limitation"], "iTerm2: only the visible page");
        assert_eq!(macos[0]["evidence"]["manual"].as_array().unwrap().len(), 3);
        assert_eq!(
            fixture.records(Platform::LinuxX86_64)[0]["state"],
            "not run"
        );
    }

    fn candidate(version: &str) -> Candidate {
        Candidate {
            version: version.to_owned(),
            sha256: format!("{version} zip"),
        }
    }

    /// One host's checklist record of `scenario`, run on `on`.
    fn checked(scenario: &str, host: &str, passed: bool, on: &Candidate) -> Manual {
        Manual {
            scenario: scenario.to_owned(),
            layer: Layer::HostManual,
            os: "windows".to_owned(),
            host: host.to_owned(),
            gesture: "c2 exec pwsh, detach, reattach".to_owned(),
            observed: ["the screen was blank", "the screen was restored"][usize::from(passed)]
                .to_owned(),
            passed,
            limitation: String::new(),
            recorded: "owner, 2026-11-24".to_owned(),
            candidate: on.clone(),
        }
    }

    fn checklist_fixture(hosts: &[&str]) -> Fixture {
        let mut item = scenario("WINM-02", &[Layer::HostManual]);
        item.hosts = hosts.iter().map(|host| (*host).to_owned()).collect();
        item.platforms = vec![Platform::WindowsX86_64];
        let mut fixture = Fixture::new(vec![item], vec![], vec![]);
        fixture.candidate = Some(candidate("1.2.0-rc.2"));
        fixture
    }

    fn checklist_state(fixture: &Fixture) -> (String, String, String) {
        let record = &fixture.records(Platform::WindowsX86_64)[0];
        let text = |name: &str| record[name].as_str().unwrap().to_owned();
        (text("state"), text("observed"), text("limitation"))
    }

    /// A checklist item (#330) passes with a passed record for each of its
    /// hosts on the latest candidate; one host short is not run; any failed
    /// or limited record of that candidate fails it, however many passed.
    #[test]
    fn a_checklist_item_needs_every_host_passed_on_the_latest_candidate() {
        let latest = candidate("1.2.0-rc.2");
        let mut fixture = checklist_fixture(&["Windows Terminal", "VS Code"]);
        fixture.manual = vec![checked("WINM-02", "Windows Terminal", true, &latest)];
        assert_eq!(
            checklist_state(&fixture),
            (
                "not run".into(),
                "1 of 2 hosts passed on 1.2.0-rc.2".into(),
                String::new()
            )
        );
        fixture
            .manual
            .push(checked("WINM-02", "VS Code", true, &latest));
        assert_eq!(checklist_state(&fixture).0, "passed");
        assert_eq!(
            fixture.records(Platform::WindowsX86_64)[0]["evidence"]["manual"][1],
            json!({
                "host": "VS Code", "gesture": "c2 exec pwsh, detach, reattach",
                "observed": "the screen was restored", "result": "passed", "limitation": "",
                "recorded": "owner, 2026-11-24", "candidate": "1.2.0-rc.2",
                "sha256": "1.2.0-rc.2 zip",
            })
        );
        fixture
            .manual
            .push(checked("WINM-02", "VS Code", false, &latest));
        assert_eq!(
            checklist_state(&fixture),
            (
                "failed".into(),
                "2 of 2 hosts passed on 1.2.0-rc.2".into(),
                "VS Code: the screen was blank".into()
            )
        );
    }

    /// Only records of the latest candidate, its exact zip, this item, its
    /// hosts and Windows count; without a candidate named, the item is not
    /// run.
    #[test]
    fn a_checklist_item_counts_only_the_latest_candidates_records() {
        let latest = candidate("1.2.0-rc.2");
        let mut fixture = checklist_fixture(&["VS Code"]);
        let mut other_zip = checked("WINM-02", "VS Code", true, &latest);
        other_zip.candidate.sha256 = "rebuilt zip".to_owned();
        let mut other_os = checked("WINM-02", "VS Code", true, &latest);
        other_os.os = "linux".to_owned();
        fixture.manual = vec![
            checked("WINM-02", "VS Code", true, &candidate("1.2.0-rc.1")),
            checked("WINM-02", "VS Code", false, &candidate("1.2.0-rc.1")),
            other_zip,
            other_os,
            checked("WINM-03", "VS Code", true, &latest),
            checked("WINM-02", "Windows Terminal", false, &latest),
        ];
        assert_eq!(
            checklist_state(&fixture),
            (
                "not run".into(),
                "0 of 1 hosts passed on 1.2.0-rc.2".into(),
                String::new()
            )
        );
        fixture
            .manual
            .push(checked("WINM-02", "VS Code", true, &latest));
        assert_eq!(checklist_state(&fixture).0, "passed");
        fixture.candidate = None;
        assert_eq!(
            checklist_state(&fixture),
            (
                "not run".into(),
                "no release candidate named: the checklist runs on a published candidate".into(),
                String::new()
            )
        );
    }

    /// A Docs case holds from its claimed tests or a review record for any
    /// OS; a failed review fails it.
    #[test]
    fn docs_cases_take_tests_and_reviews() {
        let mut fixture = Fixture::new(
            vec![
                scenario("DOC-02", &[Layer::Docs]),
                scenario("CFG-01", &[Layer::Docs]),
            ],
            vec![claim(
                "CFG-01",
                &[Layer::Docs],
                false,
                &["documents_defaults"],
            )],
            vec![run(
                Source::Workspace,
                &[("documents_defaults", Outcome::Passed)],
            )],
        );
        assert_eq!(
            fixture.states(Platform::LinuxX86_64),
            states(&[
                ("DOC-02", "Docs", "not implemented"),
                ("CFG-01", "Docs", "passed")
            ])
        );
        let review = |passed| Manual {
            scenario: "DOC-02".to_owned(),
            layer: Layer::Docs,
            os: "any".to_owned(),
            host: String::new(),
            gesture: String::new(),
            observed: "status lines match the evidence".to_owned(),
            passed,
            limitation: String::new(),
            recorded: "owner".to_owned(),
            candidate: Candidate::default(),
        };
        fixture.manual = vec![review(true)];
        let records = fixture.records(Platform::MacosX86_64);
        assert_eq!(records[0]["state"], "passed");
        assert_eq!(
            records[0]["observed"],
            "reviewed: status lines match the evidence"
        );
        fixture.manual = vec![review(true), review(false)];
        assert_eq!(fixture.records(Platform::MacosX86_64)[0]["state"], "failed");
    }

    fn report(path: &str, sha256: &str, duration: &str, cases: Value, findings: Value) -> Report {
        Report {
            path: path.to_owned(),
            json: json!({
                "environment": {
                    "candidate": format!("/dist/climon2.tar.gz (sha256 {sha256})"),
                    "duration": duration,
                },
                "qualification": {"findings": findings},
                "cases": cases,
            }),
        }
    }

    fn cases(geometry: &str, results: &[&str]) -> Value {
        Value::Array(
            results
                .iter()
                .enumerate()
                .map(|(index, result)| {
                    json!({"id": format!("PERF-01/{geometry}/{index}/one-release/passive-idle"), "outcome": {"result": result}})
                })
                .collect(),
        )
    }

    const WINDOW: &str = "60 s per scenario (the owner's decision)";

    fn measure_state(reports: Vec<Report>) -> String {
        let mut fixture =
            Fixture::new(vec![scenario("PERF-01", &[Layer::Measure])], vec![], vec![]);
        fixture.reports = reports;
        fixture.records(Platform::LinuxX86_64)[0]["state"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// A Measure case passes only when this candidate's reports cover every
    /// geometry over the decided window and every case met its targets; a
    /// missed target or requirement fails it (the owner's written
    /// disposition decides, #121), a case or geometry not run leaves it not
    /// run, and another artifact's or a reduced window's report blocks it.
    #[test]
    fn measure_cases_follow_this_candidates_reports() {
        let all = |result: &str| {
            GEOMETRIES
                .map(|geometry| {
                    report(
                        geometry,
                        "feed",
                        WINDOW,
                        cases(geometry, &["met", result]),
                        json!([]),
                    )
                })
                .to_vec()
        };
        assert_eq!(measure_state(all("met")), "passed");
        assert_eq!(measure_state(all("failed")), "failed");
        assert_eq!(measure_state(all("not-run")), "not run");
        assert_eq!(measure_state(Vec::new()), "not run");
        let mut two = all("met");
        two.pop();
        assert_eq!(measure_state(two), "not run");
        let mut short = all("met");
        short[1] = report(
            "200x60",
            "feed",
            WINDOW,
            cases("200x60", &["met"]),
            json!(["workers short"]),
        );
        assert_eq!(measure_state(short), "failed");
        let mut other = all("failed");
        other[2] = report(
            "500x200",
            "beef",
            WINDOW,
            cases("500x200", &["met"]),
            json!([]),
        );
        assert_eq!(measure_state(other), "blocked");
        let mut reduced = all("met");
        reduced[0] = report(
            "80x24",
            "feed",
            "5 s per scenario: REDUCED from the decided 60 s, not qualification evidence",
            cases("80x24", &["met"]),
            json!([]),
        );
        assert_eq!(measure_state(reduced), "blocked");
    }
}
