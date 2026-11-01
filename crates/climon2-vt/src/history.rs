//! Counting pushes into the normal screen's history (plan section 3.2,
//! "History accounting"). The engine cannot report pushes, and a push at
//! capacity replaces the oldest row silently, so the engine keeps `2P` rows
//! of headroom beyond retention and the ledger counts growth per step.

/// The adapter's record of normal-screen history.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Ledger {
    pub(crate) max_rows: usize,
    push_bound: usize,
    /// Rows pushed into normal history; monotonic.
    pub(crate) pushed: u64,
    /// Incremented whenever normal history is cleared or becomes unaccountable.
    pub(crate) epoch: u64,
    /// Some row left retention since history was last cleared.
    pub(crate) evicted: bool,
}

impl Ledger {
    pub(crate) fn new(max_rows: usize, push_bound: usize) -> Self {
        Self {
            max_rows,
            push_bound,
            ..Self::default()
        }
    }

    /// History capacity the engine must have: retention plus headroom.
    pub(crate) fn capacity(&self) -> usize {
        self.max_rows + 2 * self.push_bound
    }

    /// Account for one engine step. `before` and `after` are the normal
    /// screen's history sizes when it was the active screen.
    pub(crate) fn record(&mut self, before: Option<usize>, after: Option<usize>, cleared: bool) {
        if cleared {
            self.clear();
        } else if let (Some(before), Some(after)) = (before, after) {
            self.pushed += after.saturating_sub(before) as u64;
            self.evicted |= after > self.max_rows;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.epoch += 1;
        self.evicted = false;
    }

    /// A resize may reflow history; any view of it is retired.
    pub(crate) fn resized(&mut self, push_bound: usize, maybe_dropped: bool) {
        self.epoch += 1;
        self.evicted |= maybe_dropped;
        self.push_bound = push_bound;
    }
}
