use climon2_cli::list::{LiveSession, discover, json, table};
use climon2_core::{SessionId, hello::HelloExtensions, metadata::SessionMetadata};
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
        "daemon_linux_identity":process::linux_identity(std::process::id()).unwrap(),
        "command":"bash","args":["space arg","quote'arg","雪","\u{1b}[31m"],
        "cwd":home,"depth":3,"created_at":created,"climon2_version":"999.0.0",
        "future_field":true
    }))
    .unwrap()
}

/// A probed session whose hello carried `extensions` beside the mandatory fields.
fn live(metadata: &SessionMetadata, extensions: serde_json::Value) -> LiveSession {
    LiveSession {
        metadata: metadata.clone(),
        extensions: HelloExtensions::from_value(&extensions),
    }
}

fn legacy(metadata: &SessionMetadata) -> LiveSession {
    live(metadata, serde_json::json!({}))
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
    let first_live = legacy(&first);
    let rendered = table(std::slice::from_ref(&first_live), 120).unwrap();
    assert!(rendered.contains(first.session_id.as_str()));
    assert!(!rendered.contains('\x1b'));
    assert!(rendered.lines().last().unwrap().ends_with('…'));
    assert!(
        rendered
            .lines()
            .all(|line| unicode_width::UnicodeWidthStr::width(line) <= 120)
    );
    let prefix = rendered.lines().last().unwrap().split('\'').next().unwrap();
    let prefix_width = unicode_width::UnicodeWidthStr::width(prefix);
    let filled = table(std::slice::from_ref(&first_live), prefix_width).unwrap();
    assert_eq!(filled.lines().last().unwrap(), prefix);
    let one_cell = table(std::slice::from_ref(&first_live), prefix_width + 1).unwrap();
    assert_eq!(one_cell.lines().last().unwrap(), format!("{prefix}…"));
    let full = table(std::slice::from_ref(&first_live), 200).unwrap();
    assert!(full.contains("'space arg'"));
    assert!(full.contains("'quote'\\''arg'"));
    assert!(full.contains("\\u{1b}"));
    let json = serde_json::to_string(&first).unwrap();
    assert!(json.contains("\\u001b[31m"));
    assert!(json.contains("\"depth\":3"));
}

/// #37, DSC-06 (CLI part): malformed metadata is hidden, warned about, and retained.
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

