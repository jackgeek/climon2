#![cfg(any(target_os = "macos", target_os = "linux"))]
//! Kill mode (plan 1.12, #100): no terminal and no attachment. The client
//! executable loads its own version folder's client library and submits one
//! Kill through it, which the library sends on its own hello-validated
//! connection over daemon wire revision 1. Each test installs the client
//! executable and the workspace's client library in a private version
//! folder, and answers as a revision-1 peer whose frames are written here by
//! hand from the wire reference, never by the code under test.

use climon2_core::protocol::{ClientMessage, PROTOCOL_VERSION, read_frame};
use climon2_fixtures::stand_in;
use climon2_test_term::{client_library_path, copy_executable, process_command};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::{Child, Output, Stdio},
    time::{Duration, Instant},
};

const ID: &str = "01234567-89ab-4cde-8fab-0123456789ab";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const KILL: &str = r#"{"schema":1,"operation":"kill"}"#;
/// Longer than the library's ten-second `kill_ms` and a loaded host's
/// process start-up.
const BOUND: Duration = Duration::from_secs(40);

struct Peer {
    home: tempfile::TempDir,
    metadata: PathBuf,
    listener: UnixListener,
}

impl Peer {
    /// A session of this release whose socket the test serves, and an
    /// executable home whose version folder holds the client executable and
    /// the workspace's client library.
    fn new() -> Self {
        let home = tempfile::Builder::new()
            .prefix("kill-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = home.path().join("peer.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let metadata = home.path().join(format!("{ID}.pty"));
        let value = json!({
            "format_version": 1, "session_id": ID, "socket_path": socket,
            "daemon_pid": std::process::id(), "daemon_start_time": 1,
            "command": "fixture", "args": [], "cwd": home.path(),
            "depth": 1, "created_at": "2026-10-28T00:00:00.000Z",
            "climon2_version": VERSION
        });
        fs::write(&metadata, serde_json::to_vec(&value).unwrap()).unwrap();
        let peer = Self {
            home,
            metadata,
            listener,
        };
        fs::create_dir_all(peer.folder()).unwrap();
        copy_executable(
            Path::new(env!("CARGO_BIN_EXE_climon2-client")),
            &peer.folder().join("climon2-client"),
        );
        copy_executable(&client_library_path(), &peer.library());
        peer
    }

    fn executables(&self) -> PathBuf {
        self.home.path().join("exe")
    }

    /// The installed client's version folder.
    fn folder(&self) -> PathBuf {
        self.executables().join(VERSION)
    }

    fn library(&self) -> PathBuf {
        self.folder()
            .join(climon2_core::layout::client_library_name())
    }

    /// The installed client in kill mode, as the CLI starts it, with a
    /// cleared environment; stdin is not a terminal: kill mode must never
    /// need one.
    fn command(&self) -> std::process::Command {
        self.command_at(&self.folder().join("climon2-client"))
    }

    /// Kill mode, as [`Peer::command`], run from `executable`.
    fn command_at(&self, executable: &Path) -> std::process::Command {
        let mut command = process_command(executable);
        command
            .args(["attach", "--metadata"])
            .arg(&self.metadata)
            .env_clear()
            .env("HOME", self.home.path())
            .env("CLIMON2_HOME", self.home.path())
            .env("CLIMON2_CLIENT_STARTUP", KILL)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        command
    }

    fn client(&self) -> Child {
        self.command().spawn().unwrap()
    }

    /// Accepts the library's connection and answers its frozen hello with
    /// `extensions` merged over a reply advertising revision 1 (a null
    /// removes a member).
    fn hello(&self, extensions: Value) -> UnixStream {
        let mut stream = self.accept();
        assert_eq!(
            read_frame(&mut stream)
                .ok()
                .map(|frame| ClientMessage::from_frame(frame).unwrap()),
            Some(ClientMessage::Hello {
                protocol: PROTOCOL_VERSION
            })
        );
        let mut body = json!({
            "session_id": ID, "climon2_version": VERSION, "protocol": 1,
            "capabilities": ["attach", "kill", "state-v1", "client-startup-v1", "daemon-wire-v1"],
            "state": { "lifecycle": "detached", "attachments": 0, "controlled": false },
            "daemon_wire_v1": {
                "max_request_frame_bytes": 1_048_576, "max_server_frame_bytes": 1_048_576
            },
        });
        let reply = body.as_object_mut().unwrap();
        for (name, value) in extensions.as_object().unwrap() {
            match value {
                Value::Null => reply.remove(name),
                value => reply.insert(name.clone(), value.clone()),
            };
        }
        send(&mut stream, 1, &body);
        stream
    }

    fn accept(&self) -> UnixStream {
        self.listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + BOUND;
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(BOUND)).unwrap();
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

    /// Nothing connected to the session's socket.
    fn assert_never_connected(&self) {
        self.listener.set_nonblocking(true).unwrap();
        assert_eq!(
            self.listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    /// The Kill, wire ID 1 and revision 1, requiring nothing beyond core
    /// support, as the first and only request on its connection.
    fn kill_request(&self) -> UnixStream {
        let mut stream = self.hello(json!({}));
        assert_eq!(
            receive(&mut stream),
            Some((
                0x38,
                json!({ "request_id": "1", "revision": 1, "required_capabilities": [] })
            ))
        );
        stream
    }
}

/// One revision-1 frame: a big-endian length counting the type byte, the
/// type and a JSON body.
fn send(stream: &mut UnixStream, code: u8, body: &Value) {
    let body = body.to_string().into_bytes();
    let length = u32::try_from(body.len() + 1).unwrap();
    let frame = [&length.to_be_bytes()[..], &[code], &body].concat();
    stream.write_all(&frame).unwrap();
}

/// The next revision-1 frame's type and JSON body, or `None` once the
/// client closed the connection.
fn receive(stream: &mut UnixStream) -> Option<(u8, Value)> {
    let mut length = [0; 4];
    stream.read_exact(&mut length).ok()?;
    let mut frame = vec![0; u32::from_be_bytes(length) as usize];
    stream.read_exact(&mut frame).unwrap();
    Some((frame[0], serde_json::from_slice(&frame[1..]).unwrap()))
}

/// The daemon's Response to the Kill.
fn answer(stream: &mut UnixStream, outcome: &str, result: Value, error: Value) {
    send(
        stream,
        0x40,
        &json!({ "request_id": "1", "outcome": outcome, "result": result, "error": error }),
    );
}

fn termination(command: Value, cleanup: &str) -> Value {
    json!({ "kind": "termination", "command_result": command, "cleanup": cleanup })
}

fn exit(code: u32) -> Value {
    json!({ "kind": "exit", "code": code })
}

/// The client's status and output once it exits, within `BOUND`; a client
/// still running then is killed.
fn finish(mut child: Child) -> (i32, String, String) {
    let deadline = Instant::now() + BOUND;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("client outlived {BOUND:?}: {:?}", child.wait_with_output());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
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

/// The client's one-line diagnostic of a failed kill, status 125, without
/// its logs suffix.
fn failed(child: Child) -> String {
    let (code, stdout, stderr) = finish(child);
    assert_eq!((code, stdout.as_str()), (125, ""), "{stderr}");
    stderr
        .strip_prefix("climon2: error: ")
        .and_then(|rest| rest.split_once(" (logs: "))
        .unwrap_or_else(|| panic!("{stderr}"))
        .0
        .to_owned()
}

/// LIB-01 executable case: a confirmed termination, answered with the
/// command's exit 143 and complete cleanup, is silent success, status 0
/// whatever the command's status, through the library beside the
/// executable; the Kill is sent once.
#[test]
fn confirmed_termination_is_silent_success_whatever_the_command_status() {
    let peer = Peer::new();
    let child = peer.client();
    let mut stream = peer.kill_request();
    answer(
        &mut stream,
        "ok",
        termination(exit(143), "complete"),
        Value::Null,
    );
    assert_eq!(finish(child), (0, String::new(), String::new()));
    assert_eq!(receive(&mut stream), None, "the Kill was sent again");
}

/// A daemon's refusal, a partial termination, a lost answer and an answer
/// that never comes within the ten-second `kill_ms` each fail the kill with
/// status 125 and one diagnostic: the daemon's own message, the library's
/// for the lost connection, and the CLI's for the unconfirmed termination.
/// The Kill is never sent again.
#[test]
fn a_refused_partial_lost_or_unconfirmed_kill_is_a_failed_kill() {
    let peer = Peer::new();
    let refusal = json!({ "code": "invalid_state", "message": "session refused" });
    let partial = json!({ "code": "cleanup_failed", "message": "cleanup failed" });
    for (outcome, result, error, expected) in [
        ("rejected", Value::Null, refusal, "session refused"),
        (
            "failed",
            termination(exit(143), "incomplete"),
            partial,
            "cleanup failed",
        ),
    ] {
        let child = peer.client();
        let mut stream = peer.kill_request();
        answer(&mut stream, outcome, result, error);
        assert_eq!(failed(child), expected);
        assert_eq!(receive(&mut stream), None, "the Kill was sent again");
    }

    let child = peer.client();
    drop(peer.kill_request());
    assert_eq!(failed(child), "the session daemon closed the connection");

    // `kill_ms` starts after the client does.
    let started = Instant::now();
    let child = peer.client();
    let mut stream = peer.kill_request();
    assert_eq!(
        failed(child),
        format!("termination of session {ID} was not confirmed")
    );
    assert!(started.elapsed() >= Duration::from_secs(10));
    assert_eq!(receive(&mut stream), None, "the Kill was sent again");
}

/// Kill needs daemon wire revision 1: a session whose hello does not
/// advertise it is refused with the library's diagnostic, and no request is
/// ever sent.
#[test]
fn kill_needs_daemon_wire_revision_1_and_never_sends_a_request_without_it() {
    let peer = Peer::new();
    let child = peer.client();
    let mut stream = peer.hello(json!({
        "capabilities": ["attach", "kill", "state-v1", "client-startup-v1"],
        "daemon_wire_v1": null,
    }));
    assert_eq!(receive(&mut stream), None);
    assert_eq!(
        failed(child),
        "the session daemon does not advertise daemon wire revision 1"
    );
}

/// HUP, TERM or INT while waiting for the Kill's answer returns 128+signal
/// silently, without waiting for `kill_ms`.
#[test]
fn a_signal_while_waiting_for_confirmation_is_the_signal_status() {
    let peer = Peer::new();
    let child = peer.client();
    let _stream = peer.kill_request();
    let signalled = Instant::now();
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    assert_eq!(finish(child), (143, String::new(), String::new()));
    assert!(
        signalled.elapsed() < Duration::from_secs(9),
        "waited for kill_ms"
    );
}

/// LIB-01 executable case: the client loads the library in its own version
/// folder whatever `current`, the working directory and the ambient search
/// paths name. Each of those holds a client library stand-in describing
/// this release, whose calls abort, so the kill succeeds only through the
/// library beside the executable.
#[test]
fn the_library_beside_the_executable_is_used_whatever_current_cwd_and_search_paths() {
    let peer = Peer::new();
    let decoys = peer.home.path().join("decoys");
    let other = peer.executables().join("9.9.9");
    for folder in [&decoys, &other] {
        fs::create_dir_all(folder).unwrap();
        let decoy = folder.join(climon2_core::layout::client_library_name());
        stand_in::compile(&decoy, VERSION, &[]).unwrap();
    }
    fs::write(peer.executables().join("current"), "9.9.9\n").unwrap();
    let child = peer
        .command()
        .current_dir(&decoys)
        .env("LD_LIBRARY_PATH", &decoys)
        .env("DYLD_FALLBACK_LIBRARY_PATH", &decoys)
        .spawn()
        .unwrap();
    let mut stream = peer.kill_request();
    answer(
        &mut stream,
        "ok",
        termination(exit(0), "complete"),
        Value::Null,
    );
    assert_eq!(finish(child), (0, String::new(), String::new()));
}

/// LIB-02 executable case: a missing library, one describing another
/// release, and faulty ones (no bootstrap, an incomplete function table)
/// each fail the kill with status 125 and the loader's explicit diagnostic
/// naming the library beside the executable, before anything connects to
/// the session. An executable outside any version folder (Cargo's own
/// output) has no library of its release and fails the same way.
#[test]
fn a_missing_wrong_release_or_faulty_library_fails_before_any_connection() {
    let peer = Peer::new();
    let uninstalled = peer
        .command_at(Path::new(env!("CARGO_BIN_EXE_climon2-client")))
        .spawn()
        .unwrap();
    let message = failed(uninstalled);
    assert!(
        message.starts_with("cannot locate the client library: "),
        "{message}"
    );
    peer.assert_never_connected();

    let library = peer.library();
    let named = format!("cannot load the client library {}: ", library.display());
    fs::remove_file(&library).unwrap();
    let message = failed(peer.client());
    assert!(message.starts_with(&named), "{message}");
    peer.assert_never_connected();

    for (release, options) in [
        ("0.0.1", &[][..]),
        (VERSION, &["-DNO_BOOTSTRAP"][..]),
        (VERSION, &["-DINCOMPLETE_TABLE"][..]),
    ] {
        stand_in::compile(&library, release, options).unwrap();
        let message = failed(peer.client());
        assert!(message.starts_with(&named), "{options:?}: {message}");
        peer.assert_never_connected();
        fs::remove_file(&library).unwrap();
    }
}
