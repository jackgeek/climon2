//! The PERF-01 workload matrix (docs/for-humans/behaviour/monitor/acceptance.md, PERF-01): the
//! evidence points, the scenarios each runs and the fixed output rates each
//! scenario's sessions write at.

use std::{fmt, str::FromStr};

/// A host terminal size, columns by rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Geometry {
    pub cols: u16,
    pub rows: u16,
}

/// The three host sizes PERF-01 names, smallest first.
pub const GEOMETRIES: [Geometry; 3] = [
    Geometry { cols: 80, rows: 24 },
    Geometry {
        cols: 200,
        rows: 60,
    },
    Geometry {
        cols: 500,
        rows: 200,
    },
];

/// The boundary geometry, where PERF-01 records the measured capacity
/// instead of 100 sessions (owner, 2026-11-15, #166).
const BOUNDARY: Geometry = GEOMETRIES[2];

impl fmt::Display for Geometry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}x{}", self.cols, self.rows)
    }
}

impl FromStr for Geometry {
    type Err = String;

    /// One of the matrix's own geometries, written `<cols>x<rows>`.
    fn from_str(text: &str) -> Result<Self, String> {
        GEOMETRIES
            .into_iter()
            .find(|geometry| geometry.to_string() == text)
            .ok_or_else(|| format!("unknown geometry {text:?}: use 80x24, 200x60 or 500x200"))
    }
}

/// How many live sessions a case offers the monitor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Population {
    /// Exactly this many, each of which the monitor must admit.
    Sessions(u32),
    /// [`CAPACITY_OFFERED`] sessions, of which the monitor's admission
    /// count is the measured capacity.
    Capacity,
}

/// The sessions a [`Population::Capacity`] case offers.
pub const CAPACITY_OFFERED: u32 = 100;

impl Population {
    /// The sessions the case starts.
    pub fn offered(self) -> u32 {
        match self {
            Self::Sessions(count) => count,
            Self::Capacity => CAPACITY_OFFERED,
        }
    }
}

impl fmt::Display for Population {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sessions(count) => write!(formatter, "{count}"),
            Self::Capacity => formatter.write_str("capacity"),
        }
    }
}

/// The releases a case's sessions belong to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Releases {
    /// Every session is the candidate release's.
    One,
    /// Sessions spread over ten releases (the mixed-version fixture).
    Mixed,
}

impl fmt::Display for Releases {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::One => "one-release",
            Self::Mixed => "ten-releases",
        })
    }
}

/// What a case's sessions and its user do for the measured window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scenario {
    /// No output and no input.
    PassiveIdle,
    /// Every unselected session writes; the selected one is controlled.
    BackgroundOutput,
    /// The client starts cold, then again warm, until it is ready.
    Loading,
    /// The user switches the selected session through the selector.
    Switching,
    /// The selected session writes at a high fixed rate.
    SustainedOutput,
    /// Every session daemon stalls, then resumes, at once.
    ReconnectStorm,
    /// New sessions arrive at a fixed rate.
    ContinuousArrivals,
    /// The newest sessions' daemons never answer while older ones start.
    StalledNewerStarts,
}

/// Every scenario, in the order PERF-01 lists them.
pub const SCENARIOS: [Scenario; 8] = [
    Scenario::PassiveIdle,
    Scenario::BackgroundOutput,
    Scenario::Loading,
    Scenario::Switching,
    Scenario::SustainedOutput,
    Scenario::ReconnectStorm,
    Scenario::ContinuousArrivals,
    Scenario::StalledNewerStarts,
];

impl fmt::Display for Scenario {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PassiveIdle => "passive-idle",
            Self::BackgroundOutput => "background-output",
            Self::Loading => "cold-warm-loading",
            Self::Switching => "restoration-switching",
            Self::SustainedOutput => "selected-sustained-output",
            Self::ReconnectStorm => "reconnect-storm",
            Self::ContinuousArrivals => "continuous-arrivals",
            Self::StalledNewerStarts => "stalled-newer-starts",
        })
    }
}

/// The fixed output rates of a scenario's sessions, in bytes per second.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rates {
    /// Each session other than the selected one.
    pub background: u32,
    /// The selected session.
    pub selected: u32,
}

/// Ten 72-byte lines a second.
const BACKGROUND_RATE: u32 = 720;
/// 256 KiB a second.
const SUSTAINED_RATE: u32 = 256 * 1024;

impl Scenario {
    /// The fixed output rates the scenario's sessions write at.
    pub fn rates(self) -> Rates {
        let (background, selected) = match self {
            Self::BackgroundOutput => (BACKGROUND_RATE, 0),
            Self::SustainedOutput => (0, SUSTAINED_RATE),
            Self::PassiveIdle
            | Self::Loading
            | Self::Switching
            | Self::ReconnectStorm
            | Self::ContinuousArrivals
            | Self::StalledNewerStarts => (0, 0),
        };
        Rates {
            background,
            selected,
        }
    }
}

