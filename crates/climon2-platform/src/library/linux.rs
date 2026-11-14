//! Linux's library checks (library embedding): a file the dynamic loader
//! refused is loadable here only if its ELF header names x86-64, and an
//! absolute path is never redirected.
use std::{
    ffi::{CStr, c_void},
    fs::File,
    io::Read,
};

/// What a loadable client library is here.
pub(super) const ARCHITECTURE: &str = "an x86-64 ELF library";

/// The size of an ELF64 header.
const ELF64_HEADER: u64 = 64;

/// Whether `file` starts with an x86-64 ELF header. A header that cannot be
/// read is not one.
pub(super) fn runs_here(file: &File) -> bool {
    let mut header = Vec::new();
    let _ = file.take(ELF64_HEADER).read_to_end(&mut header);
    super::is_x86_64_elf(&header)
}

/// Always `None`: an absolute path is never redirected on Linux, so the path
/// and the release identity built into the library prove the exact release.
pub(super) fn loaded_instead(_: *const c_void, _: &CStr) -> Option<String> {
    None
}
