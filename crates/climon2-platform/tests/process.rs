#![cfg(target_os = "macos")]

use climon2_platform::process;
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn fixture(mode: &str) -> (Child, File) {
    let (mut master, mut slave) = (-1, -1);
    let allocation = climon2_platform::pty::allocation().unwrap();
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    drop(allocation);
    let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
    for file in [&master, &slave] {
        assert_ne!(
            unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
            -1
        );
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--ignored",
        "--exact",
        "owned_process_fixture",
        "--nocapture",
    ]);
    command
        .env("CLIMON2_PLATFORM_FIXTURE", mode)
        .stdin(slave)
        .stdout(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::ioctl(0, libc::TIOCSCTTY.into(), 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    while !line.contains("fixture-ready") {
        assert_ne!(reader.read_line(&mut line).unwrap(), 0);
    }
    (child, master)
}

#[test]
#[ignore = "owned subprocess fixture"]
fn owned_process_fixture() {
    let mode = std::env::var("CLIMON2_PLATFORM_FIXTURE").unwrap();
    if mode == "ignore" {
        unsafe {
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
        }
    }
    println!("fixture-ready");
    std::io::stdout().flush().unwrap();
    match mode.as_str() {
        "exit" => std::process::exit(37),
        "signal" => unsafe {
            libc::raise(libc::SIGTERM);
        },
        _ => loop {
            std::thread::sleep(Duration::from_secs(60));
        },
    }
}

#[test]
fn issue26_identity_uses_start_time_and_refuses_zero_or_pid_reuse() {
    let (mut child, _master) = fixture("sleep");
    let pid = child.id();
    let started = process::start_time(pid).unwrap().unwrap();
    assert!(started > 1_000_000_000_000_000);
    assert!(process::is_same_process(pid, started).unwrap());
    assert!(!process::is_same_process(pid, started + 1).unwrap());
    assert!(process::start_time(0).is_err());
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!process::is_same_process(pid, started).unwrap());
}

#[test]
fn issue19_waitpid_preserves_exit_and_real_signal_status() {
    let (mut child, _master) = fixture("exit");
    assert_eq!(process::wait(child.id()).unwrap().code().unwrap(), 37);
    let _ = child.wait();
    let (mut child, _master) = fixture("signal");
    assert_eq!(process::wait(child.id()).unwrap().code().unwrap(), 143);
    let _ = child.wait();
}

#[test]
fn issue19_waitpid_refuses_non_child_instead_of_fabricating_exit_status() {
    let error = process::wait(std::process::id()).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
}

#[test]
fn issue19_hangup_escalates_group_after_real_five_seconds() {
    let (mut child, master) = fixture("ignore");
    let started = Instant::now();
    let status = process::hangup(child.id(), || drop(master)).unwrap();
    let _ = child.wait();
    assert_eq!(status.code().unwrap(), 137);
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert!(started.elapsed() < Duration::from_secs(8));
}

#[test]
fn issue19_closing_real_controlling_pty_delivers_hangup() {
    let (mut child, master) = fixture("sleep");
    let status = process::hangup(child.id(), || drop(master)).unwrap();
    let _ = child.wait();
    assert_eq!(status.code().unwrap(), 129);
}

#[test]
fn issue15_bad_cwd_is_rejected_before_pty_can_fall_back() {
    assert!(process::validate_cwd(std::path::Path::new("/nonexistent-climon2-cwd")).is_err());
    let cwd = std::env::current_dir().unwrap();
    process::validate_cwd(&cwd).unwrap();
}
