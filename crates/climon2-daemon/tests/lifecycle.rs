use climon2_core::{
    hello::{HelloExtensions, Lifecycle},
    metadata::SessionMetadata,
    protocol::{ClientMessage, ServerMessage, TerminalSize, read_frame, write_frame},
};
use climon2_platform::{daemon, process, runtime::RuntimeDirectory};
use std::{
    cell::RefCell,
    fs,
    io::{self, Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

/// Daemon wire revision 1 beside the private protocol, through this harness.
mod revision1;

const ID: &str = "01234567-89ab-4cde-8fab-0123456789ab";

/// Test-only fixtures live in climon2-fixtures and are built beside this test binary (ADR 0004).
fn daemon_fixture() -> PathBuf {
    let path = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("climon2-daemon-fixture");
    assert!(
        path.is_file(),
        "missing {}; run cargo test --workspace",
        path.display()
    );
    path
}

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
    restored: RefCell<Vec<u8>>,
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
        let (prepared, command) = Prepared::new(mode, configure, bad_cwd);
        let Prepared {
            directory,
            home,
            socket,
            metadata,
            startup_socket,
        } = prepared;
        let pending = daemon::spawn(command, |command, fd| {
            command_tail(command, fd, &home, mode, missing);
        })
        .unwrap();
        let pid = pending.id();
        let readiness = pending.wait_ready(&[&metadata, &socket]);
        if !matches!(readiness, Ok(daemon::Readiness::Ready)) {
            assert_startup_command_gone(&home);
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
                restored: RefCell::new(Vec::new()),
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
        self.ready(&mut stream);
        stream
    }

    fn ready(&self, stream: &mut UnixStream) {
        let period = loop {
            match receive(stream) {
                ServerMessage::Granted { period } => break period,
                ServerMessage::Output(bytes) => self.restored.borrow_mut().extend(bytes),
                other => panic!("unexpected grant frame: {other:?}"),
            }
        };
        loop {
            match receive(stream) {
                ServerMessage::Output(bytes) => self.restored.borrow_mut().extend(bytes),
                ServerMessage::RestoreEnd { period: actual } => {
                    assert_eq!(actual, period);
                    break;
                }
                other => panic!("unexpected restoration frame: {other:?}"),
            }
        }
        send(stream, ClientMessage::Ready { period });
    }

    fn finish(&mut self) {
        let fixture = fs::read_to_string(self.directory.path().join("fixture-pid"))
            .ok()
            .and_then(|pid| pid.parse::<u32>().ok());
        let status = exit_by(std::mem::take(&mut self.pid), after(FINISH_BOUND));
        assert_eq!(
            status.map(|status| status.code().ok()),
            Some(Some(0)),
            "daemon log: {:?}",
            fs::read_to_string(self.directory.path().join(format!("logs/{ID}/daemon.log")))
        );
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
        let mut output = std::mem::take(&mut *self.restored.borrow_mut());
        while !output.windows(13).any(|bytes| bytes == b"fixture-ready") {
            match receive(stream) {
                ServerMessage::Output(bytes) => output.extend(bytes),
                other => panic!("fixture not ready: {other:?}"),
            }
        }
        self.restored.borrow_mut().extend(output);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.pid != 0 {
            unsafe {
                libc::kill(self.pid as i32, libc::SIGTERM);
            }
            exit_by(self.pid, after(FINISH_BOUND));
        }
    }
}

/// Generous for every orderly exit, the five-second leave grace included.
const FINISH_BOUND: Duration = Duration::from_secs(20);

fn after(bound: Duration) -> Instant {
    Instant::now() + bound
}

/// Reaps the daemon; one still running at `deadline` is killed and reaped instead, so a wedged
/// daemon fails its test rather than hanging it (and with it, every mutation run).
fn exit_by(pid: u32, deadline: Instant) -> Option<climon2_core::exit::ExitStatus> {
    while Instant::now() < deadline {
        if let Some(status) = process::try_wait(pid).unwrap() {
            return Some(status);
        }
        thread::sleep(Duration::from_millis(5));
    }
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    let _ = process::wait(pid);
    None
}

/// Whether `file` turns readable before `deadline`.
fn readable_by(file: &fs::File, deadline: Instant) -> bool {
    use std::os::fd::AsRawFd;
    let mut descriptor = [libc::pollfd {
        fd: file.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    }];
    let milliseconds = deadline
        .saturating_duration_since(Instant::now())
        .as_millis();
    let milliseconds = i32::try_from(milliseconds).unwrap_or(i32::MAX);
    unsafe { libc::poll(descriptor.as_mut_ptr(), 1, milliseconds) > 0 }
}

struct Prepared {
    directory: tempfile::TempDir,
    home: PathBuf,
    socket: PathBuf,
    metadata: PathBuf,
    startup_socket: PathBuf,
}

impl Prepared {
    fn new(
        mode: &str,
        configure: impl FnOnce(&mut Command, &Path),
        bad_cwd: bool,
    ) -> (Self, Command) {
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
            climon2_test_term::copy_executable(
                Path::new(env!("CARGO_BIN_EXE_climon2-daemon")),
                &executable,
            );
            executable
        } else {
            PathBuf::from(env!("CARGO_BIN_EXE_climon2-daemon"))
        };
        let mut command = Command::new(executable);
        command.args(["--session-id", ID, "--version", "0.1.0", "--cli-pid", "1"]);
        command.args(["--lifetime", "ordinary"]);
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
        (
            Self {
                directory,
                home,
                socket,
                metadata,
                startup_socket,
            },
            command,
        )
    }
}

fn command_tail(command: &mut Command, fd: u32, home: &Path, mode: &str, missing: bool) {
    command.args([
        "--scrollback-max-rows",
        mode.strip_prefix("retain-").unwrap_or("10000"),
        "--readiness-fd",
        &fd.to_string(),
        "--",
    ]);
    if missing {
        command.arg(home.join("missing-command"));
    } else if mode == "cannot-run" {
        command.arg(home.join("not-executable"));
    } else if mode == "job-control" {
        command.args(["bash", "--norc", "--noprofile", "-i"]);
    } else if mode == "literal" {
        command.arg(daemon_fixture());
        command.args(["", "space argument", "--looks-like-option"]);
        use std::os::unix::ffi::OsStringExt;
        command.arg(std::ffi::OsString::from_vec(b"invalid-\xff".to_vec()));
    } else {
        command.arg(std::env::current_exe().unwrap());
        command.args(["--exact", "fixture_child", "--ignored", "--nocapture"]);
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

fn assert_exit_after_attachment(mode: &str, code: u32) {
    let mut session = Session::start(mode);
    if mode == "early" {
        thread::sleep(Duration::from_millis(150));
    }
    let mut stream = if mode == "early" {
        let mut stream = session.connect();
        send(
            &mut stream,
            ClientMessage::Attach(TerminalSize { cols: 91, rows: 37 }),
        );
        assert_eq!(receive(&mut stream), ServerMessage::Attached);
        session.ready(&mut stream);
        stream
    } else {
        session.attach()
    };
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
        let mut complete = std::mem::take(&mut *session.restored.borrow_mut());
        complete.extend(output);
        assert!(
            complete.windows(11).any(|bytes| bytes == b"early-final"),
            "final PTY bytes were not delivered before the result"
        );
    }
    session.finish();
}

/// #19: reaping preserves an exact signal exit status.
#[test]
fn exact_signal_exit_is_delivered() {
    assert_exit_after_attachment("signal", 143);
}

/// #20: an early exit still completes restoration readiness before its result.
#[test]
fn early_exit_is_delivered_after_restore_readiness() {
    assert_exit_after_attachment("early", 43);
}

/// #20: a drained exit waits for the controller's `Ready`, however long it is withheld, so
/// the controller is never handed a result before it confirmed its restoration.
#[test]
fn an_exit_waits_for_the_controllers_ready() {
    let mut session = Session::start("early-after-restore");
    let mut stream = attach_at(&session, TerminalSize { cols: 91, rows: 37 });
    let (period, _) = restoration(&mut stream);
    fs::write(session.directory.path().join("finish-early"), b"").unwrap();
    let mut output = Vec::new();
    while !output.windows(11).any(|bytes| bytes == b"early-final") {
        match receive(&mut stream) {
            ServerMessage::Output(bytes) => output.extend(bytes),
            other => panic!("a result before Ready: {other:?}"),
        }
    }
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let withheld = read_frame(&mut stream).map(|frame| frame.type_code());
    assert!(
        withheld
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock),
        "a frame before Ready: {withheld:?}"
    );
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    send(&mut stream, ClientMessage::Ready { period });
    let (_, result) = until_terminal(&mut stream);
    assert_eq!(result, ServerMessage::Exit { code: 43 });
    session.finish();
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

/// #20/#27: unfinished hellos and live probes never hold the controlling connection, and a
/// hello's deadline runs from accept however recently the peer wrote.
#[test]
fn concurrent_probes_absolute_hello_deadline_and_second_attach() {
    let mut session = Session::start("exit");
    drop(session.connect());
    assert!(session.metadata.exists());
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    let mut slow = UnixStream::connect(&session.socket).unwrap();
    let accepted = Instant::now();
    slow.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let trickle = trickle(slow.try_clone().unwrap());
    for _ in 0..8 {
        drop(session.connect());
    }
    let mut second = session.connect();
    send(
        &mut second,
        ClientMessage::Attach(TerminalSize { cols: 80, rows: 24 }),
    );
    assert_eq!(receive(&mut second), ServerMessage::Attached);
    session.ready(&mut second);
    let reply = receive(&mut slow);
    assert!(matches!(reply, ServerMessage::Error { .. }));
    assert!(accepted.elapsed() >= Duration::from_secs(2));
    trickle.join().unwrap();
    send(
        &mut controlling,
        ClientMessage::Input(b"ignored\n".to_vec()),
    );
    send(&mut second, ClientMessage::Input(b"go\n".to_vec()));
    while !matches!(receive(&mut second), ServerMessage::Exit { code: 42 }) {}
    session.finish();
}

/// Writes a hello that never finishes and never goes quiet: a 256-byte control frame's
/// prefix and type, then a payload byte every 50 ms until the daemon closes the connection.
/// A deadline that restarted on each byte would not expire within the reader's 10 seconds.
fn trickle(mut stream: UnixStream) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if stream.write_all(&[0, 0, 1, 0, 0x01]).is_err() {
            return;
        }
        for _ in 0..254 {
            thread::sleep(Duration::from_millis(50));
            if stream.write_all(b" ").is_err() {
                return;
            }
        }
    })
}

