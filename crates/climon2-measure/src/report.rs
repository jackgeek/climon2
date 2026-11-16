//! The Measure-case record: each case's runs, its outcome, the declared
//! environment and the qualified-requirement evidence, written as JSON for
//! keeping and Markdown for reading. A failed case is recorded, never
//! failed silently or turned into a failed job: it needs the owner's
//! written disposition (#121, 2026-11-16), and #122 blocks qualification
//! until then.

use crate::{
    evaluate::{
        Distribution, Finding, Latencies, Probe, Requirement, latency_findings,
        requirement_findings,
    },
    matrix::{Case, Population},
    sample::{Components, Usage},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fmt::Write as _, time::Duration};

/// Who does a case's work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    /// One `climon2 monitor` on one host terminal.
    Monitor,
    /// One `climon2 attach` per session, each on its own host terminal.
    Standalone,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Monitor => "monitor",
            Self::Standalone => "standalone",
        }
    }
}

/// One mode's run of a case.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub mode: Mode,
    /// The disclosed monitor configuration, or none for the defaults.
    pub configuration: Option<Value>,
    pub offered: u32,
    /// Sessions the monitor admitted (not resource-blocked) once ready.
    pub admitted: Option<u32>,
    /// The monitor's or every standalone client's processes.
    pub clients: Usage,
    /// Every session daemon, separately.
    pub daemons: Usage,
    /// Where the monitor's resident memory lies, where the platform shows it.
    pub components: Option<Components>,
    pub latencies: BTreeMap<Probe, Latencies>,
    /// Bytes the host terminals received over the window: the display.
    pub display_bytes: u64,
    /// The most sessions seen waiting to start or for resources.
    pub backlog_max: Option<u32>,
    pub notes: Vec<String>,
}

/// A Measure case's outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    Met,
    /// Needs the owner's written disposition.
    Failed(Vec<Finding>),
    /// Not measured, and why.
    NotRun(String),
}

/// The outcome `runs` show: the monitor's latency findings, and a default
/// configuration admitting fewer sessions than an exact population offers.
/// Standalone runs are the comparison and set no target.
pub fn outcome(case: &Case, runs: &[Run]) -> Outcome {
    let mut findings = Vec::new();
    for run in runs.iter().filter(|run| run.mode == Mode::Monitor) {
        let exact = matches!(case.population, Population::Sessions(_));
        if let (true, None, Some(admitted)) = (exact, &run.configuration, run.admitted)
            && admitted < run.offered
        {
            findings.push(Finding::NotAdmitted {
                admitted,
                offered: run.offered,
            });
        }
        for (&probe, latencies) in &run.latencies {
            findings.extend(latency_findings(probe, latencies));
        }
    }
    if findings.is_empty() {
        Outcome::Met
    } else {
        Outcome::Failed(findings)
    }
}

/// One case's record.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub case: Case,
    pub runs: Vec<Run>,
    pub outcome: Outcome,
}

/// The declared environment: what was measured, where and how.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Environment {
    pub entries: Vec<(&'static str, String)>,
}

/// The requirements measured on this platform, beside the embedded ones,
/// and how they were measured.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Qualification {
    pub requirements: Vec<Requirement>,
    pub evidence: Vec<String>,
}

/// Everything one measurement run records.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub environment: Environment,
    pub qualification: Qualification,
    pub records: Vec<Record>,
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn distribution_json(latencies: &Latencies) -> Value {
    let summary = Distribution::of(&latencies.samples).map(|d| {
        json!({
            "count": d.count, "min_ms": millis(d.min), "p50_ms": millis(d.p50),
            "p90_ms": millis(d.p90), "p99_ms": millis(d.p99), "max_ms": millis(d.max),
        })
    });
    json!({
        "summary": summary,
        "timeouts": latencies.timeouts,
        "samples_ms": latencies.samples.iter().copied().map(millis).collect::<Vec<_>>(),
    })
}

fn usage_json(usage: &Usage) -> Value {
    json!({
        "cpu_percent": usage.cpu_percent,
        "resident_mean_bytes": usage.resident_mean,
        "resident_max_bytes": usage.resident_max,
        "threads_max": usage.threads_max,
        "descriptors_max": usage.descriptors_max,
    })
}

