//! Headless sessions (#73, ticket #78) through the installed product: the
//! confirmed headless-creation scenario and the Monitor-layer case of HDL-01.
//! Each test owns the headless session it starts, ends it by command
//! completion or `climon2 kill`, and finishes with `assert_clean()`.
#![cfg(target_os = "macos")]

#[allow(dead_code)]
mod support;

use climon2_test_term::Terminal;
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
use support::{Package, TIMEOUT};

/// Reports the command's initial terminal size, then runs until `finish` exists.
const SIZE_THEN_HOLD: &str = r#"stty size > "$HOME/size.tmp" && mv "$HOME/size.tmp" "$HOME/size"
until [ -e "$HOME/finish" ]; do sleep 0.05; done
exit 3"#;

/// Waits for `go`, outlives the five-second initial attach window, announces
/// it and runs until `finish` exists. A session ended by that window hangs up
/// its terminal, which kills this script before it writes `survived`.
const OUTLIVE_THE_WINDOW: &str = r#"until [ -e "$HOME/go" ]; do sleep 0.05; done
sleep 6
echo HEADLESS-ALIVE
: > "$HOME/survived"
until [ -e "$HOME/finish" ]; do sleep 0.05; done
exit 3"#;

/// `climon2 <args>` as a script runs it: stdin from /dev/null and stdout and
/// stderr into pipes, so none of them is a terminal.
fn scripted(package: &Package, args: &[&str]) -> Output {
    scripted_to(package, args, Stdio::piped())
}

fn scripted_to(package: &Package, args: &[&str], stdout: Stdio) -> Output {
    let isolated = package.command("climon2");
    Command::new(package.executable_home().join("climon2"))
        .args(args)
        .env_clear()
        .envs(isolated.iter_full_env_as_str())
        .current_dir(package.isolation.home())
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
        .wait_with_output()
        .unwrap()
}

fn headless_args(script: &str) -> [&str; 5] {
    ["exec", "--headless", "/bin/bash", "-c", script]
}

/// The session the CLI created: exactly the full lowercase GUID and a
/// newline on stdout, status 0 and nothing on stderr.
fn created(package: &Package, output: &Output) -> String {
    let id = package.metadata().session_id.to_string();
    assert_eq!(
        (output.status.code(), output.stderr.as_slice()),
        (Some(0), &b""[..]),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), format!("{id}\n"));
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

/// `climon2 kill` keeps its existing semantics for a headless session: silent,
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
/// terminal, `climon2 exec --headless` prints exactly the session's GUID and a
/// newline and exits 0 only once the daemon confirmed creation: the session is
/// already listed, `detached` with no client, when the CLI returns. Its command
/// starts at 80x24, its later status 3 is not the CLI's, and its completion
/// ends the session with zero clients.
#[test]
fn installed_headless_exec_from_a_script_prints_only_the_guid_and_starts_at_80x24() {
    let package = Package::new();
    let output = scripted(&package, &headless_args(SIZE_THEN_HOLD));
    let id = created(&package, &output);
    assert_eq!(listed_state(&package, &id), detached());
    assert_eq!(wait_file(&package.isolation.home().join("size")), "24 80\n");

    touch(&package, "finish");
    package.assert_clean();
    assert!(package.list().is_empty());
}

/// Confirmed scenario (headless TTY size) and HDL-01 (Monitor layer), Kill case:
/// with a terminal on stdin, the session starts at that terminal's size; the
/// GUID is the only output; existing `climon2 kill` ends the headless session.
#[test]
fn installed_headless_exec_from_a_terminal_starts_at_its_size_and_kill_ends_it() {
    let package = Package::new();
    let mut command = package.command("climon2");
    command.args(headless_args(SIZE_THEN_HOLD));
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
/// headless exec ends the session it started and fails with 125.
#[test]
fn installed_headless_exec_that_cannot_print_the_guid_ends_the_session_and_fails() {
    let package = Package::new();
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let output = scripted_to(&package, &headless_args(SIZE_THEN_HOLD), writer.into());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(125), "{stderr}");
    assert!(stderr.starts_with("climon2: error: "), "{stderr}");
    assert!(stderr.contains("Broken pipe"), "{stderr}");
    assert!(stderr.contains(" (logs: "), "{stderr}");
    package.assert_clean();
    assert!(package.list().is_empty());
}

/// HDL-01 (Monitor layer), zero-client lifetime: a headless session survives
/// with zero clients past the five-second initial attach window and after its
/// only attached client is killed, then `climon2 kill` ends it. The window is
/// measured from the first listing, which the daemon only answers once serving.
#[test]
fn installed_headless_session_survives_zero_clients_and_a_killed_client() {
    let package = Package::new();
    let output = scripted(&package, &headless_args(OUTLIVE_THE_WINDOW));
    let id = created(&package, &output);
    assert_eq!(listed_state(&package, &id), detached());
    touch(&package, "go");
    wait_file(&package.isolation.home().join("survived"));
    assert_eq!(listed_state(&package, &id), detached());

    let mut command = package.command("climon2");
    command.args(["attach", &id]);
    let mut client = Terminal::spawn(command, 30, 120).unwrap();
    client.wait_for_text("HEADLESS-ALIVE", TIMEOUT).unwrap();
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(package.client_pid()),
        nix::sys::signal::Signal::SIGKILL,
    )
    .unwrap();
    assert_eq!(client.wait_exit(TIMEOUT).unwrap(), 137);
    drop(client);
    // The killed client's connection closed before this listing connected.
    assert_eq!(listed_state(&package, &id), detached());

    kill_session(&package, &id);
    package.assert_clean();
}
