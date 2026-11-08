//! Risk proof for #74: Cargo builds this package's real binary during
//! `cargo test`, and tests find it as a sibling build artifact, never through
//! a nested Cargo build.
use std::fs;

const MARKER: &[u8] = b"CLIMON2_TEST_HOOKS_ENABLED";

/// ADR 0004 / #74: `cargo test -p climon2-monitor` builds the plain binary;
/// workspace tests unify `test-hooks` through `climon2-test-term`'s normal
/// dependencies, and the same sibling binary then carries the hooks.
#[test]
fn cargo_builds_the_plain_or_hooked_monitor_binary_for_its_tests() {
    let binary = fs::read(env!("CARGO_BIN_EXE_climon2-monitor")).unwrap();
    let hooked = binary.windows(MARKER.len()).any(|window| window == MARKER);
    assert_eq!(hooked, cfg!(feature = "test-hooks"));
}
