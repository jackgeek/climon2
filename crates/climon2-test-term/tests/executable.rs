#![cfg(any(target_os = "macos", target_os = "linux"))]

use climon2_test_term::{copy_executable, output_within};
use nix::libc;
use std::{
    ffi::CString,
    fs::OpenOptions,
    io::Write,
    os::{fd::AsRawFd, unix::ffi::OsStrExt},
    process::Command,
    thread,
    time::{Duration, Instant},
};

/// Linux refuses to execute a file that any process holds open for writing (ETXTBSY), and
/// a child forked by another test thread holds every descriptor it inherited until it
/// execs. A fork while the copy is being written must not leave the copy unexecutable.
#[test]
fn a_fork_while_the_copy_is_written_leaves_the_copy_executable() {
    let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let source = directory.path().join("source");
    let fifo = CString::new(source.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o755) }, 0);
    let copy = directory.path().join("copy");
    let copier = {
        let (source, copy) = (source.clone(), copy.clone());
        thread::spawn(move || copy_executable(&source, &copy))
    };
    // Opening blocks until the copier opens the source; the copy then stays open for
    // writing until the copier reads end of file, which needs this writer closed.
    let mut writer = OpenOptions::new().write(true).open(&source).unwrap();
    writer.write_all(b"#!/bin/sh\nexit 7\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !copy.exists() {
        assert!(Instant::now() < deadline, "the copy was never created");
        thread::sleep(Duration::from_millis(1));
    }
    let holder = Holder::fork(writer.as_raw_fd());
    drop(writer);
    copier.join().unwrap();
    let status = Command::new(&copy).status().unwrap();
    assert_eq!(status.code(), Some(7));
    drop(holder);
}

/// #75 bounded waits: a command still running at its deadline is killed and
/// reaped, and the waiting test fails instead of hanging.
#[test]
fn output_within_kills_and_reaps_a_command_that_outlives_its_deadline() {
    let mut sleeper = Command::new("sleep");
    sleeper.arg("600");
    let started = Instant::now();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        output_within(&mut sleeper, Duration::from_secs(1))
    }))
    .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(60));
    let message = failure.downcast_ref::<String>().unwrap();
    assert!(
        message.ends_with("did not exit within 1s; killed and reaped"),
        "{message}"
    );
    let pid: libc::pid_t = message
        .split_once("(pid ")
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(pid, _)| pid.parse().unwrap())
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "the sleeper survives");
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}

/// A forked child that keeps every descriptor it inherited, except the source's writer,
/// until it is dropped.
struct Holder(libc::pid_t);

impl Holder {
    fn fork(writer: i32) -> Self {
        match unsafe { libc::fork() } {
            -1 => panic!("fork: {}", std::io::Error::last_os_error()),
            0 => unsafe {
                libc::close(writer);
                loop {
                    libc::pause();
                }
            },
            pid => Self(pid),
        }
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.0, libc::SIGKILL);
            libc::waitpid(self.0, std::ptr::null_mut(), 0);
        }
    }
}
