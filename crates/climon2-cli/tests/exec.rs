#![cfg(target_os = "macos")]

use climon2_test_term::{Terminal, bin_path};
use portable_pty::CommandBuilder;
use std::{
    ffi::{OsStr, OsString},
    fs,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

struct Layout {
    directory: tempfile::TempDir,
    installation: Arc<Installation>,
}

struct Installation(tempfile::TempDir);

static INSTALLATION: Mutex<Weak<Installation>> = Mutex::new(Weak::new());

impl Installation {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("binaries-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap();
        let version = directory.path().join("exe/0.1.0");
        fs::create_dir_all(&version).unwrap();
        for binary in ["climon2-cli", "climon2-client", "climon2-daemon"] {
            fs::copy(bin_path(binary), version.join(binary)).unwrap();
        }
        Self(directory)
    }

    fn shared() -> Arc<Self> {
        let mut current = INSTALLATION.lock().unwrap();
        if let Some(installation) = current.upgrade() {
            return installation;
        }
        let installation = Arc::new(Self::new());
        *current = Arc::downgrade(&installation);
        installation
    }

    fn version(&self) -> PathBuf {
        self.0.path().join("exe/0.1.0")
    }
}

impl Layout {
    fn new() -> Self {
        Self::with_installation(Installation::shared())
    }

    fn private() -> Self {
        Self::with_installation(Arc::new(Installation::new()))
    }

    fn with_installation(installation: Arc<Installation>) -> Self {
        let directory = tempfile::Builder::new()
            .prefix("exec-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap();
        Self {
            directory,
            installation,
        }
    }

    fn home(&self) -> PathBuf {
        self.directory.path().join("data")
    }
    fn cli(&self) -> PathBuf {
        self.installation.version().join("climon2-cli")
    }
    fn fixture(&self) -> PathBuf {
        bin_path("climon2-cli-fixture")
    }
    fn marker(&self) -> &OsStr {
        self.directory.path().file_name().unwrap()
    }
    fn command(&self) -> CommandBuilder {
        let mut command = CommandBuilder::new(self.cli());
        self.configure(&mut command);
        command
    }

    fn configure(&self, command: &mut CommandBuilder) {
        command.env_clear();
        command.env("HOME", self.directory.path());
        command.env("CLIMON2_HOME", self.home());
        command.env("PATH", "/usr/bin:/bin");
        command.env("CLIMON2_TEST_ID", self.marker());
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        command.cwd(self.directory.path());
    }

    fn process(&self) -> Command {
        let mut command = Command::new(self.cli());
        command
            .env_clear()
            .env("HOME", self.directory.path())
            .env("CLIMON2_HOME", self.home());
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        command
    }

    fn clean(&self) {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !self.files_clean() || !self.live_pids().is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "session metadata/socket cleanup did not finish"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn live_pids(&self) -> Vec<i32> {
        let output = Command::new("/bin/ps")
            .args(["-axEww", "-o", "pid=,stat=,command="])
            .output()
            .unwrap();
        assert!(output.status.success());
        let marker = format!("CLIMON2_TEST_ID={}", self.marker().to_string_lossy());
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let pid = fields.next()?.parse().ok()?;
                let zombie = fields.next().is_some_and(|state| state.starts_with('Z'));
                (!zombie && fields.any(|field| field == marker)).then_some(pid)
            })
            .collect()
    }

    fn files_clean(&self) -> bool {
        let home = self.home();
        let sessions = home.join("sessions");
        if sessions.exists() && fs::read_dir(sessions).unwrap().count() != 0 {
            return false;
        }
        if home.exists() {
            let runtime = climon2_core::layout::RuntimeLayout::new(
                std::path::Path::new("/tmp"),
                unsafe { libc::geteuid() },
                &home.canonicalize().unwrap(),
            )
            .unwrap();
            if runtime.directory().exists()
                && fs::read_dir(runtime.directory()).unwrap().count() != 0
            {
                return false;
            }
        }
        true
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        for pid in self.live_pids() {
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        while !self.live_pids().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        for pid in self.live_pids() {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

/// #17/#20: reject either redirected terminal descriptor before spawning.
#[test]
fn exec_rejects_non_tty_before_creating_a_session() {
    let layout = Layout::new();
    let output = layout
        .process()
        .arg("exec")
        .arg(layout.fixture())
        .arg("hold")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(125));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("climon2 exec needs a terminal"));
    layout.clean();
}

/// #19/#20: real installed siblings, initial dimensions, early command exit.
#[test]
fn packaged_exec_preserves_final_command_status() {
    let layout = Layout::new();
    let mut command = layout.command();
    command.args(["exec"]);
    command.arg(layout.fixture());
    command.args(["exit", "43"]);
    let mut terminal = Terminal::spawn(command, 31, 97).unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(8)).unwrap(), 43);
    layout.clean();
}

fn hex(value: &OsStr) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn only_metadata(layout: &Layout) -> climon2_core::metadata::SessionMetadata {
    let paths = fs::read_dir(layout.home().join("sessions"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(paths.len(), 1);
    climon2_core::metadata::SessionMetadata::from_json(&paths[0], &fs::read(&paths[0]).unwrap())
        .unwrap()
}

fn fixture_runtime_environment(layout: &Layout) -> std::collections::BTreeMap<String, String> {
    let mut command = Command::new(layout.fixture());
    command
        .arg("environment")
        .env_clear()
        .current_dir(layout.directory.path());
    let mut expected = std::collections::BTreeMap::new();
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", &profile);
        expected.insert(hex(OsStr::new("LLVM_PROFILE_FILE")), hex(&profile));
    }
    let output = command.output().unwrap();
    assert!(output.status.success());
    let actual: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&output.stdout).unwrap();
    // Coverage's pre-main runtime may add this marker even without climon2.
    let marker = hex(OsStr::new("__LLVM_PROFILE_RT_INIT_ONCE"));
    if actual.contains_key(&marker) {
        expected.insert(marker.clone(), marker);
    }
    assert_eq!(actual, expected);
    actual
}

/// #29/#36: exact OS strings, cwd, whole inherited environment and absent TERM.
#[test]
fn packaged_exec_preserves_runtime_argv_cwd_environment_and_initial_size() {
    let layout = Layout::new();
    let mut command = layout.command();
    let odd = OsString::from_vec(b"non-utf8-\xff".to_vec());
    command.env("SECRET_VALUE", &odd);
    command.env("TERM", "");
    command.env("SHELL", "");
    command.args(["exec"]);
    command.arg(layout.fixture());
    command.args(["report", "", "--looks-like-option", "literal $ ; ' \""]);
    command.arg(&odd);
    let mut terminal = Terminal::spawn(command, 31, 97).unwrap();
    terminal
        .wait_for_text("FIXTURE_READY", Duration::from_secs(8))
        .unwrap();
    let metadata = only_metadata(&layout);
    let expected_args = serde_json::json!([
        "",
        hex(OsStr::new("--looks-like-option")),
        hex(OsStr::new("literal $ ; ' \"")),
        hex(&odd)
    ]);
    terminal
        .wait_for_bytes(
            format!("\"argv\":{expected_args}").as_bytes(),
            Duration::from_secs(1),
        )
        .unwrap();
    terminal
        .wait_for_bytes(
            format!("\"cwd\":\"{}\"", hex(layout.directory.path().as_os_str())).as_bytes(),
            Duration::from_secs(1),
        )
        .unwrap();
    terminal
        .wait_for_bytes(b"\"cols\":97", Duration::from_secs(1))
        .unwrap();
    terminal
        .wait_for_bytes(b"\"rows\":31", Duration::from_secs(1))
        .unwrap();
    let mut expected_environment: std::collections::BTreeMap<_, _> = [
        ("HOME", layout.directory.path().as_os_str().to_owned()),
        ("CLIMON2_HOME", layout.home().into_os_string()),
        ("PATH", "/usr/bin:/bin".into()),
        ("CLIMON2_TEST_ID", layout.marker().to_owned()),
        ("SECRET_VALUE", odd),
        ("TERM", "".into()),
        ("SHELL", "".into()),
        ("CLIMON2_SESSION_ID", metadata.session_id.as_str().into()),
        ("CLIMON2_SESSION_DEPTH", "1".into()),
    ]
    .into_iter()
    .map(|(key, value)| (hex(OsStr::new(key)), hex(&value)))
    .collect();
    expected_environment.extend(fixture_runtime_environment(&layout));
    terminal
        .wait_for_bytes(
            format!(
                "\"env\":{}",
                serde_json::to_string(&expected_environment).unwrap()
            )
            .as_bytes(),
            Duration::from_secs(1),
        )
        .unwrap();
    terminal.send(b"\n").unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(8)).unwrap(), 0);
    let logs = layout
        .home()
        .join("logs")
        .join(metadata.session_id.as_str());
    for role in ["cli", "daemon", "client"] {
        let contents = fs::read_to_string(logs.join(format!("{role}.log"))).unwrap();
        assert!(contents.contains(&format!("role={role}")));
        assert!(!contents.contains("SECRET_VALUE"));
        assert!(!contents.contains("looks-like-option"));
    }
    let general = fs::read_dir(layout.home().join("logs/general"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let contents = fs::read_to_string(&general).unwrap();
    assert!(contents.contains(&format!(
        "session {} started; continuing in ",
        metadata.session_id
    )));
    assert!(
        fs::read_to_string(logs.join("cli.log"))
            .unwrap()
            .contains(general.to_str().unwrap())
    );
    layout.clean();
}

/// #29: malformed enclosing depth defaults to one; ordinal exceptions are exact.
#[test]
fn nested_warning_uses_exact_ordinal_and_overflow_never_spawns() {
    let layout = Layout::new();
    for (depth, ordinal) in [
        ("garbage", "1st"),
        ("0", "1st"),
        ("1", "1st"),
        ("2", "2nd"),
        ("3", "3rd"),
        ("11", "11th"),
        ("12", "12th"),
        ("13", "13th"),
        ("21", "21st"),
        ("112", "112th"),
    ] {
        let mut command = layout.command();
        command.env("CLIMON2_SESSION_ID", "outer");
        command.env("CLIMON2_SESSION_DEPTH", depth);
        command.arg("exec");
        command.arg(layout.fixture());
        command.args(["exit", "0"]);
        let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
        terminal
            .wait_for_bytes(
                format!("Starting climon2 session nested for the {ordinal} time").as_bytes(),
                Duration::from_secs(8),
            )
            .unwrap();
        assert_eq!(terminal.wait_exit(Duration::from_secs(8)).unwrap(), 0);
        layout.clean();
    }
    let mut command = layout.command();
    command.env("CLIMON2_SESSION_ID", "outer");
    command.env("CLIMON2_SESSION_DEPTH", "4294967295");
    command.arg("exec");
    command.arg(layout.fixture());
    command.arg("hold");
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    terminal
        .wait_for_text("session depth exceeds u32", Duration::from_secs(3))
        .unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(3)).unwrap(), 125);
    layout.clean();
}

