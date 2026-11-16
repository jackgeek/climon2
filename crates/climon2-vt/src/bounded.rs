//! One allocation-bounded engine instance of a monitor model: the unchanged
//! Alacritty engine, never asked to grow past what its model admitted.

use crate::{
    classify::{Clear, Policy, Scrolling, classify},
    cost,
    engine::{Core, Parser},
    geometry::{Geometry, STEP},
    model::{Mandatory, Rejected},
    printed,
    render::cells,
    scroll::{Cursor, Region},
    token::{Kind, Token, Tokenizer, executed, incomplete_utf8, line_feed},
    view::{UNSET, View},
};
use alacritty_terminal::{Grid, event::VoidListener, grid::Dimensions, term::cell::Cell};
use std::slice;

/// What vte prints for a cut-off UTF-8 sequence, encoded.
const REPLACEMENT: &[u8] = "\u{fffd}".as_bytes();

pub(crate) struct Bounded {
    core: Core<VoidListener>,
    tokenizer: Tokenizer,
    geometry: Geometry,
    history: History,
    extras: Extras,
    /// The character vte printed last for this instance, which `CSI n b`
    /// repeats: the parser is shared, so its own is another model's.
    preceding: Option<char>,
    /// The engine's scroll region, which decides where line feeds can
    /// first push rows into history.
    region: Region,
    /// How far the normal screen's view is scrolled back into history,
    /// one past the oldest row shown for the truncation notice above it.
    scrolled: usize,
    /// The application's synchronized update is open: what the screen
    /// shows may be half drawn.
    synchronizing: bool,
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
    /// No chunk yet: the first, which holds the protected newest page, is
    /// admitted when the normal screen first scrolls.
    fn new(shown: usize) -> Self {
        Self {
            shown,
            chunks: 0,
            peak: 0,
            truncated: false,
        }
    }

    /// Whether the first chunk is still to be admitted.
    fn awaited(self) -> bool {
        !self.begun() && self.shown > 0
    }

    /// Whether the first chunk has been admitted: only then can further
    /// chunks be.
    fn begun(self) -> bool {
        self.chunks > 0
    }

