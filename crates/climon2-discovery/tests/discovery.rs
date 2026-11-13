//! Shared discovery through its public boundary: real daemons for the DSC-06
//! Contract case, otherwise synthetic metadata and raw-socket hello peers.
use climon2_core::{
    SessionId,
    hello::{Lifecycle, SessionState},
    layout::HomeLayout,
    metadata::{LinuxProcessIdentity, SessionMetadata},
    protocol::{
        ClientMessage, HANGUP_GRACE, HELLO_TIMEOUT, ServerMessage, TerminalSize, read_frame,
        write_frame,
    },
};
use climon2_discovery::{
    Batch, Cleanup, Continuation, Incomplete, InvalidReason, MetadataLimits, Options, Outcome,
    Pass, Progress, Scan, StaleMetadata, Uncertainty, discover,
};
use climon2_platform::{
    daemon::{self, Readiness},
    process, runtime,
};
use climon2_test_term::{Isolation, bin_path, exit_within, hello_peer};
use std::{
    fs,
    io::{self, Read},
    num::NonZeroUsize,
    os::unix::{
        fs::PermissionsExt,
        net::{UnixDatagram, UnixStream},
    },
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

const RETAIN: Options = Options {
    limits: MetadataLimits::UNBOUNDED,
    stale: StaleMetadata::Retain,
};
const REMOVE: Options = Options {
    stale: StaleMetadata::Remove,
    ..RETAIN
};
const OUTER: &str = "0d5c0000-0000-4000-8000-000000000001";
const NESTED: &str = "0d5c0000-0000-4000-8000-000000000002";
const CHILD: &str = "CLIMON2_DISCOVERY_SILENT_PASSES";

/// A canonical climon2 home of its own.
fn home() -> (tempfile::TempDir, HomeLayout) {
    let directory = tempfile::tempdir().unwrap();
    let root = runtime::canonical_home(&directory.path().join("climon2")).unwrap();
    (directory, layout(&root))
}

/// The layout of a canonical climon2 home, which is what a pass reads.
fn layout(root: &Path) -> HomeLayout {
    HomeLayout::resolve(Some(root.as_os_str()), Path::new("/")).unwrap()
}

/// Valid metadata whose socket is missing and whose daemon identity is provably gone: this
/// process's PID, with a start time (macOS) and Linux start ticks it never had.
fn record(home: &HomeLayout, suffix: u8, created: &str) -> SessionMetadata {
    let id: SessionId = format!("12345678-1234-4234-8234-123456789a{suffix:02x}")
        .parse()
        .unwrap();
    let socket = runtime::RuntimeDirectory::prepare(home.root())
        .unwrap()
        .socket_path(&id)
        .unwrap();
    let mut metadata: SessionMetadata = serde_json::from_value(serde_json::json!({
        "format_version":1,"session_id":id.as_str(),"socket_path":socket,
        "daemon_pid":std::process::id(),"daemon_start_time":0,
        "command":"bash","args":[],"cwd":home.root(),"depth":1,"created_at":created,
        "climon2_version":"0.1.0"
    }))
    .unwrap();
    metadata.daemon_linux_identity = own_identity().map(|identity| LinuxProcessIdentity {
        start_ticks: 0,
        ..identity
    });
    metadata
}

/// This process's Linux identity, as a daemon records its own; macOS records none.
fn own_identity() -> Option<LinuxProcessIdentity> {
    process::linux_identity(std::process::id()).unwrap()
}

fn path(home: &HomeLayout, metadata: &SessionMetadata) -> PathBuf {
    home.root()
        .join("sessions")
        .join(format!("{}.pty", metadata.session_id))
}

fn save(home: &HomeLayout, metadata: &SessionMetadata) -> PathBuf {
    fs::create_dir_all(home.root().join("sessions")).unwrap();
    let path = path(home, metadata);
    fs::write(&path, metadata.to_json().unwrap()).unwrap();
    path
}

fn hello_body(id: &str) -> String {
    format!("{{\"session_id\":\"{id}\",\"climon2_version\":\"0.1.0\",\"protocol\":1}}")
}

fn ids(sessions: &[Outcome]) -> Vec<&str> {
    sessions
        .iter()
        .map(|outcome| outcome.metadata().session_id.as_str())
        .collect()
}

/// The CLI's own resolution of `CLIMON2_HOME`, created and canonicalized once.
fn resolved(isolation: &Isolation) -> HomeLayout {
    let climon2_home = isolation.climon2_home();
    let resolved = HomeLayout::resolve(Some(climon2_home.as_os_str()), isolation.home()).unwrap();
    layout(&runtime::canonical_home(resolved.root()).unwrap())
}

fn send(stream: &mut UnixStream, message: ClientMessage) {
    write_frame(stream, &message.to_frame().unwrap()).unwrap();
}

fn receive(stream: &mut UnixStream) -> ServerMessage {
    ServerMessage::from_frame(read_frame(stream).unwrap()).unwrap()
}

fn hello(socket: &Path) -> UnixStream {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(HANGUP_GRACE + Duration::from_secs(5)))
        .unwrap();
    send(&mut stream, ClientMessage::Hello { protocol: 1 });
    assert!(matches!(receive(&mut stream), ServerMessage::Hello(_)));
    stream
}

