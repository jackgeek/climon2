//! A monitor's virtual terminal for one session: an allocation-bounded copy
//! kept from the session's terminal bytes (ADR 0007).

use crate::{
    bounded::{Bounded, Room},
    cost,
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

/// Growth refused before it happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rejected {
    /// The bytes the model would have had to hold, its worst case for the
    /// refused update included.
    pub needed: usize,
    /// The limit that refused it.
    pub limit: usize,
}

/// One session's terminal model: the current model the monitor paints and,
/// while a restoration is in progress, the separate model it is staged into.
/// After a rejection the current model is read-only, keeping its last safe
/// page, until a staged restoration is applied.
pub struct Model {
    limits: Limits,
    current: Bounded,
    staged: Option<Bounded>,
    /// The rejection that made the current model read-only.
    rejected: Option<Rejected>,
}

impl Model {
    /// Bytes every model charges beyond what its engine allocates: room for
    /// the front filter's held token and the tokens one slice completes, and
    /// for the engine's brief copies.
    pub const RESERVE: usize = cost::RESERVE;

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
            staged: None,
            rejected: None,
        })
    }

    /// Begin a restoration at `geometry`: a blank model to stage it into,
    /// replacing any restoration already staged. If both models cannot fit,
    /// nothing is allocated and the current model becomes read-only.
    pub fn stage(&mut self, geometry: Geometry) -> Result<(), Rejected> {
        self.staged = None;
        self.current.measure();
        let needed = self.current.charge() + Bounded::mandatory(geometry, self.limits.history_rows);
        if needed > self.limits.bytes {
            let rejected = Rejected {
                needed,
                limit: self.limits.bytes,
            };
            self.rejected = Some(rejected);
            return Err(rejected);
        }
        self.staged = Some(Bounded::new(geometry, self.limits.history_rows));
        Ok(())
    }

    /// Complete the staged restoration: it replaces the current model and its
    /// history whole. False when nothing is staged.
    pub fn apply(&mut self) -> bool {
        let Some(staged) = self.staged.take() else {
            return false;
        };
        self.current = staged;
        self.rejected = None;
        true
    }

    /// Abandon the staged restoration, if any.
    pub fn discard(&mut self) {
        self.staged = None;
    }

    /// The current model's geometry.
    pub fn geometry(&self) -> Geometry {
        self.current.geometry()
    }

    /// Apply terminal bytes from the session. Output that would exceed the
    /// limit is rejected before it reaches the engine; the model keeps the
    /// page its accepted output produced and refuses everything after.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), Rejected> {
        match self.staged.as_mut() {
            Some(staged) => {
                let room = Room {
                    other: self.current.charge(),
                    limit: self.limits.bytes,
                };
                let fed = staged.feed(bytes, room);
                self.restored(fed)
            }
            None => self.live(bytes),
        }
    }

    /// A rejected restoration is discarded whole.
    fn restored(&mut self, fed: Result<(), Rejected>) -> Result<(), Rejected> {
        if let Err(rejected) = fed {
            self.staged = None;
            self.rejected = Some(rejected);
        }
        fed
    }

    fn live(&mut self, bytes: &[u8]) -> Result<(), Rejected> {
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

    /// The allocation capacity charged against the limit, for the current
    /// and any staged model.
    pub fn footprint(&self) -> usize {
        self.current.charge() + self.staged.as_ref().map_or(0, Bounded::charge)
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
