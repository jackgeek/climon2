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
}

/// Bytes in one engine step of text and C0 controls.
pub(crate) const STEP: usize = 64;
