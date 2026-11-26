use climon2_core::{
    hello::{HelloExtensions, Lifecycle},
    protocol::{HelloReply, HelloRequest, read_frame, write_frame},
};
use climon2_platform::{daemon, process, runtime::RuntimeDirectory};
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Daemon wire revision 1, the only protocol after hello, through this harness.
mod revision1;

const ID: &str = "01234567-89ab-4cde-8fab-0123456789ab";
/// The release this build is, as the CLI passes it to every daemon it starts.
const VERSION: &str = env!("CARGO_PKG_VERSION");

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
    for args in [
        vec![],
        vec!["--secret\nvalue"],
        vec!["--exec-command"],
        // Windows' Ctrl+Break helper (#326) does not exist on Unix.
        vec!["--break-console", "1"],
    ] {
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

    /// A session that must start: a failed readiness wait keeps its evidence
    /// (#158) before anything else can end the test.
    fn configured(mode: &str, configure: impl FnOnce(&mut Command, &Path)) -> Self {
        let (session, readiness, wait) = Self::launch(mode, configure, false, false);
        if !matches!(readiness, Ok(daemon::Readiness::Ready)) {
            let home = session.directory.path();
            let log = home.join(format!("logs/{ID}/daemon.log"));
            eprintln!("daemon log: {:?}", fs::read_to_string(&log));
            let report = wait.describe(&readiness);
            eprintln!("{report}");
            keep_evidence(&report, &log);
            assert_startup_command_gone(home);
        }
        assert_eq!(readiness.unwrap(), daemon::Readiness::Ready);
        session
    }

    /// A session that may fail to start, as some tests expect.
    fn attempt(
        mode: &str,
        configure: impl FnOnce(&mut Command, &Path),
        bad_cwd: bool,
        missing: bool,
    ) -> (Self, io::Result<daemon::Readiness>) {
        let (session, readiness, _) = Self::launch(mode, configure, bad_cwd, missing);
        if !matches!(readiness, Ok(daemon::Readiness::Ready)) {
            assert_startup_command_gone(session.directory.path());
        }
        (session, readiness)
    }

    fn launch(
        mode: &str,
        configure: impl FnOnce(&mut Command, &Path),
        bad_cwd: bool,
        missing: bool,
    ) -> (Self, io::Result<daemon::Readiness>, ReadinessWait) {
        let (prepared, command) = Prepared::new(mode, configure, bad_cwd);
        let Prepared {
            directory,
            home,
            socket,
            metadata,
            startup_socket,
        } = prepared;
        let spawning = SystemTime::now();
        let pending = daemon::spawn(command, |command, fd| {
            command_tail(command, fd, &home, mode, missing);
        })
        .unwrap();
        let pid = pending.id();
        let spawned = SystemTime::now();
        let readiness = pending.wait_ready(&[&metadata, &socket]);
        let wait = ReadinessWait {
            pid,
            spawning,
            spawned,
            answered: SystemTime::now(),
        };
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
            wait,
        )
    }

    fn connect(&self) -> UnixStream {
        let mut stream = UnixStream::connect(&self.socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(8)))
            .unwrap();
        send_hello(&mut stream);
        receive_hello(&mut stream);
        stream
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

/// The test's side of one readiness wait (#158): the signal waited for is the
/// daemon's reply on the readiness pipe, which `wait_ready` reads.
struct ReadinessWait {
    pid: u32,
    /// `daemon::spawn` was called; its readiness deadline starts just after.
    spawning: SystemTime,
    /// `daemon::spawn` returned: the daemon had exec'd.
    spawned: SystemTime,
    /// `wait_ready` returned.
    answered: SystemTime,
}

impl ReadinessWait {
    fn describe(&self, readiness: &io::Result<daemon::Readiness>) -> String {
        let since = |time: SystemTime| {
            time.duration_since(self.spawning)
                .unwrap_or_default()
                .as_millis()
        };
        format!(
            "readiness wait for daemon pid {}, on its readiness pipe reply:\n\
             spawn called   {}\n\
             spawn returned {} (+{} ms)\n\
             wait returned  {} (+{} ms): {readiness:?}\n",
            self.pid,
            utc(self.spawning),
            utc(self.spawned),
            since(self.spawned),
            utc(self.answered),
            since(self.answered),
        )
    }
}

/// Keeps a failed readiness wait's evidence where CI uploads it (#158): the
/// wait's timestamps and the daemon's log, under the test's own name in
/// `CLIMON2_TEST_EVIDENCE`. Without that variable the panic output is all.
fn keep_evidence(wait: &str, log: &Path) {
    let Some(root) = std::env::var_os("CLIMON2_TEST_EVIDENCE") else {
        return;
    };
    let name = thread::current()
        .name()
        .unwrap_or("unnamed")
        .replace("::", "-");
    let directory = Path::new(&root).join(format!("lifecycle-{name}"));
    let _ = fs::create_dir_all(&directory);
    let _ = fs::write(directory.join("readiness-wait.txt"), wait);
    let _ = fs::copy(log, directory.join("daemon.log"));
}

/// `time` in UTC as daemon.log writes it, to the microsecond.
fn utc(time: SystemTime) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = since.as_secs();
    let (days, of_day) = (seconds / 86_400, seconds % 86_400);
    // Howard Hinnant's civil_from_days, for days since 1970-01-01.
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let of_era = shifted % 146_097;
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:06}Z",
        of_day / 3600,
        of_day / 60 % 60,
        of_day % 60,
        since.subsec_micros()
    )
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
        command.args(["--session-id", ID, "--version", VERSION, "--cli-pid", "1"]);
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

