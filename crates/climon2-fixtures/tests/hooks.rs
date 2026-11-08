use climon2_test_term::bin_path;
use std::fs;

/// ADR 0004 / #21/#31: test-term normal dependencies unify hooks into real binaries.
#[test]
fn workspace_binaries_contain_test_hooks() {
    let marker = b"CLIMON2_TEST_HOOKS_ENABLED";
    for name in [
        "climon2-cli",
        "climon2-daemon",
        "climon2-client",
        "climon2-install",
    ] {
        let bytes = fs::read(bin_path(name)).unwrap();
        assert!(
            bytes.windows(marker.len()).any(|window| window == marker),
            "{name} lacks test hooks; run cargo test --workspace"
        );
    }
}

/// ADR 0004 / #75: `cargo test` builds the client library's hooked cdylib
/// without nested builds. Cargo uplifts only binaries, so the dylib is found
/// beside the test executables in `deps/`.
#[test]
fn workspace_client_library_is_a_hooked_sibling_of_the_test_executables() {
    let marker = b"CLIMON2_TEST_HOOKS_ENABLED";
    let executable = std::env::current_exe().unwrap();
    let library = executable.parent().unwrap().join("libclimon2_client.dylib");
    let bytes = fs::read(&library)
        .unwrap_or_else(|error| panic!("missing sibling {}: {error}", library.display()));
    assert_eq!(bytes[..4], [0xcf, 0xfa, 0xed, 0xfe], "64-bit Mach-O");
    assert_eq!(bytes[12..16], 6u32.to_le_bytes(), "MH_DYLIB file type");
    assert!(
        bytes.windows(marker.len()).any(|window| window == marker),
        "libclimon2_client.dylib lacks test hooks; run cargo test --workspace"
    );
}
