//! A monitor's virtual terminal for one session: an allocation-bounded copy
//! kept from the session's terminal bytes (ADR 0007).

use crate::{
    bounded::{Bounded, Room},
    cost,
    engine::Parser,
    geometry::Geometry,
    view::View,
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

/// What a model must be able to hold, by part. The monitor's one shared
/// [`crate::Parser`] is not part of it: it counts that once
/// ([`crate::Parser::BYTES`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mandatory {
    /// The current model alone, which [`Model::new`] admits a blank model
    /// with: the engine for its geometry and the reserve.
    pub current_model: usize,
    /// The first history chunk, which holds the protected newest page and is
    /// admitted when the normal screen first scrolls; none for a model that
    /// keeps no history.
    pub recent_page: usize,
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
        Ok(Self {
            limits,
            current: Room::alone(limits.bytes).blank(geometry, limits.history_rows)?,
            staged: None,
            rejected: None,
        })
    }

    /// What a model at `geometry` must be able to hold, by part: the current
    /// model, which `Model::new` admits, and, when it keeps any of
    /// `history_rows`, the first history chunk, which holds the protected
    /// newest page and is admitted at the first scroll.
    pub fn mandatory(geometry: Geometry, history_rows: usize) -> Mandatory {
        Bounded::mandatory(geometry, history_rows)
    }

    /// Begin a restoration at `geometry`: a blank model to stage it into,
    /// replacing any restoration already staged. The current model's oldest
    /// history yields first, a chunk at a time; if both models still cannot
    /// fit, nothing is allocated and the current model becomes read-only.
    pub fn stage(&mut self, geometry: Geometry) -> Result<(), Rejected> {
        self.staged = None;
        self.current.measure();
        let staged = Room::beside(&mut self.current, self.limits.bytes)
            .blank(geometry, self.limits.history_rows);
        match staged {
            Ok(staged) => {
                self.staged = Some(staged);
                Ok(())
            }
            Err(rejected) => {
                self.rejected = Some(rejected);
                Err(rejected)
            }
        }
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

    /// Apply terminal bytes from the session. History yields to them, oldest
    /// chunk first: the current model's while a restoration is staged, then
    /// that of the model they update, never the first chunk, which holds the
    /// protected newest page. Output that would still exceed the limit is
    /// rejected before it reaches the engine; the model keeps the page its
    /// accepted output produced and refuses everything after.
    pub fn feed(&mut self, parser: &mut Parser, bytes: &[u8]) -> Result<(), Rejected> {
        match self.staged.as_mut() {
            Some(staged) => {
                let fed = staged.feed(
                    parser,
                    bytes,
                    &mut Room::beside(&mut self.current, self.limits.bytes),
                );
                self.restored(fed)
            }
            None => self.live(parser, bytes),
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

    fn live(&mut self, parser: &mut Parser, bytes: &[u8]) -> Result<(), Rejected> {
        if let Some(rejected) = self.rejected {
            return Err(rejected);
        }
        let fed = self
            .current
            .feed(parser, bytes, &mut Room::alone(self.limits.bytes));
        self.rejected = fed.err();
        fed
    }

    /// Discard the current model's oldest history chunk and release its rows
    /// at once, for the monitor's pressure policy across sessions (#119).
    /// The first chunk, which holds the protected newest page, is never
    /// discarded: false when only it is left, or the model keeps no history.
    pub fn shorten_history(&mut self) -> bool {
        self.current.shorten()
    }

    /// The allocation capacity charged against the limit, for the current
    /// and any staged model.
    pub fn footprint(&self) -> usize {
        self.current.charge() + self.staged.as_ref().map_or(0, Bounded::charge)
    }

    /// The current model's normal-screen history, oldest row first, while
    /// the normal screen is active.
    pub fn history(&self) -> Vec<String> {
        self.view().history_text()
    }

    /// The current model's active screen, one string per row: each
    /// character with its combining marks, trailing blanks trimmed.
    pub fn screen(&self) -> Vec<String> {
        self.view().screen_text()
    }

    /// Scroll the current model's normal-screen view `rows` back into its
    /// history (forward, toward the live screen, when negative), as far as
    /// its oldest row, or the truncation notice above it when older rows
    /// were discarded, and no further than the live screen. Nothing scrolls
    /// while the alternate screen is active. Output keeps a scrolled view
    /// anchored on the rows it shows ([`View::scrolled`]).
    pub fn scroll(&mut self, rows: isize) {
        self.current.scroll(rows);
    }

    /// Return the current model's view to the live screen.
    pub fn return_live(&mut self) {
        self.current.return_live();
    }

    /// What the current model shows, for painting.
    pub fn view(&self) -> View<'_> {
        self.current.view()
    }
}
