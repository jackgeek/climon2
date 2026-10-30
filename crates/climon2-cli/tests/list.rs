use climon2_cli::list::{discover, table};
use climon2_core::{SessionId, metadata::SessionMetadata};
use climon2_platform::{logging, process, runtime};
use std::{fs, io::Write};

fn metadata(home: &std::path::Path, suffix: u8, created: &str) -> SessionMetadata {
    let id: SessionId = format!("12345678-1234-4234-8234-123456789a{suffix:02x}")
        .parse()
        .unwrap();
    let socket = runtime::RuntimeDirectory::prepare(home)
        .unwrap()
        .socket_path(&id)
        .unwrap();
    serde_json::from_value(serde_json::json!({
        "format_version":999,"session_id":id.as_str(),"socket_path":socket,
        "daemon_pid":std::process::id(),
        "daemon_start_time":process::start_time(std::process::id()).unwrap().unwrap(),
        "command":"bash","args":["space arg","quote'arg","雪","\u{1b}[31m"],
        "cwd":home,"depth":3,"created_at":created,"climon2_version":"999.0.0",
        "future_field":true
    }))
    .unwrap()
}

fn save(home: &std::path::Path, metadata: &SessionMetadata) -> std::path::PathBuf {
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home
        .join("sessions")
        .join(format!("{}.pty", metadata.session_id));
    fs::write(&path, metadata.to_json().unwrap()).unwrap();
    path
}

/// #37: empty presentation is exact, table escapes/quotes/fits terminal cells (a full prefix leaves
/// ARGS empty, one spare cell holds only the ellipsis); JSON stays lossless.
#[test]
fn list_presentation_quotes_controls_and_fits_terminal_width() {
    assert_eq!(table(&[], 80).unwrap(), "No climon2 sessions running.\n");
    let home = tempfile::tempdir().unwrap();
    let first = metadata(home.path(), 1, "2026-10-28T07:21:02.930Z");
    let rendered = table(std::slice::from_ref(&first), 80).unwrap();
    assert!(rendered.contains(first.session_id.as_str()));
    assert!(!rendered.contains('\x1b'));
    assert!(rendered.lines().last().unwrap().ends_with('…'));
    assert!(
        rendered
            .lines()
            .all(|line| unicode_width::UnicodeWidthStr::width(line) <= 80)
    );
    let prefix = rendered.lines().last().unwrap().split('\'').next().unwrap();
    let prefix_width = unicode_width::UnicodeWidthStr::width(prefix);
    let filled = table(std::slice::from_ref(&first), prefix_width).unwrap();
    assert_eq!(filled.lines().last().unwrap(), prefix);
    let one_cell = table(std::slice::from_ref(&first), prefix_width + 1).unwrap();
    assert_eq!(one_cell.lines().last().unwrap(), format!("{prefix}…"));
    let full = table(std::slice::from_ref(&first), 200).unwrap();
    assert!(full.contains("'space arg'"));
    assert!(full.contains("'quote'\\''arg'"));
    assert!(full.contains("\\u{1b}"));
    let json = serde_json::to_string(&first).unwrap();
    assert!(json.contains("\\u001b[31m"));
    assert!(json.contains("\"depth\":3"));
}

/// #37: malformed metadata is hidden, warned about, and retained.
#[test]
fn malformed_metadata_is_hidden_warned_and_never_deleted() {
    let home = tempfile::tempdir().unwrap();
    let metadata = metadata(home.path(), 2, "2026-10-28T07:21:02.930Z");
    let path = save(home.path(), &metadata);
    fs::write(&path, b"not-json").unwrap();
    let invalid_name = home.path().join("sessions/not-guid.pty");
    fs::write(&invalid_name, b"not-json").unwrap();
    let log = logging::LogFile::general(home.path()).unwrap();
    let dispatch = log.dispatch(Some("trace")).unwrap();
    let sessions = tracing::dispatcher::with_default(&dispatch, || discover(home.path())).unwrap();
    assert!(sessions.is_empty());
    assert!(path.exists());
    assert!(invalid_name.exists());
    assert_eq!(
        fs::read_to_string(log.path())
            .unwrap()
            .matches("invalid session metadata")
            .count(),
        2
    );
}

