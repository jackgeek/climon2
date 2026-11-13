use climon2_test_term::bin_path;
use std::fs;

/// ADR 0004 / #21/#31/#74: test-term normal dependencies unify hooks into real
/// binaries, which other packages' tests find as siblings, never by nested builds.
#[test]
fn workspace_binaries_contain_test_hooks() {
    let marker = b"CLIMON2_TEST_HOOKS_ENABLED";
    for name in [
        "climon2-cli",
        "climon2-daemon",
        "climon2-client",
        "climon2-install",
        "climon2-monitor",
    ] {
        let bytes = fs::read(bin_path(name)).unwrap();
        assert!(
            bytes.windows(marker.len()).any(|window| window == marker),
            "{name} lacks test hooks; run cargo test --workspace"
        );
    }
}

/// The client library's file name in every version folder (library embedding).
#[cfg(target_os = "macos")]
const LIBRARY: &str = "libclimon2_client.dylib";
#[cfg(target_os = "linux")]
const LIBRARY: &str = "libclimon2_client.so";

/// A 64-bit Mach-O of file type `MH_DYLIB`.
#[cfg(target_os = "macos")]
fn is_shared_library(bytes: &[u8]) -> bool {
    bytes[..4] == [0xcf, 0xfa, 0xed, 0xfe] && bytes[12..16] == 6u32.to_le_bytes()
}

/// A little-endian ELF64 shared object (`ET_DYN`) for x86-64.
#[cfg(target_os = "linux")]
fn is_shared_library(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x7fELF\x02\x01\x01")
        && bytes[16..18] == 3u16.to_le_bytes()
        && bytes[18..20] == 62u16.to_le_bytes()
}

/// ADR 0004 / #75: `cargo test` builds the client library's hooked cdylib
/// without nested builds. Cargo uplifts only binaries, so the library is found
/// beside the test executables in `deps/`.
#[test]
fn workspace_client_library_is_a_hooked_sibling_of_the_test_executables() {
    let marker = b"CLIMON2_TEST_HOOKS_ENABLED";
    let executable = std::env::current_exe().unwrap();
    let library = executable.parent().unwrap().join(LIBRARY);
    let bytes = fs::read(&library)
        .unwrap_or_else(|error| panic!("missing sibling {}: {error}", library.display()));
    assert!(is_shared_library(&bytes), "{} header", library.display());
    assert!(
        bytes.windows(marker.len()).any(|window| window == marker),
        "{LIBRARY} lacks test hooks; run cargo test --workspace"
    );
}
