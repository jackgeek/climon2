//! One allocation-bounded engine instance of a monitor model: the unchanged
//! Alacritty engine, never asked to grow past what its model admitted.

use crate::{
    classify::{Clear, classify},
    cost,
    engine::Core,
    geometry::{Geometry, STEP},
    model::Rejected,
    render::cells,
    token::{Kind, Token, Tokenizer},
    view::{UNSET, View},
};
use alacritty_terminal::{Grid, event::VoidListener, grid::Dimensions, term::cell::Cell};

pub(crate) struct Bounded {
    core: Core<VoidListener>,
    tokenizer: Tokenizer,
    geometry: Geometry,
    history: History,
    extras: Extras,
}

/// The normal screen's history: the rows the model shows, and the engine's
/// admitted limit in whole chunks. The engine adds storage a chunk at a
/// time, so it never holds more rows than the limit and, once history is
/// full, no row beyond it.
#[derive(Clone, Copy)]
struct History {
    /// The newest rows the model shows at most (`Limits::history_rows`).
    shown: usize,
    /// The engine's history limit, in chunks of [`cost::CHUNK`] rows.
    chunks: usize,
    /// The most chunks the engine has held: its row vector keeps the
    /// capacity they took.
    peak: usize,
    /// Rows have been discarded, or may have been, since the history was
    /// last cleared.
    truncated: bool,
}

impl History {
    /// The first chunk, which holds the protected newest page; none when the
    /// model keeps no history.
    fn new(shown: usize) -> Self {
        let first = shown.min(1);
        Self {
            shown,
            chunks: first,
            peak: first,
            truncated: false,
        }
    }

    /// Whether this history is truncated after a token that left `held`
    /// rows, having first cleared the history if `cleared`: a cleared history
    /// has discarded nothing, and rows beyond those shown are discarded.
    /// A full history discards its oldest row for each row pushed into it
    /// without the engine reporting it, so a token that could push more rows
    /// than the admitted chunks had room for (`overrun`) and left them full
    /// may have discarded rows.
    fn truncated_after(self, held: usize, cleared: bool, overrun: bool) -> bool {
        (self.truncated && !cleared) || self.lost(held, overrun)
    }

    /// Rows the engine may have discarded from a history that holds `held`.
    fn lost(self, held: usize, overrun: bool) -> bool {
        held > self.shown || (overrun && held == self.rows())
    }

    /// The engine's history limit in rows.
    fn rows(self) -> usize {
        self.chunks * cost::CHUNK
    }

    /// The chunks it takes to hold `rows` history rows, but no more than the
    /// model shows.
    fn wanted(self, rows: usize) -> usize {
        rows.div_ceil(cost::CHUNK)
            .min(self.shown.div_ceil(cost::CHUNK))
    }

    /// One chunk more.
    fn next(self) -> Self {
        let chunks = self.chunks + 1;
        Self {
            chunks,
            peak: self.peak.max(chunks),
            ..self
        }
    }

    /// One chunk less, unless only the first is left.
    fn shorter(self) -> Option<Self> {
        let first = Self::new(self.shown).chunks;
        (self.chunks > first).then(|| Self {
            chunks: self.chunks - 1,
            ..self
        })
    }

    fn cost(self, geometry: Geometry) -> usize {
        cost::history(geometry, self.chunks, self.peak)
    }
}

/// One of the engine's two screens.
#[derive(Clone, Copy)]
enum Screen {
    Normal,
    Alternate,
}

/// Upper bounds on the cell extra data each screen holds. The inactive
/// screen's bound stays as it was while the engine cannot change that
/// screen.
#[derive(Default)]
struct Extras {
    normal: usize,
    alternate: usize,
}

impl Extras {
    fn of(&mut self, screen: Screen) -> &mut usize {
        match screen {
            Screen::Normal => &mut self.normal,
            Screen::Alternate => &mut self.alternate,
        }
    }

    fn total(&self) -> usize {
        self.normal + self.alternate
    }
}

/// Room for an instance: the model's limit, shared with the current model
/// while a restoration is staged beside it.
pub(crate) struct Room<'a> {
    beside: Option<&'a mut Bounded>,
    limit: usize,
}

