//! Discovery for one link (#502; X12-13): passes over the relay's climon2
//! home, run on a thread of their own only while a link is established,
//! one at a time, a second apart, as the monitor runs them. What each pass
//! finds goes to the link's connection as it happens. Dropping the
//! [`Discovery`] ends the passes once the current one ends: a pass cannot
//! be interrupted, and it deletes nothing. With no link there is no
//! [`Discovery`], so the relay leaves every session daemon alone.

use crate::advertise::Found;
use climon2_core::{config::Limits, layout::HomeLayout};
use climon2_discovery::{Continuation, MetadataLimits, Outcome, Scan};
use std::{
    num::NonZeroUsize,
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::Duration,
};

/// The pause between one pass's end and the next pass's start: the
/// monitor's frozen discovery cadence.
const CADENCE: Duration = Duration::from_secs(1);

/// What discovery tells a link's connection.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Heard {
    Found(Found),
    /// A pass ended.
    Passed,
}

/// The passes running for one link.
pub(crate) struct Discovery {
    heard: Receiver<Heard>,
    /// Dropped with the discovery, which tells the passes to stop.
    _stop: Sender<()>,
}

impl Discovery {
    /// A discovery whose passes report on `heard` and stop once `stop`'s
    /// receiver sees it dropped.
    pub(crate) fn new(heard: Receiver<Heard>, stop: Sender<()>) -> Self {
        Self { heard, _stop: stop }
    }

    /// Starts passes over `home` on a thread of their own. `relay.limits`
    /// arrives later (#514); until then they keep `monitor.limits`'
    /// discovery defaults, as that decision says they will.
    pub(crate) fn start(home: HomeLayout) -> Self {
        let (report, heard) = mpsc::channel();
        let (stop, stopped) = mpsc::channel();
        let scan = scan();
        thread::spawn(move || passes(&home, &scan, &report, &stopped));
        Self::new(heard, stop)
    }

    /// What the passes found since the last call.
    pub(crate) fn heard(&self) -> Vec<Heard> {
        self.heard.try_iter().collect()
    }
}

fn passes(home: &HomeLayout, scan: &Scan, report: &Sender<Heard>, stopped: &Receiver<()>) {
    let mut continuation = Continuation::default();
    loop {
        continuation.pass(home, scan, |batch| {
            for outcome in batch.sessions {
                let _ = report.send(Heard::Found(found(outcome)));
            }
        });
        if report.send(Heard::Passed).is_err() {
            return;
        }
        if stopped.recv_timeout(CADENCE) != Err(RecvTimeoutError::Timeout) {
            return;
        }
    }
}

/// What a probe's outcome means for the advertisement.
fn found(outcome: Outcome) -> Found {
    match outcome {
        Outcome::Live(live) => Found::Live(Box::new(live.metadata)),
        Outcome::Uncertain(..) => Found::Uncertain,
        Outcome::Stale(metadata, _) => Found::Stale(metadata.session_id),
    }
}

