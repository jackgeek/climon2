//! #501 (spec #488): monitor links are specified for macOS and Linux only,
//! so on Windows the monitor executable refuses to pair.
#![cfg(windows)]

use std::process::Command;

#[test]
fn the_monitor_refuses_to_pair_on_windows() {
    let output = Command::new(env!("CARGO_BIN_EXE_climon2-monitor"))
        .args(["pair", "127.0.0.1:7222"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(125));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "climon2: error: c2 monitor pair runs on macOS and Linux only\n"
    );
}
