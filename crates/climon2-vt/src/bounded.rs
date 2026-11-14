//! One allocation-bounded engine instance of a monitor model: the unchanged
//! Alacritty engine, never asked to grow past what its model admitted.

use crate::{
    classify::{Clear, Policy, Scrolling, classify},
    cost,
    engine::{Core, Parser},
    geometry::{Geometry, STEP},
    model::Rejected,
    printed,
    render::cells,
    scroll::{Cursor, Region},
    token::{Kind, Token, Tokenizer, incomplete_utf8},
};
use alacritty_terminal::{
    Grid,
    event::VoidListener,
    grid::Dimensions,
    term::cell::{Cell, Flags},
};
use std::{iter::once, slice};

/// What vte prints for a cut-off UTF-8 sequence, encoded.
const REPLACEMENT: &[u8] = "\u{fffd}".as_bytes();

const SPACERS: Flags = Flags::WIDE_CHAR_SPACER.union(Flags::LEADING_WIDE_CHAR_SPACER);

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
}

impl History {
    /// No chunk yet: the first, which holds the protected newest page, is
    /// admitted when the normal screen first scrolls.
    fn new(shown: usize) -> Self {
        Self {
            shown,
            chunks: 0,
            peak: 0,
        }
    }

    /// Whether the first chunk is still to be admitted.
    fn awaited(self) -> bool {
        self.chunks == 0 && self.shown > 0
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
        self.fit(Bounded::mandatory(geometry))?;
        Ok(Bounded::new(geometry, history_rows))
    }
}

impl Bounded {
    /// What a fresh instance holds and may grow to before any output: the
    /// engine and both screens. History waits for the first scroll.
    pub(crate) fn mandatory(geometry: Geometry) -> usize {
        cost::fixed(geometry)
    }

    pub(crate) fn new(geometry: Geometry, history_rows: usize) -> Self {
        let history = History::new(history_rows);
        Self {
            core: Core::new(geometry, history.rows(), VoidListener),
            tokenizer: Tokenizer::new(),
            geometry,
            history,
            extras: Extras::default(),
            preceding: None,
            region: Region::full(usize::from(geometry.rows())),
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
        let page = if first { self.first_chunk() } else { 0 };
        self.admit(self.growth(kind, bytes, policy.repeats), page, room)?;
        if first {
            self.history = self.history.next();
            self.core.set_history(self.history.rows());
        }
        self.grow(bytes.len() + policy.repeats, room);
        self.apply(parser, kind, bytes, policy.repeats);
        self.settle(policy);
        Ok(())
    }

    /// Whether the first history chunk waits for the normal screen to
    /// scroll: the alternate screen never pushes history.
    fn awaiting(&self) -> bool {
        self.history.awaited() && !self.core.alternate()
    }

    /// What admitting the first history chunk adds to the charge.
    fn first_chunk(&self) -> usize {
        self.charge_with(self.history.next()) - self.charge()
    }

    fn could_scroll(&self, kind: Kind, bytes: &[u8], policy: &Policy) -> bool {
        let cursor = self.cursor();
        if kind == Kind::Text {
            let cols = usize::from(self.geometry.cols());
            return self.region.text_scrolls(bytes, cursor, cols);
        }
        let feeds = bytes
            .iter()
            .filter(|byte| (0x0a..=0x0c).contains(*byte))
            .count();
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
            .filter(|byte| byte.is_ascii_control() && **byte != 0x1b)
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

    /// Charge `growth`, and `page` for the first history chunk, if they
    /// fit. When the running charge leaves no room, the active screen is
    /// measured first; then history yields, oldest chunk first: the current
    /// model's beside a staged restoration, then this instance's own.
    fn admit(&mut self, growth: usize, page: usize, room: &mut Room) -> Result<(), Rejected> {
        let needed = growth + page;
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
    fn grow(&mut self, reach: usize, room: &mut Room) {
        let Some(history) = self
            .core
            .normal_history()
            .filter(|_| self.history.chunks > 0)
        else {
            return;
        };
        let wanted = self
            .history
            .wanted(history + reach.max(usize::from(self.geometry.rows())));
        let mut grown = self.history;
        while grown.chunks < wanted && room.fit(self.charge_with(grown.next())).is_ok() {
            grown = grown.next();
        }
        if grown.chunks != self.history.chunks {
            self.history = grown;
            self.core.set_history(grown.rows());
        }
    }

    /// Discard the oldest history chunk, on either screen: the engine drops
    /// the rows beyond the lower limit, and their cell data, at once, and
    /// what the normal screen still holds is measured. False when only the
    /// first chunk, which holds the protected newest page, is left.
    pub(crate) fn shorten(&mut self) -> bool {
        let Some(history) = self.history.shorter() else {
            return false;
        };
        self.history = history;
        *self.extras.of(Screen::Normal) = self.core.lower_history(history.rows(), held);
        true
    }

    /// Follow the scroll region the token set, or reset. After a token
    /// clears history the engine keeps its storage rows, cell data and all,
    /// beyond the history it shows; drop them at once, so measuring sees
    /// every row that holds cell data.
    fn settle(&mut self, policy: &Policy) {
        let rows = usize::from(self.geometry.rows());
        if let Scrolling::Region { top, bottom } = policy.scrolling {
            self.region = self.region.set(top, bottom, rows);
        }
        if policy.clear == Clear::Reset {
            self.region = Region::full(rows);
        }
        if policy.clear != Clear::Nothing {
            self.core.truncate();
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

    /// The normal screen's newest history rows the model shows, as text,
    /// oldest first; empty while the alternate screen is active.
    pub(crate) fn history(&self) -> Vec<String> {
        let grid = self.core.active();
        let shown = self
            .core
            .normal_history()
            .unwrap_or(0)
            .min(self.history.shown);
        (1..=shown)
            .rev()
            .map(|age| text(cells(grid, -(age as i32))))
            .collect()
    }

    /// The active screen's rows as text.
    pub(crate) fn screen(&self) -> Vec<String> {
        let grid = self.core.active();
        (0..self.geometry.rows())
            .map(|line| text(cells(grid, i32::from(line))))
            .collect()
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

/// A row's characters with each one's combining marks; spacers are skipped
/// and trailing blanks trimmed.
fn text(cells: &[Cell]) -> String {
    let text: String = cells
        .iter()
        .filter(|cell| !cell.flags.intersects(SPACERS))
        .flat_map(|cell| once(cell.c).chain(cell.zerowidth().unwrap_or_default().iter().copied()))
        .collect();
    text.trim_end().to_owned()
}