/// Attaches and finishes restoration, so this connection controls the session.
fn attach(socket: &Path) -> UnixStream {
    let mut stream = hello(socket);
    send(
        &mut stream,
        ClientMessage::Attach(TerminalSize { cols: 80, rows: 24 }),
    );
    assert_eq!(receive(&mut stream), ServerMessage::Attached);
    loop {
        match receive(&mut stream) {
            ServerMessage::Granted { .. } | ServerMessage::Output(_) => {}
            ServerMessage::RestoreEnd { period } => {
                send(&mut stream, ClientMessage::Ready { period });
                return stream;
            }
            other => panic!("unexpected restoration frame: {other:?}"),
        }
    }
}

/// An explicit detach leaves the session running without a client.
fn detach(mut stream: UnixStream) {
    send(&mut stream, ClientMessage::Detach);
    while receive(&mut stream) != ServerMessage::Detached {}
}

/// A real session daemon running `/bin/cat`, so it lives until killed.
struct Daemon {
    pid: u32,
    socket: PathBuf,
}

impl Daemon {
    fn start(isolation: &Isolation, home: &HomeLayout, id: &str, depth: u32) -> Self {
        let id: SessionId = id.parse().unwrap();
        let socket = runtime::RuntimeDirectory::prepare(home.root())
            .unwrap()
            .socket_path(&id)
            .unwrap();
        let mut command = Command::new(bin_path("climon2-daemon"));
        command
            .env_clear()
            .env("HOME", isolation.home())
            .env("TERM", "xterm-256color")
            .env("PATH", "/usr/bin:/bin")
            .env("CLIMON2_TEST_ID", isolation.id())
            .env("CLIMON2_SESSION_DEPTH", depth.to_string())
            .env("CLIMON2_SESSION_ID", id.as_str())
            .args(["--session-id", id.as_str(), "--home"])
            .arg(home.root())
            .arg("--socket")
            .arg(&socket)
            .args(["--version", env!("CARGO_PKG_VERSION"), "--cli-pid"])
            .arg(std::process::id().to_string())
            .args(["--lifetime", "ordinary"])
            .args(["--rows", "24", "--cols", "80", "--cwd"])
            .arg(isolation.home());
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let pending = daemon::spawn(command, |command, fd| {
            command
                .args(["--scrollback-max-rows", "10000", "--readiness-fd"])
                .arg(fd.to_string())
                .args(["--", "/bin/cat"]);
        })
        .unwrap();
        let pid = pending.id();
        assert_eq!(pending.wait_ready(&[]).unwrap(), Readiness::Ready);
        Self { pid, socket }
    }
}

/// Kills each session through its own protocol, so their hang-up graces overlap, then
/// reaps the daemons.
fn kill(daemons: [Daemon; 2]) {
    let killers = daemons.each_ref().map(|daemon| {
        let mut killer = hello(&daemon.socket);
        send(&mut killer, ClientMessage::Kill);
        killer
    });
    for (mut daemon, mut killer) in daemons.into_iter().zip(killers) {
        assert!(matches!(
            receive(&mut killer),
            ServerMessage::Terminated { .. }
        ));
        // The daemon is reaped even when it fails the assertion, so `Drop` must not signal or
        // wait for it again.
        let pid = std::mem::take(&mut daemon.pid);
        assert!(exit_within(pid, EXIT_BOUND).is_some());
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if self.pid != 0 {
            unsafe { libc::kill(self.pid as i32, libc::SIGTERM) };
            exit_within(self.pid, EXIT_BOUND);
        }
    }
}

/// Generous for any orderly exit under load, a session's five-second hang-up grace included.
const EXIT_BOUND: Duration = Duration::from_secs(30);

