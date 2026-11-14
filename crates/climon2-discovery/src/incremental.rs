//! Incremental, bounded and fair discovery passes (spec #73, "Incremental and
//! fair"; #106): what the monitor runs off its UI thread. A pass walks the
//! sessions directory in name order from where the previous pass stopped,
//! listing, reading and probing one bounded batch at a time, so it never holds
//! more than one batch of entries. It never deletes metadata.
use crate::{
    InvalidMetadata, MetadataLimits, Outcome, StaleMetadata, metadata, newest_first, outcome,
};
use climon2_core::{layout::HomeLayout, metadata::SessionMetadata};
use std::{
    collections::BinaryHeap,
    ffi::{OsStr, OsString},
    fs, io,
    num::NonZeroUsize,
    path::Path,
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

/// The bounds of every pass a caller runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Scan {
    pub limits: MetadataLimits,
    /// The most hello probes in flight at once.
    pub probes: NonZeroUsize,
    /// The most entries one batch lists, reads and probes.
    pub batch_records: NonZeroUsize,
    /// The most metadata bytes one batch reads. A batch ends before the entry
    /// that would take it past this, but always holds its first entry.
    pub batch_bytes: u64,
    /// How long a pass may start new probes. A pass always starts its first
    /// probe, so every pass makes progress.
    pub pass_time: Duration,
}

/// What one batch of a pass found.
#[derive(Debug, Default)]
pub struct Batch {
    /// Newest created first, then ascending full session ID.
    pub sessions: Vec<Outcome>,
    pub invalid: Vec<InvalidMetadata>,
}

/// Whether a pass reached every entry.
#[derive(Debug)]
pub enum Progress {
    /// The pass reported every entry it found, possibly none.
    Complete,
    /// The pass stopped before reaching every entry; the next pass continues
    /// after the last entry it reported.
    Incomplete(Incomplete),
}

/// Why a pass stopped early.
#[derive(Debug)]
pub enum Incomplete {
    /// The pass deadline passed.
    Deadline,
    /// The sessions directory cannot be listed.
    Inaccessible(io::Error),
}

/// Where the next pass starts: just after the last entry an incomplete pass
/// reached, otherwise at the first name.
#[derive(Debug, Default)]
pub struct Continuation {
    after: Option<OsString>,
}

impl Continuation {
    /// One pass, giving `report` each batch once it is probed. The pass
    /// reaches names after its start, then wraps to those up to its start, so
    /// entries added behind its position wait for the next pass.
    pub fn pass(
        &mut self,
        home: &HomeLayout,
        scan: &Scan,
        mut report: impl FnMut(Batch),
    ) -> Progress {
        let walk = Walk {
            home,
            sessions: home.sessions(),
            scan,
            deadline: Instant::now() + scan.pass_time,
            progressed: AtomicBool::new(false),
        };
        let start = self.after.take();
        let mut ranges = vec![Range {
            after: start.clone(),
            through: None,
        }];
        ranges.extend(start.map(|start| Range {
            after: None,
            through: Some(start),
        }));
        for mut range in ranges {
            let stopped = walk.range(&mut range, &mut report);
            if let Some(incomplete) = stopped {
                self.after = range.after;
                return Progress::Incomplete(incomplete);
            }
        }
        Progress::Complete
    }
}

/// Entry names after `after` (exclusive) through `through` (inclusive).
struct Range {
    after: Option<OsString>,
    through: Option<OsString>,
}

impl Range {
    /// Whether `name` is a metadata file in this range.
    fn admits(&self, name: &OsStr) -> bool {
        metadata::is_metadata(Path::new(name)) && self.holds(name)
    }

    fn holds(&self, name: &OsStr) -> bool {
        self.after.as_deref().is_none_or(|after| name > after)
            && self
                .through
                .as_deref()
                .is_none_or(|through| name <= through)
    }
}

struct Walk<'a> {
    home: &'a HomeLayout,
    sessions: std::path::PathBuf,
    scan: &'a Scan,
    /// The pass deadline, from the pass's start.
    deadline: Instant,
    /// Whether this pass has reached an entry or started a probe: until it
    /// has, the deadline stops nothing, so every pass makes progress.
    progressed: AtomicBool,
}

