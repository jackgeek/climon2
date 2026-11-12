#![cfg(any(target_os = "macos", target_os = "linux"))]

use climon2_core::{
    SessionId,
    hello::{Lifecycle, SessionState, advertise},
    metadata::SessionMetadata,
    protocol::{Frame, HelloReply, PROTOCOL_VERSION, read_frame, write_frame},
};
use climon2_platform::{process, runtime};
use climon2_test_term::{Isolation, Terminal, bin_path, copy_executable};
use std::{
    fs,
    path::{Path, PathBuf},
    thread::{self, JoinHandle},
    time::Duration,
};

/// A metadata file plus a socket that answers exactly one hello probe.
struct FakeSession {
    metadata: SessionMetadata,
    path: PathBuf,
    server: Option<JoinHandle<runtime::SocketListener>>,
}

impl FakeSession {
    fn start(home: &Path, suffix: u8, reply: fn(&SessionMetadata) -> Frame) -> Self {
        let metadata = metadata(home, suffix);
        fs::create_dir_all(home.join("sessions")).unwrap();
        let path = home
            .join("sessions")
            .join(format!("{}.pty", metadata.session_id));
        fs::write(&path, metadata.to_json().unwrap()).unwrap();
        let listener = runtime::bind(Path::new(metadata.socket_path.as_str())).unwrap();
        let frame = reply(&metadata);
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            read_frame(&mut socket).unwrap();
            write_frame(&mut socket, &frame).unwrap();
            listener
        });
        Self {
            metadata,
            path,
            server: Some(server),
        }
    }

    fn assert_only_probed(&mut self) {
        let listener = self.server.take().unwrap().join().unwrap();
        listener.set_nonblocking(true).unwrap();
        assert!(matches!(
            listener.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
        assert_eq!(
            fs::read(&self.path).unwrap(),
            self.metadata.to_json().unwrap()
        );
    }
}

impl Drop for FakeSession {
    fn drop(&mut self) {
        if !thread::panicking()
            && let Some(server) = self.server.take()
        {
            server.join().unwrap();
        }
        let _ = fs::remove_file(self.metadata.socket_path.as_str());
        let _ = fs::remove_file(&self.path);
    }
}

fn metadata(home: &Path, suffix: u8) -> SessionMetadata {
    let id: SessionId = format!("12345678-1234-4234-8234-123456789a{suffix:02x}")
        .parse()
        .unwrap();
    let socket = runtime::RuntimeDirectory::prepare(home)
        .unwrap()
        .socket_path(&id)
        .unwrap();
    serde_json::from_value(serde_json::json!({
        "format_version":1,"session_id":id.as_str(),"socket_path":socket,
        "daemon_pid":std::process::id(),
        "daemon_start_time":process::start_time(std::process::id()).unwrap().unwrap(),
        "command":"bash","args":[],"cwd":home,"depth":1,
        "created_at":"2026-10-28T07:21:02.930Z","climon2_version":"999.0.0"
    }))
    .unwrap()
}

fn reply(metadata: &SessionMetadata) -> HelloReply {
    HelloReply {
        session_id: metadata.session_id.clone(),
        climon2_version: metadata.climon2_version.clone(),
        protocol: PROTOCOL_VERSION,
    }
}

/// An older daemon: the mandatory reply only, so no capabilities.
fn legacy(metadata: &SessionMetadata) -> Frame {
    let body = serde_json::to_vec(&reply(metadata)).unwrap();
    Frame::new(1, body).unwrap()
}

fn current(metadata: &SessionMetadata) -> Frame {
    let state = SessionState::new(Lifecycle::Detached, 0, false).unwrap();
    advertise(&reply(metadata), state).unwrap()
}

/// Runs `climon2 attach` in a terminal and returns its status and screen.
fn attach_in_terminal(
    isolation: &Isolation,
    cli: &Path,
    selector: &str,
    session: Option<&SessionId>,
) -> (i32, String) {
    let mut command = isolation.command(cli);
    command.args(["attach", selector]);
    if let Some(id) = session {
        command.env("CLIMON2_SESSION_ID", id.as_str());
    }
    let mut terminal = Terminal::spawn(command, 24, 200).unwrap();
    let status = terminal.wait_exit(Duration::from_secs(30)).unwrap();
    (status, terminal.screen())
}

fn home(isolation: &Isolation) -> PathBuf {
    runtime::canonical_home(&isolation.climon2_home()).unwrap()
}

#[test]
fn a_selector_must_match_exactly_one_live_session() {
    let isolation = Isolation::new();
    let cli = bin_path("climon2-cli");
    let (status, screen) = attach_in_terminal(&isolation, &cli, "a", None);
    assert_eq!(status, 125);
    assert!(screen.contains("no live session matches a"), "{screen}");

    let home = home(&isolation);
    let mut first = FakeSession::start(&home, 1, current);
    let mut second = FakeSession::start(&home, 2, current);
    let (status, screen) = attach_in_terminal(&isolation, &cli, "1", None);
    assert_eq!(status, 125);
    assert!(screen.contains("is ambiguous"), "{screen}");
    assert!(
        screen.contains(first.metadata.session_id.as_str()),
        "{screen}"
    );
    assert!(
        screen.contains(second.metadata.session_id.as_str()),
        "{screen}"
    );
    first.assert_only_probed();
    second.assert_only_probed();
    drop((first, second));
    isolation.assert_clean();
}

#[test]
fn a_session_without_attach_capabilities_is_refused() {
    let isolation = Isolation::new();
    let session = FakeSession::start(&home(&isolation), 3, legacy);
    let cli = bin_path("climon2-cli");
    let (status, screen) = attach_in_terminal(&isolation, &cli, "12345678", None);
    assert_eq!(status, 125);
    let expected = format!(
        "session {} does not support attach and client startup",
        session.metadata.session_id
    );
    assert!(screen.contains(&expected), "{screen}");
    drop(session);
    isolation.assert_clean();
}

#[test]
fn a_session_cannot_attach_to_itself() {
    let isolation = Isolation::new();
    let session = FakeSession::start(&home(&isolation), 4, current);
    let id = session.metadata.session_id.clone();
    let cli = bin_path("climon2-cli");
    let (status, screen) = attach_in_terminal(&isolation, &cli, "12345678", Some(&id));
    assert_eq!(status, 125);
    let expected = format!("cannot attach to the current session {id}");
    assert!(screen.contains(&expected), "{screen}");
    drop(session);
    isolation.assert_clean();
}

#[test]
fn attach_needs_a_terminal() {
    let isolation = Isolation::new();
    let session = FakeSession::start(&home(&isolation), 5, current);
    let output = std::process::Command::new(bin_path("climon2-cli"))
        .env_clear()
        .env("HOME", isolation.home())
        .env("CLIMON2_HOME", isolation.climon2_home())
        .args(["attach", "12345678"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(125));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("climon2 attach needs a terminal"), "{error}");
    drop(session);
    isolation.assert_clean();
}

#[test]
fn a_session_of_an_uninstalled_version_names_that_version() {
    let isolation = Isolation::new();
    let cli = installed_cli(&isolation);
    let home = home(&isolation);
    let session = FakeSession::start(&home, 6, current);
    let non_live = metadata(&home, 16);
    let path = home
        .join("sessions")
        .join(format!("{}.pty", non_live.session_id));
    fs::write(&path, non_live.to_json().unwrap()).unwrap();
    let (status, screen) = attach_in_terminal(&isolation, &cli, "1", None);
    assert_eq!(status, 125);
    let expected = format!(
        "session {} needs climon2 999.0.0, which is not installed",
        session.metadata.session_id
    );
    assert!(screen.contains(&expected), "{screen}");
    fs::remove_file(path).unwrap();
    drop(session);
    isolation.assert_clean();
}

/// `climon2 kill` without a terminal: (status, stdout, stderr).
fn kill(isolation: &Isolation, cli: &Path, selector: &str) -> (i32, String, String) {
    let output = std::process::Command::new(cli)
        .env_clear()
        .env("HOME", isolation.home())
        .env("CLIMON2_HOME", isolation.climon2_home())
        .args(["kill", selector])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

fn installed_cli(isolation: &Isolation) -> PathBuf {
    let installed = isolation.home().join("exe/0.1.0");
    fs::create_dir_all(&installed).unwrap();
    let cli = installed.join("climon2-cli");
    copy_executable(&bin_path("climon2-cli"), &cli);
    cli
}

#[test]
fn kill_selects_exactly_one_live_session() {
    let isolation = Isolation::new();
    let cli = bin_path("climon2-cli");
    let (status, stdout, stderr) = kill(&isolation, &cli, "a");
    assert_eq!((status, stdout.as_str()), (125, ""));
    assert!(stderr.contains("no live session matches a"), "{stderr}");

    let home = home(&isolation);
    let mut first = FakeSession::start(&home, 7, current);
    let mut second = FakeSession::start(&home, 8, current);
    let (status, stdout, stderr) = kill(&isolation, &cli, "1");
    assert_eq!((status, stdout.as_str()), (125, ""));
    assert!(stderr.contains("is ambiguous"), "{stderr}");
    assert!(stderr.contains(first.metadata.session_id.as_str()));
    assert!(stderr.contains(second.metadata.session_id.as_str()));
    first.assert_only_probed();
    second.assert_only_probed();
    drop((first, second));
    isolation.assert_clean();
}

#[test]
fn kill_refuses_an_older_session_without_capabilities() {
    let isolation = Isolation::new();
    let session = FakeSession::start(&home(&isolation), 9, legacy);
    let (status, stdout, stderr) = kill(&isolation, &bin_path("climon2-cli"), "12345678");
    assert_eq!((status, stdout.as_str()), (125, ""));
    let expected = format!(
        "climon2: error: session {} does not support kill",
        session.metadata.session_id
    );
    assert!(stderr.starts_with(&expected), "{stderr}");
    drop(session);
    isolation.assert_clean();
}

/// Kill never reads the configuration, so a malformed file cannot stop it.
#[test]
fn kill_of_an_uninstalled_version_names_that_version_and_ignores_config() {
    let isolation = Isolation::new();
    let cli = installed_cli(&isolation);
    let home = home(&isolation);
    fs::write(home.join("config.jsonc"), "{ malformed").unwrap();
    let session = FakeSession::start(&home, 10, current);
    let non_live = metadata(&home, 17);
    let path = home
        .join("sessions")
        .join(format!("{}.pty", non_live.session_id));
    fs::write(&path, non_live.to_json().unwrap()).unwrap();
    let (status, _, stderr) = kill(&isolation, &cli, "1");
    assert_eq!(status, 125);
    let expected = format!(
        "session {} needs climon2 999.0.0, which is not installed",
        session.metadata.session_id
    );
    assert!(stderr.contains(&expected), "{stderr}");
    fs::remove_file(path).unwrap();
    drop(session);
    isolation.assert_clean();
}

/// Attach validates the client section only: an unused bad retention is ignored,
/// but a malformed file still fails before dispatch.
#[test]
fn attach_validates_only_the_client_configuration() {
    let isolation = Isolation::new();
    let cli = installed_cli(&isolation);
    let home = home(&isolation);
    let config = home.join("config.jsonc");
    fs::write(&config, r#"{"scrollback":{"max_rows":100001}}"#).unwrap();
    let session = FakeSession::start(&home, 11, current);
    let (status, screen) = attach_in_terminal(&isolation, &cli, "12345678", None);
    assert_eq!(status, 125);
    assert!(screen.contains("which is not installed"), "{screen}");
    drop(session);

    fs::write(&config, r#"{"client":{"escape_prefix":"Alt+x"}}"#).unwrap();
    let session = FakeSession::start(&home, 12, current);
    let (status, screen) = attach_in_terminal(&isolation, &cli, "12345678", None);
    assert_eq!(status, 125);
    assert!(screen.contains("config.jsonc is invalid"), "{screen}");
    assert!(!screen.contains("which is not installed"), "{screen}");
    drop(session);
    isolation.assert_clean();
}
