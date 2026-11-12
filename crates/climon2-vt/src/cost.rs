//! What a monitor model's engine can hold, in bytes of allocation capacity,
//! for unchanged `alacritty_terminal` 0.26.0 and vte 0.15.0 (ADR 0007). Each
//! figure is either what the engine allocates or an upper bound on what one
//! token can make it allocate, so a model can refuse growth before it happens.

use crate::geometry::Geometry;
use alacritty_terminal::{
    grid::Row,
    term::{LineDamageBounds, SEMANTIC_ESCAPE_CHARS, cell::Cell},
};
use std::str::Utf8Chunk;
use unicode_width::UnicodeWidthChar;

/// vte's synchronized-update buffer, allocated whole by `Processor::new`.
const PROCESSOR: usize = 2 * 1024 * 1024;

/// Room for what is held only briefly or is too small to track: the front
/// filter's partial token and the tokens one slice completes (at most about
/// 15 KiB measured, for a 4096-byte control string held across one-byte
/// reads and then parsed), the cursor templates' cell data, and the engine's
/// new configuration and tab stops while it replaces them.
pub(crate) const RESERVE: usize = 32 * 1024;

/// Rows the engine adds to the normal screen's storage whenever its history
/// outgrows it (`MAX_CACHE_SIZE`).
pub(crate) const CHUNK: usize = 1000;

/// The engine's allocation for a fresh terminal: both screens, tab stops,
/// line damage and its configuration.
fn terminal(geometry: Geometry) -> usize {
    let cols = usize::from(geometry.cols());
    let rows = usize::from(geometry.rows());
    let screen = rows * (cols * size_of::<Cell>() + size_of::<Row<Cell>>());
    2 * screen + cols + rows * size_of::<LineDamageBounds>() + SEMANTIC_ESCAPE_CHARS.len()
}

/// Everything a fresh model instance holds before any output.
pub(crate) fn fixed(geometry: Geometry) -> usize {
    PROCESSOR + RESERVE + terminal(geometry)
}

/// Normal-screen history storage for a history limit in whole chunks:
/// the rows, and the row vector, which can double and briefly holds its old
/// buffer while it grows.
pub(crate) fn history(geometry: Geometry, limit: usize) -> usize {
    if limit == 0 {
        return 0;
    }
    let cols = usize::from(geometry.cols());
    let rows = usize::from(geometry.rows()) + limit;
    limit * cols * size_of::<Cell>() + 3 * rows * size_of::<Row<Cell>>()
}

/// One cell's shared extra data (`Arc<CellExtra>`).
const EXTRA: usize = 56;

/// Charged per combining mark a cell keeps: its share of the character
/// vector and of the copy made while that vector grows.
const MARK: usize = 16;

/// The most one combining mark can add: new extra data and the character
/// vector's first four characters.
const COMBINING: usize = 72;

/// The most one printed character can add, by whether the cursor template
/// carries extra data and by display width. A combining mark may create
/// extra data; a printed cell shares the template's and may later need its
/// own copy, and a wide character writes up to three cells: a leading spacer
/// where it does not fit, itself and its spacer.
const PRINTED: [[usize; 3]; 2] = [[COMBINING, 0, 0], [COMBINING, EXTRA, WIDE]];

/// A wide character's three cells under an underline colour.
const WIDE: usize = 3 * EXTRA;

/// The most repeating one character can add: a combining mark or, under an
/// underline colour, a wide character's cells.
const REPEATED: [usize; 2] = [COMBINING, WIDE];

/// The bound for what one cell's extra data holds: the shared data and its
/// share for each combining mark.
pub(crate) fn cell(cell: &Cell) -> usize {
    cell.zerowidth()
        .map_or(0, |marks| EXTRA + MARK * marks.len())
}

/// The most feeding a text token can add while the cursor template does or
/// does not carry extra data.
pub(crate) fn text(bytes: &[u8], tinted: bool) -> usize {
    let costs = &PRINTED[usize::from(tinted)];
    bytes
        .utf8_chunks()
        .map(|chunk| chunk_cost(&chunk, costs))
        .sum()
}

/// vte prints one replacement character, a cell, per invalid sequence.
fn chunk_cost(chunk: &Utf8Chunk<'_>, costs: &[usize; 3]) -> usize {
    let valid: usize = chunk.valid().chars().map(|c| printed(c, costs)).sum();
    valid + chunk.invalid().len() * costs[1]
}

/// Controls (C0, C1 and DEL) have no width: vte executes or ignores them.
fn printed(c: char, costs: &[usize; 3]) -> usize {
    c.width().map_or(0, |width| costs[width.min(2)])
}

/// The most repeating the preceding character `count` times can add.
pub(crate) fn repeat(count: usize, tinted: bool) -> usize {
    count * REPEATED[usize::from(tinted)]
}
