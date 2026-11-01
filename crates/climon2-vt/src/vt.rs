use crate::{
    classify::{Clear, Live, classify},
    engine::Engine,
    geometry::{Geometry, STEP},
    history::Ledger,
    modes::{DIRECT, SHARED, cursor_style, transitions},
    presentation::{Mode, Presentation},
    render::{cursor, cursors, screen},
    restore::{Chunk, Restoration, lead},
    sync::Latch,
    title,
    token::{Kind, Token, Tokenizer},
};
use alacritty_terminal::{grid::Dimensions, term::TermMode};
use std::time::Instant;

/// Session settings fixed at startup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VtConfig {
    /// Retained normal-screen history rows (`scrollback.max_rows`).
    pub max_history_rows: u32,
    /// Shown when the application has not set a title.
    pub default_title: String,
}

/// The result of feeding PTY output.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fed {
    /// Filtered output for a Direct controller, in order.
    pub live: Vec<u8>,
    /// The engine's answers to terminal queries, for the PTY.
    pub replies: Vec<u8>,
    /// Oversized tokens dropped from both engine and output; log their count
    /// without content.
    pub dropped: u32,
}

/// The session's virtual terminal: the front filter, the engine and the
/// adapter state around it. Every operation is infallible.
pub struct Vt {
    engine: Engine,
    tokenizer: Tokenizer,
    latch: Latch,
    ledger: Ledger,
    geometry: Geometry,
    default_title: String,
}

impl Vt {
    pub fn new(geometry: Geometry, config: VtConfig) -> Self {
        let ledger = Ledger::new(config.max_history_rows as usize, geometry.push_bound());
        Self {
            engine: Engine::new(geometry, ledger.capacity()),
            tokenizer: Tokenizer::new(),
            latch: Latch::default(),
            ledger,
            geometry,
            default_title: config.default_title,
        }
    }

    pub fn geometry(&self) -> Geometry {
        self.geometry
    }

    /// Apply a new session geometry; an equal geometry changes nothing.
    /// Any restoration in progress is retired by the grant or repaint that
    /// must follow.
    pub fn resize(&mut self, geometry: Geometry) {
        if geometry == self.geometry {
            return;
        }
        let shrinks =
            geometry.cols() < self.geometry.cols() || geometry.rows() < self.geometry.rows();
        let normal = self.engine.normal_history().is_some();
        if normal {
            self.engine.set_history(self.ledger.max_rows);
        }
        self.engine.resize(geometry);
        let dropped = self
            .engine
            .normal_history()
            .map_or(shrinks, |held| held >= self.ledger.max_rows);
        self.ledger.resized(geometry.push_bound(), dropped);
        self.geometry = geometry;
    }

    /// Take the snapshot point S for a newly granted controller whose host
    /// is described by `presentation`. Returns phase A, to be committed
    /// now, and the restoration producing phases B and C. Never call while
    /// [`Vt::sync_pending`].
    pub fn grant(&mut self, presentation: &mut Presentation) -> (Vec<u8>, Restoration) {
        presentation.mode = Mode::of(presentation.cols, presentation.rows, self.geometry);
        let direct = presentation.mode == Mode::Direct;
        let rows = self
            .engine
            .normal(Dimensions::history_size)
            .min(self.ledger.max_rows);
        let notice = self.ledger.evicted && rows > 0;
        let suffix = self.suffix(direct);
        let restoration = Restoration::new(
            &self.ledger,
            rows,
            notice,
            presentation,
            suffix,
            self.engine.alternate(),
        );
        (lead(presentation, notice), restoration)
    }

    /// The next restoration bytes: whole history rows up to about `budget`
    /// bytes, then the screen suffix. Commit every chunk in order.
    pub fn produce(
        &mut self,
        restoration: &mut Restoration,
        presentation: &mut Presentation,
        budget: usize,
    ) -> Chunk {
        let ledger = self.ledger;
        let mut bytes = Vec::new();
        let ended = self
            .engine
            .normal(|grid| restoration.history(grid, &ledger, budget, &mut bytes));
        let done = restoration.finish(ended, presentation, &mut bytes);
        Chunk { bytes, done }
    }

