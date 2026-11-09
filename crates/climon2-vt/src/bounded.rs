//! One allocation-bounded engine instance of a monitor model: the unchanged
//! Alacritty engine, never asked to grow past what its model admitted.

use crate::{
    classify::{Live, classify},
    cost,
    engine::{Size, Unbuffered, config},
    geometry::{Geometry, STEP},
    model::Rejected,
    render::cells,
    token::{Kind, Token, Tokenizer},
};
use alacritty_terminal::{
    Term,
    event::VoidListener,
    grid::Dimensions,
    term::{
        TermMode,
        cell::{Cell, Flags},
    },
    vte::ansi::Processor,
};
use std::iter::once;

const SPACERS: Flags = Flags::WIDE_CHAR_SPACER.union(Flags::LEADING_WIDE_CHAR_SPACER);

pub(crate) struct Bounded {
    term: Term<VoidListener>,
    processor: Processor<Unbuffered>,
    tokenizer: Tokenizer,
    geometry: Geometry,
    /// The normal screen's admitted history limit.
    limit: usize,
    /// The most history rows the model may keep.
    rows: usize,
    /// The normal screen's history rows when last observed.
    seen: usize,
    /// Upper bounds on the cell extra data each screen holds: normal, then
    /// alternate. The inactive screen's bound stays as it was while the
    /// engine cannot change that screen.
    extras: [usize; 2],
    /// Some growth was charged since the active screen was last measured.
    dirty: bool,
}

/// The bytes other parts of the model hold, and the model's limit.
#[derive(Clone, Copy)]
pub(crate) struct Room {
    pub(crate) other: usize,
    pub(crate) limit: usize,
}

impl Bounded {
    /// What a fresh instance holds and may grow to before any output: the
    /// engine, both screens and the protected newest page of history.
    pub(crate) fn mandatory(geometry: Geometry, history_rows: usize) -> usize {
        cost::fixed(geometry) + cost::history(geometry, page(geometry, history_rows))
    }

    pub(crate) fn new(geometry: Geometry, history_rows: usize) -> Self {
        let limit = page(geometry, history_rows);
        Self {
            term: Term::new(config(limit), &Size(geometry), VoidListener),
            processor: Processor::new(),
            tokenizer: Tokenizer::new(),
            geometry,
            limit,
            rows: history_rows,
            seen: 0,
            extras: [0; 2],
            dirty: false,
        }
    }

    /// The bytes this instance may hold.
    pub(crate) fn charge(&self) -> usize {
        cost::fixed(self.geometry)
            + cost::history(self.geometry, self.limit)
            + self.extras.iter().sum::<usize>()
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
    /// token can only be malformed input and is dropped whole.
    fn token(&mut self, token: Token, room: Room) -> Result<(), Rejected> {
        let bytes = match classify(&token).live {
            Live::Forward => token.bytes,
            Live::Replace(bytes) => bytes,
            Live::Strip => return Ok(()),
        };
        self.admit(self.growth(token.kind, &bytes), room)?;
        self.grow(room);
        self.processor.advance(&mut self.term, &bytes);
        self.settle();
        Ok(())
    }

    /// At full history, admit the engine's next chunk of rows if it fits;
    /// otherwise the engine discards its oldest row for each new one.
    fn grow(&mut self, room: Room) {
        if self.normal_history() != Some(self.limit) || self.limit == self.rows {
            return;
        }
        let next = self.rows.min(self.limit + cost::CHUNK);
        let growth = cost::history(self.geometry, next) - cost::history(self.geometry, self.limit);
        if self.dirty && !self.fits(growth, room) {
            self.measure();
        }
        if self.fits(growth, room) {
            self.limit = next;
            self.term.set_options(config(next));
        }
    }

    /// Drop storage rows the engine keeps beyond its history once history
    /// first fills or is cleared, so no row it can no longer show holds
    /// cell data that measuring would miss.
    fn settle(&mut self) {
        let Some(now) = self.normal_history() else {
            return;
        };
        if now < self.seen || (now == self.limit && self.seen < self.limit) {
            self.term.grid_mut().truncate();
        }
        self.seen = now;
    }

    /// The normal screen's history rows, while it is the active screen.
    fn normal_history(&self) -> Option<usize> {
        (self.active() == 0).then(|| self.term.grid().history_size())
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
        self.extras[self.active()] += growth;
        self.dirty |= growth > 0;
        Ok(())
    }

    fn fits(&self, growth: usize, room: Room) -> bool {
        self.needed(growth, room) <= room.limit
    }

    fn needed(&self, growth: usize, room: Room) -> usize {
        room.other + self.charge() + growth
    }

    /// Replace the active screen's bound with what its cells hold now.
    pub(crate) fn measure(&mut self) {
        let grid = self.term.grid();
        let lines = grid.topmost_line().0..=grid.bottommost_line().0;
        let held = lines
            .flat_map(|line| cells(grid, line))
            .map(cost::cell)
            .sum();
        self.extras[self.active()] = held;
        self.dirty = false;
    }

    /// The most one token can make the engine allocate.
    fn growth(&self, kind: Kind, bytes: &[u8]) -> usize {
        let tinted = self.term.grid().cursor.template.extra.is_some();
        match kind {
            Kind::Text => cost::text(bytes, tinted),
            Kind::Csi => cost::repeat(bytes, tinted),
            _ => 0,
        }
    }

    fn active(&self) -> usize {
        usize::from(self.term.mode().contains(TermMode::ALT_SCREEN))
    }

    pub(crate) fn geometry(&self) -> Geometry {
        self.geometry
    }

    /// The normal screen's history rows as text, oldest first; empty while
    /// the alternate screen is active.
    pub(crate) fn history(&self) -> Vec<String> {
        let grid = self.term.grid();
        let rows = self.normal_history().unwrap_or(0);
        (1..=rows)
            .rev()
            .map(|age| text(cells(grid, -(age as i32))))
            .collect()
    }

    /// The active screen's rows as text.
    pub(crate) fn screen(&self) -> Vec<String> {
        let grid = self.term.grid();
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

/// The protected newest page: the screen's height in rows, or fewer if the
/// history limit is smaller.
fn page(geometry: Geometry, history_rows: usize) -> usize {
    usize::from(geometry.rows()).min(history_rows)
}
