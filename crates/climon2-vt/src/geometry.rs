/// Session geometry: the controller's host size clamped to what the engine
/// supports, so an out-of-range size cannot be constructed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    cols: u16,
    rows: u16,
}

impl Geometry {
    /// Clamp a host size to 2..=500 columns and 1..=200 rows; a zero
    /// dimension is rejected so the caller keeps its previous size.
    pub fn for_host(cols: u16, rows: u16) -> Option<Self> {
        (cols != 0 && rows != 0).then(|| Self {
            cols: cols.clamp(2, 500),
            rows: rows.min(200),
        })
    }

    pub fn cols(self) -> u16 {
        self.cols
    }

    pub fn rows(self) -> u16 {
        self.rows
    }

    /// The most rows one engine step can push into history at this
    /// geometry: 64 line feeds or wraps from a 64-byte step, a screenful from
    /// one scrolling sequence, or one REP (`CSI 65535 b`) of a wide character.
    /// The engine keeps twice this many rows as counting headroom.
    pub fn push_bound(self) -> usize {
        let repeat = REPEATED_CELLS.div_ceil(usize::from(self.cols)) + 1;
        repeat.max(STEP).max(usize::from(self.rows))
    }
}

/// Cells one maximal REP of a wide character covers.
const REPEATED_CELLS: usize = 2 * 65535;

/// Bytes in one engine step of text and C0 controls.
pub(crate) const STEP: usize = 64;