/// #37/#26: no socket plus a reused PID is stale; a matching live identity protects files.
#[test]
fn stale_cleanup_requires_connection_refusal_and_proven_dead_identity() {
    let home = tempfile::tempdir().unwrap();
    let live = metadata(home.path(), 3, "2026-10-28T07:21:02.930Z");
    let mut reused = metadata(home.path(), 4, "2026-10-28T07:21:02.930Z");
    reused.daemon_start_time = 0;
    let live_path = save(home.path(), &live);
    let stale_path = save(home.path(), &reused);
    refusing_socket(live.socket_path.as_str());
    refusing_socket(reused.socket_path.as_str());
    assert!(discover(home.path()).unwrap().is_empty());
    assert!(live_path.exists());
    assert!(!stale_path.exists());
    assert!(!std::path::Path::new(reused.socket_path.as_str()).exists());
    assert!(std::path::Path::new(live.socket_path.as_str()).exists());
    fs::remove_file(live.socket_path.as_str()).unwrap();
}

/// A socket file left by a dead daemon: bound but never listening, so it refuses connections
/// even while a concurrently forked, not yet exec'd child holds a copy of the descriptor (a
/// closed listener would keep accepting through such a copy).
fn refusing_socket(path: &str) {
    let socket = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    assert!(socket >= 0);
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    assert!(path.len() < address.sun_path.len());
    for (slot, byte) in address.sun_path.iter_mut().zip(path.as_bytes()) {
        *slot = *byte as libc::c_char;
    }
    let length = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
    assert_eq!(
        unsafe { libc::bind(socket, (&raw const address).cast(), length) },
        0
    );
    assert_eq!(unsafe { libc::close(socket) }, 0);
}

/// #37: missing sockets with a gone PID are stale, unlike failed hello exchanges.
#[test]
fn gone_pid_is_stale_but_wrong_or_malformed_hello_never_is() {
    use climon2_core::protocol::{Frame, read_frame, write_frame};
    let home = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    let mut gone = metadata(home.path(), 11, "2026-10-28T07:21:02.930Z");
    gone.daemon_pid = pid;
    let gone_path = save(home.path(), &gone);
    assert!(discover(home.path()).unwrap().is_empty());
    assert!(!gone_path.exists());
    for (suffix, type_code, body) in [
        (12, 0x01, b"{".to_vec()),
        (13, 0x01, b"{\"session_id\":\"12345678-1234-4234-8234-123456789aff\",\"climon2_version\":\"0.1.0\",\"protocol\":1}".to_vec()),
        (14, 0x21, b"not a hello".to_vec()),
    ] {
        let mut metadata = metadata(home.path(), suffix, "2026-10-28T07:21:02.930Z");
        metadata.daemon_pid = pid;
        let path = save(home.path(), &metadata);
        let listener = runtime::bind(std::path::Path::new(metadata.socket_path.as_str())).unwrap();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            read_frame(&mut socket).unwrap();
            write_frame(&mut socket, &Frame::new(type_code, body).unwrap()).unwrap();
        });
        assert!(discover(home.path()).unwrap().is_empty());
        worker.join().unwrap();
        assert!(path.exists());
        assert!(std::path::Path::new(metadata.socket_path.as_str()).exists());
        fs::remove_file(metadata.socket_path.as_str()).unwrap();
        fs::remove_file(path).unwrap();
    }
}

/// #37: ARGS truncation follows the real stdout terminal width, not a fixed character cap.
#[test]
fn list_args_fit_the_real_stdout_terminal_width() {
    use climon2_core::protocol::{Frame, read_frame, write_frame};
    use climon2_test_term::{Isolation, Terminal, bin_path};
    let isolation = Isolation::new();
    let home = runtime::canonical_home(&isolation.climon2_home()).unwrap();
    let mut metadata = metadata(&home, 10, "2026-10-28T07:21:02.930Z");
    metadata.args = vec!["a".repeat(200)];
    let path = save(&home, &metadata);
    let listener = runtime::bind(std::path::Path::new(metadata.socket_path.as_str())).unwrap();
    let id = metadata.session_id.clone();
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        read_frame(&mut socket).unwrap();
        let body =
            format!("{{\"session_id\":\"{id}\",\"climon2_version\":\"0.1.0\",\"protocol\":1}}");
        write_frame(&mut socket, &Frame::new(1, body.into_bytes()).unwrap()).unwrap();
    });
    let mut command = isolation.command(bin_path("climon2-cli"));
    command.arg("list");
    let mut terminal = Terminal::spawn(command, 24, 100).unwrap();
    assert_eq!(
        terminal
            .wait_exit(std::time::Duration::from_secs(3))
            .unwrap(),
        0
    );
    let screen = terminal.screen();
    let row = screen
        .lines()
        .find(|line| line.contains(metadata.session_id.as_str()))
        .unwrap();
    assert!(row.chars().skip(80).any(|character| character != ' '));
    assert!(row.trim_end().ends_with('…'));
    drop(terminal);
    worker.join().unwrap();
    fs::remove_file(metadata.socket_path.as_str()).unwrap();
    fs::remove_file(path).unwrap();
    isolation.assert_clean();
}

