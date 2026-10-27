/// #30 only: an unfinished binary must not report successful product behavior.
#[test]
fn scaffold_fails_explicitly() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_climon2-daemon"))
        .output()
        .expect("run session daemon");
    assert_eq!(output.status.code(), Some(125));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"climon2: error: session daemon is not implemented\n"
    );
}
