//! Evaluation of one run's measurements (docs/monitor-acceptance.md,
//! PERF-01; owner decisions on #121, 2026-11-16): latency distributions
//! against the 100 ms 99th-percentile targets, stalls of 250 ms or more for
//! diagnosis, admission at default limits, and measured requirements against
//! the ones embedded in the release with 25% headroom. A miss never fails the
//! job: it makes the Measure case failed until the owner records a written
//! disposition.

use std::{fmt, time::Duration};

/// A timed user-visible action.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Probe {
    /// A typed key until the session's echo is painted.
    Input,
    /// Prefix+s until the selector is painted.
    Selector,
    /// A global action (closing the selector) until it is painted.
    GlobalAction,
    /// An attention-bearing report until the host bell of its toast.
    Attention,
    /// Prefix+x until the monitor releases the host.
    ExitStart,
    /// Enter on another row until its session is shown.
    Switch,
    /// A client's start until it shows every session it must.
    Readiness,
    /// A new session's creation until the client lists it.
    Arrival,
}

impl Probe {
    /// Whether PERF-01 sets the 100 ms 99th-percentile target for it.
    pub fn targeted(self) -> bool {
        matches!(
            self,
            Self::Input | Self::Selector | Self::GlobalAction | Self::Attention
        )
    }
}

impl fmt::Display for Probe {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Input => "input",
            Self::Selector => "selector",
            Self::GlobalAction => "global-action",
            Self::Attention => "attention",
            Self::ExitStart => "exit-start",
            Self::Switch => "switch",
            Self::Readiness => "readiness",
            Self::Arrival => "arrival",
        })
    }
}

/// The 99th-percentile target for targeted probes.
pub const TARGET: Duration = Duration::from_millis(100);
/// A latency at least this long is a stall that needs diagnosis.
pub const STALL: Duration = Duration::from_millis(250);

/// A latency distribution's summary, by nearest rank.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Distribution {
    pub count: usize,
    pub min: Duration,
    pub p50: Duration,
    pub p90: Duration,
    pub p99: Duration,
    pub max: Duration,
}

impl Distribution {
    /// The summary of `samples`, or none without any.
    pub fn of(samples: &[Duration]) -> Option<Self> {
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let rank = |percent: usize| sorted[(percent * sorted.len()).div_ceil(100).max(1) - 1];
        Some(Self {
            count: sorted.len(),
            min: *sorted.first()?,
            p50: rank(50),
            p90: rank(90),
            p99: rank(99),
            max: *sorted.last()?,
        })
    }
}

/// One probe's samples in a run, and how many of its tries never completed
/// within their bound.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Latencies {
    pub samples: Vec<Duration>,
    pub timeouts: usize,
}

/// Why a Measure case needs the owner's written disposition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Finding {
    /// A targeted probe's 99th percentile exceeded [`TARGET`].
    TargetMissed { probe: Probe, p99: Duration },
    /// Samples at or above [`STALL`], which need diagnosis.
    Stalls {
        probe: Probe,
        count: usize,
        worst: Duration,
    },
    /// Tries that never completed within their bound.
    TimedOut { probe: Probe, count: usize },
    /// The default limits admitted fewer sessions than the case offered.
    NotAdmitted { admitted: u32, offered: u32 },
    /// A measured requirement without 25% headroom below the embedded one.
    Requirement(Requirement),
}

impl fmt::Display for Finding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetMissed { probe, p99 } => write!(
                formatter,
                "{probe} p99 {} ms exceeds the {} ms target",
                p99.as_millis(),
                TARGET.as_millis()
            ),
            Self::Stalls {
                probe,
                count,
                worst,
            } => write!(
                formatter,
                "{probe}: {count} stall(s) of {} ms or more (worst {} ms) need diagnosis",
                STALL.as_millis(),
                worst.as_millis()
            ),
            Self::TimedOut { probe, count } => {
                write!(formatter, "{probe}: {count} tr(ies) never completed")
            }
            Self::NotAdmitted { admitted, offered } => write!(
                formatter,
                "default limits admitted {admitted} of {offered} sessions"
            ),
            Self::Requirement(requirement) => write!(
                formatter,
                "{}: measured {} lacks 25% headroom below the embedded {} (embed at least {})",
                requirement.field,
                requirement.measured,
                requirement.embedded,
                requirement.recommended()
            ),
        }
    }
}

/// What a probe's latencies show: a missed target, stalls, and tries that
/// timed out, in that order.
pub fn latency_findings(probe: Probe, latencies: &Latencies) -> Vec<Finding> {
    let mut findings = Vec::new();
    if let Some(distribution) = Distribution::of(&latencies.samples) {
        if probe.targeted() && distribution.p99 > TARGET {
            findings.push(Finding::TargetMissed {
                probe,
                p99: distribution.p99,
            });
        }
        let count = latencies
            .samples
            .iter()
            .filter(|&&sample| sample >= STALL)
            .count();
        if count > 0 {
            findings.push(Finding::Stalls {
                probe,
                count,
                worst: distribution.max,
            });
        }
    }
    if latencies.timeouts > 0 {
        findings.push(Finding::TimedOut {
            probe,
            count: latencies.timeouts,
        });
    }
    findings
}

/// One requirement field as measured beside the value the release embeds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Requirement {
    pub field: &'static str,
    pub measured: u64,
    pub embedded: u64,
}

impl Requirement {
    /// Whether the embedded value leaves 25% headroom above the measured
    /// one: measured × 1.25 ≤ embedded.
    pub fn qualified(&self) -> bool {
        u128::from(self.measured) * 5 <= u128::from(self.embedded) * 4
    }

