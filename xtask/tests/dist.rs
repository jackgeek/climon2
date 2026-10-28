use std::process::Command;

/// #24.6/#39: a mismatched release tag fails before attempting a universal build.
#[test]
fn dist_rejects_a_tag_that_does_not_match_the_workspace() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("dist")
        .env("GITHUB_REF_TYPE", "tag")
        .env("GITHUB_REF_NAME", "v999.0.0")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("release tag v999.0.0 does not match"),
        "{output:?}"
    );
}