/// #20: malformed probes are terminal but never end the session.
#[test]
fn malformed_unknown_oversized_and_out_of_state_messages_close_only_the_probe() {
    let mut session = Session::start("exit");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    // A decoding failure names itself; any other framing failure is generic.
    let not_allowed = "message not allowed in connection state";
    let generic = "invalid session protocol";
    for (hello, bytes, message) in [
        (false, vec![0, 0, 0, 1, 0x11], not_allowed),
        (true, vec![0, 0, 0, 1, 0x11], not_allowed),
        (
            false,
            vec![0, 0, 0, 2, 0x01, b'['],
            "malformed control JSON",
        ),
        (true, vec![0, 0, 0, 1, 0x66], "unknown message type"),
        (true, vec![0, 0, 0, 0], generic),
        (true, vec![0, 16, 0, 1], generic),
        (true, vec![0, 1, 0, 1, 0x11], generic),
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
            not_allowed,
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
        assert_eq!(
            receive(&mut probe),
            ServerMessage::Error {
                message: message.into()
            },
            "{bytes:?}"
        );
        assert!(read_frame(&mut probe).is_err());
    }
    send(&mut controlling, ClientMessage::Input(b"go\n".to_vec()));
    while !matches!(receive(&mut controlling), ServerMessage::Exit { code: 42 }) {}
    session.finish();
}

/// #20: the daemon serves at most 64 connections at a time; one more is closed unanswered
/// and the session goes on.
#[test]
fn a_connection_beyond_the_cap_is_closed_unanswered() {
    let mut session = Session::start("exit");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    let probes: Vec<_> = (1..64).map(|_| session.connect()).collect();
    let mut extra = UnixStream::connect(&session.socket).unwrap();
    extra
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    assert_eq!(extra.read(&mut [0]).unwrap(), 0);
    drop(probes);
    send(&mut controlling, ClientMessage::Input(b"go\n".to_vec()));
    while !matches!(receive(&mut controlling), ServerMessage::Exit { code: 42 }) {}
    session.finish();
}

/// #20: initial zero size falls back; changed Resize applies, zero dimensions are ignored.
#[test]
fn initial_fallback_and_resize_use_the_real_pty() {
    let mut session = Session::start("zero-size");
    zero_size_started(&session);
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

fn release_at(row: u16, col: u16) -> Vec<u8> {
    let mut bytes = climon2_vt::Presentation::new(91, 37).release();
    bytes.extend(format!("\x1b[{row};{col}H").bytes());
    bytes
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
    assert_eq!(&actual[..payload.len()], payload);
    assert_eq!(&actual[payload.len()..], release_at(37, 83));
    session.finish();
}

#[test]
fn attached_client_restores_a_waiting_alternate_screen_before_input_is_ready() {
    let mut session = Session::start("tui-restore");
    let ready = session.directory.path().join("tui-ready");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "TUI fixture did not draw");
        thread::sleep(Duration::from_millis(2));
    }

    let mut stream = session.connect();
    send(
        &mut stream,
        ClientMessage::Attach(TerminalSize { cols: 91, rows: 37 }),
    );
    let mut restoration = Vec::new();
    let period = loop {
        match receive(&mut stream) {
            ServerMessage::Attached => {}
            ServerMessage::Granted { period } => break period,
            ServerMessage::Output(bytes) => restoration.extend(bytes),
            other => panic!("unexpected initial restoration frame: {other:?}"),
        }
    };
    loop {
        match receive(&mut stream) {
            ServerMessage::Output(bytes) => restoration.extend(bytes),
            ServerMessage::RestoreEnd { period: actual } => {
                assert_eq!(actual, period);
                break;
            }
            other => panic!("unexpected restoration frame: {other:?}"),
        }
    }
    for sequence in [
        b"RESTORED-DISPLAY".as_slice(),
        b"\x1b[?1049h",
        b"\x1b[?1000h",
        b"\x1b[?2004h",
        b"\x1b[?1h",
    ] {
        assert!(
            restoration
                .windows(sequence.len())
                .any(|bytes| bytes == sequence),
            "restoration omitted {sequence:?}: {restoration:?}"
        );
    }
    assert_eq!(
        fs::read_to_string(session.directory.path().join("draw-count")).unwrap(),
        "1"
    );

    send(&mut stream, ClientMessage::Ready { period });
    send(&mut stream, ClientMessage::Input(vec![b'x']));
    let deadline = Instant::now() + Duration::from_secs(2);
    while !fs::read_to_string(session.directory.path().join("input-accepted"))
        .is_ok_and(|value| value == "x")
    {
        assert!(
            Instant::now() < deadline,
            "input did not reach the waiting TUI"
        );
        thread::sleep(Duration::from_millis(2));
    }
    let mut output = Vec::new();
    loop {
        match receive(&mut stream) {
            ServerMessage::Output(bytes) => output.extend(bytes),
            ServerMessage::Exit { code } => {
                assert_eq!(code, 42);
                break;
            }
            other => panic!("unexpected TUI result: {other:?}"),
        }
    }
    assert!(
        output
            .windows(b"INPUT-ACCEPTED".len())
            .any(|bytes| bytes == b"INPUT-ACCEPTED")
    );
    session.finish();
}

/// #19/#27: all controlling leave causes share the real five-second policy.
/// HDL-01 (Contract layer), ordinary case: an ordinary session's final
/// unexpected departure still ends it.
#[test]
fn every_controlling_leave_hangs_up_then_kills_after_real_grace() {
    for cause in [
        "eof",
        "error",
        "repeated-hello",
        "unknown",
        "term",
        "input-backpressure",
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

fn until_terminal(stream: &mut UnixStream) -> (Vec<u8>, ServerMessage) {
    let mut output = Vec::new();
    loop {
        match receive(stream) {
            ServerMessage::Output(bytes) => output.extend(bytes),
            other => return (output, other),
        }
    }
}

fn restoration(stream: &mut UnixStream) -> (u64, Vec<u8>) {
    let mut bytes = Vec::new();
    let period = loop {
        match receive(stream) {
            ServerMessage::Granted { period } => break period,
            ServerMessage::Output(output) => bytes.extend(output),
            other => panic!("unexpected grant frame: {other:?}"),
        }
    };
    let (prefix, end) = until_terminal(stream);
    assert_eq!(end, ServerMessage::RestoreEnd { period });
    bytes.extend(prefix);
    (period, bytes)
}

fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        thread::sleep(Duration::from_millis(2));
    }
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|bytes| *bytes == needle)
        .count()
}

fn advertised_state(session: &Session) -> (Lifecycle, u32, bool) {
    let mut probe = UnixStream::connect(&session.socket).unwrap();
    probe
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    send(&mut probe, ClientMessage::Hello { protocol: 1 });
    let extensions = HelloExtensions::from_body(read_frame(&mut probe).unwrap().body());
    let capabilities = extensions.capabilities.valid().unwrap();
    assert!(capabilities.supports_attach() && capabilities.supports_kill());
    assert!(capabilities.supports_state());
    let state = extensions.state.valid().unwrap();
    (state.lifecycle(), state.attachments(), state.controlled())
}

fn attach_at(session: &Session, size: TerminalSize) -> UnixStream {
    let mut stream = session.connect();
    send(&mut stream, ClientMessage::Attach(size));
    assert_eq!(receive(&mut stream), ServerMessage::Attached);
    stream
}

/// Section 6.2 release ordering: release bytes, then `Detached`; the session
/// outlives the detach, advertises its state and accepts a reattachment.
#[test]
fn detach_releases_then_confirms_and_a_reattachment_resumes() {
    let mut session = Session::start("exit");
    assert_eq!(advertised_state(&session), (Lifecycle::Starting, 0, false));
    let mut first = session.attach();
    session.wait_marker(&mut first);
    assert_eq!(advertised_state(&session), (Lifecycle::Attached, 1, true));
    send(&mut first, ClientMessage::Detach);
    let (released, confirmation) = until_terminal(&mut first);
    assert_eq!(confirmation, ServerMessage::Detached);
    assert_eq!(released, release_at(3, 14));
    assert!(read_frame(&mut first).is_err());
    assert_eq!(advertised_state(&session), (Lifecycle::Detached, 0, false));

    let mut second = attach_at(&session, TerminalSize { cols: 91, rows: 37 });
    let (period, restored) = restoration(&mut second);
    assert_eq!(count(&restored, b"fixture-ready"), 1);
    send(&mut second, ClientMessage::Ready { period });
    send(&mut second, ClientMessage::Input(b"go\n".to_vec()));
    let (output, result) = until_terminal(&mut second);
    assert_eq!(result, ServerMessage::Exit { code: 42 });
    assert!(output.ends_with(&release_at(4, 12)));
    assert_eq!(count(&output, b"final-bytes"), 1);
    session.finish();
}

