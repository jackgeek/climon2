//! The serializer: pure functions from engine cells to host bytes, shared by
//! restoration and derived presentation.

use alacritty_terminal::{
    Grid,
    grid::{Cursor, Dimensions, GridCell},
    index::{Column, Line},
    term::cell::{Cell, Flags},
    vte::ansi::Color,
};

const SGR_RESET: &[u8] = b"\x1b[0m";
const SPACERS: Flags = Flags::WIDE_CHAR_SPACER.union(Flags::LEADING_WIDE_CHAR_SPACER);
const RENDITION: Flags = Flags::BOLD
    .union(Flags::DIM)
    .union(Flags::ITALIC)
    .union(Flags::ALL_UNDERLINES)
    .union(Flags::INVERSE)
    .union(Flags::HIDDEN)
    .union(Flags::STRIKEOUT);

/// SGR parameters for rendition flags, in emission order.
const FLAGS: &[(Flags, &str)] = &[
    (Flags::BOLD, "1"),
    (Flags::DIM, "2"),
    (Flags::ITALIC, "3"),
    (Flags::UNDERLINE, "4"),
    (Flags::DOUBLE_UNDERLINE, "4:2"),
    (Flags::UNDERCURL, "4:3"),
    (Flags::DOTTED_UNDERLINE, "4:4"),
    (Flags::DASHED_UNDERLINE, "4:5"),
    (Flags::INVERSE, "7"),
    (Flags::HIDDEN, "8"),
    (Flags::STRIKEOUT, "9"),
];

/// Offsets from 30 (foreground) or 40 (background) for the 16 named colours.
const NAMED: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 60, 61, 62, 63, 64, 65, 66, 67];

/// The rendition SGR can express for one cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pen {
    fg: Color,
    bg: Color,
    flags: Flags,
    underline: Option<Color>,
}

impl Pen {
    pub(crate) fn of(cell: &Cell) -> Self {
        Self {
            fg: cell.fg,
            bg: cell.bg,
            flags: cell.flags.intersection(RENDITION),
            underline: cell.underline_color(),
        }
    }

    pub(crate) fn plain() -> Self {
        Self::of(&Cell::default())
    }

    /// `ESC [ 0 ; ... m` selecting exactly this rendition.
    pub(crate) fn sgr(&self) -> Vec<u8> {
        let mut parameters = vec!["0".to_owned()];
        parameters.extend(
            FLAGS
                .iter()
                .filter(|(flag, _)| self.flags.contains(*flag))
                .map(|(_, code)| (*code).to_owned()),
        );
        parameters.extend(colour(self.fg, 30, "38"));
        parameters.extend(colour(self.bg, 40, "48"));
        parameters.extend(self.underline.and_then(underline));
        format!("\x1b[{}m", parameters.join(";")).into_bytes()
    }
}

fn colour(colour: Color, base: u8, extended: &str) -> Option<String> {
    match colour {
        Color::Named(name) => NAMED
            .get(name as usize)
            .map(|offset| (base + offset).to_string()),
        Color::Indexed(index) => Some(format!("{extended};5;{index}")),
        Color::Spec(rgb) => Some(format!("{extended};2;{};{};{}", rgb.r, rgb.g, rgb.b)),
    }
}

fn underline(colour: Color) -> Option<String> {
    match colour {
        Color::Indexed(index) => Some(format!("58:5:{index}")),
        Color::Spec(rgb) => Some(format!("58:2::{}:{}:{}", rgb.r, rgb.g, rgb.b)),
        Color::Named(_) => None,
    }
}

/// The cells of one grid line.
pub(crate) fn cells(grid: &Grid<Cell>, line: i32) -> &[Cell] {
    &grid[Line(line)][..Column(grid.columns())]
}

/// Cells up to and including the last one that is not a default blank.
pub(crate) fn content(cells: &[Cell]) -> &[Cell] {
    let end = cells.iter().rposition(|cell| !cell.is_empty());
    &cells[..end.map_or(0, |last| last + 1)]
}