/// #37: CREATED uses local time, not UTC, with deterministic subprocess timezone fixtures.
#[test]
fn list_created_column_uses_the_local_timezone() {
    if std::env::var_os("CLIMON2_TEST_LOCAL_LIST").is_some() {
        let home = tempfile::tempdir().unwrap();
        let metadata = metadata(home.path(), 9, "2026-10-28T07:21:02.930Z");
        assert!(
            table(&[metadata], 200)
                .unwrap()
                .contains("2026-10-28 09:21:02")
        );
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "list_created_column_uses_the_local_timezone"])
        .env("CLIMON2_TEST_LOCAL_LIST", "1")
        .env("TZ", "Etc/GMT-2")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

/// #37: list probes do not attach, interrupt the controller, or need version companions.
#[test]
fn live_daemon_list_preserves_the_controlling_session_and_json_arguments() {
    use climon2_core::protocol::{
        ClientMessage, ServerMessage, TerminalSize, read_frame, write_frame,
    };
    use climon2_platform::daemon::{self, Readiness};
    use climon2_test_term::{Isolation, bin_path};
    let isolation = Isolation::new();
    let home = runtime::canonical_home(&isolation.climon2_home()).unwrap();
    let id: SessionId = "12345678-1234-4234-8234-123456789aff".parse().unwrap();
    let socket = runtime::RuntimeDirectory::prepare(&home)
        .unwrap()
        .socket_path(&id)
        .unwrap();
    let mut command = std::process::Command::new(bin_path("climon2-daemon"));
    command
        .env_clear()
        .env("HOME", isolation.home())
        .env("TERM", "xterm-256color")
        .env("PATH", "/usr/bin:/bin")
        .env("CLIMON2_TEST_ID", isolation.id())
        .env("CLIMON2_SESSION_DEPTH", "1")
        .env("CLIMON2_SESSION_ID", id.as_str())
        .args(["--session-id", id.as_str(), "--home"])
        .arg(&home)
        .arg("--socket")
        .arg(&socket)
        .args(["--version", env!("CARGO_PKG_VERSION"), "--cli-pid"])
        .arg(std::process::id().to_string())
        .args(["--rows", "24", "--cols", "80", "--cwd"])
        .arg(isolation.home());
    let pending = daemon::spawn(command, |command, fd| {
        command
            .arg("--readiness-fd")
            .arg(fd.to_string())
            .arg("--")
            .arg(bin_path("climon2-fixture"))
            .args(["print", "list-live", "sleep", "2500", "exit", "7"]);
    })
    .unwrap();
    let pid = pending.id();
    assert_eq!(pending.wait_ready(&[]).unwrap(), Readiness::Ready);
    let mut controller = runtime::SocketStream::connect(&socket).unwrap();
    controller
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    write_frame(
        &mut controller,
        &ClientMessage::Hello { protocol: 1 }.to_frame().unwrap(),
    )
    .unwrap();
    assert!(matches!(
        ServerMessage::from_frame(read_frame(&mut controller).unwrap()).unwrap(),
        ServerMessage::Hello(_)
    ));
    write_frame(
        &mut controller,
        &ClientMessage::Attach(TerminalSize { cols: 80, rows: 24 })
            .to_frame()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        ServerMessage::from_frame(read_frame(&mut controller).unwrap()).unwrap(),
        ServerMessage::Attached
    );
    let found = discover(&home).unwrap();
    assert_eq!(found.len(), 1);
    let output = std::process::Command::new(bin_path("climon2-cli"))
        .env("HOME", isolation.home())
        .env("CLIMON2_HOME", &home)
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json[0]["session_id"], id.as_str());
    assert_eq!(json[0]["depth"], 1);
    assert_eq!(
        json[0]["args"],
        serde_json::json!(["print", "list-live", "sleep", "2500", "exit", "7"])
    );
    loop {
        match ServerMessage::from_frame(read_frame(&mut controller).unwrap()).unwrap() {
            ServerMessage::Output(_) => (),
            ServerMessage::Exit { code } => {
                assert_eq!(code, 7);
                break;
            }
            message => panic!("unexpected controlling message: {message:?}"),
        }
    }
    drop(controller);
    process::wait(pid).unwrap();
    isolation.assert_clean();
}

