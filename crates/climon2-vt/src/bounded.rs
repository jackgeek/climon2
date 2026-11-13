//! One allocation-bounded engine instance of a monitor model: the unchanged
//! Alacritty engine, never asked to grow past what its model admitted.

use crate::{
    classify::{Live, classify},
    cost,
    engine::Core,
    geometry::{Geometry, STEP},
    model::Rejected,
    render::cells,
    token::{Kind, Token, Tokenizer},
};
use alacritty_terminal::{
    event::VoidListener,
    grid::Dimensions,
    term::cell::{Cell, Flags},
};
use std::iter::once;

const SPACERS: Flags = Flags::WIDE_CHAR_SPACER.union(Flags::LEADING_WIDE_CHAR_SPACER);

pub(crate) struct Bounded {
    core: Core<VoidListener>,
    tokenizer: Tokenizer,
    geometry: Geometry,
    history: History,
    /// The normal screen's history rows when last observed.
    seen: usize,
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
}

impl History {
    /// The first chunk, which holds the protected newest page; none when the
    /// model keeps no history.
    fn new(shown: usize) -> Self {
        Self {
            shown,
            chunks: shown.min(1),
        }
    }

    /// The engine's history limit in rows.
    fn rows(self) -> usize {
        self.chunks * cost::CHUNK
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

/// The bytes other parts of the model hold, and the model's limit.
#[derive(Clone, Copy)]
pub(crate) struct Room {
    pub(crate) other: usize,
    pub(crate) limit: usize,
}

impl Bounded {
    /// What a fresh instance holds and may grow to before any output: the
    /// engine, both screens and the first chunk of history, which holds the
    /// protected newest page.
    pub(crate) fn mandatory(geometry: Geometry, history_rows: usize) -> usize {
        cost::fixed(geometry) + cost::history(geometry, History::new(history_rows).chunks)
    }

    pub(crate) fn new(geometry: Geometry, history_rows: usize) -> Self {
        let history = History::new(history_rows);
        Self {
            core: Core::new(geometry, history.rows(), VoidListener),
            seen: 0,
            tokenizer: Tokenizer::new(),
            geometry,
            history,
            extras: Extras::default(),
        }
    }

    /// The bytes this instance may hold.
    pub(crate) fn charge(&self) -> usize {
        cost::fixed(self.geometry)
            + cost::history(self.geometry, self.history.chunks)
            + self.extras.total()
    }

    /// Feed terminal bytes, a slice of at most one engine step at a time so
    /// the front filter's buffers stay within the reserve. Each token is
    /// admitted before the engine sees it; the first that does not fit is
    /// rejected, and so is the rest.
    pub(crate) fn feed(&mut self, bytes: &[u8], room: Room) -> Result<(), Rejected> {
        for slice in bytes.chunks(STEP) {
            for token in self.tokenizer.push(slice) {
                self.token(token, room)?;
            }
        }
        Ok(())
    }

    /// Only what the live output policy forwards to a Direct host reaches
    /// the engine. The daemon already applied that policy, so a stripped
    /// token can only be malformed input; only the controls it executes
    /// remain. Growth and history are admitted from the policy's own parse
    /// of the token.
    fn token(&mut self, token: Token, room: Room) -> Result<(), Rejected> {
        let policy = classify(&token);
        let bytes = match policy.live {
            Live::Forward => token.bytes,
            Live::Replace(bytes) => bytes,
        };
        self.admit(self.growth(token.kind, &bytes, policy.repeats), room)?;
        self.grow(bytes.len() + policy.repeats, room);
        self.core.advance(&bytes);
        self.settle();
        Ok(())
    }

    /// Charge `growth` if it fits, measuring the active screen first when
    /// the running charge alone does not leave room.
    fn admit(&mut self, growth: usize, room: Room) -> Result<(), Rejected> {
        if !self.fits(growth, room) {
            self.measure();
        }
        let needed = self.needed(growth, room);
        if needed > room.limit {
            return Err(Rejected {
                needed,
                limit: room.limit,
            });
        }
        *self.extras.of(self.active()) += growth;
        Ok(())
    }

    /// Before a token that could fill the history limit, admit the engine's
    /// next chunk of rows if it fits; otherwise the engine discards its
    /// oldest row for each new one.
    fn grow(&mut self, reach: usize, room: Room) {
        let Some(next) = self.next_chunks(reach) else {
            return;
        };
        let growth =
            cost::history(self.geometry, next) - cost::history(self.geometry, self.history.chunks);
        if self.fits(growth, room) {
            self.history.chunks = next;
            self.core.set_history(self.history.rows());
        }
    }

    /// The next history limit in chunks, when a token whose bytes and
    /// repetitions add up to `reach` could fill the current one and the
    /// model may show more. A token pushes at most a row per byte and per
    /// repetition, or a screenful.
    fn next_chunks(&self, reach: usize) -> Option<usize> {
        let reach = self.core.normal_history()? + reach.max(usize::from(self.geometry.rows()));
        let History { shown, chunks } = self.history;
        (reach > self.history.rows() && chunks * cost::CHUNK < shown).then_some(chunks + 1)
    }

    /// After history is cleared the engine keeps its storage rows, cell data
    /// and all, beyond the history it shows; drop them, so measuring sees
    /// every row that holds cell data.
    fn settle(&mut self) {
        let Some(now) = self.core.normal_history() else {
            return;
        };
        if now < self.seen {
            self.core.truncate();
        }
        self.seen = now;
    }

    fn fits(&self, growth: usize, room: Room) -> bool {
        self.needed(growth, room) <= room.limit
    }

    fn needed(&self, growth: usize, room: Room) -> usize {
        room.other + self.charge() + growth
    }

    /// Replace the active screen's bound with what its cells hold now.
    pub(crate) fn measure(&mut self) {
        let grid = self.core.active();
        let lines = grid.topmost_line().0..=grid.bottommost_line().0;
        let held = lines
            .flat_map(|line| cells(grid, line))
            .map(cost::cell)
            .sum();
        *self.extras.of(self.active()) = held;
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