/// Section 6.2: a newer attachment takes control; the old controller gets its
/// release (leaving the alternate screen it entered) and then `Revoked`, and
/// may take control back. Exit from the alternate screen releases it too.
#[test]
fn takeover_revokes_with_release_and_exit_leaves_the_alternate_screen() {
    let mut session = Session::start("tui-restore");
    wait_file(&session.directory.path().join("tui-ready"));
    let mut first = attach_at(&session, TerminalSize { cols: 91, rows: 37 });
    let mut host = vt100::Parser::new(
        std::num::NonZeroU16::new(37).unwrap(),
        std::num::NonZeroU16::new(91).unwrap(),
        0,
    );
    let (period, restored) = restoration(&mut first);
    host.process(&restored);
    send(&mut first, ClientMessage::Ready { period });
    let mut second = attach_at(&session, TerminalSize { cols: 80, rows: 24 });
    let (released, revoked) = until_terminal(&mut first);
    assert_eq!(revoked, ServerMessage::Revoked);
    assert_eq!(count(&released, b"\x1b[?1049l"), 1);
    host.process(&released);
    assert_ne!(host.screen().cursor_position(), (3, 20));
    let (period, restored) = restoration(&mut second);
    assert_eq!(count(&restored, b"RESTORED-DISPLAY"), 1);
    send(&mut second, ClientMessage::Ready { period });

    send(&mut first, ClientMessage::TakeControl);
    let (released, revoked) = until_terminal(&mut second);
    assert_eq!(revoked, ServerMessage::Revoked);
    assert_eq!(count(&released, b"\x1b[?1049l"), 1);
    let (period, restored) = restoration(&mut first);
    host.process(&restored);
    assert_eq!(host.screen().cursor_position(), (3, 20));
    send(&mut first, ClientMessage::Ready { period });
    send(&mut first, ClientMessage::Input(vec![b'x']));
    let (output, result) = until_terminal(&mut first);
    assert_eq!(result, ServerMessage::Exit { code: 42 });
    let accepted = output
        .windows(14)
        .position(|bytes| bytes == b"INPUT-ACCEPTED")
        .unwrap();
    assert_eq!(count(&output[accepted..], b"\x1b[?1049l"), 1);
    let (observed, result) = until_terminal(&mut second);
    assert_eq!(result, ServerMessage::Exit { code: 42 });
    assert!(observed.is_empty(), "{observed:?}");
    session.finish();
}

fn wait_contents(path: &Path, done: impl Fn(&[u8]) -> bool) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let contents = fs::read(path).unwrap_or_default();
        if done(&contents) {
            return contents;
        }
        assert!(
            Instant::now() < deadline,
            "{} stayed {:?}",
            path.display(),
            String::from_utf8_lossy(&contents)
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// Waits for the `zero-size` fixture, which starts at the 80x24 fallback,
/// to report its initial size, so a later grant resizes its terminal.
fn zero_size_started(session: &Session) {
    let report = session.directory.path().join("initial-size");
    wait_contents(&report, |size| size == b"80x24");
}

fn wait_line(session: &Session, line: &str) {
    let path = session.directory.path().join("lines");
    wait_contents(&path, |contents| {
        String::from_utf8_lossy(contents)
            .lines()
            .any(|seen| seen == line)
    });
}

/// Section 1.5/1.6: the controller's size drives the PTY; a non-controller's
/// resize is only remembered; input from a non-controller or from a controller
/// before its `Ready` is dropped; and hand-back applies the remaining size.
#[test]
fn the_controller_drives_the_size_and_input_outside_its_period_is_dropped() {
    let mut session = Session::start("lines");
    let mut first = session.attach();
    session.wait_marker(&mut first);
    send(&mut first, ClientMessage::Input(b"a1\n".to_vec()));
    wait_line(&session, "a1 91x37");

    let mut second = attach_at(&session, TerminalSize { cols: 80, rows: 24 });
    let (released, revoked) = until_terminal(&mut first);
    assert_eq!(revoked, ServerMessage::Revoked);
    assert!(released.ends_with(&release_at(4, 1)));
    send(&mut first, ClientMessage::Input(b"stale\n".to_vec()));
    let resized = TerminalSize {
        cols: 100,
        rows: 30,
    };
    send(&mut first, ClientMessage::Resize(resized));
    let (period, _) = restoration(&mut second);
    send(&mut second, ClientMessage::Ready { period });
    send(&mut second, ClientMessage::Input(b"b1\n".to_vec()));
    wait_line(&session, "b1 80x24");

    send(&mut first, ClientMessage::TakeControl);
    let (_, revoked) = until_terminal(&mut second);
    assert_eq!(revoked, ServerMessage::Revoked);
    let (period, _) = restoration(&mut first);
    send(&mut first, ClientMessage::Input(b"early\n".to_vec()));
    send(&mut first, ClientMessage::Ready { period });
    send(&mut first, ClientMessage::Input(b"a2\n".to_vec()));
    wait_line(&session, "a2 100x30");

    send(&mut first, ClientMessage::Detach);
    let (_, detached) = until_terminal(&mut first);
    assert_eq!(detached, ServerMessage::Detached);
    let (period, _) = restoration(&mut second);
    send(&mut second, ClientMessage::Ready { period });
    send(&mut second, ClientMessage::Input(b"b2\nend\n".to_vec()));
    let (_, result) = until_terminal(&mut second);
    assert_eq!(result, ServerMessage::Exit { code: 0 });
    session.finish();
    assert_eq!(
        fs::read_to_string(session.directory.path().join("lines")).unwrap(),
        "a1 91x37\nb1 80x24\na2 100x30\nb2 80x24\n"
    );
}

/// Section 1.5 paste cleanup: a paste cut short by revocation is closed with
/// exactly the closing marker, written whole across one-byte PTY writes,
/// before the next controller's input.
#[test]
fn a_revoked_paste_is_closed_before_the_next_controllers_input() {
    let mut session = Session::configured("paste-record", |command, _| {
        command.env("CLIMON2_DAEMON_FAULT", "pty-write-partial");
    });
    let mut first = session.attach();
    session.wait_marker(&mut first);
    let mut paste = b"\x1b[200~".to_vec();
    paste.extend([b'p'; 400]);
    send(&mut first, ClientMessage::Input(paste));
    let path = session.directory.path().join("pasted");
    wait_contents(&path, |contents| contents.len() >= 8);

    let mut second = attach_at(&session, TerminalSize { cols: 91, rows: 37 });
    let (_, revoked) = until_terminal(&mut first);
    assert_eq!(revoked, ServerMessage::Revoked);
    let (period, _) = restoration(&mut second);
    send(&mut second, ClientMessage::Ready { period });
    send(&mut second, ClientMessage::Input(b"after-END".to_vec()));
    let (_, result) = until_terminal(&mut second);
    assert_eq!(result, ServerMessage::Exit { code: 0 });
    session.finish();

    let pasted = fs::read(&path).unwrap();
    let payload = pasted
        .strip_prefix(b"\x1b[200~".as_slice())
        .and_then(|rest| rest.strip_suffix(b"\x1b[201~after-END".as_slice()))
        .unwrap_or_else(|| panic!("{:?}", String::from_utf8_lossy(&pasted)));
    assert!(payload.iter().all(|byte| *byte == b'p'), "{payload:?}");
    assert!((1..400).contains(&payload.len()), "{}", payload.len());
}

/// Section 3.3 one PTY writer: a terminal reply produced while input is being
/// written, one byte at a time, follows that input whole and never splits it.
#[test]
fn a_terminal_reply_never_interleaves_with_input_being_written() {
    let mut session = Session::configured("reply-record", |command, _| {
        command.env("CLIMON2_DAEMON_FAULT", "pty-write-partial");
    });
    let mut client = session.attach();
    session.wait_marker(&mut client);
    send(&mut client, ClientMessage::Input(vec![b'i'; 100]));
    let path = session.directory.path().join("recorded");
    wait_contents(&path, |contents| contents.contains(&b'c'));
    send(&mut client, ClientMessage::Input(b"END".to_vec()));
    let (_, result) = until_terminal(&mut client);
    assert_eq!(result, ServerMessage::Exit { code: 0 });
    session.finish();

    let recorded = fs::read(&path).unwrap();
    let reply = recorded
        .strip_prefix([b'i'; 100].as_slice())
        .and_then(|rest| rest.strip_suffix(b"END".as_slice()))
        .unwrap_or_else(|| panic!("{:?}", String::from_utf8_lossy(&recorded)));
    assert!(
        reply.starts_with(b"\x1b[?") && reply.ends_with(b"c") && !reply.contains(&b'i'),
        "{:?}",
        String::from_utf8_lossy(reply)
    );
}

/// Section 6.2 replacement restoration: a resize before `Ready` retires the
/// period; the new prologue leaves the committed alternate screen exactly once.
#[test]
fn resize_before_ready_replaces_the_restoration_period() {
    let mut session = Session::start("tui-restore");
    wait_file(&session.directory.path().join("tui-ready"));
    let mut stream = attach_at(&session, TerminalSize { cols: 91, rows: 37 });
    let (first, restored) = restoration(&mut stream);
    assert_eq!(count(&restored, b"\x1b[?1049h"), 1);
    send(
        &mut stream,
        ClientMessage::Resize(TerminalSize {
            cols: 100,
            rows: 40,
        }),
    );
    let (second, replacement) = restoration(&mut stream);
    assert_eq!(second, first + 1);
    assert!(replacement.starts_with(b"\x1b[?1049l"), "{replacement:?}");
    assert_eq!(count(&replacement, b"\x1b[?1049l"), 1);
    assert_eq!(count(&replacement, b"RESTORED-DISPLAY"), 1);
    send(&mut stream, ClientMessage::Ready { period: first });
    send(&mut stream, ClientMessage::Ready { period: second });
    send(&mut stream, ClientMessage::Input(vec![b'x']));
    let (_, result) = until_terminal(&mut stream);
    assert_eq!(result, ServerMessage::Exit { code: 42 });
    session.finish();
}

/// Section 6.2 resize burst: one write of several resizes is one PTY resize.
#[test]
fn a_resize_burst_is_one_pty_resize_to_the_last_size() {
    let mut session = Session::start("winch");
    let mut stream = session.attach();
    session.wait_marker(&mut stream);
    let mut burst = Vec::new();
    for cols in [100, 101, 102] {
        let resize = ClientMessage::Resize(TerminalSize { cols, rows: 30 });
        write_frame(&mut burst, &resize.to_frame().unwrap()).unwrap();
    }
    stream.write_all(&burst).unwrap();
    thread::sleep(Duration::from_millis(300));
    send(&mut stream, ClientMessage::Input(b"go\n".to_vec()));
    let (_, result) = until_terminal(&mut stream);
    assert_eq!(result, ServerMessage::Exit { code: 0 });
    assert_eq!(
        fs::read_to_string(session.directory.path().join("resized")).unwrap(),
        "1 102x30"
    );
    session.finish();
}

/// Section 6.2 unfinished token: a pending `ESC [` does not hold restoration.
/// Kill then confirms the command's result to the requester.
#[test]
fn an_unfinished_token_still_restores_and_kill_confirms_termination() {
    let mut session = Session::start("partial");
    wait_file(&session.directory.path().join("partial-ready"));
    let mut stream = session.attach();
    send(&mut stream, ClientMessage::Detach);
    assert_eq!(until_terminal(&mut stream).1, ServerMessage::Detached);
    let mut killer = session.connect();
    send(&mut killer, ClientMessage::Kill);
    assert!(matches!(
        receive(&mut killer),
        ServerMessage::Terminated { .. }
    ));
    session.finish();
}

/// Section 3.3: hanging up closes the command's terminal without typing into
/// it. A newline or end-of-file key written on the way out would reach the
/// command as if a client had sent it, for example running a shell's
/// half-typed command line.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn hanging_up_types_nothing_into_the_command_terminal() {
    let mut session = Session::start("ignore-hup");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    let mut killer = session.connect();
    // The hang-up scans every process's procfs entry; each call stops here.
    let tracer = climon2_test_term::ptrace::Tracer::seize(session.pid, Duration::from_secs(60));
    send(&mut killer, ClientMessage::Kill);
    let master = Path::new("/dev/ptmx");
    let (write, close) = (libc::SYS_write as u64, libc::SYS_close as u64);
    let mut typed = Vec::new();
    loop {
        let call = tracer.next_entry();
        if ![write, close].contains(&call.number)
            || tracer.descriptor(call.arguments[0]).as_deref() != Some(master)
        {
            continue;
        }
        if call.number == write {
            typed.push(tracer.memory(call.arguments[1], call.arguments[2]));
            continue;
        }
        tracer.finish_syscall();
        if !holds(session.pid, master) {
            break;
        }
    }
    tracer.detach();
    assert!(
        typed.is_empty(),
        "hang-up typed {typed:?} into the terminal"
    );
    assert!(matches!(
        until_terminal(&mut controlling).1,
        ServerMessage::Exit { .. }
    ));
    assert!(matches!(
        receive(&mut killer),
        ServerMessage::Terminated { .. }
    ));
    session.finish();
}

