//! THROWAWAY #48/#43: client geometry admission before it reaches the engine.

use alacritty_embedding_probe::geometry::{Geometry, GeometryLimits, GeometryRejected};

const LIMITS: GeometryLimits = GeometryLimits {
    max_columns: 500,
    max_rows: 200,
};

#[test]
fn geometry_inside_the_limits_is_admitted_unchanged() {
    for (columns, rows) in [(1, 1), (80, 24), (500, 200)] {
        let geometry = Geometry::admit(columns, rows, LIMITS).unwrap();
        assert_eq!((geometry.columns(), geometry.rows()), (columns, rows));
    }
}

#[test]
fn empty_or_oversized_geometry_is_rejected_without_clamping() {
    let cases = [
        ((0, 24), GeometryRejected::Empty),
        ((80, 0), GeometryRejected::Empty),
        ((501, 24), GeometryRejected::TooWide { columns: 501 }),
        ((80, 201), GeometryRejected::TooTall { rows: 201 }),
        (
            (u16::MAX, u16::MAX),
            GeometryRejected::TooWide { columns: u16::MAX },
        ),
    ];
    for ((columns, rows), expected) in cases {
        assert_eq!(Geometry::admit(columns, rows, LIMITS), Err(expected));
    }
}