/// One evidence point: run once by the monitor and once by standalone
/// clients doing equivalent work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Case {
    pub geometry: Geometry,
    pub population: Population,
    pub releases: Releases,
    pub scenario: Scenario,
}

impl fmt::Display for Case {
    /// The case's stable ID.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "PERF-01/{}/{}/{}/{}",
            self.geometry, self.population, self.releases, self.scenario
        )
    }
}

/// The populations PERF-01 runs at `geometry`: 1, 10 and 100 sessions, and
/// at the boundary geometry the measured capacity instead of 100.
fn populations(geometry: Geometry) -> [Population; 3] {
    let largest = if geometry == BOUNDARY {
        Population::Capacity
    } else {
        Population::Sessions(100)
    };
    [Population::Sessions(1), Population::Sessions(10), largest]
}

/// Every case at the given geometries, in matrix order: geometry, then
/// population, then releases, then scenario.
pub fn matrix(geometries: &[Geometry]) -> Vec<Case> {
    let mut cases = Vec::new();
    for &geometry in geometries {
        for population in populations(geometry) {
            for releases in [Releases::One, Releases::Mixed] {
                for scenario in SCENARIOS {
                    cases.push(Case {
                        geometry,
                        population,
                        releases,
                        scenario,
                    });
                }
            }
        }
    }
    cases
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// PERF-01: 1, 10 and 100 sessions at 80x24 and 200x60, and 1, 10 and
    /// the measured capacity at 500x200; one release and ten mixed; eight
    /// scenarios each.
    #[test]
    fn the_matrix_runs_every_evidence_point_and_capacity_at_the_boundary() {
        let cases = matrix(&GEOMETRIES);
        assert_eq!(cases.len(), 3 * 3 * 2 * 8);
        let ids: HashSet<_> = cases.iter().map(ToString::to_string).collect();
        assert_eq!(ids.len(), cases.len(), "every ID is unique");
        for id in [
            "PERF-01/80x24/1/one-release/passive-idle",
            "PERF-01/80x24/100/ten-releases/stalled-newer-starts",
            "PERF-01/200x60/10/one-release/selected-sustained-output",
            "PERF-01/200x60/100/one-release/continuous-arrivals",
            "PERF-01/500x200/1/one-release/cold-warm-loading",
            "PERF-01/500x200/10/ten-releases/reconnect-storm",
            "PERF-01/500x200/capacity/one-release/restoration-switching",
            "PERF-01/500x200/capacity/ten-releases/background-output",
        ] {
            assert!(ids.contains(id), "{id}");
        }
        assert!(!ids.contains("PERF-01/500x200/100/one-release/passive-idle"));
        assert!(!ids.contains("PERF-01/80x24/capacity/one-release/passive-idle"));
        assert_eq!(
            cases[0].to_string(),
            "PERF-01/80x24/1/one-release/passive-idle"
        );
    }

    /// A CI job measures one geometry: only its cases, in matrix order.
    #[test]
    fn a_geometry_selects_only_its_own_cases() {
        let cases = matrix(&[GEOMETRIES[1]]);
        assert_eq!(cases.len(), 48);
        assert!(cases.iter().all(|case| case.geometry == GEOMETRIES[1]));
        assert_eq!(
            cases.last().unwrap().to_string(),
            "PERF-01/200x60/100/ten-releases/stalled-newer-starts"
        );
    }

    #[test]
    fn only_the_matrix_geometries_parse() {
        assert_eq!("80x24".parse(), Ok(GEOMETRIES[0]));
        assert_eq!("500x200".parse(), Ok(GEOMETRIES[2]));
        assert!("80x25".parse::<Geometry>().is_err());
        assert!("".parse::<Geometry>().is_err());
    }

    /// The capacity case offers 100 sessions, as many as the others' largest.
    #[test]
    fn a_population_offers_its_sessions() {
        assert_eq!(Population::Sessions(10).offered(), 10);
        assert_eq!(Population::Capacity.offered(), 100);
    }

    /// Fixed rates: ten 72-byte lines a second in the background, 256 KiB a
    /// second from the selected session, and silence elsewhere.
    #[test]
    fn output_scenarios_write_at_their_fixed_rates_and_the_rest_are_silent() {
        let rates = |scenario: Scenario| {
            let rates = scenario.rates();
            (rates.background, rates.selected)
        };
        assert_eq!(rates(Scenario::BackgroundOutput), (720, 0));
        assert_eq!(rates(Scenario::SustainedOutput), (0, 262_144));
        for scenario in SCENARIOS {
            if !matches!(
                scenario,
                Scenario::BackgroundOutput | Scenario::SustainedOutput
            ) {
                assert_eq!(rates(scenario), (0, 0), "{scenario}");
            }
        }
    }
}
