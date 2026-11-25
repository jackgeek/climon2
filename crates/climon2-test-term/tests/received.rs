#![cfg(any(target_os = "macos", target_os = "linux"))]
//! Received bytes: a wait that sees only bytes after a mark (the
//! measurement harness's probes, #121), and a terminal that keeps only its
//! configured number of recent bytes.

use climon2_test_term::{Options, Terminal};
use portable_pty::CommandBuilder;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

/// A shell that prints MARK-ONE, then both marks once a line is typed.
fn printer(options: Options) -> Terminal {
    let mut command = CommandBuilder::new("/bin/sh");
    command.env_clear();
    command.env("SHELL", climon2_test_term::harness_shell());
    command.args([
        "-c",
        "printf 'MARK-ONE\\n'; read line; printf 'MARK-TWO MARK-ONE\\n'; read line",
    ]);
    Terminal::spawn_with(command, 24, 80, options).unwrap()
}

/// Bytes received before the mark never satisfy the wait; the same bytes
/// arriving after it do.
#[test]
fn a_wait_since_a_mark_sees_only_later_bytes() {
    let mut terminal = printer(Options::default());
    terminal.wait_for_text("MARK-ONE", TIMEOUT).unwrap();
    let mark = terminal.received_total();
    assert!(
        terminal
            .wait_for_bytes_since(mark, b"MARK-ONE", Duration::from_millis(200))
            .is_err()
    );
    terminal
        .wait_for_bytes_since(0, b"MARK-ONE", TIMEOUT)
        .unwrap();
    terminal.send("\n").unwrap();
    terminal
        .wait_for_bytes_since(mark, b"MARK-ONE", TIMEOUT)
        .unwrap();
    terminal
        .wait_for_bytes_since(mark, b"MARK-TWO", TIMEOUT)
        .unwrap();
}

/// A terminal keeps at most `recent` of its latest bytes, however many it
/// received in all.
#[test]
fn a_terminal_keeps_only_its_recent_bytes() {
    let mut terminal = printer(Options {
        recent: 4,
        ..Options::default()
    });
    terminal.wait_for_text("MARK-ONE", TIMEOUT).unwrap();
    assert_eq!(terminal.received(), b"NE\r\n");
    assert_eq!(terminal.received_total(), 10);
}
