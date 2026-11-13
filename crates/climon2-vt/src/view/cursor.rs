//! The cursor on the active screen: where it is and how it is drawn.

use alacritty_terminal::{
    Grid,
    term::{TermMode, cell::Cell as EngineCell},
    vte::ansi::{CursorShape as EngineShape, CursorStyle},
};

/// The cursor as the application left it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    /// Its row on the active screen, from 0 at the top.
    pub row: u16,
    /// Its column, from 0 at the left.
    pub column: u16,
    /// DECTCEM (`CSI ? 25 h` and `l`).
    pub visible: bool,
    pub shape: CursorShape,
    /// Odd DECSCUSR values and `CSI ? 12 h` blink; even values and
    /// `CSI ? 12 l` hold it steady.
    pub blinking: bool,
}

impl Cursor {
    pub(crate) fn of(grid: &Grid<EngineCell>, mode: TermMode, style: CursorStyle) -> Self {
        let point = grid.cursor.point;
        Self {
            row: point.line.0 as u16,
            column: point.column.0 as u16,
            visible: mode.contains(TermMode::SHOW_CURSOR),
            shape: CursorShape::of(style.shape),
            blinking: style.blinking,
        }
    }
}

/// The shapes DECSCUSR (`CSI Ps SP q`) selects. Ps 0, or no DECSCUSR at
/// all, selects the default: a steady block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    /// Ps 1 and 2.
    Block,
    /// Ps 3 and 4.
    Underline,
    /// Ps 5 and 6.
    Bar,
}

/// The shapes other than a block that DECSCUSR selects; the engine's other
/// shapes are its own renderer's.
const SHAPES: [(EngineShape, CursorShape); 2] = [
    (EngineShape::Underline, CursorShape::Underline),
    (EngineShape::Beam, CursorShape::Bar),
];

impl CursorShape {
    fn of(shape: EngineShape) -> Self {
        SHAPES
            .iter()
            .find(|(engine, _)| *engine == shape)
            .map_or(Self::Block, |(_, shape)| *shape)
    }
}
