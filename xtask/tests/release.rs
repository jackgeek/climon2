use std::process::Command;

/// ADR 0004 / #31: per-package release outputs exclude hooks without replacing test binaries.
#[test]
fn release_check_builds_unhooked_binaries_in_a_private_target() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("release-check")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("release markers absent; test binaries unchanged")
    );
}