    /// What admitting the first chunk adds to a blank history's cost: none
    /// for a model that keeps no history.
    fn first_chunk(self, geometry: Geometry) -> usize {
        if self.shown == 0 {
            return 0;
        }
        Self::new(self.shown).next().cost(geometry)
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

    /// One chunk less, unless only the first is left, or none.
    fn shorter(self) -> Option<Self> {
        (self.chunks > 1).then(|| Self {
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
        self.fit(Bounded::mandatory(geometry, history_rows).current_model)?;
        Ok(Bounded::new(geometry, history_rows))
    }
}

impl Bounded {
    /// What a fresh instance needs, by part: the engine and both screens,
    /// which it holds before any output, and the first chunk of history,
    /// which holds the protected newest page and is admitted when the normal
    /// screen first scrolls.
    pub(crate) fn mandatory(geometry: Geometry, history_rows: usize) -> Mandatory {
        Mandatory {
            current_model: cost::fixed(geometry),
            recent_page: History::new(history_rows).first_chunk(geometry),
        }
    }

    pub(crate) fn new(geometry: Geometry, history_rows: usize) -> Self {
        let history = History::new(history_rows);
        Self {
            core: Core::new(geometry, history.rows(), VoidListener, UNSET),
            tokenizer: Tokenizer::new(),
            geometry,
            history,
            extras: Extras::default(),
            preceding: None,
            region: Region::full(usize::from(geometry.rows())),
            scrolled: 0,
            synchronizing: false,
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
    pub(crate) fn feed(
        &mut self,
        parser: &mut Parser,
        bytes: &[u8],
        room: &mut Room,
    ) -> Result<(), Rejected> {
        for slice in bytes.chunks(STEP) {
            for token in self.tokenizer.push(slice) {
                self.token(parser, token, room)?;
            }
        }
        Ok(())
    }

    /// Only what the live output policy forwards to a Direct controller
    /// reaches the engine. The daemon already applied that policy, so a
    /// stripped token can only be malformed input; only the controls it
    /// executes remain. Text is fed in pieces while the first history chunk
    /// waits for a scroll, so that only a piece that could scroll admits it.
    fn token(
        &mut self,
        parser: &mut Parser,
        token: Token,
        room: &mut Room,
    ) -> Result<(), Rejected> {
        let policy = classify(&token);
        let mut rest = policy.forwarded(&token);
        if token.kind != Kind::Text {
            return self.step(parser, token.kind, rest, &policy, room);
        }
        while !rest.is_empty() {
            let (piece, tail) = rest.split_at(self.piece(rest));
            self.step(parser, Kind::Text, piece, &policy, room)?;
            rest = tail;
        }
        Ok(())
    }

    /// The bytes of `text` to feed as one step.
    fn piece(&self, text: &[u8]) -> usize {
        if self.awaiting() {
            self.region.piece(text, self.cursor().line)
        } else {
            text.len()
        }
    }

    /// Growth and history are admitted from the policy's own parse of the
    /// token: the first history chunk too, if the token could scroll, and
    /// the token is rejected if that chunk cannot fit.
    fn step(
        &mut self,
        parser: &mut Parser,
        kind: Kind,
        bytes: &[u8],
        policy: &Policy,
        room: &mut Room,
    ) -> Result<(), Rejected> {
        let first = self.awaiting() && self.could_scroll(kind, bytes, policy);
        let chunk = if first { self.first_chunk() } else { 0 };
        self.admit(self.growth(kind, bytes, policy.repeats), chunk, room)?;
        if first {
            self.history = self.history.next();
            self.core.set_history(self.history.rows());
        }
        let overrun = self.grow(bytes.len() + policy.repeats, room);
        let pushed = self.counted(parser, kind, bytes, policy.repeats);
        self.settle(policy, overrun);
        self.anchor(pushed);
        self.synchronize(policy);
        Ok(())
    }

    /// A synchronized update (`CSI ? 2026 h`) opens until its end
    /// (`CSI ? 2026 l`) or a full reset; another begin changes nothing.
    fn synchronize(&mut self, policy: &Policy) {
        let open = policy.sync.unwrap_or(self.synchronizing);
        self.synchronizing = open && policy.clear != Clear::Reset;
    }

    /// [`Bounded::apply`], returning the rows it pushed into the normal
    /// screen's history while its view is scrolled back, which alone needs
    /// them; otherwise none are counted.
    fn counted(&mut self, parser: &mut Parser, kind: Kind, bytes: &[u8], repeats: usize) -> usize {
        let counting = self.scrolled > 0 && self.region.at_top() && !self.core.alternate();
        if !counting {
            self.apply(parser, kind, bytes, repeats);
            return 0;
        }
        self.core.count_pushes();
        self.apply(parser, kind, bytes, repeats);
        self.core.pushed()
    }

    /// Keep a scrolled view on the rows it shows after `pushed` more rows
    /// entered the normal screen's history: once they are discarded, at the
    /// notice above the oldest row retained (none once history is cleared).
    /// Read only while the normal screen is active, which alone pushes,
    /// shortens or clears its history while it is shown.
    fn anchor(&mut self, pushed: usize) {
        if let Some(held) = self.core.normal_history() {
            let shown = held.min(self.history.shown);
            let anchored = self.scrolled + pushed;
            self.scrolled = if anchored > shown {
                self.notice(shown)
            } else {
                anchored
            };
        }
    }

    /// How far back the truncation notice above the oldest of `shown`
    /// history rows is: one past it when the history is truncated, the
    /// oldest row otherwise.
    fn notice(&self, shown: usize) -> usize {
        shown + usize::from(self.history.truncated)
    }

    /// Scroll the normal screen's view `rows` back (forward when
    /// negative), between the live screen and the notice above the oldest
    /// row.
    pub(crate) fn scroll(&mut self, rows: isize) {
        if let Some(held) = self.core.normal_history() {
            let limit = self.notice(held.min(self.history.shown));
            self.scrolled = self.scrolled.saturating_add_signed(rows).min(limit);
        }
    }

    pub(crate) fn to_live(&mut self) {
        self.scrolled = 0;
    }

    /// Whether the first history chunk waits for the normal screen to
    /// scroll: the alternate screen never pushes history.
    fn awaiting(&self) -> bool {
        self.history.awaited() && !self.core.alternate()
    }

    /// What admitting the first history chunk adds to the charge.
    fn first_chunk(&self) -> usize {
        self.history.first_chunk(self.geometry)
    }

    fn could_scroll(&self, kind: Kind, bytes: &[u8], policy: &Policy) -> bool {
        let cursor = self.cursor();
        if kind == Kind::Text {
            let cols = usize::from(self.geometry.cols());
            return self.region.text_scrolls(bytes, cursor, cols);
        }
        let feeds = bytes.iter().filter(|&&byte| line_feed(byte)).count();
        self.region
            .could_scroll(cursor.line, policy.scrolling, feeds + policy.repeats)
    }

    fn cursor(&self) -> Cursor {
        let cursor = &self.core.active().cursor;
        Cursor {
            line: cursor.point.line.0 as usize,
            column: cursor.point.column.0,
        }
    }

    /// Feed an admitted token through the shared parser so that it leaves
    /// nothing there for another model.
    fn apply(&mut self, parser: &mut Parser, kind: Kind, bytes: &[u8], repeats: usize) {
        if repeats > 0 {
            self.repeat(parser, bytes, repeats);
        } else if kind == Kind::Text {
            self.print(parser, bytes);
        } else {
            self.core.advance(parser, bytes);
        }
    }

    /// A repeat runs the controls vte executes in it, then prints this
    /// instance's own preceding character.
    fn repeat(&mut self, parser: &mut Parser, bytes: &[u8], repeats: usize) {
        bytes
            .iter()
            .filter(|&&byte| executed(byte))
            .for_each(|control| self.core.advance(parser, slice::from_ref(control)));
        if let Some(character) = self.preceding {
            self.core.repeat(character, repeats);
        }
    }

    /// Text, remembering what it printed last. A UTF-8 sequence cut off at
    /// its end, by the escape that follows, prints its replacement character
    /// now, as vte prints it when that escape arrives.
    fn print(&mut self, parser: &mut Parser, text: &[u8]) {
        let cut = incomplete_utf8(text);
        self.core.advance(parser, &text[..text.len() - cut]);
        if cut > 0 {
            self.core.advance(parser, REPLACEMENT);
        }
        self.preceding = printed::last(text).or(self.preceding);
    }

    /// Charge `growth`, and `chunk` for the first history chunk, if they
    /// fit. When the running charge leaves no room, the active screen is
    /// measured first; then history yields, oldest chunk first: the current
    /// model's beside a staged restoration, then this instance's own.
    fn admit(&mut self, growth: usize, chunk: usize, room: &mut Room) -> Result<(), Rejected> {
        let needed = growth + chunk;
        if !room.fits(self.charge() + needed) {
            self.measure();
        }
        while room.fit(self.charge() + needed).is_err() && self.shorten() {}
        room.fit(self.charge() + needed)?;
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
    /// one; the alternate screen pushes none, and nor does a token while the
    /// first chunk awaits the first scroll, since one that could scroll
    /// admitted it.
    fn grow(&mut self, reach: usize, room: &mut Room) -> bool {
        let Some(history) = self
            .core
            .normal_history()
            .filter(|_| !self.history.awaited())
        else {
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
        self.anchor(0);
        true
    }

    /// Follow the scroll region the token set, or reset. After a token
    /// clears history the engine keeps its storage rows, cell data and all,
    /// beyond the history it shows; drop them at once, so measuring sees
    /// every row that holds cell data. Then recount whether the normal
    /// screen's history is truncated, while it is the active screen, after a
    /// token that may have `overrun` its admitted chunks. RIS also restarts
    /// the engine's modes as xterm starts.
    fn settle(&mut self, policy: &Policy, overrun: bool) {
        let rows = usize::from(self.geometry.rows());
        if let Scrolling::Region { top, bottom } = policy.scrolling {
            self.region = self.region.set(top, bottom, rows);
        }
        if policy.clear == Clear::Reset {
            self.region = Region::full(rows);
            self.core.reset();
        }
        let cleared = policy.clear != Clear::Nothing;
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

    /// What this instance shows, for painting.
    pub(crate) fn view(&self) -> View<'_> {
        View::new(
            &self.core,
            self.history.shown,
            self.history.truncated,
            self.scrolled,
            self.synchronizing,
        )
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