/// DSC-06 Contract case: a session controlled elsewhere and a nested session, each in its
/// own independently resolved home, are live in that home's pass only, with their hello
/// capabilities, state and depth. Probing never takes control. Stale metadata beside a
/// live session is reported and kept, and removed only when the caller opts in.
#[test]
fn independent_homes_each_discover_their_own_nested_and_controlled_sessions() {
    let outer_isolation = Isolation::new();
    let nested_isolation = Isolation::new();
    let outer_home = resolved(&outer_isolation);
    let nested_home = resolved(&nested_isolation);
    let outer = Daemon::start(&outer_isolation, &outer_home, OUTER, 1);
    let nested = Daemon::start(&nested_isolation, &nested_home, NESTED, 2);
    let controller = attach(&outer.socket);
    detach(attach(&nested.socket));
    let stale = record(&outer_home, 1, "2000-01-01T00:00:00.000Z");
    let stale_path = save(&outer_home, &stale);
    let controlled = SessionState::new(Lifecycle::Attached, 1, true);

    let pass = discover(&outer_home, RETAIN).unwrap();
    assert!(pass.invalid.is_empty(), "{pass:?}");
    let [
        Outcome::Live(live),
        Outcome::Stale(found, Cleanup::Retained),
    ] = pass.sessions.as_slice()
    else {
        panic!("{pass:?}");
    };
    assert_eq!(live.metadata.session_id.as_str(), OUTER);
    assert_eq!(live.metadata.depth.get(), 1);
    assert_eq!(live.state(), controlled);
    let capabilities = live.extensions.capabilities.valid().unwrap();
    assert!(capabilities.supports_attach() && capabilities.supports_kill());
    assert_eq!(found, &stale);
    assert!(stale_path.exists());

    let pass = discover(&nested_home, RETAIN).unwrap();
    assert!(pass.invalid.is_empty(), "{pass:?}");
    let [Outcome::Live(live)] = pass.sessions.as_slice() else {
        panic!("{pass:?}");
    };
    assert_eq!(live.metadata.session_id.as_str(), NESTED);
    assert_eq!(live.metadata.depth.get(), 2);
    assert_eq!(
        live.state(),
        SessionState::new(Lifecycle::Detached, 0, false)
    );

    let pass = discover(&outer_home, REMOVE).unwrap();
    let [Outcome::Live(live), Outcome::Stale(found, Cleanup::Removed)] = pass.sessions.as_slice()
    else {
        panic!("{pass:?}");
    };
    assert_eq!(live.state(), controlled);
    assert_eq!(found, &stale);
    assert!(!stale_path.exists());
    assert_eq!(
        ids(&discover(&outer_home, REMOVE).unwrap().sessions),
        [OUTER]
    );

    detach(controller);
    kill([outer, nested]);
    outer_isolation.assert_clean();
    nested_isolation.assert_clean();
}

/// DSC-01 Contract case (discovery order) and #77: sessions are ordered newest created
/// first, with ascending full session ID for ties, whatever their outcome. A home without a
/// sessions directory is an empty pass.
#[test]
fn sessions_are_newest_created_first_with_ascending_session_id_ties() {
    let (_directory, home) = home();
    let empty = discover(&home, RETAIN).unwrap();
    assert!(empty.sessions.is_empty() && empty.invalid.is_empty());
    let tied = "2026-10-28T07:21:02.930Z";
    for (suffix, created) in [
        (0xa3, tied),
        (0x01, "2026-10-28T07:21:02.929Z"),
        (0xa5, tied),
        (0xa1, tied),
        (0xff, "2026-10-28T07:21:02.931Z"),
        (0xa4, tied),
        (0xa2, tied),
    ] {
        save(&home, &record(&home, suffix, created));
    }
    let id = |suffix: u8| format!("12345678-1234-4234-8234-123456789a{suffix:02x}");
    let expected: Vec<_> = [0xff, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x01].map(id).to_vec();
    assert_eq!(ids(&discover(&home, RETAIN).unwrap().sessions), expected);
}

/// DSC-04 Contract case (inaccessible discovery): a sessions directory that cannot be listed
/// fails the pass with the listing error, rather than reporting an empty home, whether or not
/// stale cleanup is requested, and nothing is deleted.
#[test]
fn a_sessions_directory_that_cannot_be_listed_fails_the_pass() {
    let (_directory, home) = home();
    let sessions = home.root().join("sessions");
    fs::write(&sessions, b"not a directory").unwrap();
    for options in [RETAIN, REMOVE] {
        let error = discover(&home, options).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ENOTDIR), "{error}");
    }
    assert_eq!(fs::read(&sessions).unwrap(), b"not a directory");
}

