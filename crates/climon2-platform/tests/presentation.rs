#![cfg(target_os = "macos")]

use climon2_platform::presentation;
use std::{
    fs::File,
    os::fd::{AsRawFd, FromRawFd},
    process::{Command, Stdio},
};

const FIXTURE: &str = "CLIMON2_PRESENTATION_FIXTURE";

#[test]
#[ignore = "owned subprocess fixture"]
fn owned_presentation_fixture() {
    match std::env::var(FIXTURE).unwrap().as_str() {
        "datetime" => {
            let created = "2026-01-02T20:30:00.000Z".parse().unwrap();
            eprintln!("result={}", presentation::local_datetime(&created).unwrap());
        }
        "columns" => eprintln!("result={}", presentation::columns()),
        mode => panic!("unknown presentation fixture {mode}"),
    }
}

fn fixture(mode: &str, configure: impl FnOnce(&mut Command)) -> String {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            "owned_presentation_fixture",
            "--nocapture",
        ])
        .env(FIXTURE, mode)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    configure(&mut command);
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    stderr
        .lines()
        .find_map(|line| line.strip_prefix("result="))
        .unwrap_or_else(|| panic!("fixture reported no result: {stderr}"))
        .to_owned()
}

fn pty_with_columns(columns: u16) -> (File, File) {
    let (mut master, mut slave) = (-1, -1);
    let _allocation = climon2_platform::pty::allocation().unwrap();
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
    let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
    let size = libc::winsize {
        ws_row: 24,
        ws_col: columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    assert_eq!(
        unsafe { libc::ioctl(slave.as_raw_fd(), libc::TIOCSWINSZ, &size) },
        0
    );
    (master, slave)
}

fn columns_on_terminal(columns: u16) -> String {
    let (_master, slave) = pty_with_columns(columns);
    fixture("columns", |command| {
        command.stdout(slave);
    })
}

/// #37: CREATED renders in the process's local timezone, not UTC.
#[test]
fn issue37_created_time_uses_the_subprocess_local_timezone() {
    let local = |zone: &str| {
        fixture("datetime", |command| {
            command.env("TZ", zone);
        })
    };
    assert_eq!(local("UTC"), "2026-01-02 20:30:00");
    assert_eq!(local("Asia/Tokyo"), "2026-01-03 05:30:00");
}

/// #37: table width follows the real stdout terminal; zero-width or redirected stdout is 80.
#[test]
fn issue37_table_width_follows_real_stdout_terminal_with_80_fallback() {
    assert_eq!(columns_on_terminal(132), "132");
    assert_eq!(columns_on_terminal(0), "80");
    assert_eq!(fixture("columns", |_| ()), "80");
}
