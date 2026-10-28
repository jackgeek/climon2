use climon2_core::{
    metadata::SessionMetadata,
    protocol::{ClientMessage, ServerMessage, TerminalSize, read_frame, write_frame},
};
use climon2_platform::{daemon, process, runtime::RuntimeDirectory};
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

const ID: &str = "01234567-89ab-4cde-8fab-0123456789ab";

/// #17: malformed private invocation is a sanitized wrapper failure.
#[test]
fn malformed_daemon_invocation_is_own_failure_with_no_stdout() {
    for args in [vec![], vec!["--secret\nvalue"], vec!["--exec-command"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_climon2-daemon"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(125));
        assert!(output.stdout.is_empty());
        assert!(
            !output
                .stderr
                .windows(b"secret".len())
                .any(|bytes| bytes == b"secret")
        );
    }
}

struct Session {
    directory: tempfile::TempDir,
    socket: PathBuf,
    metadata: PathBuf,
    startup_socket: PathBuf,
    pid: u32,
}

impl Session {
    fn start(mode: &str) -> Self {
        Self::configured(mode, |_, _| {})
    }

    fn configured(mode: &str, configure: impl FnOnce(&mut Command, &Path)) -> Self {
        let (session, readiness) = Self::attempt(mode, configure, false, false);
        if !matches!(readiness, Ok(daemon::Readiness::Ready)) {
            eprintln!(
                "daemon log: {:?}",
                fs::read_to_string(
                    session
                        .directory
                        .path()
                        .join(format!("logs/{ID}/daemon.log"))
                )
            );
        }
        assert_eq!(readiness.unwrap(), daemon::Readiness::Ready);
        session
    }

    fn attempt(
        mode: &str,
        configure: impl FnOnce(&mut Command, &Path),
        bad_cwd: bool,
        missing: bool,
    ) -> (Self, io::Result<daemon::Readiness>) {
        let prefix = if mode == "long-paths" {
            format!("daemon-{}", "long".repeat(30))
        } else {
            "daemon-".into()
        };
        let directory = tempfile::Builder::new()
            .prefix(&prefix)
            .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests"))
            .unwrap();
        let home = fs::canonicalize(directory.path()).unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = RuntimeDirectory::prepare(&home).unwrap();
        let standard_socket = runtime.socket_path(&ID.parse().unwrap()).unwrap();
        let startup_socket = standard_socket.with_extension("exec");
        let socket = if mode == "long-paths" {
            runtime
                .path()
                .join("s".repeat(103 - runtime.path().as_os_str().as_encoded_bytes().len() - 1))
        } else {
            standard_socket
        };
        let metadata = home.join("sessions").join(format!("{ID}.pty"));
        let cwd = if mode == "literal" {
            let cwd = home.join("cwd space --literal é");
            fs::create_dir(&cwd).unwrap();
            cwd
        } else {
            home.clone()
        };
        let executable = if mode == "long-paths" {
            let executable = home.join("climon2-daemon");
            fs::copy(env!("CARGO_BIN_EXE_climon2-daemon"), &executable).unwrap();
            executable
        } else {
            PathBuf::from(env!("CARGO_BIN_EXE_climon2-daemon"))
        };
        let mut command = Command::new(executable);
        command.args(["--session-id", ID, "--version", "0.1.0", "--cli-pid", "1"]);
        command
            .arg("--home")
            .arg(&home)
            .arg("--socket")
            .arg(&socket);
        command
            .arg("--cwd")
            .arg(if bad_cwd {
                home.join("missing-cwd")
            } else {
                cwd
            })
            .args(if mode == "zero-size" {
                ["--cols", "0", "--rows", "0"]
            } else {
                ["--cols", "91", "--rows", "37"]
            });
        command.env("DAEMON_FIXTURE_MODE", mode).env_remove("SHELL");
        command.env("DAEMON_REPORT", home.join("report"));
        configure(&mut command, &home);
        let pending = daemon::spawn(command, |command, fd| {
            command.args(["--readiness-fd", &fd.to_string(), "--"]);
            if missing {
                command.arg(home.join("missing-command"));
            } else if mode == "cannot-run" {
                command.arg(home.join("not-executable"));
            } else if mode == "literal" {
                command.arg(env!("CARGO_BIN_EXE_climon2-daemon-fixture"));
                command.args(["", "space argument", "--looks-like-option"]);
                use std::os::unix::ffi::OsStringExt;
                command.arg(std::ffi::OsString::from_vec(b"invalid-\xff".to_vec()));
            } else {
                command.arg(std::env::current_exe().unwrap());
                command.args(["--exact", "fixture_child", "--ignored", "--nocapture"]);
            }
        })
        .unwrap();
        let pid = pending.id();
        let readiness = pending.wait_ready(&[&metadata, &socket]);
        if !matches!(readiness, Ok(daemon::Readiness::Ready))
            && let Ok(log) = fs::read_to_string(home.join(format!("logs/{ID}/daemon.log")))
        {
            for pid in log
                .split_whitespace()
                .filter_map(|field| field.strip_prefix("command_pid="))
                .filter_map(|pid| pid.parse::<u32>().ok())
            {
                assert!(
                    process::start_time(pid).unwrap().is_none(),
                    "startup command survived"
                );
            }
        }
        let pid = if matches!(readiness, Ok(daemon::Readiness::Ready)) {
            pid
        } else {
            0
        };
        (
            Self {
                directory,
                socket,
                metadata,
                startup_socket,
                pid,
            },
            readiness,
        )
    }

    fn connect(&self) -> UnixStream {
        let mut stream = UnixStream::connect(&self.socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(8)))
            .unwrap();
        send(&mut stream, ClientMessage::Hello { protocol: 1 });
        assert!(matches!(receive(&mut stream), ServerMessage::Hello(_)));
        stream
    }

    fn attach(&self) -> UnixStream {
        let mut stream = self.connect();
        send(
            &mut stream,
            ClientMessage::Attach(TerminalSize { cols: 91, rows: 37 }),
        );
        assert_eq!(receive(&mut stream), ServerMessage::Attached);
        stream
    }

    fn finish(&mut self) {
        let fixture = fs::read_to_string(self.directory.path().join("fixture-pid"))
            .ok()
            .and_then(|pid| pid.parse::<u32>().ok());
        let status = process::wait(self.pid).unwrap();
        self.pid = 0;
        assert_eq!(status.code().unwrap(), 0);
        assert!(!self.metadata.exists());
        assert!(!self.socket.exists());
        assert_eq!(
            fs::read_dir(self.directory.path().join("sessions"))
                .unwrap()
                .count(),
            0
        );
        if let Some(pid) = fixture {
            assert!(
                process::start_time(pid).unwrap().is_none(),
                "fixture process survived"
            );
        }
        if let Ok(pid) = fs::read_to_string(self.directory.path().join("descendant-pid")) {
            let pid: u32 = pid.parse().unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while process::start_time(pid).unwrap().is_some() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            assert!(
                process::start_time(pid).unwrap().is_none(),
                "descendant survived"
            );
        }
        assert!(!self.startup_socket.exists());
    }

    fn wait_marker(&self, stream: &mut UnixStream) {
        let mut output = Vec::new();
        while !output.windows(13).any(|bytes| bytes == b"fixture-ready") {
            match receive(stream) {
                ServerMessage::Output(bytes) => output.extend(bytes),
                other => panic!("fixture not ready: {other:?}"),
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.pid != 0 {
            unsafe {
                libc::kill(self.pid as i32, libc::SIGTERM);
            }
            let _ = process::wait(self.pid);
        }
    }
}

fn send(stream: &mut UnixStream, message: ClientMessage) {
    write_frame(stream, &message.to_frame().unwrap()).unwrap();
}

fn receive(stream: &mut UnixStream) -> ServerMessage {
    ServerMessage::from_frame(read_frame(stream).unwrap()).unwrap()
}

/// #15/#19/#26: real binary, real PTY, readiness and exact final status.
#[test]
fn ready_publishes_private_metadata_and_exit_is_last() {
    let mut session = Session::start("exit");
    let metadata =
        SessionMetadata::from_json(&session.metadata, &fs::read(&session.metadata).unwrap())
            .unwrap();
    assert_eq!(metadata.daemon_pid, session.pid);
    assert_eq!(
        Some(metadata.daemon_start_time),
        process::start_time(session.pid).unwrap()
    );
    assert_eq!(metadata.climon2_version.as_str(), "0.1.0");
    assert_eq!(
        fs::read_dir(session.directory.path().join("sessions"))
            .unwrap()
            .count(),
        1
    );
    for (path, mode) in [
        (&session.metadata, 0o600),
        (&session.directory.path().join("sessions"), 0o700),
    ] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            mode
        );
    }
    let mut stream = session.attach();
    send(&mut stream, ClientMessage::Input(b"go\n".to_vec()));
    let mut output = Vec::new();
    loop {
        match receive(&mut stream) {
            ServerMessage::Output(bytes) => output.extend(bytes),
            ServerMessage::Exit { code } => {
                assert_eq!(code, 42);
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(output.windows(11).any(|bytes| bytes == b"final-bytes"));
    assert!(read_frame(&mut stream).is_err());
    session.finish();
}

/// #18/#35: long runtime-independent paths and a 103-byte socket do not enlarge the helper socket.
#[test]
fn long_home_and_executable_with_maximum_session_socket_start_and_clean_up() {
    let mut session = Session::start("long-paths");
    assert!(
        session
            .directory
            .path()
            .as_os_str()
            .as_encoded_bytes()
            .len()
            > 103
    );
    assert_eq!(session.socket.as_os_str().as_encoded_bytes().len(), 103);
    assert!(
        !session.startup_socket.exists(),
        "startup socket outlived readiness"
    );
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    send(&mut controlling, ClientMessage::Input(b"go\n".to_vec()));
    while !matches!(receive(&mut controlling), ServerMessage::Exit { code: 42 }) {}
    session.finish();
}

/// #19/#20: reaping does not lose a signal or race the first Attach.
#[test]
fn exact_signal_exit_and_early_exit_are_delivered() {
    for (mode, code) in [("signal", 143), ("early", 43)] {
        let mut session = Session::start(mode);
        if mode == "early" {
            thread::sleep(Duration::from_millis(150));
        }
        let mut stream = session.attach();
        if mode == "signal" {
            send(&mut stream, ClientMessage::Input(b"go\n".to_vec()));
        }
        let mut output = Vec::new();
        loop {
            match receive(&mut stream) {
                ServerMessage::Output(bytes) => output.extend(bytes),
                ServerMessage::Exit { code: actual } => {
                    assert_eq!(actual, code);
                    break;
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        if mode == "early" {
            assert!(output.windows(11).any(|bytes| bytes == b"early-final"));
        }
        session.finish();
    }
}

/// #20/#19: EOF/EIO may end drain before its idle/cap limits; no group member survives.
#[test]
fn final_drain_eof_or_deadline_is_bounded_and_leaves_no_descendants() {
    for mode in ["drain-idle", "drain-cap"] {
        let mut session = Session::start(mode);
        let mut controlling = session.attach();
        session.wait_marker(&mut controlling);
        send(&mut controlling, ClientMessage::Input(b"go\n".to_vec()));
        let started = Instant::now();
        let mut final_bytes = false;
        let mut tail = Vec::new();
        loop {
            match receive(&mut controlling) {
                ServerMessage::Output(bytes) => {
                    tail.extend(bytes);
                    if tail.windows(12).any(|bytes| bytes == b"leader-final") {
                        final_bytes = true;
                    }
                    if tail.len() > 11 {
                        tail.drain(..tail.len() - 11);
                    }
                }
                ServerMessage::Exit { code } => {
                    assert_eq!(code, 23);
                    break;
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(final_bytes);
        assert!(started.elapsed() < Duration::from_millis(2800));
        session.finish();
    }
}

/// #19: HUP/INT/PIPE are ignored by the daemon, not forwarded as leave requests.
#[test]
fn daemon_ignores_hup_int_pipe_without_ending_the_command() {
    let mut session = Session::start("exit");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGPIPE] {
        assert_eq!(unsafe { libc::kill(session.pid as i32, signal) }, 0);
    }
    drop(session.connect());
    send(&mut controlling, ClientMessage::Input(b"go\n".to_vec()));
    while !matches!(receive(&mut controlling), ServerMessage::Exit { code: 42 }) {}
    session.finish();
}

/// #20/#27: unfinished hellos and live probes never hold the controlling connection.
#[test]
fn concurrent_probes_absolute_hello_deadline_and_second_attach() {
    let mut session = Session::start("exit");
    drop(session.connect());
    assert!(session.metadata.exists());
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    let mut slow = UnixStream::connect(&session.socket).unwrap();
    slow.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    slow.write_all(&[0]).unwrap();
    let accepted = Instant::now();
    for _ in 0..8 {
        drop(session.connect());
    }
    let mut second = session.connect();
    send(
        &mut second,
        ClientMessage::Attach(TerminalSize { cols: 80, rows: 24 }),
    );
    assert_eq!(
        receive(&mut second),
        ServerMessage::Error {
            message: "session already has a controlling client".into()
        }
    );
    assert!(read_frame(&mut second).is_err());
    thread::sleep(Duration::from_millis(1200));
    slow.write_all(&[0]).unwrap();
    let reply = receive(&mut slow);
    assert!(matches!(reply, ServerMessage::Error { .. }));
    assert!(accepted.elapsed() >= Duration::from_secs(2));
    assert!(accepted.elapsed() < Duration::from_millis(2900));
    send(&mut controlling, ClientMessage::Input(b"go\n".to_vec()));
    while !matches!(receive(&mut controlling), ServerMessage::Exit { code: 42 }) {}
    session.finish();
}

/// #20: malformed probes are terminal but never end the session.
#[test]
fn malformed_unknown_oversized_and_out_of_state_messages_close_only_the_probe() {
    let mut session = Session::start("exit");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    for (hello, bytes) in [
        (false, vec![0, 0, 0, 1, 0x11]),
        (true, vec![0, 0, 0, 1, 0x11]),
        (false, vec![0, 0, 0, 2, 0x01, b'[']),
        (true, vec![0, 0, 0, 1, 0x66]),
        (true, vec![0, 0, 0, 0]),
        (true, vec![0, 16, 0, 1]),
        (true, vec![0, 1, 0, 1, 0x11]),
        (
            true,
            ClientMessage::Hello { protocol: 1 }
                .to_frame()
                .map(|frame| {
                    let mut bytes = Vec::new();
                    write_frame(&mut bytes, &frame).unwrap();
                    bytes
                })
                .unwrap(),
        ),
    ] {
        let mut probe = if hello {
            session.connect()
        } else {
            UnixStream::connect(&session.socket).unwrap()
        };
        probe
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        probe.write_all(&bytes).unwrap();
        assert!(matches!(receive(&mut probe), ServerMessage::Error { .. }));
        assert!(read_frame(&mut probe).is_err());
    }
    send(&mut controlling, ClientMessage::Input(b"go\n".to_vec()));
    while !matches!(receive(&mut controlling), ServerMessage::Exit { code: 42 }) {}
    session.finish();
}

/// #20: initial zero size falls back; changed Resize applies, zero dimensions are ignored.
#[test]
fn initial_fallback_and_resize_use_the_real_pty() {
    let mut session = Session::start("zero-size");
    let report = session.directory.path().join("initial-size");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !report.exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(fs::read_to_string(report).unwrap(), "80x24");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    send(
        &mut controlling,
        ClientMessage::Resize(TerminalSize {
            cols: 123,
            rows: 45,
        }),
    );
    send(
        &mut controlling,
        ClientMessage::Resize(TerminalSize { cols: 0, rows: 8 }),
    );
    send(&mut controlling, ClientMessage::Input(b"go\n".to_vec()));
    while !matches!(receive(&mut controlling), ServerMessage::Exit { code: 0 }) {}
    assert_eq!(
        fs::read_to_string(session.directory.path().join("resized")).unwrap(),
        "123x45"
    );
    session.finish();
}

/// #20: Input/Output are raw bytes, including NUL and non-UTF8, at the data-frame bound.
#[test]
fn raw_input_output_are_byte_transparent() {
    let mut session = Session::start("raw");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    let payload: Vec<u8> = (0..65535).map(|value| (value % 256) as u8).collect();
    send(&mut controlling, ClientMessage::Input(payload.clone()));
    let mut actual = Vec::new();
    loop {
        match receive(&mut controlling) {
            ServerMessage::Output(bytes) => actual.extend(bytes),
            ServerMessage::Exit { code } => {
                assert_eq!(code, 0);
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(actual, payload);
    session.finish();
}

/// #19/#27: all controlling leave causes share the real five-second policy.
#[test]
fn every_controlling_leave_hangs_up_then_kills_after_real_grace() {
    for cause in [
        "eof",
        "error",
        "repeated-hello",
        "unknown",
        "term",
        "input-backpressure",
        "second-attach",
    ] {
        let mut session = Session::start("ignore-hup");
        let mut controlling = session.attach();
        session.wait_marker(&mut controlling);
        let pid: u32 = fs::read_to_string(session.directory.path().join("fixture-pid"))
            .unwrap()
            .parse()
            .unwrap();
        let started = Instant::now();
        match cause {
            "eof" => drop(controlling),
            "error" => {
                send(
                    &mut controlling,
                    ClientMessage::Error {
                        message: "untrusted secret\nvalue".into(),
                    },
                );
                drop(controlling);
            }
            "repeated-hello" => {
                send(&mut controlling, ClientMessage::Hello { protocol: 1 });
                assert!(matches!(
                    receive(&mut controlling),
                    ServerMessage::Error { .. }
                ));
                drop(controlling);
            }
            "unknown" => {
                controlling.write_all(&[0, 0, 0, 1, 0x66]).unwrap();
                assert!(matches!(
                    receive(&mut controlling),
                    ServerMessage::Error { .. }
                ));
                drop(controlling);
            }
            "term" => unsafe {
                libc::kill(session.pid as i32, libc::SIGTERM);
            },
            "input-backpressure" => {
                for _ in 0..3 {
                    send(&mut controlling, ClientMessage::Input(vec![b'x'; 65535]));
                }
                drop(controlling);
            }
            "second-attach" => {
                send(
                    &mut controlling,
                    ClientMessage::Attach(TerminalSize { cols: 80, rows: 24 }),
                );
                assert_eq!(
                    receive(&mut controlling),
                    ServerMessage::Error {
                        message: "session already has a controlling client".into()
                    }
                );
                drop(controlling);
            }
            _ => unreachable!(),
        }
        thread::sleep(Duration::from_millis(200));
        assert!(
            process::start_time(pid).unwrap().is_some(),
            "{cause}: grace skipped"
        );
        session.finish();
        assert!(
            started.elapsed() >= Duration::from_secs(5),
            "{cause}: grace skipped"
        );
        assert!(
            started.elapsed() < Duration::from_secs(7),
            "{cause}: grace exceeded"
        );
        let log = fs::read_to_string(
            session
                .directory
                .path()
                .join(format!("logs/{ID}/daemon.log")),
        )
        .unwrap();
        assert!(!log.contains("untrusted secret"));
    }
}

/// #20: a stalled client bounds output buffering and the command itself stalls.
#[test]
fn saturation_backpressures_pty_without_blocking_probes_or_losing_bytes() {
    let mut session = Session::start("flood");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    send(&mut controlling, ClientMessage::Input(b"go\n".to_vec()));
    thread::sleep(Duration::from_millis(300));
    assert!(!session.directory.path().join("flood-finished").exists());
    drop(session.connect());
    let mut count = 0;
    loop {
        match receive(&mut controlling) {
            ServerMessage::Output(bytes) => {
                assert!(bytes.len() <= 65535);
                count += bytes.iter().filter(|byte| **byte == b'x').count();
            }
            ServerMessage::Exit { code } => {
                assert_eq!(code, 17);
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(count, 8 * 1024 * 1024);
    session.finish();
}

/// #15/#17: launch failures are reported before ready and never expose command content.
#[test]
fn invalid_cwd_missing_command_and_unsafe_sessions_are_startup_failures() {
    for (bad_cwd, missing, expected) in [(true, false, 126), (false, true, 127)] {
        let (session, readiness) = Session::attempt("exit", |_, _| {}, bad_cwd, missing);
        let daemon::Readiness::Failed { code, message } = readiness.unwrap() else {
            panic!("unexpected ready");
        };
        assert_eq!(code, expected, "{message}");
        assert!(!message.contains('\n'));
        assert!(!message.contains(session.directory.path().to_str().unwrap()));
        assert!(!session.metadata.exists());
        assert!(!session.socket.exists());
        assert!(!session.startup_socket.exists());
    }
    let (session, readiness) = Session::attempt(
        "exit",
        |_, home| {
            fs::create_dir(home.join("sessions")).unwrap();
            fs::set_permissions(home.join("sessions"), fs::Permissions::from_mode(0o755)).unwrap();
        },
        false,
        false,
    );
    assert!(matches!(
        readiness.unwrap(),
        daemon::Readiness::Failed { code: 125, .. }
    ));
    assert_eq!(
        fs::metadata(session.directory.path().join("sessions"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert!(!session.socket.exists());
    let (session, readiness) = Session::attempt(
        "cannot-run",
        |_, home| {
            fs::write(home.join("not-executable"), b"secret not executable\n").unwrap();
        },
        false,
        false,
    );
    assert!(matches!(
        readiness.unwrap(),
        daemon::Readiness::Failed { code: 126, .. }
    ));
    assert!(!session.socket.exists());
    assert!(!session.startup_socket.exists());
}

/// #25/#19: faults are named, hook-only, bounded by the production readiness deadline.
#[cfg(feature = "test-hooks")]
#[test]
fn controlled_panic_is_captured_and_never_ready_uses_real_startup_deadline() {
    for fault in ["panic", "never-ready"] {
        let started = Instant::now();
        let (session, readiness) = Session::attempt(
            "exit",
            |command, _| {
                command.env("CLIMON2_DAEMON_FAULT", fault);
            },
            false,
            false,
        );
        assert!(readiness.is_err());
        if fault == "never-ready" {
            assert!(started.elapsed() >= Duration::from_secs(5));
            assert!(started.elapsed() < Duration::from_secs(7));
        } else {
            let log = fs::read_to_string(
                session
                    .directory
                    .path()
                    .join(format!("logs/{ID}/daemon.log")),
            )
            .unwrap();
            assert!(log.contains("climon2: daemon panic"));
            assert!(!log.contains("controlled daemon fault"));
        }
        assert!(!session.metadata.exists());
        assert!(!session.socket.exists());
        assert!(!session.startup_socket.exists());
        assert!(!session.directory.path().join("fixture-pid").exists());
    }
}

/// #15/#29: the helper restores absent/empty/non-UTF8 SHELL and exact literal OS arguments.
#[cfg(feature = "test-hooks")]
#[test]
fn runtime_argv_cwd_and_inherited_environment_are_exact_not_display_metadata() {
    use std::{
        collections::BTreeMap,
        ffi::OsString,
        os::unix::ffi::{OsStrExt, OsStringExt},
    };
    for shell in [
        None,
        Some(OsString::new()),
        Some(OsString::from_vec(b"shell-\xff".to_vec())),
    ] {
        let mut session = Session::configured("literal", |command, _| {
            command.env(
                "SECRET_ENVIRONMENT",
                OsString::from_vec(b"private-\xfe".to_vec()),
            );
            command.env("CLIMON2_LOG", "trace");
            command.args(["--log", "warn"]);
            if let Some(shell) = &shell {
                command.env("SHELL", shell);
            }
        });
        let mut controlling = session.attach();
        let report = session.directory.path().join("report");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !report.with_extension("ready").exists() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        let bytes = fs::read(&report).unwrap();
        let mut input = bytes.as_slice();
        let mut fields = Vec::new();
        while !input.is_empty() {
            let mut size = [0; 4];
            input.read_exact(&mut size).unwrap();
            let mut field = vec![0; u32::from_be_bytes(size) as usize];
            input.read_exact(&mut field).unwrap();
            fields.push(field);
        }
        let cwd = fs::canonicalize(session.directory.path())
            .unwrap()
            .join("cwd space --literal é");
        assert_eq!(fields[0], cwd.as_os_str().as_bytes());
        assert_eq!(
            fields[1],
            env!("CARGO_BIN_EXE_climon2-daemon-fixture").as_bytes()
        );
        assert_eq!(
            &fields[2..6],
            &[
                b"".to_vec(),
                b"space argument".to_vec(),
                b"--looks-like-option".to_vec(),
                b"invalid-\xff".to_vec()
            ]
        );
        assert_eq!(fields[6], b"ENVIRONMENT");
        let environment: BTreeMap<_, _> = fields[7..]
            .chunks_exact(2)
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect();
        assert_eq!(
            environment.get(b"SHELL".as_slice()),
            shell
                .as_ref()
                .map(|value| value.as_os_str().as_bytes().to_vec())
                .as_ref()
        );
        assert_eq!(
            environment.get(b"SECRET_ENVIRONMENT".as_slice()).unwrap(),
            b"private-\xfe"
        );
        let mut expected: BTreeMap<Vec<u8>, Vec<u8>> = std::env::vars_os()
            .map(|(key, value)| {
                (
                    key.as_os_str().as_bytes().to_vec(),
                    value.as_os_str().as_bytes().to_vec(),
                )
            })
            .collect();
        expected.remove(b"SHELL".as_slice());
        expected.insert(b"DAEMON_FIXTURE_MODE".to_vec(), b"literal".to_vec());
        expected.insert(
            b"DAEMON_REPORT".to_vec(),
            report.as_os_str().as_bytes().to_vec(),
        );
        expected.insert(b"SECRET_ENVIRONMENT".to_vec(), b"private-\xfe".to_vec());
        expected.insert(b"CLIMON2_LOG".to_vec(), b"trace".to_vec());
        if let Some(shell) = &shell {
            expected.insert(b"SHELL".to_vec(), shell.as_os_str().as_bytes().to_vec());
        }
        assert!(environment == expected, "runtime environment was changed");
        send(&mut controlling, ClientMessage::Input(b"go\n".to_vec()));
        while !matches!(receive(&mut controlling), ServerMessage::Exit { code: 0 }) {}
        session.finish();
        let log = fs::read(
            session
                .directory
                .path()
                .join(format!("logs/{ID}/daemon.log")),
        )
        .unwrap();
        assert!(
            !log.windows(b"session ready".len())
                .any(|bytes| bytes == b"session ready"),
            "daemon ignored explicit warn filter"
        );
        for secret in [
            b"private-".as_slice(),
            b"space argument".as_slice(),
            b"invalid-".as_slice(),
        ] {
            assert!(!log.windows(secret.len()).any(|bytes| bytes == secret));
        }
    }
}

#[test]
#[ignore = "owned generic terminal fixture subprocess"]
fn fixture_child() {
    let mode = std::env::var("DAEMON_FIXTURE_MODE").unwrap();
    if mode.starts_with("holder-") {
        fs::write("descendant-pid", std::process::id().to_string()).unwrap();
        unsafe {
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
        }
        fs::write("descendant-ready", b"").unwrap();
        if mode == "holder-cap" {
            loop {
                if io::stdout().write_all(&vec![b'z'; 32768]).is_err() {
                    std::process::exit(0);
                }
            }
        }
        thread::sleep(Duration::from_secs(45));
        std::process::exit(0);
    }
    fs::write("fixture-pid", std::process::id().to_string()).unwrap();
    let mut size = std::mem::MaybeUninit::<libc::winsize>::uninit();
    assert_eq!(
        unsafe { libc::ioctl(0, libc::TIOCGWINSZ, size.as_mut_ptr()) },
        0
    );
    let size = unsafe { size.assume_init() };
    if mode == "zero-size" {
        fs::write("initial-size", format!("{}x{}", size.ws_col, size.ws_row)).unwrap();
    } else {
        assert_eq!((size.ws_col, size.ws_row), (91, 37));
    }
    if mode == "raw" {
        let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
        assert_eq!(unsafe { libc::tcgetattr(0, termios.as_mut_ptr()) }, 0);
        let mut termios = unsafe { termios.assume_init() };
        unsafe {
            libc::cfmakeraw(&mut termios);
        }
        assert_eq!(unsafe { libc::tcsetattr(0, libc::TCSANOW, &termios) }, 0);
    }
    if mode == "ignore-hup" {
        unsafe {
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
        }
    }
    print!("fixture-ready");
    io::stdout().flush().unwrap();
    if mode.starts_with("drain-") {
        let mut line = String::new();
        io::stdin().read_line(&mut line).unwrap();
        let _child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "fixture_child", "--ignored", "--nocapture"])
            .env(
                "DAEMON_FIXTURE_MODE",
                if mode == "drain-cap" {
                    "holder-cap"
                } else {
                    "holder-idle"
                },
            )
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !Path::new("descendant-ready").exists() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        print!("leader-final");
        io::stdout().flush().unwrap();
        std::process::exit(23);
    }
    if mode == "raw" {
        let mut payload = vec![0; 65535];
        io::stdin().read_exact(&mut payload).unwrap();
        io::stdout().write_all(&payload).unwrap();
        std::process::exit(0);
    }
    if mode == "zero-size" {
        let mut line = String::new();
        io::stdin().read_line(&mut line).unwrap();
        let mut size = std::mem::MaybeUninit::<libc::winsize>::uninit();
        assert_eq!(
            unsafe { libc::ioctl(0, libc::TIOCGWINSZ, size.as_mut_ptr()) },
            0
        );
        let size = unsafe { size.assume_init() };
        fs::write("resized", format!("{}x{}", size.ws_col, size.ws_row)).unwrap();
        std::process::exit(0);
    }
    if mode == "early" {
        print!("early-final");
        std::process::exit(43);
    }
    if mode == "exit" || mode == "signal" || mode == "long-paths" {
        let mut line = String::new();
        io::stdin().read_line(&mut line).unwrap();
        if mode == "signal" {
            unsafe {
                libc::raise(libc::SIGTERM);
            }
        }
        print!("final-bytes");
        std::process::exit(42);
    }
    if mode == "flood" {
        let mut line = String::new();
        io::stdin().read_line(&mut line).unwrap();
        let bytes = vec![b'x'; 32768];
        for _ in 0..256 {
            io::stdout().write_all(&bytes).unwrap();
        }
        fs::write("flood-finished", b"done").unwrap();
        std::process::exit(17);
    }
    thread::sleep(Duration::from_secs(45));
}
