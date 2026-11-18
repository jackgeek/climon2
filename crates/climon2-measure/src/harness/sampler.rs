//! Samples the measured processes on a thread of its own every
//! [`INTERVAL`]: the clients' process trees and the session daemons,
//! separately, and how many sessions are still waiting for a client.

use crate::sample::{self, Sample, Tick};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const INTERVAL: Duration = Duration::from_millis(250);

/// What is sampled; the run changes it as clients and sessions come.
#[derive(Clone, Debug, Default)]
pub struct Watched {
    /// Each client's root process: a monitor or a standalone client.
    pub clients: Vec<i32>,
    pub daemons: Vec<i32>,
    /// Sessions that should have a client.
    pub expected: u32,
}

/// A window's samples.
#[derive(Clone, Debug, Default)]
pub struct Sampled {
    pub clients: Vec<Tick>,
    pub daemons: Vec<Tick>,
    pub backlog_max: u32,
}

/// The descriptors a session daemon holds with no client connected; each
/// connected client adds its connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Baseline(pub u32);

impl Baseline {
    /// The descriptors most of `daemons` hold now, before any client: one
    /// still answering the CLI that started it holds one more.
    pub fn of(daemons: &[i32]) -> Self {
        Self(mode(
            daemons
                .iter()
                .filter_map(|&pid| sample::sample(pid))
                .map(|sample| sample.descriptors),
        ))
    }

    /// Whether a client is connected to `daemon`.
    pub fn connected(self, daemon: i32) -> bool {
        sample::sample(daemon).is_some_and(|sample| sample.descriptors > self.0)
    }

    /// How many of `daemons` have a client connected.
    pub fn count(self, daemons: &[i32]) -> u32 {
        daemons.iter().filter(|&&pid| self.connected(pid)).count() as u32
    }
}

/// The most frequent value, the least of equally frequent ones; 0 for none.
pub fn mode(values: impl IntoIterator<Item = u32>) -> u32 {
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|(a, ca), (b, cb)| ca.cmp(cb).then(b.cmp(a)))
        .map_or(0, |(value, _)| value)
}

/// How long a daemon must hold a client's connection, sample after sample,
/// for it to count: discovery's hello probes connect only briefly.
pub const HOLD: Duration = Duration::from_millis(300);

/// Each daemon's current run of samples above its baseline.
#[derive(Debug, Default)]
pub struct Streaks(BTreeMap<i32, Duration>);

impl Streaks {
    /// Records whether each daemon was above its baseline at `at`.
    pub fn observe(&mut self, at: Duration, daemons: impl IntoIterator<Item = (i32, bool)>) {
        for (daemon, above) in daemons {
            if above {
                self.0.entry(daemon).or_insert(at);
            } else {
                self.0.remove(&daemon);
            }
        }
    }

    /// The daemons whose connection has been held for [`HOLD`] at `at`,
    /// each with when it began.
    pub fn held(&self, at: Duration) -> Vec<(i32, Duration)> {
        self.0
            .iter()
            .filter(|(_, began)| at.saturating_sub(**began) >= HOLD)
            .map(|(&daemon, &began)| (daemon, began))
            .collect()
    }
}

pub struct Sampler {
    pub watched: Arc<Mutex<Watched>>,
    stop: Arc<AtomicBool>,
    handle: JoinHandle<Sampled>,
}

fn tick(at: Duration, pids: impl IntoIterator<Item = i32>) -> Tick {
    let processes: BTreeMap<i32, Sample> = pids
        .into_iter()
        .filter_map(|pid| Some((pid, sample::sample(pid)?)))
        .collect();
    Tick { at, processes }
}

impl Sampler {
    pub fn start(watched: Watched, baseline: Baseline) -> Self {
        let watched = Arc::new(Mutex::new(watched));
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, stopping) = (watched.clone(), stop.clone());
        let handle = thread::spawn(move || {
            let start = Instant::now();
            let mut sampled = Sampled::default();
            let mut streaks = Streaks::default();
            loop {
                let now = shared.lock().expect("watched").clone();
                let at = start.elapsed();
                sampled
                    .clients
                    .push(tick(at, sample::descendants(&now.clients)));
                sampled.daemons.push(tick(at, now.daemons.iter().copied()));
                streaks.observe(
                    at,
                    now.daemons
                        .iter()
                        .map(|&pid| (pid, baseline.connected(pid))),
                );
                // A connection counts only once held, so no backlog is
                // known before then.
                if at >= HOLD {
                    let backlog = now.expected.saturating_sub(streaks.held(at).len() as u32);
                    sampled.backlog_max = sampled.backlog_max.max(backlog);
                }
                if stopping.load(Ordering::Relaxed) {
                    return sampled;
                }
                thread::sleep(INTERVAL);
            }
        });
        Self {
            watched,
            stop,
            handle,
        }
    }

    pub fn add_client(&self, root: i32) {
        self.watched.lock().expect("watched").clients.push(root);
    }

    pub fn add_session(&self, daemon: i32) {
        let mut watched = self.watched.lock().expect("watched");
        watched.daemons.push(daemon);
        watched.expected += 1;
    }

    /// Takes one last sample and returns the window's.
    pub fn finish(self) -> Sampled {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.join().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One daemon still answering its CLI holds one more descriptor than
    /// the rest; ties go to the fewer.
    #[test]
    fn the_baseline_is_what_most_daemons_hold() {
        assert_eq!(mode([10, 10, 11, 10, 9]), 10);
        assert_eq!(mode([11, 10]), 10);
        assert_eq!(mode([]), 0);
    }

    /// A connection counts once held for 300 ms, sample after sample, from
    /// when it began; a brief probe, or a gap, never counts.
    #[test]
    fn only_a_connection_held_for_300_ms_counts() {
        let ms = Duration::from_millis;
        let mut streaks = Streaks::default();
        streaks.observe(ms(0), [(1, true), (2, true)]);
        streaks.observe(ms(10), [(1, true), (2, false)]);
        streaks.observe(ms(20), [(1, true), (2, true)]);
        assert_eq!(streaks.held(ms(299)), []);
        assert_eq!(streaks.held(ms(300)), [(1, ms(0))]);
        assert_eq!(streaks.held(ms(320)), [(1, ms(0)), (2, ms(20))]);
        streaks.observe(ms(330), [(1, false)]);
        assert_eq!(streaks.held(ms(400)), [(2, ms(20))]);
    }
}
