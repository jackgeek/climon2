//! Headless sessions (spec #73, ticket #78) through the installed product on
//! macOS and GNU/Linux: the confirmed headless-creation scenarios and the
//! Monitor-layer cases of HDL-01. Each test owns the headless session it
//! starts, ends it by command completion or `c2 kill`, and finishes with
//! `assert_clean()`; a failing test's `Package` kills what it owns. Every
//! session command ends itself after two minutes, so even a test killed
//! before cleaning up cannot leave a headless session running forever.
#![cfg(any(target_os = "macos", target_os = "linux"))]

#[allow(dead_code)]
mod support;

use climon2_test_term::{Terminal, process_command};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Output, Stdio},
    thread,
    time::{Duration, Instant},
};
use support::{Package, TIMEOUT};

/// A bash loop that waits for `$HOME/<name>`, giving up after two minutes.
fn until_file(name: &str) -> String {
    format!(r#"until [ -e "$HOME/{name}" ] || [ "$SECONDS" -ge 120 ]; do sleep 0.05; done"#)
}

/// Reports the command's initial terminal size, then runs until `finish`
/// exists and exits 3.
fn size_then_hold() -> String {
    format!(
        "stty size > \"$HOME/size.tmp\" && mv \"$HOME/size.tmp\" \"$HOME/size\"\n{}\nexit 3",
        until_file("finish")
    )
}

/// Waits for `go`, outlives the five-second initial attach window, announces
/// it and runs until `finish` exists. A session ended by that window hangs up
/// its terminal, which kills this script before it writes `survived`.
fn outlive_the_window() -> String {
    format!(
        "{}\nsleep 6\necho HEADLESS-ALIVE\n: > \"$HOME/survived\"\n{}\nexit 3",
        until_file("go"),
        until_file("finish")
    )
}

/// `c2 <args>` as a script runs it: stdin from /dev/null, stdout and stderr
/// into pipes, so none of them is a terminal.
fn scripted(package: &Package, args: &[&str]) -> Output {
    scripted_to(package, args, Stdio::piped())
}

/// Like [`scripted`], with stdout going to `stdout`. A CLI still running at
/// the deadline is killed and reaped, and the test fails.
fn scripted_to(package: &Package, args: &[&str], stdout: Stdio) -> Output {
    let isolated = package.command("c2");
    let mut cli = process_command(package.executable_home().join("c2"))
        .args(args)
        .env_clear()
        .envs(isolated.iter_full_env_as_str())
        .current_dir(package.isolation.home())
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while cli.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            cli.kill().unwrap();
            cli.wait().unwrap();
            panic!("c2 {args:?} was still running after {TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
    cli.wait_with_output().unwrap()
}

fn headless_args(script: &str) -> [&str; 5] {
    ["exec", "--headless", "/bin/bash", "-c", script]
}

/// The session the CLI created: exactly the full lowercase GUID and a newline
/// on stdout, status 0 and nothing on stderr.
fn created(package: &Package, output: &Output) -> String {
    let id = package.metadata().session_id.to_string();
    assert_eq!(
        (output.status.code(), output.stderr.as_slice()),
        (Some(0), &b""[..]),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), format!("{id}\n"));
    let hyphens: Vec<_> = id.match_indices('-').map(|(index, _)| index).collect();
    assert_eq!(hyphens, [8, 13, 18, 23], "{id}");
    assert!(
        id.len() == 36
            && id
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f' | b'-')),
        "{id}"
    );
    id
}

/// The listed session's `state` object, after checking it is `id`.
fn listed_state(package: &Package, id: &str) -> Value {
    let sessions = package.list();
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    assert_eq!(sessions[0]["session_id"], id);
    sessions[0]["state"].clone()
}

fn detached() -> Value {
    json!({"lifecycle": "detached", "attachments": 0, "controlled": false})
}

fn wait_file(path: &Path) -> String {
    let deadline = Instant::now() + TIMEOUT;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
    fs::read_to_string(path).unwrap()
}

fn touch(package: &Package, name: &str) {
    fs::write(package.isolation.home().join(name), "").unwrap();
}

fn attach(package: &Package, id: &str) -> Terminal {
    let mut command = package.command("c2");
    command.args(["attach", id]);
    Terminal::spawn(command, 30, 120).unwrap()
}

/// Detaches `client` with the escape prefix and `d`. A client drops keys typed
/// before it confirmed its restoration, and nothing on the screen says when it
/// has, so the keys are retyped until the client exits.
fn detach(client: &mut Terminal, id: &str) {
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        let _ = client.send(b"\x1dd");
        if let Ok(status) = client.wait_exit(Duration::from_secs(1)) {
            break status;
        }
        assert!(Instant::now() < deadline, "{}", client.screen());
    };
    assert_eq!(status, 0, "{}", client.screen());
    client
        .wait_for_bytes(
            format!("climon2: detached from session {id}; reattach with: climon2 attach {id}")
                .as_bytes(),
            TIMEOUT,
        )
        .unwrap();
}

/// `c2 kill` keeps its existing semantics for a headless session: silent,
/// status 0, confirmed only after the session is gone.
fn kill_session(package: &Package, id: &str) {
    let output = scripted(package, &["kill", id]);
    assert_eq!(
        (
            output.status.code(),
            output.stdout.as_slice(),
            output.stderr.as_slice()
        ),
        (Some(0), &b""[..], &b""[..])
    );
    assert!(package.list().is_empty());
}

