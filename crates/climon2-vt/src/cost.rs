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

/// Normal-screen history storage for a history limit of `chunks` whole
/// chunks, once the engine has held `peak` chunks: the rows, and the row
/// vector, which keeps the capacity it reached, can double, and briefly
/// holds its old buffer while it grows.
pub(crate) fn history(geometry: Geometry, chunks: usize, peak: usize) -> usize {
    if peak == 0 {
        return 0;
    }
    let cols = usize::from(geometry.cols());
    let vector = usize::from(geometry.rows()) + peak * CHUNK;
    chunks * CHUNK * cols * size_of::<Cell>() + 3 * vector * size_of::<Row<Cell>>()
}

/// One cell's shared extra data (`Arc<CellExtra>`).
const EXTRA: usize = 56;

/// Charged per combining mark a cell keeps: its share of the character
/// vector and of the copy made while that vector grows.
const MARK: usize = 16;

/// The most one combining mark can add: new extra data and the character
/// vector's first four characters.
const COMBINING: usize = 72;

/// The most printing one character can add, by its display width.
#[derive(Clone, Copy)]
struct Printed {
    /// A combining mark may create extra data.
    combining: usize,
    /// A narrow cell shares the cursor template's extra data and may later
    /// need its own copy.
    narrow: usize,
    /// A wide character writes up to three cells: a leading spacer where it
    /// does not fit, itself and its spacer.
    wide: usize,
}

impl Printed {
    /// The table for a cursor template that does or does not carry extra
    /// data, such as an underline colour.
    fn under(tinted: bool) -> Self {
        if tinted { TINTED } else { PLAIN }
    }

    /// Controls (C0, C1 and DEL) have no width: vte executes or ignores them.
    fn character(self, c: char) -> usize {
        c.width().map_or(0, |width| match width {
            0 => self.combining,
            1 => self.narrow,
            _ => self.wide,
        })
    }

    /// The most any one character adds, which is what repeating the
    /// preceding character adds each time.
    fn worst(self) -> usize {
        self.combining.max(self.narrow).max(self.wide)
    }
}

const PLAIN: Printed = Printed {
    combining: COMBINING,
    narrow: 0,
    wide: 0,
};

const TINTED: Printed = Printed {
    combining: COMBINING,
    narrow: EXTRA,
    wide: 3 * EXTRA,
};

/// The bound for what one cell's extra data holds: the shared data and its
/// share for each combining mark.
pub(crate) fn cell(cell: &Cell) -> usize {
    cell.zerowidth()
        .map_or(0, |marks| EXTRA + MARK * marks.len())
}

/// The most feeding a text token can add while the cursor template does or
/// does not carry extra data.
pub(crate) fn text(bytes: &[u8], tinted: bool) -> usize {
    let printed = Printed::under(tinted);
    bytes
        .utf8_chunks()
        .map(|chunk| chunk_cost(&chunk, printed))
        .sum()
}

/// vte prints one replacement character, a narrow cell, per invalid
/// sequence.
fn chunk_cost(chunk: &Utf8Chunk<'_>, printed: Printed) -> usize {
    let valid: usize = chunk.valid().chars().map(|c| printed.character(c)).sum();
    valid + chunk.invalid().len() * printed.narrow
}

/// The most repeating the preceding character `count` times can add.
pub(crate) fn repeat(count: usize, tinted: bool) -> usize {
    count * Printed::under(tinted).worst()
}
