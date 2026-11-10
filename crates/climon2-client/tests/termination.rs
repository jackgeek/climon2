#![cfg(any(target_os = "macos", target_os = "linux"))]
//! Kill mode (plan 1.12): no terminal, no attachment, one Kill request.

use climon2_core::protocol::{
    ClientMessage, Frame, PROTOCOL_VERSION, ServerMessage, read_frame, write_frame,
};
use std::{
    fs,
    io::Read,
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    process::{Child, Output, Stdio},
    time::{Duration, Instant},
};

const ID: &str = "01234567-89ab-4cde-8fab-0123456789ab";
const KILL: &str = r#"{"schema":1,"operation":"kill"}"#;

struct Peer {
    home: tempfile::TempDir,
    metadata: PathBuf,
    listener: UnixListener,
}

impl Peer {
    fn new() -> Self {
        let home = tempfile::Builder::new()
            .prefix("kill-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = home.path().join("peer.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let metadata = home.path().join(format!("{ID}.pty"));
        let value = serde_json::json!({
            "format_version": 1, "session_id": ID, "socket_path": socket,
            "daemon_pid": std::process::id(), "daemon_start_time": 1,
            "command": "fixture", "args": [], "cwd": home.path(),
            "depth": 1, "created_at": "2026-10-28T00:00:00.000Z",
            "climon2_version": "0.1.0"
        });
        fs::write(&metadata, serde_json::to_vec(&value).unwrap()).unwrap();
        Self {
            home,
            metadata,
            listener,
        }
    }

    /// Stdin is not a terminal: kill mode must never need one.
    fn client(&self, envelope: &str) -> Child {
        let mut command = climon2_test_term::process_command(env!("CARGO_BIN_EXE_climon2-client"));
        command
            .args(["attach", "--metadata"])
            .arg(&self.metadata)
            .env_clear()
            .env("HOME", self.home.path())
            .env("CLIMON2_HOME", self.home.path())
            .env("CLIMON2_CLIENT_STARTUP", envelope)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        command.spawn().unwrap()
    }

    /// Accepts the client and answers its hello with the given capabilities.
    fn hello(&self, capabilities: &[&str]) -> UnixStream {
        let mut stream = self.accept();
        stream
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        assert_eq!(
            receive(&mut stream),
            Some(ClientMessage::Hello {
                protocol: PROTOCOL_VERSION
            })
        );
        let body = serde_json::json!({
            "session_id": ID, "climon2_version": "0.1.0", "protocol": 1,
            "capabilities": capabilities,
        });
        write_frame(
            &mut stream,
            &Frame::new(1, serde_json::to_vec(&body).unwrap()).unwrap(),
        )
        .unwrap();
        stream
    }

    fn accept(&self) -> UnixStream {
        self.listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    return stream;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "client did not connect");
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        }
    }

    fn kill_request(&self) -> UnixStream {
        let mut stream = self.hello(&["attach", "kill", "client-startup-v1"]);
        assert_eq!(receive(&mut stream), Some(ClientMessage::Kill));
        stream
    }
}

fn receive(stream: &mut UnixStream) -> Option<ClientMessage> {
    read_frame(stream)
        .ok()
        .map(|frame| ClientMessage::from_frame(frame).unwrap())
}

fn send(stream: &mut UnixStream, message: ServerMessage) {
    write_frame(stream, &message.to_frame().unwrap()).unwrap();
}

fn finish(child: Child) -> (i32, String, String) {
    let Output {
        status,
        stdout,
        stderr,
    } = child.wait_with_output().unwrap();
    (
        status.code().unwrap(),
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[test]
fn confirmed_termination_is_silent_success_whatever_the_command_status() {
    let peer = Peer::new();
    let child = peer.client(KILL);
    let mut stream = peer.kill_request();
    send(&mut stream, ServerMessage::Terminated { code: 143 });
    assert_eq!(finish(child), (0, String::new(), String::new()));
}

#[test]
fn a_daemon_error_or_lost_connection_is_a_failed_kill() {
    let peer = Peer::new();
    let child = peer.client(KILL);
    let mut stream = peer.kill_request();
    send(
        &mut stream,
        ServerMessage::Error {
            message: "session refused".into(),
        },
    );
    let (code, stdout, stderr) = finish(child);
    assert_eq!((code, stdout.as_str()), (125, ""));
    assert!(
        stderr.starts_with("climon2: error: session refused (logs: "),
        "{stderr}"
    );

    let child = peer.client(KILL);
    drop(peer.kill_request());
    let (code, _, stderr) = finish(child);
    assert_eq!(code, 125);
    assert!(
        stderr.starts_with(&format!("climon2: error: lost connection to session {ID}")),
        "{stderr}"
    );
}

#[test]
fn kill_needs_the_kill_capability_and_never_sends_a_request_without_it() {
    let peer = Peer::new();
    for capabilities in [
        &["attach", "client-startup-v1", "state-v1"][..],
        &["kill"][..],
    ] {
        let child = peer.client(KILL);
        let mut stream = peer.hello(capabilities);
        assert_eq!(receive(&mut stream), None);
        let (code, _, stderr) = finish(child);
        assert_eq!(code, 125);
        assert!(
            stderr.starts_with("climon2: error: session does not support kill"),
            "{stderr}"
        );
    }
}

#[test]
fn a_kill_capable_session_without_attach_refuses_attach_mode() {
    let peer = Peer::new();
    let isolation = climon2_test_term::Isolation::new();
    let mut command = isolation.command(env!("CARGO_BIN_EXE_climon2-client"));
    command.args(["attach", "--metadata"]);
    command.arg(&peer.metadata);
    command.env("CLIMON2_HOME", peer.home.path());
    command.env(
        "CLIMON2_CLIENT_STARTUP",
        r#"{"schema":1,"operation":"attach","settings":{"escape_prefix":29,"take_control_key":20}}"#,
    );
    let mut terminal = climon2_test_term::Terminal::spawn(command, 24, 120).unwrap();
    let mut stream = peer.hello(&["kill", "client-startup-v1"]);
    assert_eq!(receive(&mut stream), None);
    assert_eq!(terminal.wait_exit(Duration::from_secs(5)).unwrap(), 125);
    assert!(
        terminal
            .screen()
            .contains("climon2: error: session does not support attach"),
        "{}",
        terminal.screen()
    );
}

#[test]
fn a_signal_while_waiting_for_confirmation_is_the_signal_status() {
    let peer = Peer::new();
    let mut child = peer.client(KILL);
    let _stream = peer.kill_request();
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "client ignored SIGTERM");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert_eq!(
        (child.wait().unwrap().code(), stderr.as_str()),
        (Some(143), "")
    );
}