/// #37/#26, DSC-06 (CLI part): no socket plus a reused PID is stale; a matching live identity
/// protects files.
#[test]
fn stale_cleanup_requires_connection_refusal_and_proven_dead_identity() {
    let home = tempfile::tempdir().unwrap();
    let live = metadata(home.path(), 3, "2026-10-28T07:21:02.930Z");
    let mut reused = metadata(home.path(), 4, "2026-10-28T07:21:02.930Z");
    reused.daemon_start_time = 0;
    if let Some(identity) = &mut reused.daemon_linux_identity {
        identity.start_ticks = 0;
    }
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

/// DSC-06 (CLI part), Linux: a session with no socket and no recorded Linux daemon identity is
/// uncertain: hidden, warned about and never deleted.
#[cfg(target_os = "linux")]
#[test]
fn uncertain_linux_daemon_identity_is_warned_and_metadata_is_retained() {
    let home = tempfile::tempdir().unwrap();
    let mut metadata = metadata(home.path(), 29, "2026-10-28T07:21:02.930Z");
    metadata.daemon_linux_identity = None;
    let path = save(home.path(), &metadata);
    let log = logging::LogFile::general(home.path()).unwrap();
    let dispatch = log.dispatch(Some("warn")).unwrap();
    let sessions = tracing::dispatcher::with_default(&dispatch, || discover(home.path())).unwrap();
    assert!(sessions.is_empty());
    assert!(path.exists());
    assert!(
        fs::read_to_string(log.path())
            .unwrap()
            .contains("daemon identity is uncertain")
    );
}

/// DSC-06 (CLI part): stale metadata whose socket is already gone is removed, and the absent
/// socket is not reported as a failed cleanup.
#[test]
fn already_missing_stale_socket_does_not_report_cleanup_failure() {
    let home = tempfile::tempdir().unwrap();
    let mut metadata = metadata(home.path(), 30, "2026-10-28T07:21:02.930Z");
    metadata.daemon_start_time = 0;
    if let Some(identity) = &mut metadata.daemon_linux_identity {
        identity.start_ticks = 0;
    }
    let path = save(home.path(), &metadata);
    let log = logging::LogFile::general(home.path()).unwrap();
    let dispatch = log.dispatch(Some("warn")).unwrap();
    let sessions = tracing::dispatcher::with_default(&dispatch, || discover(home.path())).unwrap();
    assert!(sessions.is_empty());
    assert!(!path.exists());
    assert!(
        !fs::read_to_string(log.path())
            .unwrap()
            .contains("stale session cleanup failed")
    );
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

/// #37, DSC-06 (CLI part): missing sockets with a gone PID are stale, unlike failed hello
/// exchanges.
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

/// #37, DSC-06 (CLI part): ARGS truncation follows the real stdout terminal width, not a fixed
/// character cap.
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
            table(&[legacy(&metadata)], 200)
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

/// #37, DSC-06 (CLI part): list probes do not attach, interrupt the controller, or need
/// version companions.
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
            .args(["--scrollback-max-rows", "10000"])
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
    let live = climon2_cli::list::discover_live(&home).unwrap();
    assert_eq!(live.len(), 1);
    let capabilities = live[0].extensions.capabilities.valid().unwrap();
    assert!(capabilities.supports_attach());
    assert!(capabilities.supports_kill());
    assert!(capabilities.supports_state());
    assert_eq!(live[0].extensions.state.valid().unwrap().attachments(), 1);
    let found = discover(&home).unwrap();
    assert_eq!(found.len(), 1);
    // Section 1.3: list never reads the configuration.
    fs::write(home.join("config.jsonc"), "{ not jsonc").unwrap();
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
    assert_eq!(
        json[0]["state"],
        serde_json::json!({"lifecycle":"attached","attachments":1,"controlled":true})
    );
    loop {
        match ServerMessage::from_frame(read_frame(&mut controller).unwrap()).unwrap() {
            ServerMessage::Output(_) | ServerMessage::Granted { .. } => (),
            ServerMessage::RestoreEnd { period } => {
                write_frame(
                    &mut controller,
                    &ClientMessage::Ready { period }.to_frame().unwrap(),
                )
                .unwrap();
            }
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

/// #37, DSC-06 (CLI part): partial hello replies never justify deletion.
#[test]
fn failed_hello_retains_artifacts_under_a_single_two_second_deadline() {
    assert_failed_hello_retains_artifacts(|socket| {
        for byte in [0_u8, 0, 0, 10, 1, b'{'] {
            std::thread::sleep(std::time::Duration::from_millis(600));
            if socket.write_all(&[byte]).is_err() {
                break;
            }
        }
    });
}

/// DSC-06 (CLI part): a hello reply that stalls after one byte still ends its probe at the
/// single two-second discovery deadline, and nothing is deleted.
#[test]
fn stalled_partial_hello_does_not_block_past_the_discovery_deadline() {
    assert_failed_hello_retains_artifacts(|socket| {
        socket.write_all(&[0]).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(4)))
            .unwrap();
        let mut byte = [0];
        assert_eq!(std::io::Read::read(socket, &mut byte).unwrap(), 0);
    });
}

fn assert_failed_hello_retains_artifacts(
    serve: impl FnOnce(&mut runtime::SocketStream) + Send + 'static,
) {
    use std::time::{Duration, Instant};
    let home = tempfile::tempdir().unwrap();
    let metadata = metadata(home.path(), 7, "2026-10-28T07:21:02.930Z");
    let path = save(home.path(), &metadata);
    let listener = runtime::bind(std::path::Path::new(metadata.socket_path.as_str())).unwrap();
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        climon2_core::protocol::read_frame(&mut socket).unwrap();
        serve(&mut socket);
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

/// #37, DSC-06 (CLI part): a dead identity must never turn a non-socket path into a deletion
/// target.
#[test]
fn stale_metadata_cannot_unlink_a_regular_file_as_a_socket() {
    let home = tempfile::tempdir().unwrap();
    let mut metadata = metadata(home.path(), 8, "2026-10-28T07:21:02.930Z");
    metadata.daemon_start_time = 0;
    if let Some(identity) = &mut metadata.daemon_linux_identity {
        identity.start_ticks = 0;
    }
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

/// ADR0001/#37, DSC-06 (CLI part): probes use only frozen hello, ignore version/protocol and
/// future fields.
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

/// Section 1.11: VERSION, STATE and CLIENTS; state only from `state-v1` with a
/// valid snapshot, otherwise `unknown` and `-`.
#[test]
fn list_shows_version_state_and_clients_only_when_advertised() {
    let home = tempfile::tempdir().unwrap();
    let state = serde_json::json!({"lifecycle":"terminating","attachments":2,"controlled":true});
    let mut current = metadata(home.path(), 20, "2026-10-28T07:21:02.930Z");
    current.climon2_version = "1.20.300".parse().unwrap();
    let sessions = [
        live(
            &current,
            serde_json::json!({"capabilities":["state-v1"],"state":state}),
        ),
        live(
            &metadata(home.path(), 21, "2026-10-28T07:21:02.930Z"),
            serde_json::json!({"capabilities":["attach"],"state":state}),
        ),
        live(
            &metadata(home.path(), 22, "2026-10-28T07:21:02.930Z"),
            serde_json::json!({"capabilities":["state-v1"],"state":{"lifecycle":"attached","attachments":0,"controlled":true}}),
        ),
        live(
            &metadata(home.path(), 23, "2026-10-28T07:21:02.930Z"),
            serde_json::json!({"capabilities":["state-v1"],"state":{"lifecycle":"detached","attachments":0,"controlled":false}}),
        ),
    ];
    let rendered = table(&sessions, 400).unwrap();
    let lines: Vec<_> = rendered.lines().collect();
    assert_eq!(
        lines[0],
        format!(
            "{:<36}  {:<19}  VERSION   STATE        CLIENTS  COMMAND  ARGS",
            "SESSION", "CREATED"
        )
    );
    fn columns(line: &str) -> Vec<&str> {
        line.split_whitespace().skip(3).take(4).collect()
    }
    assert_eq!(columns(lines[1]), ["1.20.300", "terminating", "2", "bash"]);
    assert_eq!(columns(lines[2]), ["999.0.0", "unknown", "-", "bash"]);
    assert_eq!(columns(lines[3]), ["999.0.0", "unknown", "-", "bash"]);
    assert_eq!(columns(lines[4]), ["999.0.0", "detached", "0", "bash"]);
    assert_eq!(lines[1].find("1.20.300"), Some(59));
    assert_eq!(lines[1].find("terminating"), Some(69));
    assert_eq!(lines[4].find(" 0  "), Some(81));
}

/// Section 1.11: each JSON object is the unchanged metadata plus `state`.
#[test]
fn list_json_adds_the_state_object_or_null() {
    let home = tempfile::tempdir().unwrap();
    let first = metadata(home.path(), 24, "2026-10-28T07:21:02.930Z");
    let second = metadata(home.path(), 25, "2026-10-28T07:21:02.930Z");
    let rendered = json(&[
        live(
            &first,
            serde_json::json!({"capabilities":["state-v1"],"state":{"lifecycle":"attached","attachments":1,"controlled":true}}),
        ),
        legacy(&second),
    ])
    .unwrap();
    assert!(rendered.ends_with("]\n"));
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    let mut expected = serde_json::to_value(&first).unwrap();
    expected["state"] =
        serde_json::json!({"lifecycle":"attached","attachments":1,"controlled":true});
    assert_eq!(value[0], expected);
    let mut expected = serde_json::to_value(&second).unwrap();
    expected["state"] = serde_json::Value::Null;
    assert_eq!(value[1], expected);
}

/// Section 1.11, DSC-06 (CLI part): malformed optional hello data is logged; the session stays
/// listed.
#[test]
fn malformed_hello_extensions_are_logged_and_the_session_stays_live() {
    use climon2_core::protocol::{Frame, read_frame, write_frame};
    let home = tempfile::tempdir().unwrap();
    let metadata = metadata(home.path(), 26, "2026-10-28T07:21:02.930Z");
    save(home.path(), &metadata);
    let listener = runtime::bind(std::path::Path::new(metadata.socket_path.as_str())).unwrap();
    let id = metadata.session_id.clone();
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        read_frame(&mut socket).unwrap();
        let body = format!(
            "{{\"session_id\":\"{id}\",\"climon2_version\":\"0.1.0\",\"protocol\":1,\"capabilities\":7}}"
        );
        write_frame(&mut socket, &Frame::new(1, body.into_bytes()).unwrap()).unwrap();
    });
    let log = logging::LogFile::general(home.path()).unwrap();
    let dispatch = log.dispatch(Some("warn")).unwrap();
    let found = tracing::dispatcher::with_default(&dispatch, || {
        climon2_cli::list::discover_live(home.path())
    })
    .unwrap();
    worker.join().unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].extensions.capabilities.is_malformed());
    assert!(
        fs::read_to_string(log.path())
            .unwrap()
            .contains("malformed hello extensions")
    );
    fs::remove_file(metadata.socket_path.as_str()).unwrap();
}