impl<'a> Room<'a> {
    /// The whole limit, for the current model.
    pub(crate) fn alone(limit: usize) -> Self {
        Self {
            beside: None,
            limit,
        }
    }

    /// The limit shared with the current model, for a staged restoration.
    pub(crate) fn beside(current: &'a mut Bounded, limit: usize) -> Self {
        Self {
            beside: Some(current),
            limit,
        }
    }

    /// Rejected unless `bytes` fit beside what the current model holds now.
    fn check(&self, bytes: usize) -> Result<(), Rejected> {
        let needed = self.beside.as_ref().map_or(0, |current| current.charge()) + bytes;
        if needed > self.limit {
            return Err(Rejected {
                needed,
                limit: self.limit,
            });
        }
        Ok(())
    }

    fn fits(&self, bytes: usize) -> bool {
        self.check(bytes).is_ok()
    }

    /// Rejected unless `bytes` fit, once the current model beside has
    /// discarded its oldest history, a chunk at a time, as far as that
    /// helps: applying the restoration replaces it whole.
    pub(crate) fn fit(&mut self, bytes: usize) -> Result<(), Rejected> {
        while !self.fits(bytes) && self.beside.as_deref_mut().is_some_and(Bounded::shorten) {}
        self.check(bytes)
    }

    /// A blank instance at `geometry`, if what it holds before any output
    /// fits.
    pub(crate) fn blank(
        &mut self,
        geometry: Geometry,
        history_rows: usize,
    ) -> Result<Bounded, Rejected> {
        self.fit(Bounded::mandatory(geometry, history_rows))?;
        Ok(Bounded::new(geometry, history_rows))
    }
}

impl Bounded {
    /// What a fresh instance holds and may grow to before any output: the
    /// engine, both screens and the first chunk of history, which holds the
    /// protected newest page.
    pub(crate) fn mandatory(geometry: Geometry, history_rows: usize) -> usize {
        cost::fixed(geometry) + History::new(history_rows).cost(geometry)
    }

    pub(crate) fn new(geometry: Geometry, history_rows: usize) -> Self {
        let history = History::new(history_rows);
        Self {
            core: Core::new(geometry, history.rows(), VoidListener, UNSET),
            tokenizer: Tokenizer::new(),
            geometry,
            history,
            extras: Extras::default(),
        }
    }

    /// The bytes this instance may hold.
    pub(crate) fn charge(&self) -> usize {
        self.charge_with(self.history)
    }

    /// The bytes this instance may hold with `history`.
    fn charge_with(&self, history: History) -> usize {
        cost::fixed(self.geometry) + history.cost(self.geometry) + self.extras.total()
    }

    /// Feed terminal bytes, a slice of at most one engine step at a time so
    /// the front filter's buffers stay within the reserve. Each token is
    /// admitted before the engine sees it; the first that does not fit is
    /// rejected, and so is the rest.
    pub(crate) fn feed(&mut self, bytes: &[u8], room: &mut Room) -> Result<(), Rejected> {
        for slice in bytes.chunks(STEP) {
            for token in self.tokenizer.push(slice) {
                self.token(token, room)?;
            }
        }
        Ok(())
    }

    /// Only what the live output policy forwards to a Direct controller
    /// reaches the engine. The daemon already applied that policy, so a
    /// stripped token can only be malformed input; only the controls it
    /// executes remain. Growth and history are admitted from the policy's own
    /// parse of the token.
    fn token(&mut self, token: Token, room: &mut Room) -> Result<(), Rejected> {
        let policy = classify(&token);
        let bytes = policy.forwarded(&token);
        self.admit(self.growth(token.kind, bytes, policy.repeats), room)?;
        let overrun = self.grow(bytes.len() + policy.repeats, room);
        self.core.advance(bytes);
        self.settle(policy.clear, overrun);
        Ok(())
    }