/// #29/#36: shells remain programs; real nested bash inherits depth and TERM.
#[test]
fn actual_nested_bash_session_increments_depth_and_defaults_only_absent_term() {
    let layout = Layout::new();
    let mut command = layout.command();
    command.args([
        "exec",
        "/bin/bash",
        "--noprofile",
        "--norc",
        "-c",
        "\"$1\" exec \"$2\" report",
        "bash",
    ]);
    command.arg(layout.cli());
    command.arg(layout.fixture());
    let mut terminal = Terminal::spawn(command, 31, 120).unwrap();
    terminal
        .wait_for_bytes(
            b"Starting climon2 session nested for the 1st time",
            Duration::from_secs(8),
        )
        .unwrap();
    terminal
        .wait_for_bytes(
            format!(
                "\"{}\":\"{}\"",
                hex(OsStr::new("CLIMON2_SESSION_DEPTH")),
                hex(OsStr::new("2"))
            )
            .as_bytes(),
            Duration::from_secs(8),
        )
        .unwrap();
    terminal
        .wait_for_bytes(
            format!(
                "\"{}\":\"{}\"",
                hex(OsStr::new("TERM")),
                hex(OsStr::new("xterm-256color"))
            )
            .as_bytes(),
            Duration::from_secs(1),
        )
        .unwrap();
    terminal.send(b"\n").unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(8)).unwrap(), 0);
    layout.clean();
}

