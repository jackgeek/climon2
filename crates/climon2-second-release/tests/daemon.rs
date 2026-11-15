//! The second-release fixture's daemon (#97) serves only its compiled-in
//! release. Its sessions are tested end to end in `climon2-e2e`'s
//! `tests/mixed.rs`.

use std::process::Command;

fn daemon(version: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_climon2-second-release-daemon"))
        .args(["--session-id", "00112233-4455-4677-8899-aabbccddeeff"])
        .args(["--version", version, "--", "/bin/true"])
        .env_clear()
        .output()
        .unwrap()
}

/// An invocation naming another release is refused before the daemon reads
/// anything else; one naming its own release reaches the daemon itself,
/// which refuses this incomplete invocation in its own words.
#[test]
fn second_release_daemon_serves_only_its_compiled_in_release() {
    let other = daemon("0.1.0");
    assert_eq!(other.status.code(), Some(125));
    assert_eq!(
        String::from_utf8_lossy(&other.stderr),
        "climon2: error: this daemon serves only release 0.0.97\n"
    );
    let own = daemon("0.0.97");
    assert_eq!(own.status.code(), Some(125));
    assert_eq!(
        String::from_utf8_lossy(&own.stderr),
        "climon2: error: invalid daemon invocation\n"
    );
}
