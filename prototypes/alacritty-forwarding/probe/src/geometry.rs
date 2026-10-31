//! THROWAWAY #48/#43: client-reported geometry is parsed once, at entry.
//!
//! Alacritty accepts any `Dimensions` (term/mod.rs:410, 655) and allocates
//! rows x columns cells per grid, so the cap must precede `Term::new`/`resize`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryLimits {
    pub max_columns: u16,
    pub max_rows: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryRejected {
    Empty,
    TooWide { columns: u16 },
    TooTall { rows: u16 },
}

/// A size the engine may be created or resized to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    columns: u16,
    rows: u16,
}

impl Geometry {
    pub fn admit(
        columns: u16,
        rows: u16,
        limits: GeometryLimits,
    ) -> Result<Self, GeometryRejected> {
        if columns == 0 || rows == 0 {
            Err(GeometryRejected::Empty)
        } else if columns > limits.max_columns {
            Err(GeometryRejected::TooWide { columns })
        } else if rows > limits.max_rows {
            Err(GeometryRejected::TooTall { rows })
        } else {
            Ok(Self { columns, rows })
        }
    }

    pub fn columns(self) -> u16 {
        self.columns
    }

    pub fn rows(self) -> u16 {
        self.rows
    }
}