/// #17/#19: command failures are not collapsed into the climon2 error code.
#[test]
fn startup_failure_codes_and_signal_exit_are_preserved() {
    let layout = Layout::new();
    let invalid = layout.directory.path().join("not-executable");
    fs::write(&invalid, "not executable").unwrap();
    for (program, args, expected) in [
        (
            PathBuf::from("certainly-no-such-climon2-command"),
            vec![],
            127,
        ),
        (invalid, vec![], 126),
        (layout.fixture(), vec!["signal", "15"], 143),
        (layout.fixture(), vec!["exit", "125"], 125),
        (layout.fixture(), vec!["exit", "127"], 127),
    ] {
        let mut command = layout.command();
        command.arg("exec");
        command.arg(program);
        command.args(args);
        let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
        assert_eq!(
            terminal.wait_exit(Duration::from_secs(8)).unwrap(),
            expected
        );
        layout.clean();
    }
}

/// #20: actual client SIGWINCH changes the command's real PTY dimensions.
#[test]
fn resizing_outer_terminal_resizes_the_running_command() {
    let layout = Layout::new();
    let mut command = layout.command();
    command.arg("exec");
    command.arg(layout.fixture());
    command.arg("hold");
    let mut terminal = Terminal::spawn(command, 31, 97).unwrap();
    terminal
        .wait_for_text("FIXTURE_READY", Duration::from_secs(8))
        .unwrap();
    terminal.resize(44, 132).unwrap();
    terminal.send(b"size\n").unwrap();
    terminal
        .wait_for_text("SIZE:132x44", Duration::from_secs(3))
        .unwrap();
    terminal.send(b"exit\n").unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(8)).unwrap(), 0);
    layout.clean();
}