/// Confirmed scenario (headless GUID output, no-TTY 80x24 start) and HDL-01
/// (Monitor layer), creation and completion cases. From a script with no
/// terminal, `c2 exec --headless` prints exactly the session's full lowercase
/// GUID and a newline and exits 0 only once the daemon confirmed creation: the
/// session is already listed, `detached` with no client, when the CLI
/// returns. Its command starts at 80x24, its later status 3 is not the CLI's,
/// and its completion ends the session with zero clients.
#[test]
fn installed_headless_exec_from_a_script_prints_only_the_guid_and_starts_at_80x24() {
    let package = Package::new();
    let output = scripted(&package, &headless_args(&size_then_hold()));
    let id = created(&package, &output);
    assert_eq!(listed_state(&package, &id), detached());
    assert_eq!(wait_file(&package.isolation.home().join("size")), "24 80\n");

    touch(&package, "finish");
    package.assert_clean();
    assert!(package.list().is_empty());
}

/// Confirmed scenario (headless TTY size) and HDL-01 (Monitor layer), Kill case:
/// with a terminal on stdin, the session starts at that terminal's size; the
/// GUID is the only output; existing `c2 kill` ends the headless session.
#[test]
fn installed_headless_exec_from_a_terminal_starts_at_its_size_and_kill_ends_it() {
    let package = Package::new();
    let mut command = package.command("c2");
    command.args(headless_args(&size_then_hold()));
    let mut terminal = Terminal::spawn(command, 31, 97).unwrap();
    assert_eq!(
        terminal.wait_exit(TIMEOUT).unwrap(),
        0,
        "{}",
        terminal.screen()
    );
    let id = package.metadata().session_id.to_string();
    assert_eq!(terminal.screen().trim(), id);
    drop(terminal);
    assert_eq!(wait_file(&package.isolation.home().join("size")), "31 97\n");

    kill_session(&package, &id);
    package.assert_clean();
}

/// HDL-01 (Monitor layer), failed creation: headless creation failures keep
/// exec's exit statuses and diagnostics, print nothing on stdout and leave no
/// session behind: 127 for a missing command, 126 for one that cannot be
/// executed, and the climon2 error code 125 for invalid configuration.
#[test]
fn installed_headless_creation_failures_keep_exec_statuses_and_diagnostics() {
    let package = Package::new();
    let unexecutable = package.isolation.home().join("not-executable");
    fs::write(&unexecutable, "not executable").unwrap();
    let invalid = r#"{"scrollback":{"max_rows":100001}}"#;
    for (command, config, status, diagnostic) in [
        (
            "certainly-no-such-climon2-command",
            None,
            127,
            "command not found",
        ),
        (
            unexecutable.to_str().unwrap(),
            None,
            126,
            "command cannot be executed",
        ),
        ("/bin/bash", Some(invalid), 125, "scrollback.max_rows"),
    ] {
        if let Some(config) = config {
            fs::write(
                package.isolation.climon2_home().join("config.jsonc"),
                config,
            )
            .unwrap();
        }
        let output = scripted(&package, &["exec", "--headless", command]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(status), "{stderr}");
        assert!(output.stdout.is_empty(), "{command}");
        assert!(stderr.starts_with("climon2: error: "), "{stderr}");
        assert!(stderr.contains(diagnostic), "{stderr}");
        assert!(stderr.contains(" (logs: "), "{stderr}");
        assert_eq!(stderr.lines().count(), 1, "{stderr}");
        package.assert_clean();
    }
}

/// HDL-01 (Monitor layer), failed creation: a script that cannot read the
/// session ID could not target the session, so when stdout cannot be written,
/// headless exec ends the session it started and fails with 125, leaving
/// nothing behind.
#[test]
fn installed_headless_exec_that_cannot_print_the_guid_ends_the_session_and_fails() {
    let package = Package::new();
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let output = scripted_to(
        &package,
        &headless_args(&until_file("finish")),
        writer.into(),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(125), "{stderr}");
    assert!(stderr.starts_with("climon2: error: "), "{stderr}");
    assert!(stderr.contains("Broken pipe"), "{stderr}");
    assert!(stderr.contains(" (logs: "), "{stderr}");
    package.assert_clean();
    assert!(package.list().is_empty());
}

/// HDL-01 (Monitor layer), zero-client lifetime while attachments come and
/// go: a headless session survives with zero clients past the five-second
/// initial attach window, after its only attached client is killed and after
/// a client deliberately detaches, staying `detached` each time; then `c2 kill`
/// ends it. The window is measured from the first listing, which the daemon
/// only answers once serving.
#[test]
fn installed_headless_session_survives_zero_clients_a_killed_client_and_a_detach() {
    let package = Package::new();
    let output = scripted(&package, &headless_args(&outlive_the_window()));
    let id = created(&package, &output);
    assert_eq!(listed_state(&package, &id), detached());
    touch(&package, "go");
    wait_file(&package.isolation.home().join("survived"));
    assert_eq!(listed_state(&package, &id), detached());

    let mut client = attach(&package, &id);
    client.wait_for_text("HEADLESS-ALIVE", TIMEOUT).unwrap();
    kill(Pid::from_raw(package.client_pid()), Signal::SIGKILL).unwrap();
    assert_eq!(client.wait_exit(TIMEOUT).unwrap(), 137);
    drop(client);
    // The killed client's connection closed before this listing connected.
    assert_eq!(listed_state(&package, &id), detached());

    let mut client = attach(&package, &id);
    client.wait_for_text("HEADLESS-ALIVE", TIMEOUT).unwrap();
    detach(&mut client, &id);
    drop(client);
    assert_eq!(listed_state(&package, &id), detached());

    kill_session(&package, &id);
    package.assert_clean();
}
