//! The qualification verdict (#122) over every platform's evidence records
//! and the owner's recorded dispositions. Pure.
//!
//! Unqualified conformance needs every mandatory case passed. A failed,
//! blocked, not-run or not-implemented case prevents it, a missing
//! platform's cases included. The owner's disposition of a case lets the
//! release be published, qualified with that disposition, which stays
//! visible in the records. A disposition names its exact scenario and
//! layer, so a performance (Measure) disposition never waives a functional
//! case.
//!
//! A release candidate publishes without the Windows manual checklist
//! (#330): its checklist cases that did not pass carry the release rule's
//! disposition instead of the owner's. A Windows record that does not name
//! its runner image and Windows build is blocked, whatever it observed.

use super::{Layer, Platform, State, candidate::Stage, sources::Scenario};
use serde_json::{Value, json};
use std::fmt::Write as _;

/// The owner's recorded disposition of some cases.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Disposition {
    pub(crate) scenario: String,
    pub(crate) layer: Layer,
    /// None means every platform.
    pub(crate) platforms: Option<Vec<Platform>>,
    /// The states it accepts; never passed.
    pub(crate) states: Vec<State>,
    pub(crate) decision: String,
    pub(crate) recorded: String,
}

impl Disposition {
    fn covers(&self, scenario: &str, layer: Layer, platform: Platform, state: State) -> bool {
        let platforms = self.platforms.as_ref();
        self.scenario == scenario
            && self.layer == layer
            && platforms.is_none_or(|platforms| platforms.contains(&platform))
            && self.states.contains(&state)
    }
}

/// Parses the dispositions file, refusing any disposition the matrix does
/// not make sense of: an unknown scenario, a layer the scenario does not
/// require, an unknown platform or state, a passed state, or no decision.
pub(crate) fn dispositions(text: &str, scenarios: &[Scenario]) -> Result<Vec<Disposition>, String> {
    let json: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let entries = json["dispositions"]
        .as_array()
        .ok_or("the dispositions file has no \"dispositions\" array")?;
    entries
        .iter()
        .map(|entry| disposition(entry, scenarios))
        .collect()
}