/// #19/#25: after Ready, every unsuccessful exec ends daemon and command.
#[test]
fn missing_client_after_ready_cleans_command_and_names_version_with_logs() {
    let layout = Layout::private();
    fs::remove_file(layout.installation.version().join("climon2-client")).unwrap();
    let pid_path = layout.directory.path().join("command-pid");
    let mut command = layout.command();
    command.env("FIXTURE_PID_PATH", &pid_path);
    command.arg("exec");
    command.arg(layout.fixture());
    command.arg("hold");
    let mut terminal = Terminal::spawn(command, 24, 120).unwrap();
    terminal
        .wait_for_text(
            "needs climon2 0.1.0, which is not installed",
            Duration::from_secs(10),
        )
        .unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(3)).unwrap(), 125);
    assert!(terminal.screen().contains("(logs: "));
    if let Ok(pid) = fs::read_to_string(pid_path) {
        assert_eq!(unsafe { libc::kill(pid.parse().unwrap(), 0) }, -1);
    }
    layout.clean();
}

/// #19: startup timeout remains a real five seconds and never launches command.
#[test]
fn never_ready_daemon_is_killed_after_real_five_seconds() {
    let layout = Layout::new();
    let pid_path = layout.directory.path().join("command-pid");
    let mut command = layout.command();
    command.env("CLIMON2_DAEMON_FAULT", "never-ready");
    command.env("FIXTURE_PID_PATH", &pid_path);
    command.arg("exec");
    command.arg(layout.fixture());
    command.arg("hold");
    let started = std::time::Instant::now();
    let mut terminal = Terminal::spawn(command, 24, 120).unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(8)).unwrap(), 125);
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert!(started.elapsed() < Duration::from_secs(8));
    assert!(!pid_path.exists());
    assert!(terminal.screen().contains("(logs: "));
    layout.clean();
}