/// #37: partial hello replies never justify deletion.
#[test]
fn failed_hello_retains_artifacts_under_a_single_two_second_deadline() {
    use std::time::{Duration, Instant};
    let home = tempfile::tempdir().unwrap();
    let metadata = metadata(home.path(), 7, "2026-10-28T07:21:02.930Z");
    let path = save(home.path(), &metadata);
    let listener = runtime::bind(std::path::Path::new(metadata.socket_path.as_str())).unwrap();
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        climon2_core::protocol::read_frame(&mut socket).unwrap();
        for byte in [0_u8, 0, 0, 10, 1, b'{'] {
            std::thread::sleep(Duration::from_millis(600));
            if socket.write_all(&[byte]).is_err() {
                break;
            }
        }
    });
    let started = Instant::now();
    assert!(discover(home.path()).unwrap().is_empty());
    assert!(started.elapsed() >= Duration::from_secs(2));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(path.exists());
    assert!(std::path::Path::new(metadata.socket_path.as_str()).exists());
    worker.join().unwrap();
    fs::remove_file(metadata.socket_path.as_str()).unwrap();
}

/// #37: a dead identity must never turn a non-socket path into a deletion target.
#[test]
fn stale_metadata_cannot_unlink_a_regular_file_as_a_socket() {
    let home = tempfile::tempdir().unwrap();
    let mut metadata = metadata(home.path(), 8, "2026-10-28T07:21:02.930Z");
    metadata.daemon_start_time = 0;
    let path = save(home.path(), &metadata);
    fs::write(metadata.socket_path.as_str(), b"not a socket").unwrap();
    assert!(discover(home.path()).unwrap().is_empty());
    assert_eq!(
        fs::read(metadata.socket_path.as_str()).unwrap(),
        b"not a socket"
    );
    assert!(path.exists());
    fs::remove_file(metadata.socket_path.as_str()).unwrap();
}

/// ADR0001/#37: probes use only frozen hello, ignore version/protocol and future fields.
#[test]
fn frozen_probes_are_concurrent_and_sort_newest_first() {
    let home = tempfile::tempdir().unwrap();
    let entries = [
        metadata(home.path(), 5, "2026-10-28T07:21:02.930Z"),
        metadata(home.path(), 6, "2026-10-28T07:22:02.930Z"),
    ];
    let mut workers = Vec::new();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    for metadata in &entries {
        save(home.path(), metadata);
        let listener = runtime::bind(std::path::Path::new(metadata.socket_path.as_str())).unwrap();
        let id = metadata.session_id.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let hello = climon2_core::protocol::read_frame(&mut socket).unwrap();
            assert_eq!(hello.type_code(), 1);
            assert_eq!(hello.body(), b"{\"protocol\":1}");
            barrier.wait();
            let body = format!("{{\"session_id\":\"{id}\",\"climon2_version\":\"123.0.0\",\"protocol\":123,\"future\":true}}");
            climon2_core::protocol::write_frame(&mut socket, &climon2_core::protocol::Frame::new(1,body.into_bytes()).unwrap()).unwrap();
            socket.flush().unwrap();
        }));
    }

    let found = discover(home.path()).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].session_id, entries[1].session_id);
    for worker in workers {
        worker.join().unwrap();
    }
    for metadata in &entries {
        fs::remove_file(metadata.socket_path.as_str()).unwrap();
    }
}