/// Whether the line continues on the next line (soft wrap).
pub(crate) fn wrapped(cells: &[Cell]) -> bool {
    cells
        .last()
        .is_some_and(|cell| cell.flags.contains(Flags::WRAPLINE))
}

/// Render cells from the default rendition back to it: wide-character
/// spacers are skipped and zero-width characters follow their base.
pub(crate) fn render(cells: &[Cell], out: &mut Vec<u8>) {
    out.extend_from_slice(SGR_RESET);
    let mut pen = Pen::plain();
    for cell in cells.iter().filter(|cell| !cell.flags.intersects(SPACERS)) {
        pen = print(cell, pen, out);
    }
    out.extend_from_slice(SGR_RESET);
}

/// Print one cell, switching rendition only when it changes.
fn print(cell: &Cell, pen: Pen, out: &mut Vec<u8>) -> Pen {
    let next = Pen::of(cell);
    if next != pen {
        out.extend(next.sgr());
    }
    let base = if cell.c == '\t' { ' ' } else { cell.c };
    let text: String = std::iter::once(base)
        .chain(cell.zerowidth().unwrap_or_default().iter().copied())
        .collect();
    out.extend_from_slice(text.as_bytes());
    next
}

/// The screen's characters, every cell of each line and lines separated by
/// a line feed: each cell's character, then its zero-width characters. No
/// rendition, cursor or history is part of it.
pub(crate) fn characters(grid: &Grid<Cell>) -> String {
    let lines = (0..grid.screen_lines()).map(|line| {
        cells(grid, line as i32)
            .iter()
            .flat_map(|cell| {
                std::iter::once(cell.c).chain(cell.zerowidth().unwrap_or_default().iter().copied())
            })
            .collect::<String>()
    });
    lines.collect::<Vec<_>>().join("\n")
}

pub(crate) fn goto(out: &mut Vec<u8>, row: usize, column: usize) {
    out.extend(format!("\x1b[{row};{column}H").into_bytes());
}

/// Paint every non-blank screen line at its absolute position.
pub(crate) fn screen(grid: &Grid<Cell>, out: &mut Vec<u8>) {
    for line in 0..grid.screen_lines() {
        let content = content(cells(grid, line as i32));
        if !content.is_empty() {
            goto(out, line + 1, 1);
            render(content, out);
        }
    }
}

/// Establish the saved cursor (position and pen, then `ESC 7`) and then the
/// current cursor. In Direct presentation a pending wrap is recreated by
/// re-printing the last column's character, so the host wraps as the engine
/// would.
pub(crate) fn cursors(grid: &Grid<Cell>, direct: bool, out: &mut Vec<u8>) {
    position(&grid.saved_cursor, out);
    out.extend(Pen::of(&grid.saved_cursor.template).sgr());
    out.extend_from_slice(b"\x1b7");
    cursor(grid, direct, out);
}

/// Establish the current cursor's position, pending wrap and pen.
pub(crate) fn cursor(grid: &Grid<Cell>, direct: bool, out: &mut Vec<u8>) {
    let cursor = &grid.cursor;
    if direct && cursor.input_needs_wrap {
        reprint(grid, out);
    } else {
        position(cursor, out);
    }
    out.extend(Pen::of(&cursor.template).sgr());
}

fn position(cursor: &Cursor<Cell>, out: &mut Vec<u8>) {
    goto(
        out,
        cursor.point.line.0 as usize + 1,
        cursor.point.column.0 + 1,
    );
}

/// Re-print the character the cursor wrapped after, starting at its first
/// column (a wide character's spacer occupies the last column).
fn reprint(grid: &Grid<Cell>, out: &mut Vec<u8>) {
    let line = grid.cursor.point.line.0;
    let row = cells(grid, line);
    let last = grid.cursor.point.column.0;
    let column = last - usize::from(row[last].flags.contains(Flags::WIDE_CHAR_SPACER));
    goto(out, line as usize + 1, column + 1);
    out.extend_from_slice(SGR_RESET);
    print(&row[column], Pen::plain(), out);
}
