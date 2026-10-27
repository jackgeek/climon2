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
