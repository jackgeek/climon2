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
fn arbitrary_process_name_bytes_do_not_hide_owned_processes() {
    assert_owned_until_exit(
        Command::new("/bin/sh")
            .args([
                "-c",
                "printf 'owned ) sh\\377' > /proc/self/comm; printf 'READY\\n'; read answer",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped()),
    );
}

#[cfg(target_os = "linux")]
#[test]
fn non_terminal_commands_do_not_inherit_other_tests_descriptors() {
    use std::os::fd::AsRawFd;

    let file = tempfile::NamedTempFile::new().unwrap();
    let fd = file.as_raw_fd();
    assert_eq!(unsafe { nix::libc::fcntl(fd, nix::libc::F_SETFD, 0) }, 0);
    let status = climon2_test_term::process_command("/bin/sh")
        .args([
            "-c",
            "[ ! /proc/self/fd/$1 -ef \"$2\" ]",
            "descriptor-check",
        ])
        .arg(fd.to_string())
        .arg(file.path())
        .status()
        .unwrap();
    assert!(status.success(), "unrelated descriptor survived exec");
}

/// Runs `harness_command_shell_probe` in a child test process whose own
/// `SHELL` is `parent` (`None` leaves it unset), and asserts the probe saw
/// `expected` as the harness command's `SHELL`.
fn assert_harness_shell(parent: Option<&str>, expected: &str) {
    let mut command = climon2_test_term::process_command(std::env::current_exe().unwrap());
    command
        .args(["--exact", "harness_command_shell_probe", "--ignored"])
        .env("CLIMON2_EXPECTED_SHELL", expected);
    match parent {
        Some(shell) => command.env("SHELL", shell),
        None => command.env_remove("SHELL"),
    };
    let output = climon2_test_term::output_within(&mut command, Duration::from_secs(30));
    assert!(
        output.status.success(),
        "SHELL {parent:?} did not reach the harness command as {expected:?}:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "run in a child process by harness_commands_take_shell_from_the_test_process"]
fn harness_command_shell_probe() {
    let expected = std::env::var_os("CLIMON2_EXPECTED_SHELL").expect("expected SHELL");
    let command = Isolation::new().command("true");
    assert_eq!(command.get_env("SHELL"), Some(expected.as_os_str()));
}

/// #170: without SHELL, portable-pty looks the shell up with getpwuid(3),
/// which parallel test threads race. The harness sets it from its own SHELL.
#[test]
fn harness_commands_take_shell_from_the_test_process() {
    assert_harness_shell(
        Some("/climon2-test/sentinel-shell"),
        "/climon2-test/sentinel-shell",
    );
}

/// #170: a test process without SHELL gives its commands /bin/sh, not a
/// password-database lookup.
#[test]
fn harness_commands_default_shell_to_bin_sh() {
    assert_harness_shell(None, "/bin/sh");
}
