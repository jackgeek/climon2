use std::time::Duration;

/// ADR 0001: reject any invocation outside the frozen three-argument contract
/// `attach --metadata <absolute path>`, with no arguments, a relative path, or
/// any other operation or flag, before reading anything.
#[test]
fn missing_metadata_is_a_one_line_failure_without_stdout() {
    let home = tempfile::tempdir().unwrap();
    for arguments in [
        &[][..],
        &["attach", "--metadata"],
        &["attach", "--metadata", "relative.pty"],
        &["attach", "--file", "/absolute.pty"],
        &["kill", "--metadata", "/absolute.pty"],
        &["attach", "--metadata", "/absolute.pty", "extra"],
    ] {
        let mut client = std::process::Command::new(env!("CARGO_BIN_EXE_climon2-client"));
        client
            .args(arguments)
            .env("HOME", home.path())
            .env_remove("CLIMON2_HOME")
            .env_remove("CLIMON2_CLIENT_STARTUP");
        let output = climon2_test_term::output_within(&mut client, Duration::from_secs(20));
        assert_eq!(output.status.code(), Some(125), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert_eq!(
            output.stderr,
            b"climon2: error: usage: climon2-client attach --metadata <absolute path>\n",
            "{arguments:?}"
        );
    }
}
