//! #499 (spec #488): monitor links are specified for macOS and Linux only,
//! so on Windows the relay executable refuses to start, whatever it is given.
#![cfg(windows)]

use std::process::Command;

#[test]
fn the_relay_refuses_to_start_on_windows() {
    for args in [&[][..], &["--listen", "127.0.0.1:0"], &["--nope"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_climon2-relay"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(125), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "climon2: error: c2 relay runs on macOS and Linux only\n",
            "{args:?}"
        );
    }
}
