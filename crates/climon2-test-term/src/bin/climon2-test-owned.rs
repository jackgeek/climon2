//! Linux test-only exec adapter: carry ownership through environment clearing and reparenting.

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    use std::{
        fs::File,
        os::{fd::AsRawFd, unix::process::CommandExt},
        process::{Command, ExitCode},
    };
    let mut arguments = std::env::args_os().skip(1);
    let lease =
        File::open(arguments.next().expect("ownership lease path")).expect("open ownership lease");
    let fd = lease.as_raw_fd();
    let flags = unsafe { nix::libc::fcntl(fd, nix::libc::F_GETFD) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { nix::libc::fcntl(fd, nix::libc::F_SETFD, flags & !nix::libc::FD_CLOEXEC) },
        0
    );
    let error = Command::new(arguments.next().expect("owned executable"))
        .args(arguments)
        .exec();
    eprintln!("climon2-test-owned: {error}");
    ExitCode::from(125)
}

#[cfg(not(target_os = "linux"))]
fn main() {}