/// The setup and assertions of `every_outcome_is_structured_and_no_pass_prints`, run in a
/// child process so that its standard output and error hold nothing else.
fn every_outcome_in_three_passes() {
    let (_directory, home) = home();
    let live = record(&home, 0x19, "2026-10-28T07:21:09.000Z");
    let mismatched = record(&home, 0x18, "2026-10-28T07:21:08.000Z");
    let unanswered = record(&home, 0x17, "2026-10-28T07:21:07.000Z");
    let unreachable = record(&home, 0x16, "2026-10-28T07:21:06.000Z");
    let mut alive = record(&home, 0x15, "2026-10-28T07:21:05.000Z");
    alive.daemon_start_time = process::start_time(std::process::id()).unwrap().unwrap();
    alive.daemon_linux_identity = own_identity();
    let mut unknown = record(&home, 0x14, "2026-10-28T07:21:04.000Z");
    unknown.daemon_pid = 0;
    let stale = record(&home, 0x13, "2026-10-28T07:21:03.000Z");
    let sessions = [
        &live,
        &mismatched,
        &unanswered,
        &unreachable,
        &alive,
        &unknown,
        &stale,
    ];
    let files: Vec<_> = sessions
        .iter()
        .map(|metadata| save(&home, metadata))
        .collect();
    let peers = [
        hello_peer(
            live.socket_path.as_str(),
            &hello_body(live.session_id.as_str()),
            3,
        ),
        hello_peer(mismatched.socket_path.as_str(), &hello_body(OUTER), 3),
        hello_peer(unanswered.socket_path.as_str(), "{", 3),
    ];
    // A stream connection to a datagram socket fails, with EPROTOTYPE, on both platforms.
    let _datagram = UnixDatagram::bind(unreachable.socket_path.as_str()).unwrap();
    let sessions_directory = home.root().join("sessions");
    let malformed = sessions_directory.join("not-a-session-id.pty");
    fs::write(&malformed, b"{}").unwrap();
    let corrupt = path(&home, &record(&home, 0x12, "2026-10-28T07:21:02.000Z"));
    fs::write(&corrupt, b"not-json").unwrap();
    let unreadable = path(&home, &record(&home, 0x11, "2026-10-28T07:21:01.000Z"));
    fs::create_dir(&unreadable).unwrap();
    fs::write(sessions_directory.join("notes.txt"), b"not metadata").unwrap();
    let permissions =
        |mode| fs::set_permissions(&sessions_directory, fs::Permissions::from_mode(mode)).unwrap();

    println!("<passes>");
    eprintln!("<passes>");
    let retained = discover(&home, RETAIN).unwrap();
    permissions(0o500);
    let failed = discover(&home, REMOVE).unwrap();
    permissions(0o700);
    let removed = discover(&home, REMOVE).unwrap();
    println!("</passes>");
    eprintln!("</passes>");

    for pass in [&retained, &failed, &removed] {
        assert_eq!(
            ids(&pass.sessions),
            sessions.map(|metadata| metadata.session_id.as_str())
        );
        assert_uncertain_and_invalid(pass, &malformed, &corrupt, &unreadable);
    }
    assert!(matches!(
        &retained.sessions[6],
        Outcome::Stale(found, Cleanup::Retained) if found == &stale
    ));
    let Outcome::Stale(_, Cleanup::Failed(error)) = &failed.sessions[6] else {
        panic!("stale cleanup in a read-only directory must fail");
    };
    assert_eq!(error.raw_os_error(), Some(libc::EACCES));
    assert!(matches!(
        &removed.sessions[6],
        Outcome::Stale(_, Cleanup::Removed)
    ));
    let Outcome::Live(found) = &removed.sessions[0] else {
        panic!("{removed:?}");
    };
    assert_eq!(found.metadata, live);
    assert_eq!(found.state(), None);
    for (index, file) in files.iter().enumerate() {
        assert_eq!(file.exists(), index != 6, "{}", file.display());
    }
    for file in [&malformed, &corrupt, &unreadable] {
        assert!(file.exists());
    }
    for peer in peers {
        peer.join().unwrap();
    }
    for metadata in [&live, &mismatched, &unanswered, &unreachable] {
        fs::remove_file(metadata.socket_path.as_str()).unwrap();
    }
}

/// Every pass reports the same uncertain sessions and invalid metadata, never cleaning them.
fn assert_uncertain_and_invalid(pass: &Pass, malformed: &Path, corrupt: &Path, unreadable: &Path) {
    assert!(matches!(pass.sessions[0], Outcome::Live(_)));
    assert!(matches!(
        pass.sessions[1],
        Outcome::Uncertain(_, Uncertainty::Mismatched)
    ));
    assert!(matches!(
        pass.sessions[2],
        Outcome::Uncertain(_, Uncertainty::Unanswered(_))
    ));
    let Outcome::Uncertain(_, Uncertainty::Unreachable(error)) = &pass.sessions[3] else {
        panic!("{pass:?}");
    };
    assert_eq!(error.raw_os_error(), Some(libc::EPROTOTYPE));
    assert!(matches!(
        pass.sessions[4],
        Outcome::Uncertain(_, Uncertainty::DaemonAlive)
    ));
    assert!(matches!(
        pass.sessions[5],
        Outcome::Uncertain(_, Uncertainty::IdentityUnknown(_))
    ));
    let mut invalid: Vec<_> = pass.invalid.iter().collect();
    invalid.sort_by(|first, second| first.path.cmp(&second.path));
    let [first, second, third] = invalid.as_slice() else {
        panic!("{pass:?}");
    };
    assert_eq!(
        [&first.path, &second.path, &third.path],
        [unreadable, corrupt, malformed]
    );
    assert!(matches!(third.reason, InvalidReason::Malformed(_)));
    assert!(matches!(second.reason, InvalidReason::Malformed(_)));
    let InvalidReason::Unreadable(error) = &first.reason else {
        panic!("{pass:?}");
    };
    assert_eq!(error.raw_os_error(), Some(libc::EISDIR));
}

