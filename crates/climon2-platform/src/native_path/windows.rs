//! `CM1_NATIVE_WINDOWS` paths (#318; native revision 1, `cm1_path`): UTF-16LE
//! code units, so an even byte length, naming an absolute drive-letter or
//! UNC path, without NUL. Unpaired surrogates are preserved both ways.
use climon2_core::native;
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

pub const ENCODING: u32 = native::NATIVE_WINDOWS;

/// The most code units a Windows path can hold, `\\?\` prefix and all.
const LONGEST: usize = 32_767;

/// What `decode` requires, as the library reports a refusal.
pub const RULE: &str = "paths must be absolute native Windows paths of UTF-16LE code units \
                        without NUL, at most 32767 of them";

/// The path's UTF-16 code units, little-endian.
pub fn encode(path: &Path) -> Vec<u8> {
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// The absolute path whole UTF-16LE code units name, without NUL and at most
/// `LONGEST` of them.
pub fn decode(bytes: &[u8]) -> Option<PathBuf> {
    let (units, odd) = bytes.as_chunks::<2>();
    let units: Vec<u16> = units.iter().map(|unit| u16::from_le_bytes(*unit)).collect();
    let whole = odd.is_empty() && units.len() <= LONGEST && !units.contains(&0);
    let path = PathBuf::from(OsString::from_wide(&units));
    (whole && path.is_absolute()).then_some(path)
}