/// Whether `pid` still has a descriptor open on `target`.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn holds(pid: u32, target: &Path) -> bool {
    fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .filter_map(|entry| fs::read_link(entry.unwrap().path()).ok())
        .any(|link| link == target)
}

/// Section 1.7: attaching while the session terminates is refused.
#[test]
fn attach_during_termination_is_refused() {
    let mut session = Session::start("ignore-hup");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    // The kill is serviced first; the attachment request is read while the
    // session terminates.
    let mut killer = session.connect();
    let mut late = session.connect();
    send(&mut killer, ClientMessage::Kill);
    send(
        &mut late,
        ClientMessage::Attach(TerminalSize { cols: 91, rows: 37 }),
    );
    assert!(matches!(receive(&mut late), ServerMessage::Error { .. }));
    let (released, result) = until_terminal(&mut controlling);
    assert!(matches!(result, ServerMessage::Exit { .. }));
    assert_eq!(released, release_at(3, 14));
    assert!(matches!(
        receive(&mut killer),
        ServerMessage::Terminated { .. }
    ));
    session.finish();
}

/// Section 3.3: the hang-up grace is a loop deadline. Probes promptly see
/// `terminating`, attaches are refused, a second kill joins without
/// restarting the grace, and both requesters are confirmed only after cleanup.
#[test]
fn the_hang_up_grace_keeps_serving_and_a_second_kill_joins_it() {
    let mut session = Session::start("ignore-hup");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    let mut first = session.connect();
    first
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let started = Instant::now();
    send(&mut first, ClientMessage::Kill);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(advertised_state(&session).0, Lifecycle::Terminating);
    let mut late = session.connect();
    send(
        &mut late,
        ClientMessage::Attach(TerminalSize { cols: 91, rows: 37 }),
    );
    assert!(matches!(receive(&mut late), ServerMessage::Error { .. }));
    let mut second = session.connect();
    second
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    send(&mut second, ClientMessage::Kill);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "grace blocked the loop"
    );
    assert!(matches!(
        receive(&mut first),
        ServerMessage::Terminated { .. }
    ));
    let confirmed = started.elapsed();
    assert!(!session.metadata.exists() && !session.socket.exists());
    assert!(matches!(
        receive(&mut second),
        ServerMessage::Terminated { .. }
    ));
    assert!(confirmed >= Duration::from_secs(5), "{confirmed:?}");
    assert!(
        started.elapsed() < Duration::from_millis(6500),
        "grace restarted"
    );
    assert!(matches!(
        until_terminal(&mut controlling).1,
        ServerMessage::Exit { .. }
    ));
    session.finish();
}

/// Section 1.5: killing with an application paste open queues cleanup bytes
/// that can no longer reach the hung-up terminal; termination still completes.
#[test]
fn a_kill_with_an_open_application_paste_still_terminates() {
    let mut session = Session::start("paste");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    send(
        &mut controlling,
        ClientMessage::Input(b"\x1b[200~abc".to_vec()),
    );
    thread::sleep(Duration::from_millis(200));
    let mut killer = session.connect();
    killer
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    send(&mut killer, ClientMessage::Kill);
    assert!(matches!(
        receive(&mut killer),
        ServerMessage::Terminated { .. }
    ));
    assert!(matches!(
        until_terminal(&mut controlling).1,
        ServerMessage::Exit { .. }
    ));
    session.finish();
}

/// Section 6.2 authoritative failure: a PTY write fault releases the
/// controller, then reports `Error`, then cleans up.
#[test]
fn a_pty_write_failure_releases_then_reports_error() {
    let mut session = Session::configured("exit", |command, _| {
        command.env("CLIMON2_DAEMON_FAULT", "pty-write");
    });
    let mut stream = session.attach();
    session.wait_marker(&mut stream);
    send(&mut stream, ClientMessage::Input(b"go\n".to_vec()));
    let (released, result) = until_terminal(&mut stream);
    assert!(matches!(result, ServerMessage::Error { .. }), "{result:?}");
    assert_eq!(released, release_at(3, 14));
    session.finish();
}

/// Initial deadline: a command that ends before anyone attaches keeps the
/// session for the five-second initial attach window, then it ends.
/// HDL-01 (Contract layer), ordinary case: an ordinary session keeps this rule.
#[test]
fn an_unattached_session_ends_after_the_initial_deadline() {
    let mut session = Session::start("early");
    let started = Instant::now();
    thread::sleep(Duration::from_millis(500));
    assert!(session.socket.exists());
    session.finish();
    assert!(started.elapsed() >= Duration::from_millis(4500));
    assert!(started.elapsed() < Duration::from_secs(7));
}

/// A session started like `c2 exec --headless` would start it. Its fixture
/// ends itself after two minutes, so a test killed before it ends its session
/// cannot leave that session running forever.
fn headless(mode: &str) -> Session {
    Session::configured(mode, |command, _| {
        *command = replace_option(command, "--lifetime", "headless");
        command.env("DAEMON_FIXTURE_LIFE", "120");
    })
}

/// HDL-01 (Contract layer), zero clients from startup: a headless session
/// advertises `detached` from startup on and outlives the five-second initial
/// attach window. The reference ordinary session starts after it, so its end at
/// its own initial deadline shows that window has passed for the headless one,
/// whose command still runs and reports its exit to a later attachment.
#[test]
fn a_headless_session_advertises_detached_and_outlives_the_initial_attach_window() {
    let mut headless = headless("exit");
    assert_eq!(advertised_state(&headless), (Lifecycle::Detached, 0, false));
    let mut ordinary = Session::start("early");
    assert_eq!(advertised_state(&ordinary), (Lifecycle::Starting, 0, false));
    ordinary.finish();
    assert_eq!(advertised_state(&headless), (Lifecycle::Detached, 0, false));

    let mut stream = headless.attach();
    headless.wait_marker(&mut stream);
    assert_eq!(advertised_state(&headless), (Lifecycle::Attached, 1, true));
    send(&mut stream, ClientMessage::Input(b"go\n".to_vec()));
    let (output, result) = until_terminal(&mut stream);
    assert_eq!(result, ServerMessage::Exit { code: 42 });
    assert_eq!(count(&output, b"final-bytes"), 1);
    headless.finish();
}

/// HDL-01 (Contract layer), command completion with zero clients and no initial
/// attach hold: a headless session whose command ends before anything attaches
/// ends with it, cleaned up, without keeping the exit for an initial
/// attachment. The ordinary reference session starts first and nothing attaches
/// to it, so its initial attach window closes before a later session's would:
/// it still advertising `starting` once the headless session is gone shows the
/// headless one held nothing until its own deadline. Kill then ends the
/// reference.
#[test]
fn a_headless_session_ends_with_its_command_without_the_initial_attach_hold() {
    let ordinary = Session::start("exit");
    let mut headless = headless("early");
    headless.finish();
    assert_eq!(advertised_state(&ordinary), (Lifecycle::Starting, 0, false));
    kill(ordinary);
}

/// HDL-01 (Contract layer), later attachments come and go: a headless session
/// stays `detached` after its last attachment deliberately detaches and after
/// its last attachment is lost, a loss that ends an ordinary session
/// (`every_controlling_leave_hangs_up_then_kills_after_real_grace`). A later
/// attachment still restores it, and its command still runs and reports its exit.
#[test]
fn a_headless_session_survives_the_detach_and_the_loss_of_its_last_attachment() {
    let mut session = headless("exit");
    let mut first = session.attach();
    session.wait_marker(&mut first);
    send(&mut first, ClientMessage::Detach);
    assert_eq!(until_terminal(&mut first).1, ServerMessage::Detached);
    assert_eq!(advertised_state(&session), (Lifecycle::Detached, 0, false));

    let second = session.attach();
    assert_eq!(advertised_state(&session), (Lifecycle::Attached, 1, true));
    // Shutting the socket down, unlike closing this process's descriptor, which
    // a child forked meanwhile by another test thread shares until it execs,
    // makes its EOF readable before the probe connects. The daemon services
    // connections in acceptance order, so the probe's hello is answered after
    // the loss.
    second.shutdown(std::net::Shutdown::Both).unwrap();
    drop(second);
    assert_eq!(advertised_state(&session), (Lifecycle::Detached, 0, false));

    let mut third = attach_at(&session, TerminalSize { cols: 91, rows: 37 });
    let (period, restored) = restoration(&mut third);
    assert_eq!(count(&restored, b"fixture-ready"), 1);
    send(&mut third, ClientMessage::Ready { period });
    send(&mut third, ClientMessage::Input(b"go\n".to_vec()));
    let (output, result) = until_terminal(&mut third);
    assert_eq!(result, ServerMessage::Exit { code: 42 });
    assert_eq!(count(&output, b"final-bytes"), 1);
    session.finish();
}