    /// Feed PTY output through the live output policy. Every completed
    /// token updates the engine; a partial token waits for a later read.
    pub fn feed(&mut self, bytes: &[u8], now: Instant) -> Fed {
        let mut fed = Fed::default();
        for token in self.tokenizer.push(bytes) {
            self.token(&token, now, &mut fed);
        }
        fed
    }

    /// True while a synchronized update is open and before its deadline;
    /// snapshots and derived updates wait for it.
    pub fn sync_pending(&self, now: Instant) -> bool {
        self.latch.pending(now)
    }

    /// Close a synchronized update whose deadline passed. True when this
    /// call closed it, so the daemon ends any update open on a host.
    pub fn tick(&mut self, now: Instant) -> bool {
        self.latch.tick(now)
    }

    /// Whether the application enabled bracketed paste (mode 2004).
    pub fn bracketed_paste(&self) -> bool {
        self.engine.mode().contains(TermMode::BRACKETED_PASTE)
    }

    fn token(&mut self, token: &Token, now: Instant, fed: &mut Fed) {
        fed.dropped += u32::from(token.kind == Kind::Oversized);
        let policy = classify(token);
        let title = self.engine.title().map(str::to_owned);
        self.execute(&token.bytes, policy.clear, &mut fed.replies);
        self.latch.observe(policy.sync, now);
        fed.live.extend_from_slice(live(token, &policy.live));
        if self.engine.title() != title.as_deref() {
            fed.live.extend(self.title());
        }
    }

    fn execute(&mut self, bytes: &[u8], clear: Clear, replies: &mut Vec<u8>) {
        for step in bytes.chunks(STEP) {
            self.step(step, clear, replies);
        }
        if clear == Clear::Reset {
            self.engine.reset_title();
        }
    }

    /// One engine step with history accounting. Before a step on the normal
    /// screen, history beyond retention is trimmed so the step's pushes fit
    /// in the headroom.
    fn step(&mut self, bytes: &[u8], clear: Clear, replies: &mut Vec<u8>) {
        let before = self.trim();
        self.engine.advance(bytes, replies);
        let cleared = clear == Clear::Reset || (clear == Clear::History && before.is_some());
        self.ledger
            .record(before, self.engine.normal_history(), cleared);
    }

    fn trim(&mut self) -> Option<usize> {
        if self.engine.normal_history().is_some() {
            self.engine.set_history(self.ledger.max_rows);
            self.engine.set_history(self.ledger.capacity());
        }
        self.engine.normal_history()
    }

    /// Phases C2 to C6, captured at the snapshot point.
    fn suffix(&mut self, direct: bool) -> Vec<u8> {
        let mut out = Vec::new();
        self.engine.normal(|grid| {
            screen(grid, &mut out);
            cursors(grid, direct, &mut out);
        });
        if self.engine.alternate() {
            out.extend_from_slice(b"\x1b[?1049h");
            screen(self.engine.active(), &mut out);
            cursors(self.engine.active(), direct, &mut out);
        }
        self.modes(direct, &mut out);
        out.extend(self.title());
        out
    }

    /// C5: every input and display mode, plus the Direct-only modes, which
    /// may move the cursor (origin), so it is re-established after them.
    fn modes(&self, direct: bool, out: &mut Vec<u8>) {
        let mode = self.engine.mode();
        transitions(mode, SHARED, out);
        out.extend(cursor_style(self.engine.cursor_style()));
        if direct {
            transitions(mode, DIRECT, out);
        }
        if direct && mode.contains(TermMode::ORIGIN) {
            cursor(self.engine.active(), direct, out);
        }
    }

    fn title(&self) -> Vec<u8> {
        title::osc(self.engine.title().unwrap_or(&self.default_title))
    }
}

fn live<'a>(token: &'a Token, live: &'a Live) -> &'a [u8] {
    match live {
        Live::Forward => &token.bytes,
        Live::Strip => &[],
        Live::Replace(bytes) => bytes,
    }
}
