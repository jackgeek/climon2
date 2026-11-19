//! One discovery pass over a climon2 home, shared by the CLI and the monitor:
//! enumerate session metadata, probe each valid entry with the frozen hello,
//! and return one structured outcome per entry. It never prints or logs;
//! callers own scheduling, presentation and logging.

mod hello;
mod incremental;
mod metadata;
mod outcome;

pub use incremental::{Batch, Continuation, Incomplete, Progress, Scan};
pub use metadata::{InvalidMetadata, InvalidReason, MetadataLimits};
pub use outcome::{Cleanup, LiveSession, Outcome, Uncertainty};

use climon2_core::layout::HomeLayout;
use std::{cmp::Ordering, io};

/// How a pass reads metadata, and what it may do besides observing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Options {
    pub limits: MetadataLimits,
    pub stale: StaleMetadata,
}

/// Whether a pass deletes stale metadata. Only CLI callers opt in; the monitor never does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StaleMetadata {
    /// Report stale sessions and keep their files.
    Retain,
    /// Also delete each stale session's metadata file and socket.
    Remove,
}

/// Every metadata entry in a climon2 home, each with one outcome.
#[derive(Debug)]
pub struct Pass {
    /// Sessions with valid metadata: newest created first, then ascending full session ID.
    pub sessions: Vec<Outcome>,
    /// Entries that failed the limits or the frozen metadata validation, in no particular
    /// order. Never deleted.
    pub invalid: Vec<InvalidMetadata>,
}

/// Probes every valid entry concurrently, each under the frozen two-second hello deadline.
/// `home` is the layout of a resolved, canonical climon2 home. Fails only when its sessions
/// directory cannot be listed or a probe panics.
pub fn discover(home: &HomeLayout, options: Options) -> io::Result<Pass> {
    let entries = metadata::entries(home, options.limits)?;
    let mut sessions = outcome::probe_all(home, entries.valid, options.stale)?;
    sessions.sort_by(newest_first);
    Ok(Pass {
        sessions,
        invalid: entries.invalid,
    })
}

fn newest_first(first: &Outcome, second: &Outcome) -> Ordering {
    let (first, second) = (first.metadata(), second.metadata());
    second
        .created_at
        .cmp(&first.created_at)
        .then_with(|| first.session_id.cmp(&second.session_id))
}
