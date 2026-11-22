//! #315: on Windows the CLI so far answers only help, version and usage
//! errors (`scaffold.rs`); every subcommand is its own failure.
#![cfg(windows)]

#[test]
fn subcommands_fail_as_unsupported_without_touching_home() {
    for arguments in [
        &["list"][..],
        &["exec", "cmd"],
        &["kill", "abc"],
        &["monitor"],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_climon2-cli"))
            .args(arguments)
            .env("CLIMON2_HOME", "relative/home/is/invalid")
            .output()
            .expect("run CLI");
        assert_eq!(output.status.code(), Some(125), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "climon2: error: this command is not supported on Windows yet\n",
            "{arguments:?}"
        );
    }
}
