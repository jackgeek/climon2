use crate::{
    classify::{Clear, Live, classify},
    derive::{Cache, derive},
    engine::Engine,
    geometry::{Geometry, STEP},
    history::Ledger,
    modes::{DIRECT, SHARED, cursor_style, transitions},
    presentation::{Mode, Presentation},
    render::{cursor, cursors, screen},
    restore::{Chunk, PROLOGUE, Restoration, Snapshot, lead},
    sync::Latch,
    title,
    token::{Kind, Token, Tokenizer},
};
use alacritty_terminal::{grid::Dimensions, term::TermMode};
use std::{borrow::Cow, time::Instant};

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
    /// Each change of the application's title, in order; `None` is unset.
    pub titles: Vec<Option<String>>,
    /// A token turned focus reporting (mode 1004) on, and it is still on.
    pub focus_enabled: bool,
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
        let restoration = Restoration::new(&self.ledger, rows, notice, presentation, suffix);
        presentation.cache = Cache::current(&self.engine, &self.ledger, &self.title_text());
        (lead(presentation, notice), restoration)
    }

    /// The live update for a Derived controller whose restoration has
    /// ended: rows pushed into history, changed rows, cursor, modes, title
    /// and bell. Empty when nothing changed; never call while
    /// [`Vt::sync_pending`].
    pub fn derive(&mut self, presentation: &mut Presentation) -> Vec<u8> {
        let title = self.title_text();
        derive(&mut self.engine, &self.ledger, &title, presentation)
    }

    /// Apply a ready controller's new host size (after [`Vt::resize`]).
    /// Returns what the host needs before further live output: nothing
    /// between Direct sizes, otherwise a full derived repaint, preceded by
    /// the prologue's baseline when entering Derived presentation and
    /// followed by the Direct-only modes when leaving it. Never call while
    /// [`Vt::sync_pending`].
    pub fn present(&mut self, presentation: &mut Presentation, cols: u16, rows: u16) -> Vec<u8> {
        let from = presentation.mode;
        presentation.cols = cols;
        presentation.rows = rows;
        presentation.mode = Mode::of(cols, rows, self.geometry);
        match (from, presentation.mode) {
            (Mode::Direct, Mode::Direct) => {
                self.commit_live(presentation, b"");
                Vec::new()
            }
            (Mode::Direct, Mode::Derived) => self.enter_derived(presentation),
            (Mode::Derived, Mode::Derived) => self.repaint(presentation),
            (Mode::Derived, Mode::Direct) => self.leave_derived(presentation),
        }
    }

    fn enter_derived(&mut self, presentation: &mut Presentation) -> Vec<u8> {
        presentation.sync_open = false;
        presentation.cache = Cache::current(&self.engine, &self.ledger, &self.title_text());
        [PROLOGUE.to_vec(), self.repaint(presentation)].concat()
    }

    fn repaint(&mut self, presentation: &mut Presentation) -> Vec<u8> {
        presentation.cache.repaint();
        self.derive(presentation)
    }

    fn leave_derived(&mut self, presentation: &mut Presentation) -> Vec<u8> {
        let mut out = self.repaint(presentation);
        self.direct_modes(&mut out);
        out
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
        fed.focus_enabled &= self.focus_reporting();
        fed
    }

    /// Record Direct live bytes committed through the latest [`Vt::feed`],
    /// including the normal-screen cursor to restore when presentation ends.
    pub fn commit_live(&self, presentation: &mut Presentation, bytes: &[u8]) {
        presentation.commit_live(bytes);
        presentation.release_cursor = Some(self.engine.normal_cursor());
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

    /// Whether the application enabled focus reporting (mode 1004).
    pub fn focus_reporting(&self) -> bool {
        self.engine.mode().contains(TermMode::FOCUS_IN_OUT)
    }

    /// The application's own title: `None` while it has set none (or reset
    /// it) and the default title shows.
    pub fn application_title(&self) -> Option<&str> {
        self.engine.title()
    }

    fn token(&mut self, token: &Token, now: Instant, fed: &mut Fed) {
        fed.dropped += u32::from(token.kind == Kind::Oversized);
        let policy = classify(token);
        let title = self.engine.title().map(str::to_owned);
        let reporting = self.focus_reporting();
        self.execute(&token.bytes, policy.clear, &mut fed.replies);
        fed.focus_enabled |= !reporting && self.focus_reporting();
        self.latch.observe(policy.sync, now);
        fed.live
            .extend_from_slice(live(token, &policy.live).as_ref());
        if self.engine.title() != title.as_deref() {
            fed.live.extend(self.title());
            fed.titles.push(self.application_title().map(str::to_owned));
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
    fn suffix(&mut self, direct: bool) -> Snapshot {
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
        Snapshot {
            bytes: out,
            alternate: self.engine.alternate(),
            normal_cursor: self.engine.normal_cursor(),
        }
    }

    /// C5: every input and display mode, plus the Direct-only modes, which
    /// may move the cursor (origin), so it is re-established after them.
    fn modes(&self, direct: bool, out: &mut Vec<u8>) {
        let mode = self.engine.mode();
        transitions(None, mode, SHARED, out);
        out.extend(cursor_style(self.engine.cursor_style()));
        if direct {
            self.direct_modes(out);
        }
    }

    /// Origin, autowrap, insert and new-line modes; setting or resetting
    /// origin may move the cursor, so it is re-established after them.
    fn direct_modes(&self, out: &mut Vec<u8>) {
        let mode = self.engine.mode();
        transitions(None, mode, DIRECT, out);
        cursor(self.engine.active(), true, out);
    }

    fn title(&self) -> Vec<u8> {
        title::osc(&self.title_text())
    }

    fn title_text(&self) -> String {
        self.engine
            .title()
            .unwrap_or(&self.default_title)
            .to_owned()
    }
}

fn live<'a>(token: &'a Token, live: &'a Live) -> Cow<'a, [u8]> {
    match live {
        Live::Forward => Cow::Borrowed(&token.bytes),
        Live::Strip => stripped(token),
        Live::Replace(bytes) => Cow::Borrowed(bytes),
    }
}

/// A stripped escape still performs the C0 controls embedded in it.
fn stripped(token: &Token) -> Cow<'_, [u8]> {
    if !matches!(
        token.kind,
        crate::token::Kind::Escape | crate::token::Kind::Csi
    ) {
        return Cow::Borrowed(&[]);
    }
    Cow::Owned(
        token
            .bytes
            .iter()
            .copied()
            .filter(|byte| (0x00..=0x1f).contains(byte) && *byte != 0x1b)
            .collect(),
    )
}
