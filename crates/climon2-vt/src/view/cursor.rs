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
    /// Odd DECSCUSR values and `CSI ? 12 h` blink; even values, Ps 0 and
    /// `CSI ? 12 l` do not, which leaves the [`CursorShape::Default`] shape's
    /// blinking to the host.
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

/// The shapes DECSCUSR (`CSI Ps SP q`) selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    /// The host's own default shape (`CSI 0 SP q`): Ps 0, or no DECSCUSR
    /// since the terminal started or was reset (RIS).
    Default,
    /// Ps 1 and 2.
    Block,
    /// Ps 3 and 4.
    Underline,
    /// Ps 5 and 6.
    Bar,
}

/// The style a model's engine reports while the application has set none:
/// a shape DECSCUSR never selects, so it reads as [`CursorShape::Default`],
/// which Alacritty's own default, a steady block, would not.
pub(crate) const UNSET: CursorStyle = CursorStyle {
    shape: EngineShape::Hidden,
    blinking: false,
};

/// The shapes DECSCUSR selects; any other is the unset style's.
const SHAPES: [(EngineShape, CursorShape); 3] = [
    (EngineShape::Block, CursorShape::Block),
    (EngineShape::Underline, CursorShape::Underline),
    (EngineShape::Beam, CursorShape::Bar),
];

impl CursorShape {
    fn of(shape: EngineShape) -> Self {
        SHAPES
            .iter()
            .find(|(engine, _)| *engine == shape)
            .map_or(Self::Default, |(_, shape)| *shape)
    }
}