/// #19: canonicalization succeeding does not mean the client can be executed.
#[test]
fn unexecutable_client_after_ready_cleans_daemon_and_command() {
    use std::os::unix::fs::PermissionsExt;
    let layout = Layout::private();
    let client = layout.installation.version().join("climon2-client");
    fs::set_permissions(client, fs::Permissions::from_mode(0o600)).unwrap();
    let pid_path = layout.directory.path().join("command-pid");
    let mut command = layout.command();
    command.env("FIXTURE_PID_PATH", &pid_path);
    command.arg("exec");
    command.arg(layout.fixture());
    command.arg("hold");
    let mut terminal = Terminal::spawn(command, 24, 120).unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(10)).unwrap(), 125);
    if let Ok(pid) = fs::read_to_string(pid_path) {
        assert_eq!(unsafe { libc::kill(pid.parse().unwrap(), 0) }, -1);
    }
    assert!(terminal.screen().contains("Permission denied"));
    layout.clean();
}

/// #17: invoking through a symlink still locates canonical version companions.
#[test]
fn cli_symlink_does_not_change_companion_resolution() {
    let layout = Layout::new();
    let alias = layout.directory.path().join("cli-alias");
    std::os::unix::fs::symlink(layout.cli(), &alias).unwrap();
    let mut command = CommandBuilder::new(alias);
    layout.configure(&mut command);
    command.arg("exec");
    command.arg(layout.fixture());
    command.args(["exit", "42"]);
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(8)).unwrap(), 42);
    layout.clean();
}

/// #20: zero outer dimensions are converted before the command starts.
#[test]
fn zero_initial_terminal_size_falls_back_to_80_by_24() {
    use std::io::Read;
    let layout = Layout::new();
    let allocation = climon2_platform::pty::allocation().unwrap();
    let pair = portable_pty::native_pty_system()
        .openpty(portable_pty::PtySize {
            rows: 0,
            cols: 0,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    drop(allocation);
    let mut command = layout.command();
    command.arg("exec");
    command.arg(layout.fixture());
    command.arg("size");
    let mut reader = pair.master.try_clone_reader().unwrap();
    let fd = pair.master.as_raw_fd().unwrap();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    assert_eq!(
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );
    let mut writer = pair.master.take_writer().unwrap();
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut output = Vec::new();
    let mut bytes = [0; 4096];
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let mut sent = false;
    loop {
        match reader.read(&mut bytes) {
            Ok(0) => break,
            Ok(count) => {
                output.extend_from_slice(&bytes[..count]);
                if !sent && String::from_utf8_lossy(&output).contains("SIZE:80x24") {
                    use std::io::Write;
                    writer.write_all(b"\n").unwrap();
                    sent = true;
                }
            }
            Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    }
    assert_eq!(child.wait().unwrap().exit_code(), 0);
    assert!(String::from_utf8_lossy(&output).contains("SIZE:80x24"));
    layout.clean();
}

/// #17: an inherited deleted cwd is an unlaunchable command, not own failure.
#[test]
fn deleted_working_directory_is_startup_status_126() {
    let layout = Layout::new();
    let cwd = layout.directory.path().join("removed");
    fs::create_dir(&cwd).unwrap();
    let mut command = CommandBuilder::new(layout.fixture());
    layout.configure(&mut command);
    command.cwd(cwd);
    command.arg("removed-cwd");
    command.arg(layout.cli());
    command.arg(layout.fixture());
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(8)).unwrap(), 126);
    layout.clean();
}

/// #20: either single redirected descriptor is sufficient to reject exec.
#[test]
fn exec_requires_both_stdin_and_stdout_ttys_but_not_stderr() {
    use std::{
        os::fd::{FromRawFd, OwnedFd},
        process::Stdio,
    };
    let layout = Layout::new();
    let mut master = -1;
    let mut slave = -1;
    let allocation = climon2_platform::pty::allocation().unwrap();
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    drop(allocation);
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    for (input, output) in [
        (Stdio::from(slave.try_clone().unwrap()), Stdio::piped()),
        (Stdio::null(), Stdio::from(slave.try_clone().unwrap())),
    ] {
        let output = layout
            .process()
            .arg("exec")
            .arg(layout.fixture())
            .arg("hold")
            .stdin(input)
            .stdout(output)
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(125));
        assert!(String::from_utf8_lossy(&output.stderr).contains("climon2 exec needs a terminal"));
    }
    drop(master);
    layout.clean();
}