/// DSC-06 Contract case (structured pass outcomes, no presentation) and DSC-04 Contract
/// cases (mismatched hello IDs, failed probes, uncertain disappearance, malformed metadata):
/// every outcome is returned as structured data with its diagnostic, ordered newest first.
/// Uncertainty and invalid metadata are never cleaned, a stale session's cleanup is opt-in
/// and reports its failure, and no pass prints anything.
#[test]
fn every_outcome_is_structured_and_no_pass_prints() {
    if std::env::var_os(CHILD).is_some() {
        return every_outcome_in_three_passes();
    }
    let directory = tempfile::tempdir().unwrap();
    let [stdout, stderr] = ["stdout", "stderr"].map(|name| directory.path().join(name));
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "every_outcome_is_structured_and_no_pass_prints",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .stdout(fs::File::create(&stdout).unwrap())
        .stderr(fs::File::create(&stderr).unwrap())
        .spawn()
        .unwrap();
    let status = exit_within(child, EXIT_BOUND);
    let [stdout, stderr] = [stdout, stderr].map(|path| fs::read_to_string(path).unwrap());
    assert!(
        status.is_some_and(|status| status.success()),
        "{stdout}\n{stderr}"
    );
    assert!(stdout.contains("<passes>\n</passes>\n"), "{stdout}");
    assert!(stderr.contains("<passes>\n</passes>\n"), "{stderr}");
}

const CREATED: &str = "2026-10-28T07:21:02.930Z";

fn write(home: &HomeLayout, metadata: &SessionMetadata, contents: &[u8]) -> PathBuf {
    let path = path(home, metadata);
    fs::write(&path, contents).unwrap();
    path
}

/// Valid metadata padded with trailing whitespace to exactly `bytes` bytes.
fn sized(home: &HomeLayout, suffix: u8, bytes: usize) -> PathBuf {
    let metadata = record(home, suffix, CREATED);
    let mut json = metadata.to_json().unwrap();
    json.resize(bytes, b' ');
    write(home, &metadata, &json)
}

/// Valid metadata whose first, unknown member nests `nesting` objects and arrays in all,
/// followed by strings that hide brackets, quotes and backslashes.
fn nested(home: &HomeLayout, suffix: u8, nesting: usize) -> PathBuf {
    let metadata = record(home, suffix, CREATED);
    let json = String::from_utf8(metadata.to_json().unwrap()).unwrap();
    let strings = [
        "[[[[[[[[{{{{{{{{",
        "\"[[[[[[[[{{{{{{{{",
        "\\",
        "x\\",
        "]]]]}}}}",
    ];
    let text = format!(
        "{{\"future\":{}{},\"tricky\":{},{}",
        "[".repeat(nesting - 1),
        "]".repeat(nesting - 1),
        serde_json::to_string(&strings).unwrap(),
        &json[1..]
    );
    write(home, &metadata, text.as_bytes())
}

fn reasons(pass: &Pass) -> Vec<(&Path, &'static str)> {
    let mut reasons: Vec<_> = pass
        .invalid
        .iter()
        .map(|entry| {
            let reason = match entry.reason {
                InvalidReason::TooLarge => "too large",
                InvalidReason::TooDeep => "too deep",
                _ => "other",
            };
            (entry.path.as_path(), reason)
        })
        .collect();
    reasons.sort();
    reasons
}

/// #77 and DSC-04 Contract case (oversized metadata): a caller's limits accept a file of
/// exactly the size limit and JSON nested exactly to the depth limit, while one byte or
/// one level more is invalid metadata, never deleted. Brackets inside strings never count.
/// Without limits a pass reads whole files nested to any depth, as the CLI always has.
#[test]
fn metadata_limits_accept_their_bound_and_reject_one_past_it() {
    let (_directory, home) = home();
    fs::create_dir_all(home.root().join("sessions")).unwrap();
    let at_size = sized(&home, 0x41, 4096);
    let past_size = sized(&home, 0x42, 4097);
    let at_depth = nested(&home, 0x43, 8);
    let past_depth = nested(&home, 0x44, 9);
    let deepest = nested(&home, 0x45, 600);
    let limits = MetadataLimits {
        file_bytes: Some(4096),
        json_depth: Some(8),
    };

    let pass = discover(&home, Options { limits, ..RETAIN }).unwrap();
    let id = |suffix: u8| format!("12345678-1234-4234-8234-123456789a{suffix:02x}");
    assert_eq!(ids(&pass.sessions), [id(0x41), id(0x43)]);
    assert!(
        pass.sessions
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Stale(_, Cleanup::Retained)))
    );
    assert_eq!(
        reasons(&pass),
        [
            (past_size.as_path(), "too large"),
            (past_depth.as_path(), "too deep"),
            (deepest.as_path(), "too deep"),
        ]
    );

    let pass = discover(&home, RETAIN).unwrap();
    assert!(pass.invalid.is_empty(), "{pass:?}");
    assert_eq!(ids(&pass.sessions), [0x41, 0x42, 0x43, 0x44, 0x45].map(id));
    for file in [at_size, past_size, at_depth, past_depth, deepest] {
        assert!(file.exists());
    }
}

