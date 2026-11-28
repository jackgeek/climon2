//! #499 (spec #488, RLY-01, X1-2): the packaged relay, reached as `c2 relay`
//! or `c2 r` through the installed launcher, from an outer shell in a real
//! terminal with an isolated home. The relay executable's own behaviour is
//! tested in `climon2-relay/tests/relay.rs`; this proves the release ships it
//! and the launcher and the CLI reach it.
#![cfg(any(target_os = "macos", target_os = "linux"))]

#[allow(dead_code)]
mod support;

use std::{
    fs,
    net::{SocketAddr, TcpStream},
    path::Path,
};
use support::{Package, TIMEOUT, assert_released, outer, scripted};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const ROWS: u16 = 10;
const COLS: u16 = 80;

/// The executable a process runs: the kernel's record on Linux, `ps`'s full
/// command path on macOS.
fn executable(pid: i32) -> String {
    if cfg!(target_os = "linux") {
        return fs::read_link(format!("/proc/{pid}/exe"))
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    let output = std::process::Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The executables of the processes this test owns that run a relay.
fn relays(package: &Package) -> Vec<String> {
    package
        .isolation
        .owned_pids()
        .into_iter()
        .map(executable)
        .filter(|command| command.ends_with("/climon2-relay"))
        .collect()
}

/// `c2 relay` and `c2 r` run the installed version's `climon2-relay`, which
/// listens, shows the bound address, and stops from its menu with status 0,
/// handing the terminal back to the shell.
#[test]
fn installed_relay_and_r_run_the_versions_relay() {
    let package = Package::new();
    let installed = package
        .executable_home()
        .canonicalize()
        .unwrap()
        .join(VERSION)
        .join("climon2-relay");
    for command in ["c2 relay --listen 127.0.0.1:0", "c2 r --listen 127.0.0.1:0"] {
        let mut terminal = outer(&package, Path::new("/bin/bash"), command, ROWS, COLS);
        // The key bar on the last row is drawn last.
        terminal
            .wait_for_screen(TIMEOUT, |screen| {
                let rows: Vec<_> = screen.lines().map(str::trim_end).collect();
                rows.first() == Some(&" Listening") && rows.last() == Some(&" s Stop")
            })
            .unwrap();
        let address: SocketAddr = terminal.row(1).trim().parse().unwrap();
        TcpStream::connect_timeout(&address, TIMEOUT).expect("the relay listens");
        assert_eq!(relays(&package), [installed.to_str().unwrap()], "{command}");
        terminal.send("s").unwrap();
        assert_released(&package, &mut terminal, 0);
    }
    package.assert_clean();
}

/// RLY-01 (X4): without a terminal, the installed relay refuses to start,
/// and its status and diagnostic come back through the launcher and the CLI.
#[test]
fn installed_relay_refuses_to_start_without_a_terminal() {
    let package = Package::new();
    let output = scripted(&package, "c2", &["relay", "--listen", "127.0.0.1:0"]);
    assert_eq!(output.status.code(), Some(125), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("climon2: error: c2 relay needs a terminal (logs: "),
        "{stderr}"
    );
    package.assert_clean();
}