/// #17/#25: unresolved home never advertises logs; written general logs do.
#[test]
fn invalid_home_and_filter_fail_with_only_actual_written_log_paths() {
    let layout = Layout::new();
    for home in ["", "relative"] {
        let output = layout
            .process()
            .args(["list"])
            .env("CLIMON2_HOME", home)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(125));
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("(logs: "));
    }

    let output = layout
        .process()
        .args(["list"])
        .env("CLIMON2_LOG", "[")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(125));
    let error = String::from_utf8(output.stderr).unwrap();
    assert_eq!(error.lines().count(), 1);
    assert!(
        error.contains(&format!("(logs: {}/logs/general/", layout.home().display())),
        "{error}"
    );
}

/// #25/#28: one read filter reaches daemon args/client env without altering command.
#[test]
fn log_filter_is_forwarded_to_companions_without_changing_command_environment() {
    let layout = Layout::new();
    let mut command = layout.command();
    command.env("CLIMON2_LOG", "warn");
    command.arg("exec");
    command.arg(layout.fixture());
    command.arg("report");
    let mut terminal = Terminal::spawn(command, 31, 120).unwrap();
    terminal
        .wait_for_text("FIXTURE_READY", Duration::from_secs(8))
        .unwrap();
    terminal
        .wait_for_bytes(
            format!(
                "\"{}\":\"{}\"",
                hex(OsStr::new("CLIMON2_LOG")),
                hex(OsStr::new("warn"))
            )
            .as_bytes(),
            Duration::from_secs(1),
        )
        .unwrap();
    let metadata = only_metadata(&layout);
    terminal.send(b"\n").unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(8)).unwrap(), 0);
    for role in ["cli", "daemon", "client"] {
        let log = fs::read_to_string(
            layout
                .home()
                .join("logs")
                .join(metadata.session_id.as_str())
                .join(format!("{role}.log")),
        )
        .unwrap();
        assert!(!log.contains("DEBUG"), "{role}: {log}");
        assert!(!log.contains("TRACE"), "{role}: {log}");
    }
    layout.clean();
}

/// #17/#25: unusual absolute log paths cannot inject a second diagnostic line.
#[test]
fn log_suffix_escapes_control_characters_in_the_actual_absolute_home() {
    let layout = Layout::new();
    let home = layout.directory.path().join("data\nnext");
    let output = layout
        .process()
        .args(["exec", "/usr/bin/true"])
        .env("CLIMON2_HOME", &home)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(125));
    let error = String::from_utf8(output.stderr).unwrap();
    assert_eq!(error.lines().count(), 1);
    assert!(error.contains("data\\nnext/logs/general/"), "{error}");
    assert_eq!(fs::read_dir(home.join("logs/general")).unwrap().count(), 1);
}
