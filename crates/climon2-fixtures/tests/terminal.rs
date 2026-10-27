use climon2_test_term::{Isolation, Terminal, bin_path};
use std::{fs, time::Duration};

const SECOND: Duration = Duration::from_secs(1);

/// #21/#31: real PTY output, exact argv/cwd/env, terminal identity and exit status.
#[test]
fn generic_terminal_reports_its_contract_and_exits() {
    let isolation = Isolation::new();
    let report = isolation.home().join("report.json");
    let mut command = isolation.command(bin_path("climon2-fixture"));
    command.args([
        "report",
        report.to_str().unwrap(),
        "print",
        "hello-pty",
        "exit",
        "7",
    ]);
    command.env("FIXTURE_VALUE", "spaces and unicode: \u{3bb}");
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    terminal.wait_for_text("hello-pty", SECOND).unwrap();
    terminal.wait_for_bytes(b"hello-pty", SECOND).unwrap();
    assert_eq!(terminal.wait_exit(SECOND).unwrap(), 7);
    let value: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    assert_eq!(
        value["cwd"],
        isolation.home().canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(value["env"]["CLIMON2_TEST_ID"], isolation.id());
    assert_eq!(value["env"]["FIXTURE_VALUE"], "spaces and unicode: \u{3bb}");
    assert_eq!(value["size"], serde_json::json!([24, 80]));
    assert_eq!(value["isatty"], serde_json::json!([true, true, true]));
    assert_eq!(value["pid"], value["pgid"]);
    assert_eq!(value["argv"][3], "hello-pty");
    drop(terminal);
    isolation.assert_clean();
}

/// #21/#31: kernel resize reaches the fixture; signal exits preserve 128+signal.
#[test]
fn generic_terminal_observes_resize_and_signal_exit() {
    let isolation = Isolation::new();
    let report = isolation.home().join("resized.json");
    let mut command = isolation.command(bin_path("climon2-fixture"));
    command.args([
        "await-resize",
        "report",
        report.to_str().unwrap(),
        "kill-self",
        "TERM",
    ]);
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    terminal.wait_for_text("resize-ready", SECOND).unwrap();
    terminal.resize(31, 97).unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(2)).unwrap(), 143);
    let value: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    assert_eq!(value["size"], serde_json::json!([31, 97]));
    drop(terminal);
    isolation.assert_clean();
}

/// #21/#31: writer stays alive, waits are bounded and failure output aids diagnosis.
#[test]
fn timed_waits_dump_screen_and_bytes_and_drop_cleans_ignored_sighup() {
    let isolation = Isolation::new();
    let mut command = isolation.command(bin_path("climon2-fixture"));
    command.args([
        "ignore-sighup",
        "kill-self",
        "HUP",
        "print",
        "still-alive",
        "sleep",
        "5000",
    ]);
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    terminal.wait_for_text("still-alive", SECOND).unwrap();
    for result in [
        terminal.wait_for_text("missing", Duration::from_millis(100)),
        terminal.wait_for_bytes(b"absent", Duration::from_millis(100)),
        terminal.wait_exit(Duration::from_millis(100)).map(|_| ()),
    ] {
        let error = result.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        let message = error.to_string();
        assert!(
            message.contains("SCREEN:")
                && message.contains("RECENT BYTES:")
                && message.contains("still-alive"),
            "{message}"
        );
    }
    assert!(std::panic::catch_unwind(|| isolation.assert_clean()).is_err());
    drop(terminal);
    isolation.assert_clean();
}

/// #21/#31: metadata and sockets are leaks; diagnostic logs may remain.
#[test]
fn leak_check_rejects_metadata_and_sockets_but_keeps_logs() {
    use std::{
        io::Write,
        os::unix::net::UnixListener,
        process::{Command, Stdio},
    };
    let isolation = Isolation::new();
    let sessions = isolation.climon2_home().join("sessions");
    fs::create_dir(&sessions).unwrap();
    fs::write(sessions.join("session.log"), "diagnostic").unwrap();
    isolation.assert_clean();
    let metadata = sessions.join("session.pty");
    fs::write(&metadata, "{}").unwrap();
    assert!(std::panic::catch_unwind(|| isolation.assert_clean()).is_err());
    fs::remove_file(metadata).unwrap();
    // Independent system SHA-256 implementation supplies the #18 runtime-path oracle.
    let mut digest = Command::new("/usr/bin/shasum")
        .args(["-a", "256"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    digest
        .stdin
        .take()
        .unwrap()
        .write_all(
            isolation
                .climon2_home()
                .canonicalize()
                .unwrap()
                .as_os_str()
                .as_encoded_bytes(),
        )
        .unwrap();
    let digest = digest.wait_with_output().unwrap();
    assert!(digest.status.success());
    let digest = String::from_utf8(digest.stdout).unwrap();
    let runtime = std::path::PathBuf::from(format!(
        "/tmp/climon2-{}-{}",
        nix::unistd::getuid(),
        &digest[..8]
    ));
    fs::create_dir(&runtime).unwrap();
    let socket = runtime.join("test.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    assert!(std::panic::catch_unwind(|| isolation.assert_clean()).is_err());
    drop(listener);
    fs::remove_file(socket).unwrap();
    fs::remove_dir(runtime).unwrap();
    isolation.assert_clean();
}
