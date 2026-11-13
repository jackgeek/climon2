//! Read-only views of a monitor model's current terminal, for painting
//! (#83): its active screen's cells, its cursor and the modes the monitor's
//! input router and painter honour. They borrow the model, so it cannot
//! change while a frame reads them, and they name no engine type.

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
}

impl<'a> View<'a> {
    pub(crate) fn new(core: &'a Core<VoidListener>) -> Self {
        Self { core }
    }

    /// The active screen, top row first.
    pub fn screen(&self) -> Screen<'a> {
        let grid = self.core.active();
        let rows = Rows {
            grid,
            top: 0,
            len: grid.screen_lines(),
        };
        if self.core.alternate() {
            Screen::Alternate { rows }
        } else {
            Screen::Normal { rows }
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
    /// The normal screen.
    Normal { rows: Rows<'a> },
    /// The alternate screen (`CSI ? 1049 h`), which keeps no history; the
    /// normal screen returns with `CSI ? 1049 l`.
    Alternate { rows: Rows<'a> },
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