/// HDL-01 (Contract layer), Kill: a headless session with zero clients ends
/// on Kill, confirmed only after cleanup.
#[test]
fn kill_ends_a_headless_session_with_zero_clients() {
    let session = headless("exit");
    assert_eq!(advertised_state(&session), (Lifecycle::Detached, 0, false));
    kill(session);
}

/// Kill after the command already ended, before anyone attached: the command
/// session still gets its hang-up grace, and the kill confirms the real status.
#[test]
fn kill_after_an_unreported_exit_confirms_that_status() {
    let mut session = Session::start("early");
    thread::sleep(Duration::from_millis(300));
    let mut killer = session.connect();
    killer
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    send(&mut killer, ClientMessage::Kill);
    assert_eq!(receive(&mut killer), ServerMessage::Terminated { code: 43 });
    session.finish();
}

/// Section 6.2 stalled reader: a fast client reads the large prefix; a client
/// that never reads is dropped after five seconds without progress, and the
/// leave policy ends the command.
#[test]
fn a_stalled_reader_is_dropped_after_five_seconds_without_progress() {
    let mut session = Session::start("history");
    wait_file(&session.directory.path().join("history-ready"));
    let mut fast = attach_at(&session, TerminalSize { cols: 91, rows: 37 });
    let (_, restored) = restoration(&mut fast);
    assert!(count(&restored, b"history-row") >= 9000);
    send(&mut fast, ClientMessage::Detach);
    assert_eq!(until_terminal(&mut fast).1, ServerMessage::Detached);
    let _stalled = attach_at(&session, TerminalSize { cols: 91, rows: 37 });
    let started = Instant::now();
    session.finish();
    assert!(started.elapsed() >= Duration::from_secs(5));
    // Five seconds without progress, then the hang-up grace of the leave policy.
    assert!(
        started.elapsed() < Duration::from_secs(13),
        "{:?}",
        started.elapsed()
    );
}

/// Appends one command for [`follow_script`] and waits for the fixture to
/// have written its output.
fn script(session: &Session, command: &str) {
    let issued = issue(session, command);
    wait_contents(&session.directory.path().join("acks"), |acks| {
        String::from_utf8_lossy(acks).lines().count() == issued
    });
}

/// Appends one command for [`follow_script`] without waiting for it, and
/// returns how many commands the script now holds.
fn issue(session: &Session, command: &str) -> usize {
    let path = session.directory.path().join("script");
    let mut commands = fs::read_to_string(&path).unwrap_or_default();
    commands.push_str(command);
    commands.push('\n');
    let staged = path.with_extension("tmp");
    fs::write(&staged, &commands).unwrap();
    fs::rename(staged, path).unwrap();
    commands.lines().count()
}

/// Takes over with fresh 91x37 attachments, leaving each restoration
/// unfinished, until the daemon's snapshot includes `marker`. The fixture has
/// already written it, so only the daemon's PTY read is awaited.
fn restoration_through(session: &Session, clients: &mut Vec<UnixStream>, marker: &str) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let mut client = attach_at(session, TerminalSize { cols: 91, rows: 37 });
        let (_, restored) = restoration(&mut client);
        clients.push(client);
        if count(&restored, marker.as_bytes()) > 0 {
            return restored;
        }
        assert!(Instant::now() < deadline, "{marker} never restored");
    }
}

/// The numbers of the `row-NNNN` rows in a restoration, in order.
fn numbered_rows(restored: &[u8]) -> Vec<u32> {
    restored
        .windows(8)
        .filter(|bytes| bytes.starts_with(b"row-") && bytes[4..].iter().all(u8::is_ascii_digit))
        .map(|bytes| std::str::from_utf8(&bytes[4..]).unwrap().parse().unwrap())
        .collect()
}

fn kill(mut session: Session) {
    let mut killer = session.connect();
    send(&mut killer, ClientMessage::Kill);
    assert!(matches!(
        receive(&mut killer),
        ServerMessage::Terminated { .. }
    ));
    session.finish();
}

/// Section 6.2 retention with an unfinished prefix: scrolling while a
/// restoration is unfinished still honours `max_rows`, so the takeover's
/// snapshot has no history for 0 and only the newest rows for 2.
#[test]
fn retention_with_an_unfinished_prefix_keeps_only_the_newest_rows() {
    for max_rows in [0, 2] {
        let session = Session::start(&format!("retain-{max_rows}"));
        script(&session, "rows 1 100");
        let mut clients = vec![attach_at(&session, TerminalSize { cols: 91, rows: 37 })];
        script(&session, "rows 101 200");
        let restored = restoration_through(&session, &mut clients, "row-0200");
        // 37 rows show rows 165..=200 above the empty cursor row.
        let expected: Vec<u32> = (165 - max_rows..=200).collect();
        assert_eq!(numbered_rows(&restored), expected, "max_rows {max_rows}");
        drop(clients);
        kill(session);
    }
}

/// Section 6.2 identical-row eviction: identical rows still evict and leave a
/// notice; clearing history clears it, so a later restoration has none.
#[test]
fn identical_rows_evict_and_cleared_history_has_no_stale_notice() {
    let session = Session::start("retain-2");
    let mut clients = Vec::new();
    script(&session, "same 100");
    script(&session, "rows 1 1");
    let restored = restoration_through(&session, &mut clients, "row-0001");
    let notice = b"[climon2: older session scrollback was discarded]";
    assert_eq!(count(&restored, notice), 1);
    script(&session, "clear");
    script(&session, "rows 2 2");
    let restored = restoration_through(&session, &mut clients, "row-0002");
    assert_eq!(count(&restored, notice), 0);
    assert_eq!(numbered_rows(&restored), [1, 2]);
    drop(clients);
    kill(session);
}

/// Section 6.2 withheld `Ready`: the restoration deadline is an unexpected
/// departure, so the connection closes and the leave policy ends the command.
#[test]
fn withheld_ready_is_an_unexpected_departure_after_the_restore_deadline() {
    let mut session = Session::start("exit");
    let mut stream = attach_at(&session, TerminalSize { cols: 91, rows: 37 });
    stream
        .set_read_timeout(Some(Duration::from_secs(40)))
        .unwrap();
    restoration(&mut stream);
    let started = Instant::now();
    while let Ok(frame) = read_frame(&mut stream) {
        assert!(matches!(
            ServerMessage::from_frame(frame).unwrap(),
            ServerMessage::Output(_)
        ));
    }
    assert!(started.elapsed() >= Duration::from_secs(29));
    session.finish();
}

/// Section 6.2 sync latch: repeated synchronized-update begins never starve
/// a grant; it is served through the latch. The fixture never stops beginning
/// and never ends an update, so a grant held until begins stop never arrives.
#[test]
fn repeated_sync_begins_do_not_hold_a_grant() {
    let mut session = Session::start("sync");
    wait_file(&session.directory.path().join("sync-ready"));
    let mut stream = attach_at(&session, TerminalSize { cols: 91, rows: 37 });
    let (period, _) = restoration(&mut stream);
    send(&mut stream, ClientMessage::Ready { period });
    drop(stream);
    session.finish();
}

#[test]
fn an_open_synchronized_update_is_not_released_before_its_deadline() {
    let session = Session::start("retain-10");
    let mut stream = session.attach();
    session.wait_marker(&mut stream);
    for _ in 0..3 {
        let started = Instant::now();
        script(&session, "sync");
        let mut output = Vec::new();
        while count(&output, b"\x1b[?2026l") == 0 {
            match receive(&mut stream) {
                ServerMessage::Output(bytes) => output.extend(bytes),
                other => panic!("unexpected synchronized update frame: {other:?}"),
            }
        }
        assert_eq!(count(&output, b"\x1b[?2026h"), 1);
        assert!(
            started.elapsed() >= Duration::from_millis(150),
            "synchronized update ended early: {:?}",
            started.elapsed()
        );
    }
    kill(session);
}

/// A host beyond the session's size limits sees a derived presentation; a
/// later host resize is presented without resizing the command.
#[test]
fn an_oversized_host_is_served_a_derived_presentation() {
    let mut session = Session::start("zero-size");
    let mut stream = attach_at(
        &session,
        TerminalSize {
            cols: 600,
            rows: 37,
        },
    );
    let (period, restored) = restoration(&mut stream);
    send(&mut stream, ClientMessage::Ready { period });
    session.restored.borrow_mut().extend(restored);
    session.wait_marker(&mut stream);
    send(
        &mut stream,
        ClientMessage::Resize(TerminalSize {
            cols: 700,
            rows: 37,
        }),
    );
    send(&mut stream, ClientMessage::Input(b"go\n".to_vec()));
    let (_, result) = until_terminal(&mut stream);
    assert_eq!(result, ServerMessage::Exit { code: 0 });
    assert_eq!(
        fs::read_to_string(session.directory.path().join("resized")).unwrap(),
        "500x37"
    );
    session.finish();
}

fn job_pid(home: &Path, job: &str) -> u32 {
    let ready = home.join(format!("job-{job}-ready"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "job {job} never started");
        thread::sleep(Duration::from_millis(5));
    }
    fs::read_to_string(home.join(format!("job-{job}-pid")))
        .unwrap()
        .parse()
        .unwrap()
}