impl Walk<'_> {
    /// Reports `range` batch by batch, advancing it past each entry reached,
    /// until it is exhausted (`None`) or the pass stops early.
    fn range(&self, range: &mut Range, report: &mut impl FnMut(Batch)) -> Option<Incomplete> {
        loop {
            if let Err(stopped) = self.step(range, report) {
                return stopped;
            }
        }
    }

    /// Reports `range`'s next batch. Fails with `None` once the range is
    /// exhausted, or with why the pass stops early.
    fn step(
        &self,
        range: &mut Range,
        report: &mut impl FnMut(Batch),
    ) -> Result<(), Option<Incomplete>> {
        if self.late() {
            return Err(Some(Incomplete::Deadline));
        }
        let names = self
            .names(range)
            .map_err(|error| Some(Incomplete::Inaccessible(error)))?;
        if names.is_empty() {
            return Err(None);
        }
        let (batch, reached, finished) = self.batch(names);
        if let Some(reached) = reached {
            self.progressed.store(true, Ordering::Relaxed);
            range.after = Some(reached);
            report(batch);
        }
        finished.then_some(()).ok_or(Some(Incomplete::Deadline))
    }

    /// Whether the deadline stops the pass: never before it progressed.
    fn late(&self) -> bool {
        self.progressed.load(Ordering::Relaxed) && Instant::now() >= self.deadline
    }

    /// The first `batch_records` metadata names in `range`, ascending, kept in
    /// a bounded heap while the directory is listed.
    fn names(&self, range: &Range) -> io::Result<Vec<OsString>> {
        let limit = self.scan.batch_records.get();
        let mut kept = BinaryHeap::with_capacity(limit + 1);
        for entry in listing(&self.sessions)? {
            let name = entry?.file_name();
            if range.admits(&name) {
                keep(&mut kept, name, limit);
            }
        }
        Ok(kept.into_sorted_vec())
    }

    /// Reads `names` in order within the byte budget, then probes the valid
    /// entries. Returns the batch, the last name it reached, and whether it
    /// reached every entry it read.
    fn batch(&self, names: Vec<OsString>) -> (Batch, Option<OsString>, bool) {
        let read = self.read(names);
        let valid: Vec<_> = read
            .iter()
            .filter_map(|(_, entry)| entry.as_ref().ok().cloned())
            .collect();
        let total = valid.len();
        let outcomes = self.probe(valid);
        let mut probed = outcomes.len();
        let mut batch = Batch {
            sessions: outcomes,
            invalid: Vec::new(),
        };
        let mut reached = None;
        // Probes are claimed in name order, so the reached entries are a
        // prefix of those read.
        for (name, entry) in read {
            match entry {
                Ok(_) if probed == 0 => break,
                Ok(_) => probed -= 1,
                Err(invalid) => batch.invalid.push(invalid),
            }
            reached = Some(name);
        }
        let finished = batch.sessions.len() == total;
        batch.sessions.sort_by(newest_first);
        (batch, reached, finished)
    }

    fn read(
        &self,
        names: Vec<OsString>,
    ) -> Vec<(OsString, Result<SessionMetadata, InvalidMetadata>)> {
        let mut read = Vec::with_capacity(names.len());
        let mut bytes = 0u64;
        for name in names {
            let path = self.sessions.join(&name);
            let (size, entry) = metadata::read_counted(&path, self.scan.limits);
            let total = bytes.saturating_add(size);
            if !read.is_empty() && total > self.scan.batch_bytes {
                break;
            }
            bytes = total;
            read.push((
                name,
                entry.map_err(|reason| InvalidMetadata { path, reason }),
            ));
        }
        read
    }

    /// Probes `sessions` in order, at most `probes` at once, starting none
    /// after the deadline. Returns the outcomes of the prefix it probed.
    fn probe(&self, sessions: Vec<SessionMetadata>) -> Vec<Outcome> {
        let next = AtomicUsize::new(0);
        let slots: Vec<_> = sessions.iter().map(|_| Mutex::new(None)).collect();
        let workers = self.scan.probes.get().min(sessions.len());
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    while !self.late() {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(metadata) = sessions.get(index) else {
                            return;
                        };
                        self.progressed.store(true, Ordering::Relaxed);
                        let found =
                            outcome::probe_one(self.home, metadata.clone(), StaleMetadata::Retain);
                        *slots[index].lock().unwrap_or_else(PoisonError::into_inner) = Some(found);
                    }
                });
            }
        });
        slots
            .into_iter()
            .map_while(|slot| slot.into_inner().unwrap_or_else(PoisonError::into_inner))
            .collect()
    }
}

/// The sessions directory's entries; none when it does not exist.
fn listing(sessions: &Path) -> io::Result<impl Iterator<Item = io::Result<fs::DirEntry>>> {
    let entries = match fs::read_dir(sessions) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        entries => Some(entries?),
    };
    Ok(entries.into_iter().flatten())
}

/// Adds `name` to `kept`, then drops the greatest name past `limit`.
fn keep(kept: &mut BinaryHeap<OsString>, name: OsString, limit: usize) {
    kept.push(name);
    if kept.len() > limit {
        kept.pop();
    }
}