/// DSC-04 Contract case (slow and dead probes): a daemon that accepts the probe and never
/// answers leaves its session uncertain once the frozen two-second hello deadline passes,
/// not before, and the probe then hangs up. Its metadata stays.
#[test]
fn a_silent_daemon_is_uncertain_after_the_two_second_hello_deadline() {
    let (_directory, home) = home();
    let silent = record(&home, 0x51, CREATED);
    let file = save(&home, &silent);
    let listener = runtime::bind(Path::new(silent.socket_path.as_str())).unwrap();
    let peer = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        read_frame(&mut socket).unwrap();
        // Only a probe that keeps its own deadline hangs up before this read gives up.
        socket.set_read_timeout(Some(HELLO_TIMEOUT * 5)).unwrap();
        socket.read_to_end(&mut Vec::new())
    });

    let started = Instant::now();
    let pass = discover(&home, REMOVE).unwrap();
    let elapsed = started.elapsed();
    let [Outcome::Uncertain(_, Uncertainty::Unanswered(error))] = pass.sessions.as_slice() else {
        panic!("{pass:?}");
    };
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(elapsed >= HELLO_TIMEOUT, "{elapsed:?}");
    assert_eq!(peer.join().unwrap().unwrap(), 0);
    assert!(file.exists());
    fs::remove_file(silent.socket_path.as_str()).unwrap();
}

/// DSC-04 Contract case (uncertain disappearance), Linux: a session whose socket is missing
/// is stale only when its recorded boot ID and start ticks prove the daemon gone, never by
/// its epoch start time. A live identity is a live daemon whatever start time it recorded,
/// and metadata without an identity is uncertain; neither is cleaned up, even on request.
#[cfg(target_os = "linux")]
#[test]
fn linux_daemon_identity_alone_proves_a_session_stale() {
    let (_directory, home) = home();
    let mut alive = record(&home, 0x61, CREATED);
    alive.daemon_linux_identity = own_identity();
    let mut unknown = record(&home, 0x62, CREATED);
    unknown.daemon_linux_identity = None;
    let mut dead = record(&home, 0x63, CREATED);
    dead.daemon_start_time = process::start_time(std::process::id()).unwrap().unwrap();
    let files = [&alive, &unknown, &dead].map(|metadata| save(&home, metadata));

    let pass = discover(&home, REMOVE).unwrap();
    let [
        Outcome::Uncertain(found_alive, Uncertainty::DaemonAlive),
        Outcome::Uncertain(found_unknown, Uncertainty::IdentityUnknown(_)),
        Outcome::Stale(found_dead, Cleanup::Removed),
    ] = pass.sessions.as_slice()
    else {
        panic!("{pass:?}");
    };
    assert_eq!(
        [found_alive, found_unknown, found_dead],
        [&alive, &unknown, &dead]
    );
    assert_eq!(files.map(|file| file.exists()), [true, true, false]);
}

/// DSC-04 Contract case (uncertain disappearance): a path that is not a socket is never
/// unlinked as one. Linux refuses a connection to a regular file just as it refuses one to a
/// dead daemon's socket, so with a dead identity the session is stale, but its cleanup fails
/// before deleting anything; macOS reports the path unreachable. The metadata stays.
#[test]
fn a_regular_file_at_the_socket_path_is_never_deleted() {
    let (_directory, home) = home();
    let stale = record(&home, 0x71, CREATED);
    let file = save(&home, &stale);
    let socket = Path::new(stale.socket_path.as_str());
    fs::write(socket, b"not a socket").unwrap();

    let pass = discover(&home, REMOVE).unwrap();
    let [outcome] = pass.sessions.as_slice() else {
        panic!("{pass:?}");
    };
    assert_eq!(outcome.metadata(), &stale);
    if cfg!(target_os = "linux") {
        assert!(
            matches!(outcome, Outcome::Stale(_, Cleanup::Failed(_))),
            "{pass:?}"
        );
    } else {
        assert!(
            matches!(outcome, Outcome::Uncertain(_, Uncertainty::Unreachable(error))
                if error.raw_os_error() == Some(libc::ENOTSOCK)),
            "{pass:?}"
        );
    }
    assert_eq!(fs::read(socket).unwrap(), b"not a socket");
    assert!(file.exists());
    fs::remove_file(socket).unwrap();
}

/// A monitor's scan: unbounded metadata, broad batches and an ample pass deadline, changed
/// field by field where a test bounds one.
const BROAD: Scan = Scan {
    limits: MetadataLimits::UNBOUNDED,
    probes: NonZeroUsize::new(8).unwrap(),
    batch_records: NonZeroUsize::new(128).unwrap(),
    batch_bytes: u64::MAX,
    pass_time: Duration::from_secs(600),
};

fn records(count: usize) -> NonZeroUsize {
    NonZeroUsize::new(count).unwrap()
}

fn suffix_id(suffix: u8) -> String {
    format!("12345678-1234-4234-8234-123456789a{suffix:02x}")
}

/// One incremental pass: each batch's session IDs in the order reported, then its progress.
fn batches(
    continuation: &mut Continuation,
    home: &HomeLayout,
    scan: &Scan,
) -> (Vec<Vec<String>>, Progress) {
    batches_changing(continuation, home, scan, |_| {})
}

/// As [`batches`], calling `change` with each batch's index once it has been reported.
fn batches_changing(
    continuation: &mut Continuation,
    home: &HomeLayout,
    scan: &Scan,
    mut change: impl FnMut(usize),
) -> (Vec<Vec<String>>, Progress) {
    let mut seen = Vec::new();
    let progress = continuation.pass(home, scan, |batch: Batch| {
        assert!(batch.invalid.is_empty(), "{batch:?}");
        seen.push(
            ids(&batch.sessions)
                .iter()
                .map(|id| id.to_string())
                .collect(),
        );
        change(seen.len() - 1);
    });
    (seen, progress)
}