    /// Charge `growth` if it fits. When the running charge leaves no room,
    /// the active screen is measured first; then history yields, oldest
    /// chunk first: the current model's beside a staged restoration, then
    /// this instance's own.
    fn admit(&mut self, growth: usize, room: &mut Room) -> Result<(), Rejected> {
        if !room.fits(self.charge() + growth) {
            self.measure();
        }
        while room.fit(self.charge() + growth).is_err() && self.shorten() {}
        room.fit(self.charge() + growth)?;
        *self.extras.of(self.active()) += growth;
        Ok(())
    }

    /// Before a token that could fill the history limit, admit each further
    /// chunk of rows it could fill while the next one fits, beside a staged
    /// restoration once the current model's history has yielded; beyond
    /// them the engine discards its oldest row for each new one. A token
    /// pushes at most a row per byte and per repetition, or a screenful, so
    /// one long repeat or released run of controls can fill several chunks.
    /// True when the token could push more rows than the admitted chunks
    /// then hold, because `history_rows` capped them or the bound refused
    /// one; the alternate screen pushes none.
    fn grow(&mut self, reach: usize, room: &mut Room) -> bool {
        let Some(history) = self.core.normal_history() else {
            return false;
        };
        let reached = history + reach.max(usize::from(self.geometry.rows()));
        let wanted = self.history.wanted(reached);
        let mut grown = self.history;
        while grown.chunks < wanted && room.fit(self.charge_with(grown.next())).is_ok() {
            grown = grown.next();
        }
        if grown.chunks != self.history.chunks {
            self.history = grown;
            self.core.set_history(grown.rows());
        }
        reached > grown.rows()
    }

    /// Discard the oldest history chunk, on either screen: the engine drops
    /// the rows beyond the lower limit, and their cell data, at once, and
    /// what the normal screen still holds is measured. False when only the
    /// first chunk, which holds the protected newest page, is left.
    pub(crate) fn shorten(&mut self) -> bool {
        let Some(history) = self.history.shorter() else {
            return false;
        };
        let (normal, dropped) = self.core.lower_history(history.rows(), held);
        *self.extras.of(Screen::Normal) = normal;
        self.history = history;
        self.history.truncated |= dropped;
        true
    }

    /// After a token clears history the engine keeps its storage rows, cell
    /// data and all, beyond the history it shows; drop them at once, so
    /// measuring sees every row that holds cell data. Then recount whether
    /// the normal screen's history is truncated, while it is the active
    /// screen, after a token that may have `overrun` its admitted chunks.
    fn settle(&mut self, clear: Clear, overrun: bool) {
        let cleared = clear != Clear::Nothing;
        if cleared {
            self.core.truncate();
        }
        if let Some(held) = self.core.normal_history() {
            self.history.truncated = self.history.truncated_after(held, cleared, overrun);
        }
    }

    /// Replace the active screen's bound with what its cells hold now.
    pub(crate) fn measure(&mut self) {
        *self.extras.of(self.active()) = held(self.core.active());
    }

    /// The most one token can make the engine allocate: its printed text,
    /// or the characters its sequence repeats.
    fn growth(&self, kind: Kind, bytes: &[u8], repeats: usize) -> usize {
        let tinted = self.core.active().cursor.template.extra.is_some();
        if kind == Kind::Text {
            cost::text(bytes, tinted)
        } else {
            cost::repeat(repeats, tinted)
        }
    }

    fn active(&self) -> Screen {
        if self.core.alternate() {
            Screen::Alternate
        } else {
            Screen::Normal
        }
    }

    pub(crate) fn geometry(&self) -> Geometry {
        self.geometry
    }

    /// How many of the normal screen's newest history rows the model shows:
    /// none while the alternate screen is active.
    fn shown(&self) -> usize {
        self.core
            .normal_history()
            .unwrap_or(0)
            .min(self.history.shown)
    }

    /// What this instance shows, for painting.
    pub(crate) fn view(&self) -> View<'_> {
        View::new(&self.core, self.shown(), self.history.truncated)
    }
}

/// The bound for the extra data a grid's cells hold, history included.
fn held(grid: &Grid<Cell>) -> usize {
    let lines = grid.topmost_line().0..=grid.bottommost_line().0;
    lines
        .flat_map(|line| cells(grid, line))
        .map(cost::cell)
        .sum()
}
