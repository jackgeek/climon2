//! #315: on Windows the CLI answers help, version and usage errors
//! (`scaffold.rs`); `list`, `exec`, `attach` and `kill` run
//! (`list_windows.rs`, `exec_windows.rs`, `packaged_windows.rs`); the monitor
//! is its own failure until it runs on Windows (#327).
#![cfg(windows)]

#[test]
fn the_monitor_fails_as_unsupported_without_touching_home() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_climon2-cli"))
        .arg("monitor")
        .env("CLIMON2_HOME", "relative/home/is/invalid")
        .output()
        .expect("run CLI");
    assert_eq!(output.status.code(), Some(125));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "climon2: error: this command is not supported on Windows yet\n"
    );
}
