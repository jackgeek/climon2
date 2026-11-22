//! #317: the daemon binary on Windows. `exec`'s tests in `climon2-cli` start
//! it as the CLI does; these check what it answers on its own.
#![cfg(windows)]
use std::process::Command;

fn daemon(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_climon2-daemon"))
        .args(arguments)
        .output()
        .unwrap()
}

#[test]
fn an_invalid_invocation_is_the_daemons_own_failure() {
    let output = daemon(&["--session-id"]);
    assert_eq!(output.status.code(), Some(125));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "climon2: error: invalid daemon invocation\n"
    );
}

/// Unix starts its command through an exec helper; Windows has none.
#[test]
fn there_is_no_exec_helper() {
    let output = daemon(&["--exec-command", "a", "b", "c", "d"]);
    assert_eq!(output.status.code(), Some(125));
}
