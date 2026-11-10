#![cfg(any(target_os = "macos", target_os = "linux"))]

use climon2_test_term::Isolation;
#[cfg(target_os = "linux")]
use std::process::Stdio;
use std::{
    io::{BufRead, BufReader},
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn assert_owned_until_exit(command: &mut Command) {
    let isolation = Isolation::new();
    let unrelated = Isolation::new();
    let mut child = command
        .env_clear()
        .env("CLIMON2_TEST_ID", isolation.id())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take();
    let pid = child.id() as i32;
    // Reap even if the ownership assertion fails.
    let found = std::panic::catch_unwind(|| {
        if let Some(stdout) = stdout {
            let mut ready = String::new();
            BufReader::new(stdout).read_line(&mut ready).unwrap();
            assert_eq!(ready, "READY\n");
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while !isolation.owned_pids().contains(&pid) {
            assert!(Instant::now() < deadline, "marked process was not owned");
            thread::sleep(Duration::from_millis(10));
        }
        unrelated.assert_clean();
    });
    child.kill().unwrap();
    child.wait().unwrap();
    found.unwrap();
    isolation.assert_clean();
}

#[test]
fn marked_process_outside_a_terminal_is_owned_until_it_exits() {
    assert_owned_until_exit(Command::new("/bin/sleep").arg("30"));
}

#[cfg(target_os = "linux")]
#[test]
fn process_names_with_spaces_and_parentheses_do_not_hide_owned_processes() {
    assert_owned_until_exit(
        Command::new("/bin/sh")
            .args([
                "-c",
                "printf 'owned ) shell' > /proc/self/comm; printf 'READY\\n'; read answer",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped()),
    );
}