/// The documented notification assembly limits, unless the test gave its
/// own.
const ASSEMBLY: [&str; 6] = [
    "--assembly-records",
    "32",
    "--assembly-bytes",
    "65536",
    "--assembly-ms",
    "5000",
];

fn command_tail(command: &mut Command, fd: u32, home: &Path, mode: &str, missing: bool) {
    if !command.get_args().any(|argument| argument == ASSEMBLY[0]) {
        command.args(ASSEMBLY);
    }
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
    } else if mode == "silent" {
        // Not the libtest fixture, which prints its harness lines: nothing at all.
        command.args(["sleep", "120"]);
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

/// The frozen hello request's bytes.
fn hello_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    let hello = HelloRequest { protocol: 1 }.to_frame().unwrap();
    write_frame(&mut bytes, &hello).unwrap();
    bytes
}

/// Sends the frozen hello request.
fn send_hello(stream: &mut UnixStream) {
    stream.write_all(&hello_bytes()).unwrap();
}

/// The daemon's frozen hello reply.
fn receive_hello(stream: &mut UnixStream) -> HelloReply {
    HelloReply::from_frame(&read_frame(stream).unwrap()).unwrap()
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

/// The release bytes of a 91x37 presentation, then the cursor placed at
/// `row` and `col`.
fn release_at(row: u16, col: u16) -> Vec<u8> {
    let mut bytes = climon2_vt::Presentation::new(91, 37).release();
    bytes.extend(format!("\x1b[{row};{col}H").bytes());
    bytes
}

/// Waits for a fixture's marker file. Fixtures write theirs after their own
/// output, which a loaded host can take well over five seconds to produce.
fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
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
    send_hello(&mut probe);
    let extensions = HelloExtensions::from_body(read_frame(&mut probe).unwrap().body());
    let capabilities = extensions.capabilities.valid().unwrap();
    assert!(capabilities.supports_attach() && capabilities.supports_kill());
    assert!(capabilities.supports_state());
    let state = extensions.state.valid().unwrap();
    (state.lifecycle(), state.attachments(), state.controlled())
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

/// Whether `pid` still has a descriptor open on `target`.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn holds(pid: u32, target: &Path) -> bool {
    fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .filter_map(|entry| fs::read_link(entry.unwrap().path()).ok())
        .any(|link| link == target)
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

/// #133 / ADR 0005: a real daemon records where it was created, its machine
/// and on Linux its PID namespace, as this process reads its own.
#[test]
fn issue133_a_daemon_records_its_origin() {
    let mut session = Session::start("early");
    let bytes = fs::read(&session.metadata).unwrap();
    let metadata =
        climon2_core::metadata::SessionMetadata::from_json(&session.metadata, &bytes).unwrap();
    let here = climon2_platform::origin::current(session.directory.path()).unwrap();
    assert_eq!(metadata.daemon_origin, Some(here));
    session.finish();
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

/// The numbers of the `row-NNNN` rows in a restoration, in order.
fn numbered_rows(restored: &[u8]) -> Vec<u32> {
    restored
        .windows(8)
        .filter(|bytes| bytes.starts_with(b"row-") && bytes[4..].iter().all(u8::is_ascii_digit))
        .map(|bytes| std::str::from_utf8(&bytes[4..]).unwrap().parse().unwrap())
        .collect()
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
            command.args(["--version", VERSION]);
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
/// UTF-8 bytes, `é` repeated), `titles A B` (`t-A` through `t-B` in one
/// write) and `long-titles A B` (the same, each padded with `x` to 4,000
/// bytes); `text TEXT` prints TEXT as a line, `colors N` prints `N` rows
/// whose every cell changes colour, `print TEXT` prints TEXT with each `\e`
/// replaced by ESC and no line end, `reset` is `ESC c`, and `exit` ends the
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
                ["long-titles", first, last] => {
                    for title in first.parse::<u32>().unwrap()..=last.parse().unwrap() {
                        let head = format!("t-{title}-");
                        let padding = "x".repeat(4000 - head.len());
                        write!(out, "\x1b]2;{head}{padding}\x07").unwrap();
                    }
                }
                ["title", ..] => write!(out, "\x1b]2;{}\x07", &command["title ".len()..]).unwrap(),
                ["text", ..] => write!(out, "{}\r\n", &command["text ".len()..]).unwrap(),
                ["print", ..] => {
                    let text = command["print ".len()..].replace("\\e", "\x1b");
                    write!(out, "{text}").unwrap();
                }
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
    if mode == "status-query-record" {
        record_input("recorded", Some("\x1b]7501;?\x1b\\\x1b[c"));
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

/// #158: a failed wait's timestamps read as daemon.log's, so the two line up.
#[test]
fn readiness_wait_evidence_is_in_daemon_log_time() {
    // 2026-11-15T12:21:12.197123Z, the daemon start in #158's first evidence.
    let time = UNIX_EPOCH + Duration::from_micros(1_794_745_272_197_123);
    assert_eq!(utc(time), "2026-11-15T12:21:12.197123Z");
    assert_eq!(utc(UNIX_EPOCH), "1970-01-01T00:00:00.000000Z");
    assert_eq!(
        utc(UNIX_EPOCH + Duration::from_secs(951_782_400)),
        "2000-02-29T00:00:00.000000Z"
    );
}
