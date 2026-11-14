//! Discovery off the UI thread (#106): one pass at once, the first
//! immediately and each later one a second after the previous one finished,
//! so passes never overlap. Each batch and each pass's progress goes to the
//! UI thread as it happens. Monitor discovery never deletes metadata.
use climon2_core::{config::DiscoveryLimits, layout::HomeLayout};
use climon2_discovery::{Batch, Continuation, MetadataLimits, Progress, Scan};
use std::{
    num::NonZeroUsize,
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::Duration,
};

/// The frozen discovery cadence: the pause between one pass's end and the
/// next pass's start.
const CADENCE: Duration = Duration::from_secs(1);

/// What discovery tells the UI thread.
pub(crate) enum Found {
    Batch(Batch),
    /// A pass ended.
    Pass(Progress),
}

/// The running discovery thread. Dropping it stops discovery once its
/// current pass ends; the thread is never joined, because a pass cannot be
/// interrupted, and it deletes nothing.
pub(crate) struct Discovery {
    pub(crate) found: Receiver<Found>,
    _stop: Sender<()>,
}

impl Discovery {
    pub(crate) fn start(home: HomeLayout, limits: DiscoveryLimits) -> std::io::Result<Self> {
        let (found, receiver) = mpsc::channel();
        let (stop, stopped) = mpsc::channel();
        let scan = scan(limits);
        thread::Builder::new()
            .name("discovery".into())
            .spawn(move || passes(&home, &scan, &found, &stopped))?;
        Ok(Self {
            found: receiver,
            _stop: stop,
        })
    }
}

fn passes(home: &HomeLayout, scan: &Scan, found: &Sender<Found>, stopped: &Receiver<()>) {
    let mut continuation = Continuation::default();
    loop {
        let progress = continuation.pass(home, scan, |batch| {
            let _ = found.send(Found::Batch(batch));
        });
        if found.send(Found::Pass(progress)).is_err() {
            return;
        }
        if stopped.recv_timeout(CADENCE) != Err(RecvTimeoutError::Timeout) {
            return;
        }
    }
}

/// The configured bounds; every one is positive by configuration.
fn scan(limits: DiscoveryLimits) -> Scan {
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
