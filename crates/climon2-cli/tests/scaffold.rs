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
        b"climon2: error: usage: c2 exec|x [--headless|-h] <command> [args...] | attach|a <session-id> | kill|k <session-id> | list|l [--json] | monitor|m | home|h [release [--force]]\n"
    );
}

/// #287: help has only its long flag; a bare `-h` is a usage error.
#[test]
fn dash_h_alone_is_a_usage_error_not_help() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_climon2-cli"))
        .arg("-h")
        .output()
        .expect("run CLI");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"climon2: error: usage: c2 exec|x [--headless|-h] <command> [args...] | attach|a <session-id> | kill|k <session-id> | list|l [--json] | monitor|m | home|h [release [--force]]\n"
    );
}

/// #17: help and version are presentation-only: exact stdout, status 0, no home resolution.
#[test]
fn help_and_version_print_exact_presentation_without_home() {
    for (flag, expected) in [
        (
            "--help",
            "usage: c2 exec|x [--headless|-h] <command> [args...] | attach|a <session-id> | kill|k <session-id> | list|l [--json] | monitor|m | home|h [release [--force]]\n"
                .to_owned(),
        ),
        (
            "--version",
            format!("climon2 {}\n", env!("CARGO_PKG_VERSION")),
        ),
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_climon2-cli"))
            .arg(flag)
            .env("CLIMON2_HOME", "relative/home/is/invalid")
            .output()
            .expect("run CLI");
        assert_eq!(output.status.code(), Some(0), "{flag}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected, "{flag}");
        assert!(output.stderr.is_empty(), "{flag}");
    }
}
