//! Linux's half of classifying a library the dynamic loader refused (library
//! embedding): whether its ELF header names x86-64.
use std::{
    ffi::{CStr, c_void},
    fs::File,
    io::Read,
    ops::Range,
};

/// What a loadable client library is here.
pub(super) const ARCHITECTURE: &str = "an x86-64 ELF library";

/// The identification bytes of a 64-bit little-endian ELF object.
const ELF64_LSB: &[u8] = b"\x7fELF\x02\x01";
/// Where an ELF64 header names its machine, and how it names x86-64.
const MACHINE: Range<usize> = 18..20;
const X86_64: [u8; 2] = 62_u16.to_le_bytes();

/// Whether `file` starts with an x86-64 ELF header. A header that cannot be
/// read is not one.
pub(super) fn runs_here(file: &File) -> bool {
    let mut header = Vec::new();
    let _ = file.take(MACHINE.end as u64).read_to_end(&mut header);
    header.starts_with(ELF64_LSB) && header.get(MACHINE) == Some(&X86_64[..])
}

/// Always `None`: an absolute path is never redirected on Linux, so the path
/// and the release identity built into the library prove the exact release.
pub(super) fn substitute(_: *const c_void, _: &CStr) -> Option<String> {
    None
}
