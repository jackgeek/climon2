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
#[derive(Clone, Copy, Debug)]
pub struct Baseline(pub u32);

impl Baseline {
    /// The most descriptors any of `daemons` holds now, before any client.
    pub fn of(daemons: &[i32]) -> Self {
        Self(
            daemons
                .iter()
                .filter_map(|&pid| sample::sample(pid))
                .map(|sample| sample.descriptors)
                .max()
                .unwrap_or(0),
        )
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
            loop {
                let now = shared.lock().expect("watched").clone();
                let at = start.elapsed();
                sampled
                    .clients
                    .push(tick(at, sample::descendants(&now.clients)));
                sampled.daemons.push(tick(at, now.daemons.iter().copied()));
                let backlog = now.expected.saturating_sub(baseline.count(&now.daemons));
                sampled.backlog_max = sampled.backlog_max.max(backlog);
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
