//! ADRs 0001/0005: validate external identifiers before using them as paths.
use climon2_core::{SessionId, Timestamp, VersionName, wire::Guid};

#[test]
fn adr0005_guid_and_version_are_validated_without_normalizing_untrusted_paths() {
    let id = "550e8400-e29b-41d4-a716-446655440000";
    assert_eq!(id.parse::<SessionId>().unwrap().as_str(), id);
    for bad in [
        "550E8400-E29B-41D4-A716-446655440000",
        "550e8400e29b41d4a716446655440000",
        "../current",
        "",
    ] {
        assert!(bad.parse::<SessionId>().is_err(), "{bad}");
    }
    for version in ["1.2.3", "0.1.0-alpha.1+build.2"] {
        assert_eq!(version.parse::<VersionName>().unwrap().as_str(), version);
    }
    for bad in [
        "../1.2.3",
        "/1.2.3",
        "1.2",
        "01.2.3",
        "v1.2.3",
        "1.2.3\n",
        "1.2.3\\evil",
    ] {
        assert!(bad.parse::<VersionName>().is_err(), "{bad}");
    }
}

#[test]
fn adr0005_created_at_requires_real_utc_calendar_time_with_exact_milliseconds() {
    for good in ["2024-02-29T23:59:59.999Z", "2026-10-28T07:21:02.000Z"] {
        assert_eq!(good.parse::<Timestamp>().unwrap().as_str(), good);
    }
    for bad in [
        "2025-02-29T00:00:00.000Z",
        "2024-13-01T00:00:00.000Z",
        "2024-01-01T24:00:00.000Z",
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:00.00Z",
        "2024-01-01T00:00:00.000+00:00",
        "2024-01-01T00:00:00.000z",
    ] {
        assert!(bad.parse::<Timestamp>().is_err(), "{bad}");
    }
}

/// Native revision 1 (core-v1 "GUIDs"): a GUID crosses the C ABI as 16
/// bytes in canonical textual order, without host-endian swapping. A session
/// ID is made from such bytes, and a wire GUID gives them back.
#[test]
fn native_guid_bytes_are_in_canonical_textual_order() {
    let bytes = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];
    let text = "00112233-4455-6677-8899-aabbccddeeff";
    assert_eq!(SessionId::from_bytes(bytes).as_str(), text);
    assert_eq!(text.parse::<Guid>().unwrap().bytes(), bytes);
    let nil = "00000000-0000-0000-0000-000000000000";
    assert_eq!(SessionId::from_bytes([0; 16]).as_str(), nil);
    assert_eq!(nil.parse::<Guid>().unwrap().bytes(), [0; 16]);
}
