//! #316: `c2 list` on Windows. With no sessions it says so; a session whose
//! socket refuses and whose daemon is provably gone is reclaimed; one whose
//! daemon still runs is kept; and a live session answers the frozen hello
//! over AF_UNIX and is listed to the console's width.
#![cfg(windows)]
use climon2_core::{
    SessionId,
    metadata::SessionMetadata,
    protocol::{Frame, read_frame, write_frame},
};
use climon2_platform::{process, runtime};
use climon2_test_term::{Terminal, local_app_data};
use portable_pty::CommandBuilder;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::Duration,
};

/// `climon2-cli list`, its climon2 home inside `root` and its runtime root
/// `local`.
fn list(root: &Path, local: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_climon2-cli"))
        .arg("list")
        .args(arguments)
        .env("CLIMON2_HOME", root.join("home"))
        .env("LOCALAPPDATA", local)
        .output()
        .unwrap()
}

fn home(root: &Path) -> PathBuf {
    runtime::canonical_home(&root.join("home")).unwrap()
}

/// Metadata for session `suffix` whose daemon is `pid`, started at `start`,
/// listening at `socket`, saved where `list` reads it.
fn save(
    home: &Path,
    suffix: u8,
    (pid, start): (u32, u64),
    socket: &Path,
) -> (SessionMetadata, PathBuf) {
    let id: SessionId = format!("12345678-1234-4234-8234-123456789a{suffix:02x}")
        .parse()
        .unwrap();
    let metadata: SessionMetadata = serde_json::from_value(serde_json::json!({
        "format_version": 1, "session_id": id.as_str(), "socket_path": socket,
        "daemon_pid": pid, "daemon_start_time": start,
        "command": "pwsh", "args": ["-NoLogo", "a".repeat(200)],
        "cwd": home, "depth": 1, "created_at": "2026-07-02T05:04:03.000Z",
        "climon2_version": "0.1.0",
    }))
    .unwrap();
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions").join(format!("{id}.pty"));
    fs::write(&path, metadata.to_json().unwrap()).unwrap();
    (metadata, path)
}

/// A daemon identity that is provably gone: a process that ran and ended.
fn gone_daemon() -> (u32, u64) {
    let mut child = Command::new("cmd")
        .args(["/d", "/c", "exit"])
        .spawn()
        .unwrap();
    let identity = (
        child.id(),
        process::start_time(child.id()).unwrap().unwrap_or(1),
    );
    child.wait().unwrap();
    identity
}

fn this_process() -> (u32, u64) {
    let pid = std::process::id();
    (pid, process::start_time(pid).unwrap().unwrap())
}

/// A socket file left behind by a listener that has closed.
fn stale_socket(path: &Path) {
    drop(runtime::bind(path).unwrap());
    assert!(path.exists());
}

#[test]
fn list_with_no_sessions_says_so() {
    let (root, local) = (tempfile::tempdir().unwrap(), local_app_data());

    let table = list(root.path(), local.path(), &[]);
    let json = list(root.path(), local.path(), &["--json"]);

    assert_eq!(table.status.code(), Some(0), "{table:?}");
    assert_eq!(table.stdout, b"No climon2 sessions running.\n");
    assert_eq!(json.status.code(), Some(0), "{json:?}");
    assert_eq!(json.stdout, b"[]\n");
    assert!(root.path().join("home").join("logs").is_dir());
}

#[test]
fn list_reclaims_a_stale_session_whose_daemon_is_gone() {
    let (root, local) = (tempfile::tempdir().unwrap(), local_app_data());
    let home = home(root.path());
    let socket = root.path().join("stale.sock");
    stale_socket(&socket);
    let (_, metadata) = save(&home, 1, gone_daemon(), &socket);

    let output = list(root.path(), local.path(), &[]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"No climon2 sessions running.\n");
    assert!(!metadata.exists(), "the stale metadata is removed");
    assert!(!socket.exists(), "the stale socket is removed");
}

/// A refusing socket alone proves nothing while the recorded daemon runs.
#[test]
fn list_keeps_a_refusing_session_whose_daemon_still_runs() {
    let (root, local) = (tempfile::tempdir().unwrap(), local_app_data());
    let home = home(root.path());
    let socket = root.path().join("refusing.sock");
    stale_socket(&socket);
    let (_, metadata) = save(&home, 2, this_process(), &socket);

    let output = list(root.path(), local.path(), &[]);

    assert_eq!(output.stdout, b"No climon2 sessions running.\n");
    assert!(metadata.exists());
    assert!(socket.exists());
}

/// Something that is not a socket at the socket path is never removed as
/// one, so the stale session's metadata stays too.
#[test]
fn list_leaves_a_stale_session_whose_socket_path_is_not_a_socket() {
    let (root, local) = (tempfile::tempdir().unwrap(), local_app_data());
    let home = home(root.path());
    let socket = root.path().join("file.sock");
    fs::write(&socket, "not a socket").unwrap();
    let (_, metadata) = save(&home, 3, gone_daemon(), &socket);

    let output = list(root.path(), local.path(), &[]);

    assert_eq!(output.stdout, b"No climon2 sessions running.\n");
    assert!(metadata.exists());
    assert_eq!(fs::read_to_string(&socket).unwrap(), "not a socket");
}

/// As DSC-06 on Unix: the live session's row is cut to the console's 100
/// columns, not to the 80 of redirected output, and its creation time is
/// local.
#[test]
fn list_shows_a_live_session_to_the_consoles_width() {
    let (root, local) = (tempfile::tempdir().unwrap(), local_app_data());
    let home = home(root.path());
    let socket = root.path().join("live.sock");
    let listener = runtime::bind(&socket).unwrap();
    let (metadata, path) = save(&home, 4, this_process(), &socket);
    let id = metadata.session_id.clone();
    let daemon = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_frame(&mut stream).unwrap();
        let body =
            format!("{{\"session_id\":\"{id}\",\"climon2_version\":\"0.1.0\",\"protocol\":1}}");
        write_frame(&mut stream, &Frame::new(1, body.into_bytes()).unwrap()).unwrap();
    });
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_climon2-cli"));
    command.arg("list");
    command.env("CLIMON2_HOME", root.path().join("home"));
    command.env("LOCALAPPDATA", local.path());

    let mut terminal = Terminal::spawn(command, 24, 100).unwrap();

    assert_eq!(terminal.wait_exit(Duration::from_secs(10)).unwrap(), 0);
    daemon.join().unwrap();
    let screen = terminal.screen();
    let row = screen
        .lines()
        .find(|line| line.contains(metadata.session_id.as_str()))
        .unwrap_or_else(|| panic!("{screen}"));
    assert!(
        row.chars().skip(80).any(|character| character != ' '),
        "{screen}"
    );
    assert!(row.trim_end().ends_with('…'), "{screen}");
    let created = climon2_platform::presentation::local_datetime(&metadata.created_at).unwrap();
    assert!(row.contains(&created), "{screen}");
    assert!(path.exists() && socket.exists(), "a live session is kept");
}