fn ids_of(suffixes: &[u8]) -> Vec<String> {
    suffixes.iter().map(|&suffix| suffix_id(suffix)).collect()
}

/// DSC-03 and DSC-04 Contract cases (incremental passes): a pass reports every entry once,
/// in batches of at most `batch_records` entries, a batch ending before the entry whose
/// metadata would take it past `batch_bytes` (at the bound and one byte short of it). A
/// completed pass is distinguished from an incomplete one, and a completed empty pass reports
/// no batch at all. Nothing is ever deleted.
#[test]
fn incremental_passes_report_every_entry_once_in_bounded_batches() {
    let (_directory, home) = home();
    let mut continuation = Continuation::default();
    assert!(matches!(
        batches(&mut continuation, &home, &BROAD),
        (seen, Progress::Complete) if seen.is_empty()
    ));
    let suffixes = [0x81, 0x82, 0x83, 0x84, 0x85];
    let files: Vec<_> = suffixes
        .map(|suffix| save(&home, &record(&home, suffix, CREATED)))
        .to_vec();
    let size = fs::metadata(&files[0]).unwrap().len();
    let cases = [
        (2, u64::MAX, vec![&[0x81, 0x82][..], &[0x83, 0x84], &[0x85]]),
        (5, u64::MAX, vec![&suffixes[..]]),
        (4, u64::MAX, vec![&suffixes[..4], &[0x85]]),
        (
            128,
            2 * size,
            vec![&[0x81, 0x82][..], &[0x83, 0x84], &[0x85]],
        ),
        (
            128,
            2 * size - 1,
            vec![&[0x81][..], &[0x82], &[0x83], &[0x84], &[0x85]],
        ),
    ];
    for (batch_records, batch_bytes, expected) in cases {
        let scan = Scan {
            batch_records: records(batch_records),
            batch_bytes,
            ..BROAD
        };
        let (seen, progress) = batches(&mut continuation, &home, &scan);
        let expected: Vec<_> = expected.into_iter().map(ids_of).collect();
        assert_eq!(
            seen, expected,
            "{batch_records} records, {batch_bytes} bytes"
        );
        assert!(matches!(progress, Progress::Complete), "{progress:?}");
    }
    assert!(files.iter().all(|file| file.exists()));
}

/// DSC-03 Contract case (fair continuation while the directory changes): entries added
/// after the pass's position are reported in the same pass, and one removed before it is
/// reached is not; an entry added behind the position waits for the next pass, which
/// reports every entry again.
#[test]
fn a_pass_continues_through_directory_changes_and_the_next_pass_reaches_what_it_missed() {
    let (_directory, home) = home();
    for suffix in [0x91, 0x93, 0x95, 0x97] {
        save(&home, &record(&home, suffix, CREATED));
    }
    let mut continuation = Continuation::default();
    let scan = Scan {
        batch_records: records(1),
        ..BROAD
    };
    let (seen, progress) = batches_changing(&mut continuation, &home, &scan, |index| {
        if index == 0 {
            save(&home, &record(&home, 0x90, CREATED));
            save(&home, &record(&home, 0x96, CREATED));
            fs::remove_file(path(&home, &record(&home, 0x95, CREATED))).unwrap();
        }
    });
    assert!(matches!(progress, Progress::Complete), "{progress:?}");
    let singles = |suffixes: &[u8]| -> Vec<Vec<String>> {
        suffixes.iter().map(|&suffix| ids_of(&[suffix])).collect()
    };
    assert_eq!(seen, singles(&[0x91, 0x93, 0x96, 0x97]));
    let (seen, progress) = batches(&mut continuation, &home, &scan);
    assert!(matches!(progress, Progress::Complete), "{progress:?}");
    assert_eq!(seen, singles(&[0x90, 0x91, 0x93, 0x96, 0x97]));
}

/// A peer at `metadata`'s socket that accepts `probes` hellos and never answers, each until
/// the probe hangs up.
fn silent_peer(metadata: &SessionMetadata, probes: usize) -> thread::JoinHandle<()> {
    let listener = runtime::bind(Path::new(metadata.socket_path.as_str())).unwrap();
    thread::spawn(move || {
        for _ in 0..probes {
            let (mut socket, _) = listener.accept().unwrap();
            read_frame(&mut socket).unwrap();
            socket.set_read_timeout(Some(HELLO_TIMEOUT * 5)).unwrap();
            assert_eq!(socket.read_to_end(&mut Vec::new()).unwrap(), 0);
        }
    })
}

