/// ADR 0001: reject any invocation outside the frozen three-argument contract.
#[test]
fn missing_metadata_is_a_one_line_failure_without_stdout() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_climon2-client"))
        .output()
        .expect("run client");
    assert_eq!(output.status.code(), Some(125));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"climon2: error: usage: climon2-client attach --metadata <absolute path>\n"
    );
}
