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

/// The daemon's Ctrl+Break helper (#326), started as the daemon starts it:
/// detached, so it has no console of its own.
fn break_helper(arguments: &[&str]) -> Option<i32> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    Command::new(env!("CARGO_BIN_EXE_climon2-daemon"))
        .arg("--break-console")
        .args(arguments)
        .creation_flags(DETACHED_PROCESS)
        .status()
        .unwrap()
        .code()
}

/// #326: the helper attaches to the console of the process it names and
/// generates Ctrl+Break there. `ping -t` reports Ctrl+Break and carries on,
/// while Ctrl+C would stop it, so the console got Ctrl+Break and nothing
/// else; `cmd`, which started `ping`, survives.
#[test]
fn the_break_helper_gives_the_named_console_ctrl_break() {
    use climon2_test_term::Terminal;
    use portable_pty::CommandBuilder;
    use std::time::Duration;
    const TIMEOUT: Duration = Duration::from_secs(30);

    let mut command = CommandBuilder::new("cmd");
    command.args(["/d", "/k", "ping -t 127.0.0.1"]);
    let mut terminal = Terminal::spawn(command, 30, 120).unwrap();
    terminal.wait_for_text("Reply from", TIMEOUT).unwrap();
    let shell = terminal.pid().to_string();
    assert_eq!(break_helper(&[&shell]), Some(0));
    terminal.wait_for_text("Control-Break", TIMEOUT).unwrap();
    // ping goes on replying after Ctrl+Break.
    terminal
        .wait_for_screen(TIMEOUT, |screen| {
            screen
                .split("Control-Break")
                .nth(1)
                .is_some_and(|after| after.contains("Reply from"))
        })
        .unwrap();
    assert!(!terminal.screen().contains("Control-C"));
}

/// #326: a helper without a console process to attach to fails as an
/// invalid invocation does.
#[test]
fn the_break_helper_needs_a_console_process() {
    assert_eq!(break_helper(&[]), Some(125));
    assert_eq!(break_helper(&["leader"]), Some(125));
    assert_eq!(break_helper(&["1", "2"]), Some(125));
    // This test process is not on a pseudoconsole the helper may attach to:
    // the System process, PID 4, has no console at all.
    assert_eq!(break_helper(&["4"]), Some(125));
}