fn wait_gone(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while process::start_time(pid).unwrap().is_some() {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
    true
}

fn job_control_session() -> Session {
    Session::configured("job-control", |command, _| {
        command
            .env("BASH_SILENCE_DEPRECATION_WARNING", "1")
            .env("JOB_FIXTURE", std::env::current_exe().unwrap());
    })
}

/// A shell line starting the HUP-ignoring `job-{name}` fixture, `&` for the background.
fn job(name: &str, background: &str) -> String {
    format!(
        "DAEMON_FIXTURE_MODE=job-{name} \"$JOB_FIXTURE\" --exact fixture_child --ignored --nocapture {background}\n"
    )
}

/// #19/#27: on natural exit the shell's HUP-ignoring background job, orphaned in the session
/// after the shell (and so the terminal) is gone, is killed once the final status is sent.
#[test]
fn natural_exit_kills_jobs_left_in_the_command_session() {
    let mut session = job_control_session();
    let mut controlling = session.attach();
    send(
        &mut controlling,
        ClientMessage::Input(job("background", "&").into_bytes()),
    );
    let background = job_pid(session.directory.path(), "background");
    // The daemon records job groups every 100 ms while the shell lives; allow several looks.
    thread::sleep(Duration::from_millis(500));
    send(&mut controlling, ClientMessage::Input(b"exit\n".to_vec()));
    while !matches!(receive(&mut controlling), ServerMessage::Exit { code: 0 }) {}
    session.finish();
    let gone = wait_gone(background);
    if !gone {
        unsafe { libc::kill(background as i32, libc::SIGKILL) };
    }
    assert!(gone, "background job survived natural exit");
}

/// #19/#27 (owner decision): leaving hangs up, waits the real grace, then kills every process
/// still in the command's terminal session, across job-control process groups. A process that
/// started its own session is not part of it and survives. The jobs ignore SIGHUP, so only the
/// daemon's end-of-grace kill ends them: they are gone by the time the daemon exits. How long
/// that kill takes after the grace is host load, not policy (each kill round lists every host
/// process once per recorded job group), so here the daemon's exit is bounded only by
/// `FINISH_BOUND`; `every_controlling_leave_hangs_up_then_kills_after_real_grace` bounds the
/// grace itself.
#[test]
fn leave_kills_every_job_of_the_command_session_but_not_new_sessions() {
    let mut session = job_control_session();
    let mut controlling = session.attach();
    let script = job("background", "&") + &job("daemonize", "&") + &job("foreground", "");
    send(&mut controlling, ClientMessage::Input(script.into_bytes()));
    let home = session.directory.path().to_path_buf();
    let foreground = job_pid(&home, "foreground");
    let background = job_pid(&home, "background");
    let outsider = job_pid(&home, "outsider");
    let shell = unsafe { libc::getsid(foreground as i32) };
    for pid in [foreground, background] {
        assert_eq!(
            unsafe { libc::getsid(pid as i32) },
            shell,
            "{pid} left the session"
        );
        assert_eq!(
            unsafe { libc::getpgid(pid as i32) },
            pid as i32,
            "{pid} is not its own job"
        );
    }
    assert_eq!(unsafe { libc::getsid(outsider as i32) }, outsider as i32);
    let started = Instant::now();
    drop(controlling);
    thread::sleep(Duration::from_millis(200));
    assert!(
        process::start_time(foreground).unwrap().is_some(),
        "grace skipped"
    );
    session.finish();
    assert!(started.elapsed() >= Duration::from_secs(5), "grace skipped");
    let survivors = [("foreground", foreground), ("background", background)]
        .into_iter()
        .filter(|(_, pid)| process::start_time(*pid).unwrap().is_some())
        .collect::<Vec<_>>();
    let outsider_alive = process::start_time(outsider).unwrap().is_some();
    for (_, pid) in &survivors {
        unsafe { libc::kill(*pid as i32, libc::SIGKILL) };
    }
    unsafe { libc::kill(outsider as i32, libc::SIGKILL) };
    assert!(survivors.is_empty(), "session jobs survived: {survivors:?}");
    assert!(outsider_alive, "a process in its own session was killed");
}

/// #20: a stalled client bounds output buffering and the command itself stalls; afterwards the
/// exact bytes arrive at PTY speed. A macOS PTY buffers ~1 KiB, so one read per 2 ms tick needs
/// at least 8192 x 2 ms = 16.4 s for 8 MiB on any host; the bound rejects that design. The whole
/// stream is compared, so terminal noise (e.g. libtest's 60-second notice) cannot pass as data.
#[test]
fn saturation_backpressures_pty_without_blocking_probes_or_losing_bytes() {
    let mut session = Session::start("flood");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    send(&mut controlling, ClientMessage::Input(b"go\n".to_vec()));
    let started = Instant::now();
    thread::sleep(Duration::from_millis(300));
    assert!(!session.directory.path().join("flood-finished").exists());
    drop(session.connect());
    let mut output = Vec::new();
    loop {
        match receive(&mut controlling) {
            ServerMessage::Output(bytes) => {
                assert!(bytes.len() <= 65535);
                output.extend(bytes);
            }
            ServerMessage::Exit { code } => {
                assert_eq!(code, 17);
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    let elapsed = started.elapsed();
    let mut expected = b"go\r\n".to_vec();
    expected.resize(expected.len() + 8 * 1024 * 1024, b'x');
    let release = release_at(37, 47);
    let expected_len = expected.len();
    assert!(
        output.len() == expected_len + release.len()
            && output[..expected_len] == expected
            && output[expected_len..] == release,
        "flood output differs ({} bytes, prefix {:?}, suffix {:?})",
        output.len(),
        &output[..output.len().min(160)],
        &output[output.len().saturating_sub(160)..]
    );
    assert!(elapsed < Duration::from_secs(15), "flood took {elapsed:?}");
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

/// Runs the daemon on a readiness pipe owned by this test and never kills it. The CLI's
/// `wait_ready` kills a failed daemon, so only this observes the daemon's own exit.
fn unsupervised(
    mode: &str,
    configure: impl FnOnce(&mut Command, &Path),
    bad_cwd: bool,
) -> (Prepared, Vec<u8>, std::process::ExitStatus) {
    let (prepared, mut command) = Prepared::new(mode, configure, bad_cwd);
    let (readiness, status) = run_unsupervised(&mut command, &prepared, mode);
    (prepared, readiness, status)
}

/// Kills the daemon only if it unexpectedly reports Ready, which never closes the pipe.
fn run_unsupervised(
    command: &mut Command,
    prepared: &Prepared,
    mode: &str,
) -> (Vec<u8>, std::process::ExitStatus) {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    };
    let mut fds = [-1; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let (mut reader, writer) =
        unsafe { (fs::File::from_raw_fd(fds[0]), fs::File::from_raw_fd(fds[1])) };
    for fd in fds {
        assert_ne!(
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
            -1
        );
    }
    let descriptor = writer.as_raw_fd();
    command_tail(command, descriptor as u32, &prepared.home, mode, false);
    unsafe {
        command.pre_exec(move || {
            if libc::fcntl(descriptor, libc::F_SETFD, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    drop(writer);
    let deadline = after(FINISH_BOUND);
    let mut readiness = Vec::new();
    let mut chunk = [0; 512];
    while readable_by(&reader, deadline) {
        let count = reader.read(&mut chunk).unwrap();
        readiness.extend_from_slice(&chunk[..count]);
        if readiness == b"R" {
            child.kill().unwrap();
        }
        if count == 0 || readiness == b"R" {
            break;
        }
    }
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("daemon killed at its deadline after readiness {readiness:?}");
        }
        thread::sleep(Duration::from_millis(5));
    };
    (readiness, status)
}

fn failed_readiness(code: u32, message: &str) -> Vec<u8> {
    let mut bytes = vec![b'F'];
    bytes.extend(code.to_be_bytes());
    bytes.extend((message.len() as u32).to_be_bytes());
    bytes.extend(message.as_bytes());
    bytes
}

fn assert_startup_command_gone(home: &Path) {
    if let Ok(log) = fs::read_to_string(home.join(format!("logs/{ID}/daemon.log"))) {
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
}

fn assert_startup_files_removed(prepared: &Prepared) {
    assert!(!prepared.metadata.exists());
    assert!(!prepared.socket.exists());
    assert!(!prepared.startup_socket.exists());
}

/// #15/#17: unsupervised, a failed startup reports its static diagnostic once, removes its own
/// files and the daemon itself exits with the reported code. Real exec failures are covered by
/// the supervised test above; here the helper never runs, so nothing is killed mid-exit.
#[test]
fn startup_failure_reports_cleans_up_and_exits_with_the_reported_code() {
    type Configure = fn(&mut Command, &Path);
    let nothing: Configure = |_, _| {};
    let shared_sessions: Configure = |_, home| {
        fs::create_dir(home.join("sessions")).unwrap();
        fs::set_permissions(home.join("sessions"), fs::Permissions::from_mode(0o755)).unwrap();
    };
    for (mode, configure, bad_cwd, code, message) in [
        (
            "exit",
            nothing,
            true,
            126,
            "command working directory is not accessible",
        ),
        (
            "exit",
            shared_sessions,
            false,
            125,
            "sessions directory is not private or usable",
        ),
    ] {
        let (prepared, readiness, status) = unsupervised(mode, configure, bad_cwd);
        assert_eq!(readiness, failed_readiness(code, message), "{message}");
        assert_eq!(status.code(), Some(code as i32), "{message}");
        assert_startup_files_removed(&prepared);
        assert_startup_command_gone(&prepared.home);
    }
}

/// #15/#17: a session file that already exists is someone else's. Publication fails, and the
/// daemon removes only the temporary it created, leaving the existing file as it was.
#[test]
fn an_existing_session_file_fails_publication_and_is_left_in_place() {
    for taken in ["pty", "publishing"] {
        let name = format!("{ID}.{taken}");
        let (prepared, readiness, status) = unsupervised(
            "exit",
            |_, home| {
                let sessions = home.join("sessions");
                fs::create_dir(&sessions).unwrap();
                fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();
                fs::write(sessions.join(&name), "theirs").unwrap();
            },
            false,
        );
        assert_eq!(
            readiness,
            failed_readiness(125, "cannot publish session metadata"),
            "{taken}"
        );
        assert_eq!(status.code(), Some(125), "{taken}");
        let sessions = prepared.home.join("sessions");
        let left: Vec<_> = fs::read_dir(&sessions)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(left, std::slice::from_ref(&name), "{taken}");
        assert_eq!(fs::read_to_string(sessions.join(&name)).unwrap(), "theirs");
        assert!(!prepared.socket.exists());
        assert!(!prepared.startup_socket.exists());
        assert_startup_command_gone(&prepared.home);
    }
}

/// The same daemon command with one option's value replaced (Command args are append-only).
fn replace_option(command: &Command, key: &str, value: &str) -> Command {
    let mut arguments: Vec<_> = command.get_args().map(ToOwned::to_owned).collect();
    let index = option_index(&arguments, key);
    arguments[index + 1] = value.into();
    rebuilt(command, arguments)
}

/// The same daemon command without one option and its value.
fn remove_option(command: &Command, key: &str) -> Command {
    let mut arguments: Vec<_> = command.get_args().map(ToOwned::to_owned).collect();
    let index = option_index(&arguments, key);
    arguments.drain(index..=index + 1);
    rebuilt(command, arguments)
}

fn option_index(arguments: &[std::ffi::OsString], key: &str) -> usize {
    arguments
        .iter()
        .position(|argument| argument == key)
        .unwrap()
}

fn rebuilt(command: &Command, arguments: Vec<std::ffi::OsString>) -> Command {
    let mut rebuilt = Command::new(command.get_program());
    rebuilt.args(arguments);
    for (name, value) in command.get_envs() {
        match value {
            Some(value) => rebuilt.env(name, value),
            None => rebuilt.env_remove(name),
        };
    }
    rebuilt
}

/// #17: otherwise valid invocations with a repeated, unknown, zero-PID or relative option, or
/// a missing or unknown `--lifetime`, are invalid daemon invocations: no readiness reply, no
/// session files, exit 125.
#[test]
fn repeated_unknown_zero_pid_and_relative_options_are_invalid_invocations() {
    type Configure = fn(&mut Command, &Path);
    let cases: [Configure; 6] = [
        |command, _| {
            command.args(["--version", "0.1.0"]);
        },
        |command, _| {
            command.args(["--unknown", "value"]);
        },
        |command, _| *command = replace_option(command, "--cli-pid", "0"),
        |command, _| *command = replace_option(command, "--cwd", "."),
        |command, _| *command = replace_option(command, "--lifetime", "persistent"),
        |command, _| *command = remove_option(command, "--lifetime"),
    ];
    for configure in cases {
        let (prepared, readiness, status) = unsupervised("early", configure, false);
        assert_eq!(readiness, b"");
        assert_eq!(status.code(), Some(125));
        assert_startup_files_removed(&prepared);
    }
}

/// #15/#29: the first startup-socket connection decides the launch: a confirmed 126/127 is
/// reported with its static diagnostic, anything outside the marker protocol is a daemon
/// failure, and the daemon exits with that code leaving no command running.
#[test]
fn startup_confirmation_decides_the_reported_launch_failure() {
    for (reply, code, message) in [
        (&[0, 127][..], 127, "command not found"),
        (&[0, 126][..], 126, "command cannot be executed"),
        (&b""[..], 125, "command startup confirmation failed"),
        (&[0, 9][..], 125, "command startup confirmation failed"),
        (&[7][..], 125, "command startup confirmation failed"),
    ] {
        // Holding the per-user allocation lock keeps the daemon from spawning its helper until
        // this connection is first in the startup socket's backlog.
        let allocation = climon2_platform::pty::allocation().unwrap();
        let (prepared, mut command) = Prepared::new("exit", |_, _| {}, false);
        let started = Instant::now();
        let startup_socket = prepared.startup_socket.clone();
        let impostor = thread::spawn(move || {
            // The socket path appears at bind, before it listens.
            let mut stream = loop {
                if let Ok(stream) = UnixStream::connect(&startup_socket) {
                    break stream;
                }
                assert!(started.elapsed() < Duration::from_secs(5));
                thread::sleep(Duration::from_millis(2));
            };
            stream.write_all(reply).unwrap();
            drop(stream);
            drop(allocation);
        });
        let (readiness, status) = run_unsupervised(&mut command, &prepared, "exit");
        impostor.join().unwrap();
        assert_eq!(readiness, failed_readiness(code, message), "{reply:?}");
        assert_eq!(status.code(), Some(code as i32));
        assert_startup_files_removed(&prepared);
        assert_startup_command_gone(&prepared.home);
    }
}

/// #15/#29: the startup deadline bounds a launch that never confirms, whether nothing
/// connects (the helper finds no socket) or the first connection stays silent. Behind a
/// silent connection the helper still starts the command, which ignores SIGHUP and never
/// reads: only the failed daemon's kill ends it.
#[test]
fn an_unconfirmed_launch_fails_at_the_startup_deadline() {
    for silent in [false, true] {
        // Holding the allocation lock keeps the daemon from spawning its helper until the
        // socket is gone, or this silent connection is first in its backlog.
        let allocation = climon2_platform::pty::allocation().unwrap();
        let (prepared, mut command) = Prepared::new("ignore-hup", |_, _| {}, false);
        let startup_socket = prepared.startup_socket.clone();
        let (finished, until_finished) = std::sync::mpsc::channel::<()>();
        let interloper = thread::spawn(move || {
            let started = Instant::now();
            let held = loop {
                // The socket path appears at bind, before it listens.
                let attempt = if silent {
                    UnixStream::connect(&startup_socket).map(Some)
                } else {
                    fs::remove_file(&startup_socket).map(|()| None)
                };
                if let Ok(held) = attempt {
                    break held;
                }
                assert!(started.elapsed() < Duration::from_secs(5));
                thread::sleep(Duration::from_millis(2));
            };
            drop(allocation);
            let _ = until_finished.recv_timeout(Duration::from_secs(30));
            drop(held);
        });
        let (readiness, status) = run_unsupervised(&mut command, &prepared, "ignore-hup");
        finished.send(()).unwrap();
        interloper.join().unwrap();
        assert_eq!(
            readiness,
            failed_readiness(125, "command startup deadline exceeded"),
            "silent: {silent}"
        );
        assert_eq!(status.code(), Some(125));
        let started = fs::read_to_string(prepared.home.join("fixture-pid"));
        assert_eq!(started.is_ok(), silent, "the command started: {started:?}");
        if let Ok(pid) = started {
            let pid: u32 = pid.parse().unwrap();
            let survived = process::start_time(pid).unwrap().is_some();
            if survived {
                unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            }
            assert!(!survived, "the started command survived the failed startup");
        }
        assert_startup_files_removed(&prepared);
        assert_startup_command_gone(&prepared.home);
    }
}

/// #15/#29: the private exec helper confirms a failed exec as 127 (missing command in a usable
/// cwd) or 126 (not executable, or unusable cwd) and exits with that code.
#[test]
fn exec_helper_confirms_launch_failure_codes_and_exits_with_them() {
    // The socket path must fit SUN_LEN regardless of where the checkout lives.
    let directory = tempfile::Builder::new().prefix("exec-").tempdir().unwrap();
    let home = fs::canonicalize(directory.path()).unwrap();
    fs::write(home.join("not-executable"), b"secret not executable\n").unwrap();
    for (cwd, command, code) in [
        (home.clone(), home.join("missing-command"), 127),
        (home.clone(), home.join("not-executable"), 126),
        (home.join("missing-cwd"), home.join("missing-command"), 126),
    ] {
        let path = home.join("exec.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_climon2-daemon"))
            .arg("--exec-command")
            .arg(&path)
            .arg("keep-shell")
            .arg(&cwd)
            .arg(&command)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "helper never connected");
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).unwrap();
        assert_eq!(reply, [0, code]);
        assert_eq!(child.wait().unwrap().code(), Some(i32::from(code)));
        fs::remove_file(&path).unwrap();
    }
}

/// #25: unsupervised, a controlled pre-readiness panic sends no readiness reply, creates no
/// session files and ends the daemon by itself with the sanitized diagnostic logged.
#[cfg(feature = "test-hooks")]
#[test]
fn unsupervised_controlled_panic_ends_the_daemon_without_readiness() {
    let (prepared, readiness, status) = unsupervised(
        "exit",
        |command, _| {
            command.env("CLIMON2_DAEMON_FAULT", "panic");
        },
        false,
    );
    assert!(readiness.is_empty());
    assert_eq!(status.code(), Some(101));
    let log = fs::read_to_string(prepared.home.join(format!("logs/{ID}/daemon.log"))).unwrap();
    assert!(log.contains("climon2: daemon panic"));
    assert!(!log.contains("controlled daemon fault"));
    assert_startup_files_removed(&prepared);
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
        let home = fs::canonicalize(session.directory.path()).unwrap();
        let report = home.join("report");
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
        let cwd = home.join("cwd space --literal é");
        assert_eq!(fields[0], cwd.as_os_str().as_bytes());
        assert_eq!(fields[1], daemon_fixture().as_os_str().as_bytes());
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

/// A HUP-ignoring job of the job-control shell. `daemonize` instead starts an `outsider` in a
/// new session (as setsid/daemonizing tools do) and exits.
fn job_fixture(job: &str) -> ! {
    if job == "daemonize" {
        use std::os::unix::process::CommandExt;
        let mut outsider = Command::new(std::env::current_exe().unwrap());
        outsider
            .args(["--exact", "fixture_child", "--ignored", "--nocapture"])
            .env("DAEMON_FIXTURE_MODE", "job-outsider")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        unsafe {
            outsider.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        outsider.spawn().unwrap();
        std::process::exit(0);
    }
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    fs::write(format!("job-{job}-pid"), std::process::id().to_string()).unwrap();
    fs::write(format!("job-{job}-ready"), b"").unwrap();
    thread::sleep(Duration::from_secs(45));
    std::process::exit(0);
}

static WINCHES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// `COLSxROWS`, followed by any pixel size: clients send none, so none may be reported.
fn terminal_size() -> String {
    let mut size = std::mem::MaybeUninit::<libc::winsize>::uninit();
    assert_eq!(
        unsafe { libc::ioctl(0, libc::TIOCGWINSZ, size.as_mut_ptr()) },
        0
    );
    let size = unsafe { size.assume_init() };
    let pixels = match (size.ws_xpixel, size.ws_ypixel) {
        (0, 0) => String::new(),
        (width, height) => format!(" {width}x{height}px"),
    };
    format!("{}x{}{pixels}", size.ws_col, size.ws_row)
}

/// Records each line it reads with the terminal size at that moment.
fn record_lines() -> ! {
    for line in io::stdin().lines() {
        let line = line.unwrap();
        if line == "end" {
            std::process::exit(0);
        }
        let mut lines = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("lines")
            .unwrap();
        writeln!(lines, "{line} {}", terminal_size()).unwrap();
    }
    std::process::exit(1);
}

/// Prints what the test appends to `script`, one command per line, and
/// acknowledges each in `acks`: `rows A B` prints numbered rows, `same N`
/// prints `N` identical rows and `clear` erases the scrollback. Title commands
/// set the window title: `title TEXT`, `title-empty`, `title-bytes N` (N
/// UTF-8 bytes, `é` repeated) and `titles A B` (`t-A` through `t-B` in one
/// write); `text TEXT` prints TEXT as a line, `colors N` prints `N` rows
/// whose every cell changes colour, `reset` is `ESC c`, and `exit` ends the
/// fixture with status 0, `exit N` with status `N` and `signal N` by signal
/// `N`.
fn follow_script() -> ! {
    print!("fixture-ready\r\n");
    io::stdout().flush().unwrap();
    let mut done = 0;
    loop {
        let script = fs::read_to_string("script").unwrap_or_default();
        let commands: Vec<&str> = script.lines().collect();
        for command in &commands[done..] {
            let words: Vec<&str> = command.split(' ').collect();
            let mut out = io::stdout().lock();
            match words[..] {
                ["rows", first, last] => {
                    for row in first.parse::<u32>().unwrap()..=last.parse().unwrap() {
                        write!(out, "row-{row:04}\r\n").unwrap();
                    }
                }
                ["same", rows] => {
                    for _ in 0..rows.parse().unwrap() {
                        write!(out, "same\r\n").unwrap();
                    }
                }
                ["clear"] => write!(out, "\x1b[3J").unwrap(),
                ["sync"] => write!(out, "\x1b[?2026h").unwrap(),
                ["exit"] => std::process::exit(0),
                ["exit", code] => std::process::exit(code.parse().unwrap()),
                ["signal", number] => {
                    unsafe { libc::raise(number.parse().unwrap()) };
                    unreachable!("signal {number} did not end the fixture");
                }
                ["reset"] => write!(out, "\x1bc").unwrap(),
                ["title-empty"] => write!(out, "\x1b]2;\x07").unwrap(),
                ["title-bytes", bytes] => {
                    let bytes: usize = bytes.parse().unwrap();
                    let title = "é".repeat(bytes / 2) + &"x".repeat(bytes % 2);
                    write!(out, "\x1b]2;{title}\x07").unwrap();
                }
                ["titles", first, last] => {
                    for title in first.parse::<u32>().unwrap()..=last.parse().unwrap() {
                        write!(out, "\x1b]2;t-{title}\x07").unwrap();
                    }
                }
                ["title", ..] => write!(out, "\x1b]2;{}\x07", &command["title ".len()..]).unwrap(),
                ["text", ..] => write!(out, "{}\r\n", &command["text ".len()..]).unwrap(),
                ["colors", rows] => {
                    for _ in 0..rows.parse().unwrap() {
                        for cell in 0..90 {
                            write!(out, "\x1b[{}mc", 31 + cell % 7).unwrap();
                        }
                        write!(out, "\x1b[0m\r\n").unwrap();
                    }
                }
                _ => panic!("unknown script command {command:?}"),
            }
            out.flush().unwrap();
            let mut acks = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("acks")
                .unwrap();
            writeln!(acks, "{command}").unwrap();
        }
        done = commands.len();
        thread::sleep(Duration::from_millis(2));
    }
}

/// A raw bracketed-paste application recording every byte it reads until
/// `END`; with a `query`, it asks the terminal once its first input arrives.
fn record_input(file: &str, query: Option<&str>) -> ! {
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    assert_eq!(unsafe { libc::tcgetattr(0, termios.as_mut_ptr()) }, 0);
    let mut termios = unsafe { termios.assume_init() };
    unsafe {
        libc::cfmakeraw(&mut termios);
    }
    assert_eq!(unsafe { libc::tcsetattr(0, libc::TCSANOW, &termios) }, 0);
    print!("\x1b[?2004hfixture-ready");
    io::stdout().flush().unwrap();
    let mut pasted = Vec::new();
    let mut buffer = [0; 4096];
    while !pasted.ends_with(b"END") {
        let count = io::stdin().read(&mut buffer).unwrap();
        assert_ne!(count, 0);
        if pasted.is_empty()
            && let Some(query) = query
        {
            print!("{query}");
            io::stdout().flush().unwrap();
        }
        pasted.extend_from_slice(&buffer[..count]);
        fs::write(file, &pasted).unwrap();
    }
    std::process::exit(0);
}

/// Closes every descriptor of its terminal, so the daemon reads the end of
/// its output, and keeps running: `terminal-closed` says it closed them.
/// It exits with status 3 once `finish` appears, or after 45 seconds.
fn close_terminal() -> ! {
    for descriptor in 0..3 {
        unsafe { libc::close(descriptor) };
    }
    fs::write("terminal-closed", b"").unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    while !Path::new("finish").exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    std::process::exit(3);
}

/// A raw application recording every byte it reads in `focused` until
/// `END`. Input ending in `ON` or `OFF` turns focus reporting (mode 1004) on
/// or off, acknowledged by printing `focus-on` or `focus-off`.
fn record_focus() -> ! {
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    assert_eq!(unsafe { libc::tcgetattr(0, termios.as_mut_ptr()) }, 0);
    let mut termios = unsafe { termios.assume_init() };
    unsafe {
        libc::cfmakeraw(&mut termios);
    }
    assert_eq!(unsafe { libc::tcsetattr(0, libc::TCSANOW, &termios) }, 0);
    print!("fixture-ready");
    io::stdout().flush().unwrap();
    let mut received = Vec::new();
    let mut buffer = [0; 4096];
    while !received.ends_with(b"END") {
        let count = io::stdin().read(&mut buffer).unwrap();
        assert_ne!(count, 0);
        received.extend_from_slice(&buffer[..count]);
        fs::write("focused", &received).unwrap();
        if received.ends_with(b"ON") {
            print!("\x1b[?1004hfocus-on");
        } else if received.ends_with(b"OFF") {
            print!("\x1b[?1004lfocus-off");
        }
        io::stdout().flush().unwrap();
    }
    std::process::exit(0);
}

#[test]
#[ignore = "owned generic terminal fixture subprocess"]
fn fixture_child() {
    let mode = std::env::var("DAEMON_FIXTURE_MODE").unwrap();
    if let Ok(life) = std::env::var("DAEMON_FIXTURE_LIFE") {
        // The default SIGALRM action ends the fixture after `life` seconds.
        unsafe {
            libc::alarm(life.parse().unwrap());
        }
    }
    if let Some(job) = mode.strip_prefix("job-") {
        job_fixture(job);
    }
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
    let size = terminal_size();
    if mode == "zero-size" {
        // Publish atomically: the test polls for the name, then reads the contents.
        fs::write("initial-size.tmp", size).unwrap();
        fs::rename("initial-size.tmp", "initial-size").unwrap();
    } else {
        assert_eq!(size, "91x37");
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
    if mode == "paste" {
        print!("\x1b[?2004hfixture-ready");
        io::stdout().flush().unwrap();
        thread::sleep(Duration::from_secs(45));
    }
    if mode == "sync" {
        // A begin every 20 ms for the fixture's 45-second life, and never an end.
        for frame in 0..2250 {
            print!("\x1b[?2026hframe-{frame}\r");
            io::stdout().flush().unwrap();
            if frame == 0 {
                fs::write("sync-ready", b"").unwrap();
            }
            thread::sleep(Duration::from_millis(20));
        }
        thread::sleep(Duration::from_secs(45));
    }
    if mode == "partial" {
        print!("\x1b[");
        io::stdout().flush().unwrap();
        fs::write("partial-ready", b"").unwrap();
        thread::sleep(Duration::from_secs(45));
    }
    if mode == "history" {
        let mut out = io::stdout().lock();
        for row in 0..10_000 {
            write!(out, "history-row {row:05} {}\r\n", "h".repeat(60)).unwrap();
        }
        out.flush().unwrap();
        fs::write("history-ready", b"").unwrap();
        thread::sleep(Duration::from_secs(45));
    }
    if mode == "winch" {
        extern "C" fn counted(_: libc::c_int) {
            WINCHES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        unsafe {
            libc::signal(
                libc::SIGWINCH,
                counted as extern "C" fn(libc::c_int) as libc::sighandler_t,
            );
        }
        print!("fixture-ready");
        io::stdout().flush().unwrap();
        let mut line = String::new();
        io::stdin().read_line(&mut line).unwrap();
        let winches = WINCHES.load(std::sync::atomic::Ordering::SeqCst);
        fs::write("resized", format!("{winches} {}", terminal_size())).unwrap();
        std::process::exit(0);
    }
    if mode == "tui-restore" {
        let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
        assert_eq!(unsafe { libc::tcgetattr(0, termios.as_mut_ptr()) }, 0);
        let mut termios = unsafe { termios.assume_init() };
        unsafe {
            libc::cfmakeraw(&mut termios);
        }
        assert_eq!(unsafe { libc::tcsetattr(0, libc::TCSANOW, &termios) }, 0);
        print!(
            "\x1b[?1049h\x1b[?1000h\x1b[?2004h\x1b[?1h\
             \x1b[2J\x1b[4;5HRESTORED-DISPLAY"
        );
        io::stdout().flush().unwrap();
        fs::write("draw-count", b"1").unwrap();
        fs::write("tui-ready", b"").unwrap();
        let mut byte = [0];
        io::stdin().read_exact(&mut byte).unwrap();
        fs::write("input-accepted", byte).unwrap();
        print!("INPUT-ACCEPTED");
        std::process::exit(42);
    }
    if mode == "paste-record" {
        record_input("pasted", None);
    }
    if mode == "reply-record" {
        record_input("recorded", Some("\x1b[c"));
    }
    if mode == "focus-record" {
        record_focus();
    }
    if mode.starts_with("retain-") || mode == "script" {
        follow_script();
    }
    print!("fixture-ready");
    io::stdout().flush().unwrap();
    if mode == "lines" {
        record_lines();
    }
    if mode == "close-terminal" {
        close_terminal();
    }
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
        fs::write("resized", terminal_size()).unwrap();
        std::process::exit(0);
    }
    if mode == "early" || mode == "early-after-restore" {
        if mode == "early-after-restore" {
            wait_file(Path::new("finish-early"));
        }
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
