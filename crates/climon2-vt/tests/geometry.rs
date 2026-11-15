use climon2_vt::Geometry;

fn size(cols: u16, rows: u16) -> Option<(u16, u16)> {
    Geometry::for_host(cols, rows).map(|geometry| (geometry.cols(), geometry.rows()))
}

#[test]
fn host_sizes_inside_the_supported_range_are_kept() {
    assert_eq!(size(2, 1), Some((2, 1)));
    assert_eq!(size(80, 24), Some((80, 24)));
    assert_eq!(size(500, 200), Some((500, 200)));
}

#[test]
fn host_sizes_outside_the_supported_range_are_clamped() {
    assert_eq!(size(1, 1), Some((2, 1)));
    assert_eq!(size(501, 201), Some((500, 200)));
    assert_eq!(size(u16::MAX, u16::MAX), Some((500, 200)));
    assert_eq!(
        Geometry::for_host(u16::MAX, u16::MAX),
        Some(Geometry::LARGEST)
    );
}

#[test]
fn a_zero_dimension_is_rejected() {
    assert_eq!(size(0, 24), None);
    assert_eq!(size(80, 0), None);
    assert_eq!(size(0, 0), None);
}
