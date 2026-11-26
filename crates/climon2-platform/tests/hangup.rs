//! A child forked by another test thread inherits every descriptor of the test
//! process, including a PTY master this test is about to close; while that child
//! holds the master, closing it does not hang up the terminal, and the 5 s grace
//! ends in the fallback kill instead (#248). So this test runs in its own test
//! binary, where nothing else forks.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use climon2_platform::process;
use std::{
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    process::{Command, Stdio},
};

#[test]
fn issue19_closing_real_controlling_pty_delivers_hangup() {
    let (master, slave) = pty_pair();
    let terminal = terminal_of(&slave);
    let mut command = Command::new("sleep");
    command.arg("60").stdin(slave).stdout(Stdio::null());
    // The sleeper leads a new session controlled by the PTY; `spawn` returns only after exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    drop(command); // The sleeper holds the only slave, so closing the master leaves no other end.
    let mut session = process::TerminalSession::new(child.id(), terminal).unwrap();
    let status = process::hangup(&mut session, || drop(master)).unwrap();
    let _ = child.wait();
    assert_eq!(status.code().unwrap(), 129);
}

/// A PTY master and slave, both close-on-exec.
fn pty_pair() -> (File, File) {
    let (mut master, mut slave) = (-1, -1);
    let allocation = climon2_platform::pty::allocation().unwrap();
    let opened = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(opened, 0);
    drop(allocation);
    for fd in [master, slave] {
        assert_ne!(
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
            -1
        );
    }
    unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) }
}

/// The terminal device `slave` is, as the daemon captures its PTY slave.
fn terminal_of(slave: &File) -> process::Terminal {
    let mut name = [0 as libc::c_char; 128];
    let found = unsafe { libc::ttyname_r(slave.as_raw_fd(), name.as_mut_ptr(), name.len()) };
    assert_eq!(found, 0);
    let name = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) };
    process::Terminal::from_path(std::path::Path::new(name.to_str().unwrap())).unwrap()
}
