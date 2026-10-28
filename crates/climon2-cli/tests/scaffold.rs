/// #17: usage errors do not touch home or logs and stay distinct from own errors.
#[test]
fn missing_command_is_usage_error() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_climon2-cli"))
        .output()
        .expect("run CLI");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"climon2: error: usage: climon2 exec <command> [args...] | list [--json]\n"
    );
}