/// DSC-04 Contract and Fault cases (slow and dead probes, incomplete discovery) with DSC-03
/// fair continuation: a pass whose deadline passes reports incomplete progress with what it
/// did probe, the silent daemon uncertain after the frozen two-second hello deadline. The
/// next pass continues after it, reaching the entries the first missed before probing the
/// silent daemon again, and completes.
#[test]
fn a_pass_deadline_reports_incomplete_progress_and_the_next_pass_continues_after_it() {
    let (_directory, home) = home();
    let silent = record(&home, 0xa1, CREATED);
    let files = [0xa1, 0xa2, 0xa3].map(|suffix| save(&home, &record(&home, suffix, CREATED)));
    let peer = silent_peer(&silent, 2);
    let mut continuation = Continuation::default();
    let hurried = Scan {
        probes: records(1),
        pass_time: Duration::from_millis(100),
        ..BROAD
    };
    let mut first = Vec::new();
    let started = Instant::now();
    let progress = continuation.pass(&home, &hurried, |batch| first.push(batch));
    assert!(started.elapsed() >= HELLO_TIMEOUT);
    assert!(
        matches!(progress, Progress::Incomplete(Incomplete::Deadline)),
        "{progress:?}"
    );
    let [batch] = first.as_slice() else {
        panic!("{first:?}");
    };
    let [Outcome::Uncertain(found, Uncertainty::Unanswered(error))] = batch.sessions.as_slice()
    else {
        panic!("{batch:?}");
    };
    assert_eq!(found, &silent);
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);

    let (seen, progress) = batches(&mut continuation, &home, &BROAD);
    assert!(matches!(progress, Progress::Complete), "{progress:?}");
    assert_eq!(seen, [ids_of(&[0xa2, 0xa3]), ids_of(&[0xa1])]);
    peer.join().unwrap();
    assert!(files.iter().all(|file| file.exists()));
    fs::remove_file(silent.socket_path.as_str()).unwrap();
}

/// DSC-04 Contract case (bounded probes): with one probe at a time, two silent daemons are
/// probed one after the other, so the pass lasts at least two hello deadlines.
#[test]
fn probes_are_bounded_by_the_scans_concurrency() {
    let (_directory, home) = home();
    let silent = [0xb1, 0xb2].map(|suffix| record(&home, suffix, CREATED));
    let peers = silent.each_ref().map(|metadata| {
        save(&home, metadata);
        silent_peer(metadata, 1)
    });
    let scan = Scan {
        probes: records(1),
        ..BROAD
    };
    let started = Instant::now();
    let (seen, progress) = batches(&mut Continuation::default(), &home, &scan);
    assert!(started.elapsed() >= HELLO_TIMEOUT * 2);
    assert!(matches!(progress, Progress::Complete), "{progress:?}");
    assert_eq!(seen, [ids_of(&[0xb1, 0xb2])]);
    for peer in peers {
        peer.join().unwrap();
    }
    for metadata in &silent {
        fs::remove_file(metadata.socket_path.as_str()).unwrap();
    }
}

/// DSC-04 Contract case (bounded probes): probes do run concurrently up to the bound: two
/// daemons each hold their answer until both have been reached by the same pass.
#[test]
fn probes_run_concurrently_up_to_the_bound() {
    let (_directory, home) = home();
    let live = [0xc1, 0xc2].map(|suffix| record(&home, suffix, CREATED));
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let peers = live.each_ref().map(|metadata| {
        save(&home, metadata);
        let listener = runtime::bind(Path::new(metadata.socket_path.as_str())).unwrap();
        let reply = climon2_core::protocol::Frame::new(
            1,
            hello_body(metadata.session_id.as_str()).into_bytes(),
        )
        .unwrap();
        let barrier = barrier.clone();
        thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            read_frame(&mut socket).unwrap();
            barrier.wait();
            write_frame(&mut socket, &reply).unwrap();
        })
    });
    let scan = Scan {
        probes: records(2),
        ..BROAD
    };
    let mut reported = Vec::new();
    let progress = Continuation::default().pass(&home, &scan, |batch| reported.push(batch));
    assert!(matches!(progress, Progress::Complete), "{progress:?}");
    let [batch] = reported.as_slice() else {
        panic!("{reported:?}");
    };
    assert!(
        batch
            .sessions
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Live(_))),
        "{batch:?}"
    );
    for peer in peers {
        peer.join().unwrap();
    }
    for metadata in &live {
        fs::remove_file(metadata.socket_path.as_str()).unwrap();
    }
}

/// DSC-04 Contract case (inaccessible discovery): an incremental pass over a sessions
/// directory that cannot be listed reports incomplete progress with the listing error, never
/// a completed empty pass, and deletes nothing.
#[test]
fn an_unlistable_sessions_directory_makes_the_pass_incomplete() {
    let (_directory, home) = home();
    let sessions = home.root().join("sessions");
    fs::write(&sessions, b"not a directory").unwrap();
    let (seen, progress) = batches(&mut Continuation::default(), &home, &BROAD);
    assert!(seen.is_empty());
    let Progress::Incomplete(Incomplete::Inaccessible(error)) = progress else {
        panic!("{progress:?}");
    };
    assert_eq!(error.raw_os_error(), Some(libc::ENOTDIR), "{error}");
    assert_eq!(fs::read(&sessions).unwrap(), b"not a directory");
}