fn components_json(components: &Components) -> Value {
    json!({
        "image_bytes": components.image,
        "client_library_bytes": components.client_library,
        "other_libraries_bytes": components.other_libraries,
        "anonymous_bytes": components.anonymous,
        "other_files_bytes": components.other_files,
        "client_libraries_mapped": components.client_libraries_mapped,
    })
}

fn run_json(run: &Run) -> Value {
    let latencies: serde_json::Map<String, Value> = run
        .latencies
        .iter()
        .map(|(probe, latencies)| (probe.to_string(), distribution_json(latencies)))
        .collect();
    json!({
        "mode": run.mode.name(),
        "configuration": run.configuration.clone().unwrap_or_else(|| json!("defaults")),
        "offered": run.offered,
        "admitted": run.admitted,
        "clients": usage_json(&run.clients),
        "daemons": usage_json(&run.daemons),
        "components": run.components.as_ref().map(components_json),
        "latencies": latencies,
        "display_bytes": run.display_bytes,
        "backlog_max": run.backlog_max,
        "notes": run.notes,
    })
}

fn outcome_json(outcome: &Outcome) -> Value {
    match outcome {
        Outcome::Met => json!({ "result": "met" }),
        Outcome::Failed(findings) => json!({
            "result": "failed",
            "disposition": "required: the owner's written disposition",
            "findings": findings.iter().map(ToString::to_string).collect::<Vec<_>>(),
        }),
        Outcome::NotRun(reason) => json!({ "result": "not-run", "reason": reason }),
    }
}

impl Report {
    /// Every finding of the qualification: requirements without headroom.
    pub fn qualification_findings(&self) -> Vec<Finding> {
        requirement_findings(&self.qualification.requirements)
    }

    pub fn to_json(&self) -> Value {
        let environment: serde_json::Map<String, Value> = self
            .environment
            .entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), json!(value)))
            .collect();
        let requirements: Vec<Value> = self
            .qualification
            .requirements
            .iter()
            .map(|requirement| {
                json!({
                    "field": requirement.field,
                    "measured": requirement.measured,
                    "embedded": requirement.embedded,
                    "qualified": requirement.qualified(),
                    "recommended": requirement.recommended(),
                })
            })
            .collect();
        json!({
            "environment": environment,
            "qualification": {
                "headroom": "measured x 1.25 <= embedded",
                "requirements": requirements,
                "evidence": self.qualification.evidence,
                "findings": self.qualification_findings().iter().map(ToString::to_string).collect::<Vec<_>>(),
            },
            "cases": self.records.iter().map(|record| json!({
                "id": record.case.to_string(),
                "outcome": outcome_json(&record.outcome),
                "runs": record.runs.iter().map(run_json).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    }

    /// The readable summary: environment, the failed cases needing a
    /// disposition, the qualification and one row per run.
    pub fn to_markdown(&self) -> String {
        let mut text = String::from("# PERF-01 measurement\n\n## Environment\n\n");
        for (name, value) in &self.environment.entries {
            let _ = writeln!(text, "- **{name}:** {value}");
        }
        let failed: Vec<_> = self
            .records
            .iter()
            .filter_map(|record| match &record.outcome {
                Outcome::Failed(findings) => Some((record, findings)),
                _ => None,
            })
            .collect();
        let not_run = self
            .records
            .iter()
            .filter(|record| matches!(record.outcome, Outcome::NotRun(_)))
            .count();
        let _ = write!(
            text,
            "\n## Outcome\n\n{} cases: {} met, {} failed, {} not run.\n",
            self.records.len(),
            self.records.len() - failed.len() - not_run,
            failed.len(),
            not_run
        );
        if !failed.is_empty() {
            text.push_str(
                "\n### Failed Measure cases: each needs the owner's written disposition\n\n",
            );
            for (record, findings) in failed {
                let _ = writeln!(text, "- `{}`", record.case);
                for finding in findings {
                    let _ = writeln!(text, "  - {finding}");
                }
            }
        }
        text.push_str(&self.qualification_markdown());
        text.push_str("\n## Runs\n\n| Case | Mode | Configuration | Admitted | Client CPU % | Client RSS mean/max MiB | Daemon CPU % | Daemon RSS mean MiB | Input p50/p99 ms | Selector p99 ms | Attention p99 ms | Outcome |\n|---|---|---|---|---|---|---|---|---|---|---|---|\n");
        for record in &self.records {
            text.push_str(&record_rows(record));
        }
        text
    }

    fn qualification_markdown(&self) -> String {
        let mut text = String::from(
            "\n## Qualified requirements (measured x 1.25 <= embedded)\n\n| Field | Measured | Embedded | Qualified | Least to embed |\n|---|---|---|---|---|\n",
        );
        for requirement in &self.qualification.requirements {
            let _ = writeln!(
                text,
                "| {} | {} | {} | {} | {} |",
                requirement.field,
                requirement.measured,
                requirement.embedded,
                if requirement.qualified() {
                    "yes"
                } else {
                    "**no**"
                },
                requirement.recommended()
            );
        }
        for line in &self.qualification.evidence {
            let _ = writeln!(text, "\n{line}");
        }
        text
    }
}

fn mib(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))
}

fn p99(run: &Run, probe: Probe) -> String {
    run.latencies
        .get(&probe)
        .and_then(|latencies| Distribution::of(&latencies.samples))
        .map_or_else(|| "-".to_owned(), |d| format!("{:.1}", millis(d.p99)))
}

fn input(run: &Run) -> String {
    run.latencies
        .get(&Probe::Input)
        .and_then(|latencies| Distribution::of(&latencies.samples))
        .map_or_else(
            || "-".to_owned(),
            |d| format!("{:.1}/{:.1}", millis(d.p50), millis(d.p99)),
        )
}

fn outcome_cell(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Met => "met".to_owned(),
        Outcome::Failed(_) => "**failed**".to_owned(),
        Outcome::NotRun(reason) => format!("not run: {reason}"),
    }
}

