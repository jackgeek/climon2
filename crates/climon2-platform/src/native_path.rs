//! Paths as they cross the native client interface, `cm1_path` (native
//! revision 1): native Unix bytes as `CM1_NATIVE_UNIX`, and on Windows
//! UTF-16LE code units as `CM1_NATIVE_WINDOWS`. Each platform rejects every
//! other encoding.
use climon2_core::native;
use std::path::{Path, PathBuf};

pub use native_encoding::{ENCODING, RULE, decode, encode};

#[cfg(unix)]
use unix as native_encoding;

#[cfg(windows)]
#[path = "native_path/windows.rs"]
mod native_encoding;

/// A path encoded for the native interface, owned while a `cm1_path`
/// borrows it.
pub struct NativePath(Vec<u8>);

impl NativePath {
    pub fn new(path: &Path) -> Self {
        Self(encode(path))
    }

    /// The `cm1_path` borrowing this encoding.
    pub fn native(&self) -> native::Path {
        native::Path {
            encoding: ENCODING,
            reserved: 0,
            bytes: native::Bytes {
                data: self.0.as_ptr(),
                length: self.0.len() as u64,
            },
        }
    }
}

/// The path a `cm1_path`'s `encoding` and nonempty `bytes` name, if they
/// keep this platform's rule.
pub fn parse(encoding: u32, bytes: &[u8]) -> Option<PathBuf> {
    (encoding == ENCODING).then(|| decode(bytes)).flatten()
}

#[cfg(unix)]
mod unix {
    use crate::runtime::PATH_MAX;
    use climon2_core::native;
    use std::{
        ffi::OsString,
        os::unix::ffi::{OsStrExt, OsStringExt},
        path::{Path, PathBuf},
    };

    pub const ENCODING: u32 = native::NATIVE_UNIX;

    /// What `decode` requires, as the library reports a refusal.
    pub const RULE: &str =
        "paths must be absolute native Unix paths without NUL, shorter than PATH_MAX";

    /// The path's bytes, exactly.
    pub fn encode(path: &Path) -> Vec<u8> {
        path.as_os_str().as_bytes().to_vec()
    }

    /// Absolute native Unix bytes, copied byte for byte: invalid UTF-8 is
    /// preserved; NUL and paths the system cannot open (`PATH_MAX` bytes or
    /// more) are rejected.
    pub fn decode(bytes: &[u8]) -> Option<PathBuf> {
        let absolute =
            bytes.len() < PATH_MAX && bytes.first() == Some(&b'/') && !bytes.contains(&0);
        absolute.then(|| PathBuf::from(OsString::from_vec(bytes.to_vec())))
    }
}
