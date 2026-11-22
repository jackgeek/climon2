//! #318: paths cross the native client interface as `CM1_NATIVE_WINDOWS`:
//! UTF-16LE code units naming an absolute drive-letter or UNC path, without
//! NUL, with unpaired surrogates preserved (native revision 1, `cm1_path`).
#![cfg(windows)]
use climon2_core::native;
use climon2_platform::native_path::{ENCODING, NativePath, parse};
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    slice,
};

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn le(units: &[u16]) -> Vec<u8> {
    units.iter().flat_map(|unit| unit.to_le_bytes()).collect()
}

/// The bytes a `cm1_path` borrows.
fn borrowed(path: &native::Path) -> &[u8] {
    // SAFETY: a NativePath's cm1_path covers its own bytes while it lives.
    unsafe { slice::from_raw_parts(path.bytes.data, path.bytes.length as usize) }
}

#[test]
fn a_path_crosses_as_utf16le_code_units_with_unpaired_surrogates_kept() {
    let mut wide = units(r"C:\climon2\");
    wide.extend([0xD800, u16::from(b'x'), 0xDC00]);
    let path = PathBuf::from(OsString::from_wide(&wide));

    let encoded = NativePath::new(&path);
    let crossing = encoded.native();

    assert_eq!(ENCODING, native::NATIVE_WINDOWS);
    assert_eq!(crossing.encoding, native::NATIVE_WINDOWS);
    assert_eq!(crossing.reserved, 0);
    assert_eq!(borrowed(&crossing), le(&wide));
    let parsed = parse(crossing.encoding, borrowed(&crossing)).unwrap();
    assert_eq!(parsed.as_os_str().encode_wide().collect::<Vec<_>>(), wide);
}

#[test]
fn drive_letter_unc_and_verbatim_paths_are_accepted() {
    for path in [
        r"C:\Users\me\climon2",
        r"\\server\share\climon2",
        r"\\?\C:\Users\me\climon2",
    ] {
        assert_eq!(
            parse(native::NATIVE_WINDOWS, &le(&units(path))).as_deref(),
            Some(Path::new(path)),
            "{path}"
        );
    }
}

#[test]
fn relative_paths_nul_odd_lengths_and_other_encodings_are_refused() {
    for path in [r"climon2\home", r"C:climon2", r"\climon2", "C:\\a\0b"] {
        assert_eq!(
            parse(native::NATIVE_WINDOWS, &le(&units(path))),
            None,
            "{path:?}"
        );
    }
    let mut odd = le(&units(r"C:\climon2"));
    odd.push(b'x');
    assert_eq!(parse(native::NATIVE_WINDOWS, &odd), None);
    let absolute = le(&units(r"C:\climon2"));
    assert_eq!(parse(native::NATIVE_UNIX, &absolute), None);
    assert_eq!(parse(native::NATIVE_WINDOWS + 1, &absolute), None);
}

#[test]
fn a_path_holds_at_most_32767_code_units() {
    let longest = format!(r"C:\{}", "a".repeat(32_767 - 3));
    assert!(parse(native::NATIVE_WINDOWS, &le(&units(&longest))).is_some());
    let longer = format!("{longest}a");
    assert_eq!(parse(native::NATIVE_WINDOWS, &le(&units(&longer))), None);
}
