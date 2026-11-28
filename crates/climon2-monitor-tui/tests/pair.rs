//! #501 (CM6, CM10-11, CM24, PAIR-10): the monitor executable's arguments
//! and the failures of `pair`, which the CLI hands it for
//! `c2 monitor pair`. The executable runs from a version folder of its own,
//! with stdin a pipe; nothing here needs a relay or a terminal.
#![cfg(unix)]

use std::{
    io::Write,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

/// This monitor, copied into a version folder of an executable home under
/// `root`. It is copied by `cp`, so this process never holds it open for
/// writing while another test's child is forked.
fn installed(root: &Path) -> PathBuf {
    let folder = root.join("executable home").join("1.2.3");
    std::fs::create_dir_all(&folder).unwrap();
    let monitor = folder.join("climon2-monitor");
    let copied = Command::new("cp")
        .arg(env!("CARGO_BIN_EXE_climon2-monitor"))
        .arg(&monitor)
        .status()
        .unwrap();
    assert!(copied.success());
    monitor
}

fn run(args: &[&str], stdin: &str) -> Output {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let mut monitor = Command::new(installed(&root_path))
        .args(args)
        .env_clear()
        .env("CLIMON2_HOME", root_path.join("home"))
        // Coverage runs keep counting in the child.
        .envs(std::env::var_os("LLVM_PROFILE_FILE").map(|file| ("LLVM_PROFILE_FILE", file)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    monitor
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let output = monitor.wait_with_output().unwrap();
    assert!(
        !root_path.join("home/links/relays.json").exists(),
        "nothing is remembered"
    );
    output
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// CM6: anything but nothing or `pair <host:port>` is a usage error.
#[test]
fn other_arguments_are_a_usage_error() {
    for args in [
        &["bogus"][..],
        &["pair"],
        &["pair", "a:1", "--pin", "12345678"],
    ] {
        let output = run(args, "");
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(
            stderr(&output).starts_with("climon2: error: usage: c2 "),
            "{output:?}"
        );
    }
}

/// With no arguments it is the monitor, which needs a terminal.
#[test]
fn no_arguments_start_the_monitor() {
    let output = run(&[], "");
    assert_eq!(output.status.code(), Some(125), "{output:?}");
    assert_eq!(
        stderr(&output),
        "climon2: error: c2 monitor needs a terminal\n"
    );
}

/// CM10-11, PAIR-10: the PIN is one line of stdin; one that is not 8 digits
/// fails with 125 before anything is dialled, and no variable carries one.
#[test]
fn a_pin_that_is_not_8_digits_fails_before_any_dial() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap().to_string();
    for stdin in ["", "1234567\n", "123456789\n", "1234-567x\n"] {
        let output = run(&["pair", &address], stdin);
        assert_eq!(output.status.code(), Some(125), "{stdin:?}: {output:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(
            stderr(&output),
            "climon2: error: a PIN is 8 digits; hyphens and spaces are ignored\n",
            "{stdin:?}"
        );
    }
    assert!(listener.accept().is_err(), "nothing dialled the address");
}

/// CM24 (PAIR-04): an address nobody answers is a failure with 125, named.
#[test]
fn an_unreachable_relay_fails_with_125() {
    let address = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().to_string()
    };
    let output = run(&["pair", &address], "1234-5678\n");
    assert_eq!(output.status.code(), Some(125), "{output:?}");
    assert!(output.stdout.is_empty());
    let text = stderr(&output);
    assert!(
        text.starts_with(&format!(
            "climon2: error: cannot reach the relay at {address}: "
        )),
        "{text}"
    );
}
