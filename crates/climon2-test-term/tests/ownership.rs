#![cfg(any(target_os = "macos", target_os = "linux"))]

use climon2_test_term::{Isolation, Terminal};
use portable_pty::CommandBuilder;
use std::{
    thread,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(3);

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "process ownership did not converge"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn terminal_descendants_remain_owned_after_removing_the_environment_marker() {
    let isolation = Isolation::new();
    let unrelated = Isolation::new();
    let mut command = CommandBuilder::new("/bin/sh");
    command.env_clear();
    command.env("CLIMON2_TEST_ID", isolation.id());
    command.args([
        "-c",
        "exec /usr/bin/env -i /bin/sh -c 'sleep 30 & echo CHILD=$!; read answer'",
    ]);
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    terminal.wait_for_text("CHILD=", TIMEOUT).unwrap();
    let screen = terminal.screen();
    let child: i32 = screen
        .split("CHILD=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    wait_until(|| {
        let owned = isolation.owned_pids();
        owned.contains(&terminal.pid()) && owned.contains(&child)
    });
    unrelated.assert_clean();
    drop(terminal);
    wait_until(|| isolation.owned_pids().is_empty());
    isolation.assert_clean();
}