/// #37: shell-safe arguments are shown bare, and an empty argument as `''`.
#[test]
fn list_quotes_only_the_arguments_that_need_it() {
    let home = tempfile::tempdir().unwrap();
    let mut metadata = metadata(home.path(), 27, "2026-10-28T07:21:02.930Z");
    metadata.args = ["plain", "--flag=a/b.c", "", "two words"]
        .map(str::to_owned)
        .to_vec();
    let rendered = table(&[legacy(&metadata)], 400).unwrap();
    let row = rendered.lines().last().unwrap();
    assert!(
        row.ends_with("  plain --flag=a/b.c '' 'two words'"),
        "{row}"
    );
}

/// #37: ARGS that exactly fill the terminal are shown whole; one cell less fills the row
/// up to the terminal's last cell, with the ellipsis there.
#[test]
fn list_args_that_exactly_fit_are_whole_and_one_cell_less_is_truncated() {
    let home = tempfile::tempdir().unwrap();
    let mut metadata = metadata(home.path(), 28, "2026-10-28T07:21:02.930Z");
    metadata.args = vec!["abcdefgh".to_owned()];
    let sessions = [legacy(&metadata)];
    let row = |width| {
        table(&sessions, width)
            .unwrap()
            .lines()
            .last()
            .unwrap()
            .to_owned()
    };
    let whole = row(400);
    assert!(whole.ends_with("  abcdefgh"), "{whole}");
    let width = unicode_width::UnicodeWidthStr::width(whole.as_str());
    assert_eq!(row(width), whole);
    assert_eq!(
        row(width - 1),
        format!("{}…", whole.strip_suffix("gh").unwrap())
    );
}