/// `monitor.limits`' discovery bounds at their defaults.
fn scan() -> Scan {
    let limits = Limits::default().discovery();
    let positive = |value: u32| NonZeroUsize::new(value as usize).unwrap_or(NonZeroUsize::MIN);
    Scan {
        limits: MetadataLimits {
            file_bytes: Some(limits.metadata_file_bytes),
            json_depth: Some(limits.metadata_depth as usize),
        },
        probes: positive(limits.probes),
        batch_records: positive(limits.batch_records),
        batch_bytes: limits.batch_bytes,
        pass_time: Duration::from_millis(limits.pass_ms.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::advertise::tests::{id, metadata};
    use climon2_core::hello::{Advertised, HelloExtensions};
    use climon2_discovery::{Cleanup, LiveSession, Uncertainty};
    use std::{fs, time::Instant};

    #[test]
    fn a_live_session_is_found_live_a_stale_one_stale_and_the_rest_uncertain() {
        let extensions = HelloExtensions {
            capabilities: Advertised::Absent,
            state: Advertised::Absent,
            daemon_wire_v1: Advertised::Absent,
        };
        let live = Outcome::Live(LiveSession {
            metadata: metadata(1, "vim"),
            extensions,
        });
        assert_eq!(found(live), Found::Live(Box::new(metadata(1, "vim"))));
        let uncertain = Outcome::Uncertain(metadata(2, "vim"), Uncertainty::DaemonAlive);
        assert_eq!(found(uncertain), Found::Uncertain);
        let stale = Outcome::Stale(metadata(3, "vim"), Cleanup::Retained);
        assert_eq!(found(stale), Found::Stale(id(3)));
    }

    #[test]
    fn passes_keep_the_monitors_default_discovery_bounds() {
        let defaults = Limits::default().discovery();
        let scan = scan();
        assert_eq!(scan.limits.file_bytes, Some(defaults.metadata_file_bytes));
        assert_eq!(
            scan.limits.json_depth,
            Some(defaults.metadata_depth as usize)
        );
        assert_eq!(scan.probes.get(), defaults.probes as usize);
        assert_eq!(scan.batch_records.get(), defaults.batch_records as usize);
        assert_eq!(scan.batch_bytes, defaults.batch_bytes);
        assert_eq!(scan.pass_time.as_millis(), u128::from(defaults.pass_ms));
    }

    /// Waits until `discovery` has heard `count` things, at most 10 s.
    fn hear(discovery: &Discovery, count: usize) -> Vec<Heard> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut heard = Vec::new();
        while heard.len() < count {
            assert!(Instant::now() < deadline, "heard only {heard:?}");
            heard.extend(discovery.heard());
            thread::sleep(Duration::from_millis(10));
        }
        heard
    }

    /// X12-13: each pass reports what it found, then that it ended, and
    /// the next pass follows a second later. A session whose daemon cannot
    /// be judged is only uncertain.
    #[test]
    fn passes_report_each_session_then_their_end_a_second_apart() {
        let folder = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(folder.path()).unwrap();
        let sessions = root.join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let mut unjudged = metadata(1, "vim");
        unjudged.socket_path = root.join("absent.sock").to_str().unwrap().parse().unwrap();
        unjudged.daemon_pid = 0;
        unjudged.daemon_start_time = 0;
        unjudged.daemon_linux_identity = None;
        let path = sessions.join(format!("{}.pty", unjudged.session_id));
        fs::write(path, unjudged.to_json().unwrap()).unwrap();

        let started = Instant::now();
        let discovery = Discovery::start(HomeLayout::at(&root).unwrap());
        assert_eq!(
            hear(&discovery, 2),
            [Heard::Found(Found::Uncertain), Heard::Passed]
        );
        assert_eq!(
            hear(&discovery, 2),
            [Heard::Found(Found::Uncertain), Heard::Passed]
        );
        assert!(started.elapsed() >= CADENCE);
    }

    /// Dropping the discovery stops its passes.
    #[test]
    fn passes_stop_once_their_discovery_is_dropped() {
        let (report, heard) = mpsc::channel();
        let (stop, stopped) = mpsc::channel::<()>();
        let folder = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(folder.path()).unwrap();
        let home = HomeLayout::at(&root).unwrap();
        let passing = thread::spawn(move || passes(&home, &scan(), &report, &stopped));
        assert_eq!(heard.recv().unwrap(), Heard::Passed);
        drop(stop);
        passing.join().unwrap();
        assert!(heard.try_iter().all(|heard| heard == Heard::Passed));
    }

    /// Passes nobody hears any more stop too.
    #[test]
    fn passes_stop_once_nobody_hears_them() {
        let (report, heard) = mpsc::channel();
        let (_stop, stopped) = mpsc::channel::<()>();
        let folder = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(folder.path()).unwrap();
        let home = HomeLayout::at(&root).unwrap();
        drop(heard);
        let passing = thread::spawn(move || passes(&home, &scan(), &report, &stopped));
        passing.join().unwrap();
    }
}
