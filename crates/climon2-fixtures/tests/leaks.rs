use climon2_test_term::{Isolation, Terminal, bin_path};
use nix::{
    errno::Errno,
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use std::{
    fs,
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    thread,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(5);

/// Kills the deliberate leaks even when an assertion fails, so a red run leaves nothing behind.
struct Leaks(Vec<Pid>);

impl Drop for Leaks {
    fn drop(&mut self) {
        for &pid in &self.0 {
            let _ = kill(pid, Signal::SIGKILL);
        }
    }
}

fn pid_from(path: &Path) -> Pid {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Ok(pid) = fs::read_to_string(path) {
            return Pid::from_raw(pid.parse().unwrap());
        }
        assert!(
            Instant::now() < deadline,
            "{} never written",
            path.display()
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn wait_gone(pid: Pid) {
    let deadline = Instant::now() + DEADLINE;
    while kill(pid, None) != Err(Errno::ESRCH) {
        assert!(Instant::now() < deadline, "process {pid} survived");
        thread::sleep(Duration::from_millis(5));
    }
}

fn assert_leak_found_then_clean(isolation: &Isolation, leaks: &Leaks) {
    let found = catch_unwind(AssertUnwindSafe(|| isolation.assert_clean()));
    assert!(found.is_err(), "leak check missed {:?}", leaks.0);
    for &pid in &leaks.0 {
        kill(pid, Signal::SIGKILL).unwrap();
        wait_gone(pid);
    }
    isolation.assert_clean();
}

/// #21.5: an orphan that cleared its environment in the test's session is still a leak.
#[test]
fn leak_check_finds_an_environment_cleared_orphan_in_the_session() {
    let isolation = Isolation::new();
    let pid_file = isolation.home().join("orphan.pid");
    let mut command = isolation.command("/bin/sh");
    command.args([
        "-c",
        "set -m; trap '' HUP; env -i \"$0\" write-pid \"$1\" sleep 5000 \
         </dev/null >/dev/null 2>&1 &",
    ]);
    command.arg(bin_path("climon2-fixture"));
    command.arg(&pid_file);
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    assert_eq!(terminal.wait_exit(DEADLINE).unwrap(), 0);
    drop(terminal);
    let leaks = Leaks(vec![pid_from(&pid_file)]);
    assert_leak_found_then_clean(&isolation, &leaks);
}

/// #21.5: like a session daemon and its command, environment-cleared processes in new
/// sessions whose parents have exited are still leaks.
#[test]
fn leak_check_finds_environment_cleared_processes_in_detached_sessions() {
    let isolation = Isolation::new();
    let daemon = isolation.home().join("daemon.pid");
    let command_pid = isolation.home().join("command.pid");
    let mut command = isolation.command(bin_path("climon2-fixture"));
    command.args(["sleep", "100", "detach", "10", "write-pid"]);
    command.arg(&daemon);
    command.args(["detach", "4", "write-pid"]);
    command.arg(&command_pid);
    command.args(["sleep", "5000", "sleep", "300", "exit", "0"]);
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    assert_eq!(terminal.wait_exit(DEADLINE).unwrap(), 0);
    drop(terminal);
    let daemon = pid_from(&daemon);
    let leaks = Leaks(vec![pid_from(&command_pid)]);
    wait_gone(daemon);
    assert_leak_found_then_clean(&isolation, &leaks);
}