/// #77, DSC-06 (CLI part): discovery moved into `climon2-discovery`, and `list` still logs
/// each probe's diagnostic in the general log once per session, at its old level, under the
/// CLI's own target and with its errno: a hello from another session or a failed exchange,
/// an unreachable socket, a session whose daemon still runs or whose identity is unknown,
/// and a failed stale cleanup. Uncertain sessions keep their metadata.
#[test]
fn every_probe_diagnostic_is_logged_once_with_its_errno() {
    use climon2_test_term::{Isolation, bin_path, exit_within, hello_peer, process_command};
    use std::os::unix::fs::PermissionsExt;
    let isolation = Isolation::new();
    let home = runtime::canonical_home(&isolation.climon2_home()).unwrap();
    let created = "2026-10-28T07:21:02.930Z";
    let other = "{\"session_id\":\"12345678-1234-4234-8234-123456789aff\",\"climon2_version\":\"0.1.0\",\"protocol\":1}";
    let peers: Vec<_> = [(31, other), (32, "{")]
        .into_iter()
        .map(|(suffix, body)| {
            let metadata = metadata(&home, suffix, created);
            save(&home, &metadata);
            let worker = hello_peer(metadata.socket_path.as_str(), body, 1);
            (metadata, worker)
        })
        .collect();
    let unreachable = metadata(&home, 33, created);
    let datagram =
        std::os::unix::net::UnixDatagram::bind(unreachable.socket_path.as_str()).unwrap();
    let alive = metadata(&home, 34, created);
    let mut unknown = metadata(&home, 35, created);
    unknown.daemon_pid = 0;
    let mut stale = metadata(&home, 36, created);
    stale.daemon_start_time = 0;
    if let Some(identity) = &mut stale.daemon_linux_identity {
        identity.start_ticks = 0;
    }
    let files: Vec<_> = [&unreachable, &alive, &unknown, &stale]
        .into_iter()
        .map(|metadata| save(&home, metadata))
        .collect();
    let sessions = home.join("sessions");
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o500)).unwrap();
    let stdout = isolation.home().join("stdout");
    let stderr = isolation.home().join("stderr");
    let mut command = process_command(bin_path("climon2-cli"));
    command
        .env_clear()
        .env("HOME", isolation.home())
        .env("CLIMON2_HOME", &home)
        .env("CLIMON2_LOG", "trace")
        .env("CLIMON2_TEST_ID", isolation.id())
        .arg("list")
        .stdout(fs::File::create(&stdout).unwrap())
        .stderr(fs::File::create(&stderr).unwrap());
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let status = exit_within(command.spawn().unwrap(), std::time::Duration::from_secs(30));
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();

    assert!(status.is_some_and(|status| status.success()), "{status:?}");
    assert_eq!(
        fs::read(&stdout).unwrap(),
        b"No climon2 sessions running.\n"
    );
    assert_eq!(fs::read(&stderr).unwrap(), b"");
    let [log] = fs::read_dir(home.join("logs/general"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let log = fs::read_to_string(log).unwrap();
    for line in [
        "DEBUG climon2_cli::list: session probe did not match responded=true\n".to_owned(),
        "DEBUG climon2_cli::list: session probe did not match responded=false\n".to_owned(),
        format!(
            "DEBUG climon2_cli::list: session connection failed errno={}\n",
            libc::EPROTOTYPE
        ),
        "DEBUG climon2_cli::list: refused connection belongs to a live daemon\n".to_owned(),
        " WARN climon2_cli::list: daemon identity is uncertain\n".to_owned(),
        format!(
            " WARN climon2_cli::list: stale session cleanup failed errno={}\n",
            libc::EACCES
        ),
    ] {
        assert_eq!(log.matches(&line).count(), 1, "{line}{log}");
    }
    assert!(files.iter().all(|file| file.exists()));
    for (metadata, worker) in peers {
        worker.join().unwrap();
        fs::remove_file(metadata.socket_path.as_str()).unwrap();
        fs::remove_file(
            home.join("sessions")
                .join(format!("{}.pty", metadata.session_id)),
        )
        .unwrap();
    }
    drop(datagram);
    fs::remove_file(unreachable.socket_path.as_str()).unwrap();
    files.iter().for_each(|file| fs::remove_file(file).unwrap());
    isolation.assert_clean();
}
