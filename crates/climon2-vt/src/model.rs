//! A monitor's virtual terminal for one session: an allocation-bounded copy
//! kept from the session's terminal bytes (ADR 0007).

use crate::{
    bounded::{Bounded, Room},
    geometry::Geometry,
};

/// The allocation a model may hold and the history rows it may keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Bytes of allocation capacity for the current model and any staged
    /// replacement together.
    pub bytes: usize,
    /// Normal-screen history rows to keep at most.
    pub history_rows: usize,
}

/// Growth refused before it happened: `needed` bytes would have exceeded the
/// `limit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rejected {
    pub needed: usize,
    pub limit: usize,
}

/// One session's terminal model.
pub struct Model {
    limits: Limits,
    current: Bounded,
    /// The rejection that made the current model read-only.
    rejected: Option<Rejected>,
}

impl Model {
    /// A blank model, if its mandatory allocation fits the limit.
    pub fn new(geometry: Geometry, limits: Limits) -> Result<Self, Rejected> {
        let needed = Bounded::mandatory(geometry, limits.history_rows);
        if needed > limits.bytes {
            return Err(Rejected {
                needed,
                limit: limits.bytes,
            });
        }
        Ok(Self {
            limits,
            current: Bounded::new(geometry, limits.history_rows),
            rejected: None,
        })
    }

    /// Apply terminal bytes from the session. Output that would exceed the
    /// limit is rejected before it reaches the engine; the model keeps the
    /// page its accepted output produced and refuses everything after.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), Rejected> {
        if let Some(rejected) = self.rejected {
            return Err(rejected);
        }
        let room = Room {
            other: 0,
            limit: self.limits.bytes,
        };
        let fed = self.current.feed(bytes, room);
        self.rejected = fed.err();
        fed
    }

    /// The allocation capacity charged against the limit.
    pub fn footprint(&self) -> usize {
        self.current.charge()
    }

    /// The current model's normal-screen history, oldest row first, while
    /// the normal screen is active.
    pub fn history(&self) -> Vec<String> {
        self.current.history()
    }

    /// The current model's active screen, one string per row: each
    /// character with its combining marks, trailing blanks trimmed.
    pub fn screen(&self) -> Vec<String> {
        self.current.screen()
    }
}
