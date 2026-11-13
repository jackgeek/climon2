//! Read-only views of a monitor model's current terminal, for painting
//! (#83): its active screen's cells, its cursor, the modes the monitor's
//! input router and painter honour, and the normal screen's history. They
//! borrow the model, so it cannot change while a frame reads them, and they
//! name no engine type.

mod cell;
mod cursor;
mod modes;

pub use cell::{Ansi, Cell, Colour, Content, Glyph, Rendition, Underline};
pub use cursor::{Cursor, CursorShape};
pub use modes::{Modes, MouseEncoding, MouseReporting};

use crate::{engine::Core, render::cells};
use alacritty_terminal::{
    Grid, event::VoidListener, grid::Dimensions, term::cell::Cell as EngineCell,
};

/// What a model shows, borrowed for one frame.
#[derive(Clone, Copy)]
pub struct View<'a> {
    core: &'a Core<VoidListener>,
    /// How many of the normal screen's history rows are shown.
    history: usize,
    truncated: bool,
}

impl<'a> View<'a> {
    pub(crate) fn new(core: &'a Core<VoidListener>, history: usize, truncated: bool) -> Self {
        Self {
            core,
            history,
            truncated,
        }
    }

    /// The active screen, top row first; the normal screen comes with its
    /// history.
    pub fn screen(&self) -> Screen<'a> {
        let grid = self.core.active();
        let rows = Rows {
            grid,
            top: 0,
            len: grid.screen_lines(),
        };
        if self.core.alternate() {
            return Screen::Alternate { rows };
        }
        let history = Rows {
            grid,
            top: -(self.history as i32),
            len: self.history,
        };
        Screen::Normal {
            rows,
            history: History {
                rows: history,
                truncated: self.truncated,
            },
        }
    }

    /// The cursor on the active screen.
    pub fn cursor(&self) -> Cursor {
        Cursor::of(
            self.core.active(),
            self.core.mode(),
            self.core.cursor_style(),
        )
    }

    /// The modes the monitor's input router and painter honour.
    pub fn modes(&self) -> Modes {
        Modes::of(self.core.mode())
    }
}

/// The screen the application draws on.
#[derive(Clone, Copy)]
pub enum Screen<'a> {
    /// The normal screen, below the history scrolled off its top.
    Normal {
        rows: Rows<'a>,
        history: History<'a>,
    },
    /// The alternate screen (`CSI ? 1049 h`), which keeps no history; the
    /// normal screen returns with `CSI ? 1049 l`.
    Alternate { rows: Rows<'a> },
}

/// The normal screen's history: the newest rows scrolled off its top, at
/// most `Limits::history_rows`.
#[derive(Clone, Copy)]
pub struct History<'a> {
    /// Oldest first.
    pub rows: Rows<'a>,
    /// Older rows were discarded since the history was last cleared
    /// (`CSI 3 J` or `ESC c`): beyond `Limits::history_rows`, because the
    /// allocation bound refused the history room or took it back, or by
    /// `Model::shorten_history`. The engine discards the oldest row of a full
    /// history, one holding all the rows its admitted chunks hold when that
    /// is no more than `history_rows`, for each row pushed into it without
    /// reporting it, so output that leaves the history full reports it
    /// truncated even if no row was pushed out.
    pub truncated: bool,
}

/// Rows of cells, in order.
#[derive(Clone, Copy)]
pub struct Rows<'a> {
    grid: &'a Grid<EngineCell>,
    /// The first row's line in the engine's grid.
    top: i32,
    len: usize,
}

impl<'a> Rows<'a> {
    /// How many rows there are.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when there is no row.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The row at `index`, if there is one.
    pub fn get(&self, index: usize) -> Option<Row<'a>> {
        (index < self.len).then(|| self.row(index))
    }

    /// Each row in order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = Row<'a>> + DoubleEndedIterator + use<'a> {
        let rows = *self;
        (0..self.len).map(move |index| rows.row(index))
    }

    fn row(self, index: usize) -> Row<'a> {
        Row {
            cells: cells(self.grid, self.top + index as i32),
        }
    }
}

/// One row's cells, left to right.
#[derive(Clone, Copy)]
pub struct Row<'a> {
    cells: &'a [EngineCell],
}

impl<'a> Row<'a> {
    /// Each cell, left to right, one per column.
    pub fn cells(&self) -> impl ExactSizeIterator<Item = Cell<'a>> + DoubleEndedIterator + use<'a> {
        self.cells.iter().map(Cell::of)
    }
}