fn record_rows(record: &Record) -> String {
    if record.runs.is_empty() {
        return format!(
            "| `{}` | - | - | - | - | - | - | - | - | - | - | {} |\n",
            record.case,
            outcome_cell(&record.outcome)
        );
    }
    let mut rows = String::new();
    for run in &record.runs {
        let configuration = run
            .configuration
            .as_ref()
            .map_or_else(|| "defaults".to_owned(), |value| format!("`{value}`"));
        let admitted = run.admitted.map_or_else(
            || "-".to_owned(),
            |admitted| format!("{admitted}/{}", run.offered),
        );
        let _ = writeln!(
            rows,
            "| `{}` | {} | {} | {} | {:.1} | {}/{} | {:.1} | {} | {} | {} | {} | {} |",
            record.case,
            run.mode.name(),
            configuration,
            admitted,
            run.clients.cpu_percent,
            mib(run.clients.resident_mean),
            mib(run.clients.resident_max),
            run.daemons.cpu_percent,
            mib(run.daemons.resident_mean),
            input(run),
            p99(run, Probe::Selector),
            p99(run, Probe::Attention),
            outcome_cell(&record.outcome)
        );
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matrix::{GEOMETRIES, Releases, Scenario};

    fn case(population: Population) -> Case {
        Case {
            geometry: GEOMETRIES[0],
            population,
            releases: Releases::One,
            scenario: Scenario::BackgroundOutput,
        }
    }

    fn run(mode: Mode, admitted: Option<u32>, input_ms: &[u64]) -> Run {
        Run {
            mode,
            configuration: None,
            offered: 100,
            admitted,
            clients: Usage::default(),
            daemons: Usage::default(),
            components: None,
            latencies: BTreeMap::from([(
                Probe::Input,
                Latencies {
                    samples: input_ms
                        .iter()
                        .copied()
                        .map(Duration::from_millis)
                        .collect(),
                    timeouts: 0,
                },
            )]),
            display_bytes: 0,
            backlog_max: None,
            notes: Vec::new(),
        }
    }

    /// Owner decision (#121): a miss is a failed Measure case, recorded
    /// with its findings; only the monitor's runs are held to the targets.
    #[test]
    fn a_missed_target_fails_the_case_and_standalone_runs_set_none() {
        let met = [
            run(Mode::Monitor, Some(100), &[20, 40]),
            run(Mode::Standalone, None, &[900]),
        ];
        assert_eq!(
            outcome(&case(Population::Sessions(100)), &met),
            Outcome::Met
        );
        let missed = [run(Mode::Monitor, Some(100), &[20, 140])];
        assert_eq!(
            outcome(&case(Population::Sessions(100)), &missed),
            Outcome::Failed(vec![Finding::TargetMissed {
                probe: Probe::Input,
                p99: Duration::from_millis(140)
            }])
        );
    }

    /// PERF-01: defaults that cannot admit an exact population are recorded
    /// as a finding; the repeat with a disclosed configuration, and the
    /// boundary geometry's capacity case, are not held to the offer.
    #[test]
    fn defaults_admitting_fewer_than_offered_are_a_finding_for_exact_populations_only() {
        let short = run(Mode::Monitor, Some(55), &[]);
        assert_eq!(
            outcome(
                &case(Population::Sessions(100)),
                std::slice::from_ref(&short)
            ),
            Outcome::Failed(vec![Finding::NotAdmitted {
                admitted: 55,
                offered: 100
            }])
        );
        assert_eq!(
            outcome(&case(Population::Capacity), std::slice::from_ref(&short)),
            Outcome::Met
        );
        let disclosed = Run {
            configuration: Some(json!({"memory": {"total_bytes": 1}})),
            ..short.clone()
        };
        assert_eq!(
            outcome(&case(Population::Sessions(100)), &[disclosed]),
            Outcome::Met
        );
        let all = run(Mode::Monitor, Some(100), &[]);
        assert_eq!(
            outcome(&case(Population::Sessions(100)), &[all]),
            Outcome::Met
        );
    }

    /// The record names each failed case and finding for disposition, the
    /// requirement table, and a row per run; the JSON keeps every sample.
    #[test]
    fn the_report_lists_failed_cases_for_disposition_and_every_run() {
        let failed = case(Population::Sessions(100));
        let report = Report {
            environment: Environment {
                entries: vec![("os", "linux".to_owned())],
            },
            qualification: Qualification {
                requirements: vec![Requirement {
                    field: "workers",
                    measured: 2,
                    embedded: 2,
                }],
                evidence: vec!["threads per session over 10 sessions".to_owned()],
            },
            records: vec![
                Record {
                    case: failed,
                    runs: vec![run(Mode::Monitor, Some(55), &[30])],
                    outcome: Outcome::Failed(vec![Finding::NotAdmitted {
                        admitted: 55,
                        offered: 100,
                    }]),
                },
                Record {
                    case: Case {
                        releases: Releases::Mixed,
                        ..failed
                    },
                    runs: Vec::new(),
                    outcome: Outcome::NotRun("no fixture".to_owned()),
                },
            ],
        };
        let markdown = report.to_markdown();
        for line in [
            "- **os:** linux",
            "2 cases: 0 met, 1 failed, 1 not run.",
            "- `PERF-01/80x24/100/one-release/background-output`",
            "  - default limits admitted 55 of 100 sessions",
            "| workers | 2 | 2 | **no** | 3 |",
            "| `PERF-01/80x24/100/one-release/background-output` | monitor | defaults | 55/100 | 0.0 | 0.0/0.0 | 0.0 | 0.0 | 30.0/30.0 | - | - | **failed** |",
            "| `PERF-01/80x24/100/ten-releases/background-output` | - | - | - | - | - | - | - | - | - | - | not run: no fixture |",
        ] {
            assert!(markdown.contains(line), "{line}\n{markdown}");
        }
        let json = report.to_json();
        assert_eq!(json["cases"][0]["outcome"]["result"], "failed");
        assert_eq!(
            json["cases"][0]["runs"][0]["latencies"]["input"]["samples_ms"],
            json!([30.0])
        );
        assert_eq!(json["cases"][1]["outcome"]["reason"], "no fixture");
        assert_eq!(json["qualification"]["requirements"][0]["qualified"], false);
        assert_eq!(json["environment"]["os"], "linux");
    }
}