    /// The least value with 25% headroom above the measured one.
    pub fn recommended(&self) -> u64 {
        let value = (u128::from(self.measured) * 5).div_ceil(4);
        u64::try_from(value).unwrap_or(u64::MAX)
    }
}

/// The requirements without their headroom.
pub fn requirement_findings(requirements: &[Requirement]) -> Vec<Finding> {
    requirements
        .iter()
        .filter(|requirement| !requirement.qualified())
        .map(|&requirement| Finding::Requirement(requirement))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(values: &[u64]) -> Vec<Duration> {
        values.iter().copied().map(Duration::from_millis).collect()
    }

    fn latencies(values: &[u64]) -> Latencies {
        Latencies {
            samples: ms(values),
            timeouts: 0,
        }
    }

    /// Nearest rank: of 1..=100 ms, the 99th percentile is 99 ms; a single
    /// sample is every percentile.
    #[test]
    fn a_distribution_ranks_its_samples_by_nearest_rank() {
        let hundred: Vec<_> = (1..=100).rev().collect();
        assert_eq!(
            Distribution::of(&ms(&hundred)),
            Some(Distribution {
                count: 100,
                min: Duration::from_millis(1),
                p50: Duration::from_millis(50),
                p90: Duration::from_millis(90),
                p99: Duration::from_millis(99),
                max: Duration::from_millis(100),
            })
        );
        let one = Distribution::of(&ms(&[7])).unwrap();
        assert_eq!(
            [one.min, one.p50, one.p90, one.p99, one.max],
            [Duration::from_millis(7); 5]
        );
        let ten = Distribution::of(&ms(&[10, 20, 30, 40, 50, 60, 70, 80, 90, 1000])).unwrap();
        assert_eq!(
            (ten.p50, ten.p90, ten.p99),
            (ms(&[50])[0], ms(&[90])[0], ms(&[1000])[0])
        );
        assert_eq!(Distribution::of(&[]), None);
    }

    /// PERF-01 targets: a p99 of exactly 100 ms meets the target and one
    /// millisecond more misses it, for input, selector, global actions and
    /// attention only.
    #[test]
    fn a_targeted_probe_misses_only_above_100_ms_at_the_99th_percentile() {
        let at: Vec<u64> = vec![100; 100];
        assert_eq!(latency_findings(Probe::Input, &latencies(&at)), []);
        let past: Vec<u64> = vec![101; 100];
        for probe in [
            Probe::Input,
            Probe::Selector,
            Probe::GlobalAction,
            Probe::Attention,
        ] {
            assert_eq!(
                latency_findings(probe, &latencies(&past)),
                [Finding::TargetMissed {
                    probe,
                    p99: Duration::from_millis(101)
                }]
            );
        }
        for probe in [
            Probe::ExitStart,
            Probe::Switch,
            Probe::Readiness,
            Probe::Arrival,
        ] {
            assert_eq!(latency_findings(probe, &latencies(&past)), [], "{probe}");
        }
    }

    /// One sample of 250 ms needs diagnosis even when the p99 meets the
    /// target; 249 ms does not. Stalls are counted for every probe.
    #[test]
    fn any_stall_of_250_ms_or_more_needs_diagnosis() {
        let mut samples = vec![5; 399];
        samples.push(249);
        assert_eq!(latency_findings(Probe::Selector, &latencies(&samples)), []);
        samples.extend([250, 400]);
        assert_eq!(
            latency_findings(Probe::Selector, &latencies(&samples)),
            [Finding::Stalls {
                probe: Probe::Selector,
                count: 2,
                worst: Duration::from_millis(400)
            }]
        );
        assert_eq!(
            latency_findings(Probe::Readiness, &latencies(&[300])),
            [Finding::Stalls {
                probe: Probe::Readiness,
                count: 1,
                worst: Duration::from_millis(300)
            }]
        );
    }

    /// A try that never completed is reported, with or without samples.
    #[test]
    fn tries_that_never_completed_are_reported() {
        let timed_out = Latencies {
            samples: Vec::new(),
            timeouts: 2,
        };
        assert_eq!(
            latency_findings(Probe::Attention, &timed_out),
            [Finding::TimedOut {
                probe: Probe::Attention,
                count: 2
            }]
        );
        assert_eq!(
            latency_findings(Probe::Attention, &Latencies::default()),
            []
        );
    }

    /// Owner decision (#121): measured ≤ embedded with 25% headroom. 8
    /// against 10 qualifies; 9 does not, and 12 is the least to embed.
    #[test]
    fn a_requirement_needs_25_percent_headroom_below_the_embedded_value() {
        let requirement = |measured, embedded| Requirement {
            field: "instance_descriptors",
            measured,
            embedded,
        };
        assert!(requirement(8, 10).qualified());
        assert!(requirement(0, 0).qualified());
        assert!(!requirement(9, 10).qualified());
        assert!(!requirement(1, 1).qualified());
        assert_eq!(requirement(9, 10).recommended(), 12);
        assert_eq!(requirement(8, 10).recommended(), 10);
        assert_eq!(requirement(u64::MAX, 0).recommended(), u64::MAX);
        assert!(!requirement(u64::MAX, u64::MAX).qualified());
        assert_eq!(
            requirement_findings(&[requirement(8, 10), requirement(9, 10)]),
            [Finding::Requirement(requirement(9, 10))]
        );
        assert_eq!(
            requirement_findings(&[requirement(9, 10)])[0].to_string(),
            "instance_descriptors: measured 9 lacks 25% headroom below the embedded 10 \
             (embed at least 12)"
        );
    }
}