fn disposition(entry: &Value, scenarios: &[Scenario]) -> Result<Disposition, String> {
    let text = |name: &str| entry[name].as_str().unwrap_or_default().to_owned();
    let scenario = text("scenario");
    let layer = Layer::parse(&text("layer")).ok_or_else(|| format!("{scenario}: unknown layer"))?;
    let required = scenarios
        .iter()
        .find(|known| known.id == scenario)
        .ok_or_else(|| format!("unknown scenario {scenario:?}"))?;
    if !required.layers.contains(&layer) {
        return Err(format!(
            "{scenario} does not require the {} layer",
            layer.name()
        ));
    }
    let names = |name: &str| -> Vec<String> {
        let list = entry[name].as_array().into_iter().flatten();
        list.filter_map(|value| value.as_str().map(str::to_owned))
            .collect()
    };
    let platforms = match entry.get("platforms") {
        None => None,
        Some(_) => Some(
            names("platforms")
                .iter()
                .map(|name| {
                    Platform::parse(name)
                        .filter(|platform| required.platforms.contains(platform))
                        .ok_or_else(|| format!("{scenario}: unknown platform {name:?}"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
    };
    let states = names("states")
        .iter()
        .map(|name| {
            State::parse(name)
                .filter(|state| *state != State::Passed)
                .ok_or_else(|| format!("{scenario}: a disposition cannot accept {name:?}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let decision = text("decision");
    if states.is_empty() || decision.is_empty() {
        return Err(format!(
            "{scenario}: a disposition needs its states and the owner's decision"
        ));
    }
    Ok(Disposition {
        scenario,
        layer,
        platforms,
        states,
        decision,
        recorded: text("recorded"),
    })
}

/// Whether the release may be published, and how it conforms.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Conformance {
    /// Every mandatory case passed: unqualified conformance.
    Unqualified,
    /// Every case that did not pass has the owner's disposition.
    WithDispositions,
    /// Some case neither passed nor has a disposition: publishing is
    /// blocked.
    NotQualified,
}

impl Conformance {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Unqualified => "qualified",
            Self::WithDispositions => "qualified with the owner's dispositions",
            Self::NotQualified => "not qualified",
        }
    }

    pub(crate) fn publishable(self) -> bool {
        self != Self::NotQualified
    }
}

/// The verdict and every case's record, dispositions attached.
pub(crate) struct Verdict {
    pub(crate) conformance: Conformance,
    pub(crate) records: Vec<Value>,
}

/// Judges every expected case — each scenario's required layers on every
/// platform — from `records`; a case with no record (its platform's
/// evidence is missing) is blocked. Records must share one source
/// revision.
pub(crate) fn verdict(
    scenarios: &[Scenario],
    records: &[Value],
    dispositions: &[Disposition],
    stage: Stage,
) -> Result<Verdict, String> {
    let revisions: std::collections::BTreeSet<&str> = records
        .iter()
        .filter_map(|record| record["source_revision"].as_str())
        .collect();
    if revisions.len() > 1 {
        return Err(format!(
            "the evidence is of several revisions: {revisions:?}"
        ));
    }
    let mut judged = Vec::new();
    for platform in Platform::ALL {
        let required = scenarios
            .iter()
            .filter(|scenario| scenario.platforms.contains(&platform));
        for scenario in required {
            for &layer in &scenario.layers {
                let record = find(records, &scenario.id, layer, platform)
                    .map_or_else(|| missing(scenario, layer, platform), traceable);
                let waived = stage == Stage::Candidate && scenario.is_checklist();
                judged.push(dispose(record, dispositions, waived));
            }
        }
    }
    let open = judged
        .iter()
        .any(|record| record["disposition"].is_null() && record["state"] != "passed");
    let disposed = judged.iter().any(|record| !record["disposition"].is_null());
    let conformance = match (open, disposed) {
        (true, _) => Conformance::NotQualified,
        (false, true) => Conformance::WithDispositions,
        (false, false) => Conformance::Unqualified,
    };
    Ok(Verdict {
        conformance,
        records: judged,
    })
}

fn find(records: &[Value], scenario: &str, layer: Layer, platform: Platform) -> Option<Value> {
    records
        .iter()
        .find(|record| {
            record["scenario"] == scenario
                && record["layer"] == layer.name()
                && record["platform"] == platform.name()
        })
        .cloned()
}

/// A case whose platform left no evidence: blocked, never passed.
fn missing(scenario: &Scenario, layer: Layer, platform: Platform) -> Value {
    json!({
        "scenario": scenario.id,
        "layer": layer.name(),
        "platform": platform.name(),
        "state": State::Blocked.name(),
        "observed": format!("no evidence from {}", platform.name()),
    })
}

/// The record, blocked if it is a Windows record that does not name its
/// runner image and Windows build (#330): the `windows-2025` image changes
/// weekly and cannot be pinned.
fn traceable(mut record: Value) -> Value {
    let environment = &record["environment"];
    let named = |name: &str| {
        environment[name]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    };
    let windows = record["platform"] == Platform::WindowsX86_64.name();
    if windows && !(named("runner_image") && named("windows_build")) {
        record["state"] = json!(State::Blocked.name());
        record["observed"] = json!(format!(
            "no runner image version or Windows build recorded; {}",
            record["observed"].as_str().unwrap_or_default()
        ));
    }
    record
}

/// The release rule's disposition of a release candidate's checklist case.
const CANDIDATE_RULE: &str = "a release candidate publishes without the manual checklist; the final tag needs its records (docs/for-humans/windows/quality.md)";

/// The record with the disposition that covers it, if it did not pass: the
/// owner's, else the release rule's when `waived`.
fn dispose(mut record: Value, dispositions: &[Disposition], waived: bool) -> Value {
    let text = |name: &str| record[name].as_str().unwrap_or_default().to_owned();
    let layer = Layer::parse(&text("layer"));
    let platform = Platform::parse(&text("platform"));
    let state = State::parse(&text("state"));
    let scenario = text("scenario");
    let covering = layer
        .zip(platform)
        .zip(state)
        .and_then(|((layer, platform), state)| {
            dispositions
                .iter()
                .find(|disposition| disposition.covers(&scenario, layer, platform, state))
        });
    let rule = (waived && state != Some(State::Passed))
        .then(|| json!({"decision": CANDIDATE_RULE, "recorded": "release rule"}));
    record["disposition"] = covering
        .map(|disposition| json!({"decision": disposition.decision, "recorded": disposition.recorded}))
        .or(rule)
        .unwrap_or(Value::Null);
    record
}

/// The verdict's readable summary: the conformance, every case that did not
/// pass with its disposition, and per-platform state counts.
pub(crate) fn summary(verdict: &Verdict) -> String {
    let mut text = format!(
        "# Release qualification\n\n**{}.**\n\n",
        verdict.conformance.name()
    );
    text.push_str("| Platform | passed | failed | blocked | not run | not implemented |\n|---|---|---|---|---|---|\n");
    for platform in Platform::ALL {
        let count = |state: State| {
            verdict
                .records
                .iter()
                .filter(|record| {
                    record["platform"] == platform.name() && record["state"] == state.name()
                })
                .count()
        };
        let _ = writeln!(
            text,
            "| {} | {} | {} | {} | {} | {} |",
            platform.name(),
            count(State::Passed),
            count(State::Failed),
            count(State::Blocked),
            count(State::NotRun),
            count(State::NotImplemented)
        );
    }
    text.push_str("\n## Cases that did not pass\n\n| Case | State | Observed | Disposition |\n|---|---|---|---|\n");
    for record in verdict
        .records
        .iter()
        .filter(|record| record["state"] != "passed")
    {
        let disposition = record["disposition"]["decision"]
            .as_str()
            .unwrap_or("**none: blocks publishing**");
        let _ = writeln!(
            text,
            "| {} {} on {} | {} | {} | {} |",
            cell(&record["scenario"]),
            cell(&record["layer"]),
            cell(&record["platform"]),
            cell(&record["state"]),
            cell(&record["observed"]),
            disposition.replace('|', "\\|")
        );
    }
    text
}

fn cell(value: &Value) -> String {
    value.as_str().unwrap_or_default().replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario(id: &str, layers: &[Layer]) -> Scenario {
        Scenario {
            id: id.to_owned(),
            setup: String::new(),
            expected: String::new(),
            layers: layers.to_vec(),
            platforms: Platform::ALL.to_vec(),
            hosts: Vec::new(),
        }
    }

    fn matrix() -> Vec<Scenario> {
        vec![
            scenario("NOT-02", &[Layer::Monitor, Layer::Measure]),
            scenario("PERF-01", &[Layer::Measure]),
        ]
    }

    fn record(scenario: &str, layer: &str, platform: Platform, state: &str) -> Value {
        json!({
            "source_revision": "abc",
            "scenario": scenario,
            "layer": layer,
            "platform": platform.name(),
            "state": state,
            "observed": "seen",
            "environment": {"runner_image": "win25 20251110.1", "windows_build": "10.0.26100.6899"},
        })
    }

    /// Every case's record on every platform, each in `state` unless
    /// `except` names its scenario, layer and platform.
    fn evidence(state: &str, except: &[(&str, &str, Platform, &str)]) -> Vec<Value> {
        let mut records = Vec::new();
        for platform in Platform::ALL {
            for (scenario, layer) in [
                ("NOT-02", "Monitor"),
                ("NOT-02", "Measure"),
                ("PERF-01", "Measure"),
            ] {
                let state = except
                    .iter()
                    .find(|(id, named, on, _)| {
                        *id == scenario && *named == layer && *on == platform
                    })
                    .map_or(state, |(_, _, _, state)| *state);
                records.push(record(scenario, layer, platform, state));
            }
        }
        records
    }

    fn disposition(
        scenario: &str,
        layer: Layer,
        platforms: Option<Vec<Platform>>,
        states: &[State],
    ) -> Disposition {
        Disposition {
            scenario: scenario.to_owned(),
            layer,
            platforms,
            states: states.to_vec(),
            decision: format!("accepted {scenario}"),
            recorded: "owner, 2026-11-20".to_owned(),
        }
    }

    fn conformance(records: &[Value], dispositions: &[Disposition]) -> Conformance {
        verdict(&matrix(), records, dispositions, Stage::Final)
            .unwrap()
            .conformance
    }

    /// Unqualified conformance only when every case on every platform
    /// passed; any failed, blocked, not-run or not-implemented case
    /// prevents it.
    #[test]
    fn every_mandatory_case_must_pass_for_unqualified_conformance() {
        assert_eq!(
            conformance(&evidence("passed", &[]), &[]),
            Conformance::Unqualified
        );
        for state in ["failed", "blocked", "not run", "not implemented"] {
            let one = evidence(
                "passed",
                &[("NOT-02", "Monitor", Platform::MacosX86_64, state)],
            );
            assert_eq!(conformance(&one, &[]), Conformance::NotQualified, "{state}");
        }
    }

    /// A platform that left no evidence (an unavailable architecture) is
    /// blocked, never passed.
    #[test]
    fn a_missing_platform_blocks_its_cases() {
        let mut records = evidence("passed", &[]);
        records.retain(|record| record["platform"] != "macos-arm64");
        let verdict = verdict(&matrix(), &records, &[], Stage::Final).unwrap();
        assert_eq!(verdict.conformance, Conformance::NotQualified);
        let blocked: Vec<_> = verdict
            .records
            .iter()
            .filter(|record| record["state"] == "blocked")
            .map(|record| (record["platform"].clone(), record["observed"].clone()))
            .collect();
        assert_eq!(blocked.len(), 3);
        assert!(
            blocked
                .iter()
                .all(|(platform, observed)| platform == "macos-arm64"
                    && observed == "no evidence from macos-arm64")
        );
    }

    /// The owner's disposition of a failed Measure case lets the release be
    /// published, qualified with that disposition, which stays in its
    /// record; a disposition limited to other platforms or other states
    /// does not cover it.
    #[test]
    fn a_disposition_covers_exactly_its_cases_and_stays_visible() {
        let failed = evidence(
            "passed",
            &[("PERF-01", "Measure", Platform::MacosArm64, "failed")],
        );
        let accepted = disposition("PERF-01", Layer::Measure, None, &[State::Failed]);
        let verdict = verdict(
            &matrix(),
            &failed,
            std::slice::from_ref(&accepted),
            Stage::Final,
        )
        .unwrap();
        assert_eq!(verdict.conformance, Conformance::WithDispositions);
        assert!(verdict.conformance.publishable());
        let disposed: Vec<_> = verdict
            .records
            .iter()
            .filter(|record| !record["disposition"].is_null())
            .collect();
        assert_eq!(disposed.len(), 1);
        assert_eq!(
            disposed[0]["disposition"],
            json!({"decision": "accepted PERF-01", "recorded": "owner, 2026-11-20"})
        );
        let elsewhere = disposition(
            "PERF-01",
            Layer::Measure,
            Some(vec![Platform::LinuxX86_64]),
            &[State::Failed],
        );
        assert_eq!(
            conformance(&failed, &[elsewhere]),
            Conformance::NotQualified
        );
        let other_state = disposition("PERF-01", Layer::Measure, None, &[State::NotRun]);
        assert_eq!(
            conformance(&failed, &[other_state]),
            Conformance::NotQualified
        );
        let here = disposition(
            "PERF-01",
            Layer::Measure,
            Some(vec![Platform::MacosArm64]),
            &[State::Failed],
        );
        assert_eq!(conformance(&failed, &[here]), Conformance::WithDispositions);
    }

    /// A performance disposition never waives a functional failure: neither
    /// PERF-01's nor NOT-02's own Measure disposition covers NOT-02's failed
    /// Monitor case.
    #[test]
    fn a_performance_disposition_never_waives_a_functional_failure() {
        let failed = evidence(
            "passed",
            &[
                ("NOT-02", "Monitor", Platform::LinuxX86_64, "failed"),
                ("NOT-02", "Measure", Platform::LinuxX86_64, "failed"),
            ],
        );
        let performance = [
            disposition("PERF-01", Layer::Measure, None, &[State::Failed]),
            disposition("NOT-02", Layer::Measure, None, &[State::Failed]),
        ];
        let verdict = verdict(&matrix(), &failed, &performance, Stage::Final).unwrap();
        assert_eq!(verdict.conformance, Conformance::NotQualified);
        let monitor = verdict
            .records
            .iter()
            .find(|record| record["layer"] == "Monitor" && record["state"] == "failed")
            .unwrap();
        assert!(monitor["disposition"].is_null());
        assert!(!verdict.conformance.publishable());
    }

    /// A Windows-only checklist item, WINM-02, beside the matrix.
    fn with_checklist() -> Vec<Scenario> {
        let mut item = scenario("WINM-02", &[Layer::HostManual]);
        item.platforms = vec![Platform::WindowsX86_64];
        item.hosts = vec!["Windows Terminal".into(), "VS Code".into()];
        let mut scenarios = matrix();
        scenarios.push(item);
        scenarios
    }

    /// A Windows-only scenario is required on Windows alone: other
    /// platforms' cases never wait for it.
    #[test]
    fn a_windows_only_scenario_is_required_on_windows_alone() {
        let records = evidence("passed", &[]);
        let verdict = verdict(&with_checklist(), &records, &[], Stage::Final).unwrap();
        let checklist: Vec<_> = verdict
            .records
            .iter()
            .filter(|record| record["scenario"] == "WINM-02")
            .map(|record| (record["platform"].clone(), record["state"].clone()))
            .collect();
        assert_eq!(checklist, [(json!("windows-x86_64"), json!("blocked"))]);
        assert_eq!(verdict.conformance, Conformance::NotQualified);
    }

    /// A release candidate publishes without the checklist: its checklist
    /// cases that did not pass carry the release rule's disposition. A
    /// final tag needs them; other cases are never waived; the owner's own
    /// disposition comes first.
    #[test]
    fn a_release_candidate_publishes_without_the_checklist() {
        let mut records = evidence("passed", &[]);
        records.push(record(
            "WINM-02",
            "Host manual",
            Platform::WindowsX86_64,
            "not run",
        ));
        let judged = |records: &[Value], dispositions: &[Disposition], stage| {
            verdict(&with_checklist(), records, dispositions, stage).unwrap()
        };
        let candidate = judged(&records, &[], Stage::Candidate);
        assert_eq!(candidate.conformance, Conformance::WithDispositions);
        let waived = candidate
            .records
            .iter()
            .find(|record| record["scenario"] == "WINM-02")
            .unwrap();
        assert_eq!(waived["disposition"]["recorded"], "release rule");
        assert_eq!(waived["disposition"]["decision"], CANDIDATE_RULE);
        assert_eq!(
            judged(&records, &[], Stage::Final).conformance,
            Conformance::NotQualified
        );
        let owner = disposition("WINM-02", Layer::HostManual, None, &[State::NotRun]);
        let disposed = judged(&records, std::slice::from_ref(&owner), Stage::Candidate);
        let kept = disposed
            .records
            .iter()
            .find(|record| record["scenario"] == "WINM-02")
            .unwrap();
        assert_eq!(kept["disposition"]["decision"], "accepted WINM-02");
        let last = records.len() - 1;
        records[last]["state"] = json!("passed");
        assert_eq!(
            judged(&records, &[], Stage::Candidate).conformance,
            Conformance::Unqualified
        );
        records[0]["state"] = json!("failed");
        assert_eq!(
            judged(&records, &[], Stage::Candidate).conformance,
            Conformance::NotQualified
        );
    }

    /// A Windows record that does not name its runner image version and
    /// Windows build is blocked, even when it passed; other platforms'
    /// records need neither.
    #[test]
    fn a_windows_record_must_name_its_image_and_build() {
        for missing in ["runner_image", "windows_build"] {
            let mut records = evidence("passed", &[]);
            for record in &mut records {
                record["environment"][missing] = json!("");
            }
            let verdict = verdict(&matrix(), &records, &[], Stage::Final).unwrap();
            let blocked: Vec<_> = verdict
                .records
                .iter()
                .filter(|record| record["state"] == "blocked")
                .map(|record| (record["platform"].clone(), record["observed"].clone()))
                .collect();
            assert_eq!(blocked.len(), 3, "{missing}");
            assert!(
                blocked
                    .iter()
                    .all(|(platform, observed)| platform == "windows-x86_64"
                        && observed == "no runner image version or Windows build recorded; seen")
            );
        }
    }

    /// Evidence of different source revisions is refused.
    #[test]
    fn evidence_must_share_one_revision() {
        let mut records = evidence("passed", &[]);
        records[0]["source_revision"] = json!("other");
        assert_eq!(
            verdict(&matrix(), &records, &[], Stage::Final).err(),
            Some("the evidence is of several revisions: {\"abc\", \"other\"}".to_owned())
        );
    }

    /// The dispositions file is checked against the matrix: an unknown
    /// scenario, a layer it does not require, an unknown platform or state,
    /// accepting a pass, or no decision is refused.
    #[test]
    fn dispositions_are_checked_against_the_matrix() {
        let parse =
            |entry: Value| dispositions(&json!({"dispositions": [entry]}).to_string(), &matrix());
        let good = json!({
            "scenario": "PERF-01", "layer": "Measure", "platforms": ["linux-x86_64"],
            "states": ["failed", "not run"], "decision": "accepted", "recorded": "owner",
        });
        assert_eq!(
            parse(good.clone()),
            Ok(vec![Disposition {
                scenario: "PERF-01".into(),
                layer: Layer::Measure,
                platforms: Some(vec![Platform::LinuxX86_64]),
                states: vec![State::Failed, State::NotRun],
                decision: "accepted".into(),
                recorded: "owner".into(),
            }])
        );
        let mut every = good.clone();
        every.as_object_mut().unwrap().remove("platforms");
        assert_eq!(parse(every).unwrap()[0].platforms, None);
        let changed = |name: &str, value: Value| {
            let mut entry = good.clone();
            entry[name] = value;
            parse(entry).unwrap_err()
        };
        assert_eq!(
            changed("scenario", json!("ZZZ-01")),
            "unknown scenario \"ZZZ-01\""
        );
        assert_eq!(
            changed("layer", json!("Monitor")),
            "PERF-01 does not require the Monitor layer"
        );
        assert_eq!(changed("layer", json!("Smoke")), "PERF-01: unknown layer");
        assert_eq!(
            changed("platforms", json!(["windows"])),
            "PERF-01: unknown platform \"windows\""
        );
        let windows_only = dispositions(
            &json!({"dispositions": [{
                "scenario": "WINM-02", "layer": "Host manual", "platforms": ["linux-x86_64"],
                "states": ["not run"], "decision": "accepted",
            }]})
            .to_string(),
            &with_checklist(),
        );
        assert_eq!(
            windows_only.unwrap_err(),
            "WINM-02: unknown platform \"linux-x86_64\""
        );
        assert_eq!(
            changed("states", json!(["passed"])),
            "PERF-01: a disposition cannot accept \"passed\""
        );
        assert_eq!(
            changed("states", json!(["skipped"])),
            "PERF-01: a disposition cannot accept \"skipped\""
        );
        let needs = "PERF-01: a disposition needs its states and the owner's decision";
        assert_eq!(changed("states", json!([])), needs);
        assert_eq!(changed("decision", json!("")), needs);
        assert_eq!(
            dispositions("{}", &matrix()).unwrap_err(),
            "the dispositions file has no \"dispositions\" array"
        );
        assert!(dispositions("not json", &matrix()).is_err());
    }

    /// The summary states the conformance, counts each platform's states
    /// and lists every case that did not pass with its disposition or that
    /// it blocks publishing.
    #[test]
    fn the_summary_lists_what_did_not_pass() {
        let records = evidence(
            "passed",
            &[
                ("PERF-01", "Measure", Platform::MacosArm64, "failed"),
                ("NOT-02", "Monitor", Platform::LinuxX86_64, "not run"),
            ],
        );
        let accepted = disposition("PERF-01", Layer::Measure, None, &[State::Failed]);
        let verdict = verdict(&matrix(), &records, &[accepted], Stage::Final).unwrap();
        let summary = summary(&verdict);
        assert!(
            summary.starts_with("# Release qualification\n\n**not qualified.**\n"),
            "{summary}"
        );
        assert!(
            summary.contains("| linux-x86_64 | 2 | 0 | 0 | 1 | 0 |\n"),
            "{summary}"
        );
        assert!(
            summary.contains("| macos-arm64 | 2 | 1 | 0 | 0 | 0 |\n"),
            "{summary}"
        );
        assert!(
            summary.contains(
                "| PERF-01 Measure on macos-arm64 | failed | seen | accepted PERF-01 |\n"
            )
        );
        assert!(summary.contains(
            "| NOT-02 Monitor on linux-x86_64 | not run | seen | **none: blocks publishing** |\n"
        ));
    }
}
