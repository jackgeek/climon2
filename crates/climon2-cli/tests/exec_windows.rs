//! #317: `c2 exec --headless` on Windows. A detached daemon, outside every
//! job, runs the command in the bundled ConPTY inside a session job; `list`
//! shows the session until the command exits, whose full status the daemon
//! records. Without either bundled ConPTY file, and in a job the daemon
//! cannot leave, `exec` exits 125 and leaves nothing running.
//!
//! #318: `c2 kill` ends such a session through the session version's client
//! and its client library: the pseudoconsole closes, the full grace passes,
//! the session job ends with `0xC000013A`, and kill exits 0 once the session
//! is cleaned up.
//!
//! cargo runs tests in a job that refuses breakaway, and so does the CI
//! runner, so each `exec` runs outside both, through the harness
//! (`climon2_test_term::Outside`, whose relay this binary declares).
#![cfg(windows)]
use climon2_core::metadata::SessionMetadata;
use climon2_platform::process;
use climon2_test_term::{ConPty, Finished, JobShape, Outside};
use std::{
    fs,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::PathBuf,
    process::Command,
    time::{Duration, Instant, SystemTime},
};
use windows_sys::Win32::{
    Foundation::{FILETIME, WAIT_OBJECT_0},
    System::{
        JobObjects::IsProcessInJob,
        Threading::{
            GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
        },
    },
};

climon2_test_term::outside_relay!();

/// `0xC000013A`, `STATUS_CONTROL_C_EXIT`, at full width.
const CONTROL_C_EXIT: &str = "0xc000013a";

/// A version folder, `<root>/exe/0.1.0`, outside the crate folder, holding
/// the CLI, the daemon, the client and its library and, unless left out, the
/// two bundled ConPTY files.
struct Install(tempfile::TempDir);

impl Install {
    fn new(left_out: &[&str]) -> Self {
        let install = Self(tempfile::tempdir().unwrap());
        fs::create_dir_all(install.version()).unwrap();
        let cli = PathBuf::from(env!("CARGO_BIN_EXE_climon2-cli"));
        let daemon = cli.with_file_name("climon2-daemon.exe");
        let client = cli.with_file_name("climon2-client.exe");
        let library = cli.with_file_name("climon2_client.dll");
        for (built, package) in [
            (&daemon, "climon2-daemon"),
            (&client, "climon2-client"),
            (&library, "climon2-client"),
        ] {
            assert!(
                built.is_file(),
                "missing {}; build {package} first (cargo test -p {package})",
                built.display()
            );
        }
        let conpty = ConPty::pinned().unwrap().folder();
        for from in [
            cli,
            daemon,
            client,
            library,
            conpty.join("conpty.dll"),
            conpty.join("OpenConsole.exe"),
        ] {
            let name = from.file_name().unwrap();
            if !left_out.iter().any(|left| name == *left) {
                fs::copy(&from, install.version().join(name)).unwrap();
            }
        }
        install
    }

    fn version(&self) -> PathBuf {
        self.0.path().join("exe").join("0.1.0")
    }

    fn cli(&self) -> PathBuf {
        self.version().join("climon2-cli.exe")
    }

    /// Whether no process runs this folder's daemon: Windows refuses to
    /// delete a running executable.
    fn daemon_stopped(&self) -> bool {
        fs::remove_file(self.version().join("climon2-daemon.exe")).is_ok()
    }
}

/// A climon2 home, runtime root and working directory outside the crate
/// folder. Dropping it ends every daemon its sessions name, so a failed test
/// leaves nothing running.
struct Home(tempfile::TempDir);

impl Home {
    fn new() -> Self {
        let home = Self(tempfile::tempdir().unwrap());
        fs::create_dir_all(home.work()).unwrap();
        home
    }

    fn home(&self) -> PathBuf {
        self.0.path().join("home")
    }

    fn work(&self) -> PathBuf {
        self.0.path().join("work")
    }

    fn outside(&self, install: &Install) -> Outside {
        Outside::new(install.cli())
            .env("CLIMON2_HOME", self.home())
            .env("LOCALAPPDATA", self.0.path().join("local"))
            .cwd(self.work())
    }

    /// `c2 exec --headless -- <command>` outside the test's job.
    fn exec(&self, install: &Install, command: &[&str]) -> Finished {
        self.exec_in(install, command, &[])
    }

    fn exec_in(&self, install: &Install, command: &[&str], jobs: &[JobShape]) -> Finished {
        let mut outside = self
            .outside(install)
            .args(["exec", "--headless", "--"])
            .args(command);
        for job in jobs {
            outside = outside.job(*job);
        }
        outside.run(Duration::from_secs(30)).unwrap()
    }

    fn list(&self, install: &Install) -> String {
        let output = Command::new(install.cli())
            .arg("list")
            .env("CLIMON2_HOME", self.home())
            .env("LOCALAPPDATA", self.0.path().join("local"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    fn sessions(&self) -> Vec<SessionMetadata> {
        let Ok(entries) = fs::read_dir(self.home().join("sessions")) else {
            return Vec::new();
        };
        entries
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "pty"))
            .filter_map(|path| SessionMetadata::from_json(&path, &fs::read(&path).ok()?).ok())
            .collect()
    }

    fn daemon_log(&self, id: &str) -> String {
        fs::read_to_string(self.home().join("logs").join(id).join("daemon.log")).unwrap()
    }

    /// Every log of session `id`, for a failure message.
    fn session_logs(&self, id: &str) -> String {
        let Ok(entries) = fs::read_dir(self.home().join("logs").join(id)) else {
            return String::new();
        };
        entries
            .map(|entry| entry.unwrap().path())
            .map(|path| {
                format!(
                    "{}:\n{}",
                    path.display(),
                    fs::read_to_string(&path).unwrap()
                )
            })
            .collect()
    }

    /// Waits until `list` no longer shows a session.
    fn wait_until_ended(&self, install: &Install) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.list(install).contains("No climon2 sessions running.") {
            assert!(Instant::now() < deadline, "{}", self.list(install));
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// `c2 kill <selector>`, which starts no daemon, so it runs in the
    /// test's own job.
    fn kill(&self, install: &Install, selector: &str) -> std::process::Output {
        Command::new(install.cli())
            .args(["kill", selector])
            .env("CLIMON2_HOME", self.home())
            .env("LOCALAPPDATA", self.0.path().join("local"))
            .output()
            .unwrap()
    }

    /// The process whose PID the session's command wrote to `name` in the
    /// working directory, once it has.
    fn written_process(&self, name: &str) -> Held {
        let path = self.work().join(name);
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let written = fs::read_to_string(&path).unwrap_or_default();
            if let Ok(pid) = written.trim().parse() {
                return Held::open(pid);
            }
            assert!(
                Instant::now() < deadline,
                "{} was never written",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// A handle on a process a session started, which outlives the process so
/// its exit code and exit time can be read. Dropping it ends the process if
/// it still runs, so a failed test leaves nothing running.
struct Held(OwnedHandle);

impl Held {
    fn open(pid: u32) -> Self {
        let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE;
        Self(open(pid, access).expect("open the session's process"))
    }

    /// The exit code, once the process ends within `timeout`.
    fn exit_code(&self, timeout: Duration) -> Option<u32> {
        let milliseconds = u32::try_from(timeout.as_millis()).unwrap();
        // The handle is open with SYNCHRONIZE.
        if unsafe { WaitForSingleObject(self.0.as_raw_handle(), milliseconds) } != WAIT_OBJECT_0 {
            return None;
        }
        let mut code = 0;
        // The handle is open with query access; the call writes one DWORD.
        assert_ne!(
            unsafe { GetExitCodeProcess(self.0.as_raw_handle(), &mut code) },
            0
        );
        Some(code)
    }

    /// When the process ended; it must have.
    fn exit_time(&self) -> SystemTime {
        let mut times = [FILETIME::default(); 4];
        let [creation, exit, kernel, user] = &mut times;
        // The handle is open with query access; the call writes four FILETIMEs.
        assert_ne!(
            unsafe { GetProcessTimes(self.0.as_raw_handle(), creation, exit, kernel, user) },
            0
        );
        // FILETIME counts 100 ns intervals since 1601.
        let intervals = (u64::from(exit.dwHighDateTime) << 32) | u64::from(exit.dwLowDateTime);
        SystemTime::UNIX_EPOCH + Duration::from_nanos((intervals - 116_444_736_000_000_000) * 100)
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        // The handle is open with PROCESS_TERMINATE; ending an ended process
        // only fails.
        unsafe { TerminateProcess(self.0.as_raw_handle(), 1) };
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        for session in self.sessions() {
            if process::is_same_daemon(&session).unwrap_or(false) {
                terminate(session.daemon_pid);
            }
        }
    }
}

fn open(pid: u32, access: u32) -> Option<OwnedHandle> {
    // Opening by PID has no preconditions.
    let raw = unsafe { OpenProcess(access, 0, pid) };
    // A new handle nothing else owns.
    (!raw.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(raw) })
}

fn terminate(pid: u32) {
    if let Some(process) = open(pid, PROCESS_TERMINATE) {
        // The handle is open with PROCESS_TERMINATE.
        unsafe { TerminateProcess(process.as_raw_handle(), 1) };
    }
}

fn in_any_job(pid: u32) -> bool {
    let process = open(pid, PROCESS_QUERY_LIMITED_INFORMATION).expect("open the daemon");
    let mut in_job = 1;
    // The handle is open; the call writes one BOOL.
    assert_ne!(
        unsafe { IsProcessInJob(process.as_raw_handle(), std::ptr::null_mut(), &mut in_job) },
        0
    );
    in_job != 0
}

/// A command that runs until `release` exists in its working directory, then
/// exits with `0xC000013A`.
const HELD: &[&str] = &[
    "powershell",
    "-NoLogo",
    "-NoProfile",
    "-Command",
    "while (-not (Test-Path release)) { Start-Sleep -Milliseconds 50 }; exit -1073741510",
];

fn session_id(finished: &Finished) -> String {
    assert_eq!(finished.status, 0, "{finished:?}");
    let id = finished.stdout.trim_end().to_owned();
    assert_eq!(finished.stdout, format!("{id}\n"), "{finished:?}");
    id
}

#[test]
fn a_headless_session_runs_detached_until_its_command_exits_with_its_full_status() {
    let (install, home) = (Install::new(&[]), Home::new());
    let id = session_id(&home.exec(&install, HELD));
    let listed = home.list(&install);
    assert!(
        listed.contains(&id) && listed.contains("detached"),
        "{listed}"
    );
    let [session] = home.sessions().try_into().unwrap();
    assert!(!in_any_job(session.daemon_pid));
    fs::write(home.work().join("release"), b"").unwrap();
    home.wait_until_ended(&install);
    let log = home.daemon_log(&id);
    assert!(
        log.contains(&format!("command exited status=\"{CONTROL_C_EXIT}\"")),
        "{log}"
    );
    assert!(home.sessions().is_empty());
}

#[test]
fn the_command_sees_the_terminal_identity_and_its_session_at_the_fallback_size() {
    let (install, home) = (Install::new(&[]), Home::new());
    let command = [
        "cmd",
        "/d",
        "/c",
        "(echo [%TERM%][%COLORTERM%][%CLIMON2_SESSION_ID%]& mode con)> seen.txt",
    ];
    let finished = home
        .outside(&install)
        .env("TERM", "dumb")
        .env("COLORTERM", "none")
        .args(["exec", "--headless", "--"])
        .args(command)
        .run(Duration::from_secs(30))
        .unwrap();
    let id = session_id(&finished);
    home.wait_until_ended(&install);
    let seen = fs::read_to_string(home.work().join("seen.txt")).unwrap();
    assert!(
        seen.contains(&format!("[xterm-256color][truecolor][{id}]")),
        "{seen}"
    );
    let numbers: Vec<String> = seen
        .lines()
        .filter(|line| line.contains("Lines:") || line.contains("Columns:"))
        .map(|line| line.split_whitespace().last().unwrap().to_owned())
        .collect();
    assert_eq!(numbers, ["24", "80"], "{seen}");
}

#[test]
fn exec_exits_125_when_a_bundled_conpty_file_is_missing() {
    for missing in ["conpty.dll", "OpenConsole.exe"] {
        let (install, home) = (Install::new(&[missing]), Home::new());
        let finished = home.exec(&install, HELD);
        assert_eq!(finished.status, 125, "{missing}: {finished:?}");
        assert!(finished.stdout.is_empty(), "{missing}: {finished:?}");
        assert!(
            finished.stderr.contains("bundled ConPTY"),
            "{missing}: {finished:?}"
        );
        assert!(home.sessions().is_empty(), "{missing}");
        assert!(install.daemon_stopped(), "{missing}");
    }
}

#[test]
fn a_command_that_cannot_start_is_status_126() {
    let (install, home) = (Install::new(&[]), Home::new());
    let finished = home.exec(&install, &["climon2-no-such-program"]);
    assert_eq!(finished.status, 126, "{finished:?}");
    assert!(
        finished.stderr.contains("command cannot be executed"),
        "{finished:?}"
    );
    assert!(home.sessions().is_empty());
}

#[test]
fn a_cli_in_a_job_that_allows_breakaway_starts_its_daemon_outside_every_job() {
    let (install, home) = (Install::new(&[]), Home::new());
    let id = session_id(&home.exec_in(&install, HELD, &[JobShape::AllowsBreakaway]));
    let [session] = home.sessions().try_into().unwrap();
    assert_eq!(session.session_id.as_str(), id);
    assert!(!in_any_job(session.daemon_pid));
    fs::write(home.work().join("release"), b"").unwrap();
    home.wait_until_ended(&install);
}

#[test]
fn a_daemon_that_cannot_leave_the_clis_job_is_ended_and_exec_exits_125() {
    let shapes: [&[JobShape]; 2] = [
        // Breakaway is refused outright.
        &[JobShape::RefusesBreakaway],
        // Breakaway leaves only the inner job; the outer one still holds it.
        &[JobShape::RefusesBreakaway, JobShape::AllowsBreakaway],
    ];
    for jobs in shapes {
        let (install, home) = (Install::new(&[]), Home::new());
        let finished = home.exec_in(&install, HELD, jobs);
        assert_eq!(finished.status, 125, "{jobs:?}: {finished:?}");
        assert!(
            finished.stderr.contains("cannot leave the job"),
            "{jobs:?}: {finished:?}"
        );
        assert!(home.sessions().is_empty(), "{jobs:?}");
        assert!(install.daemon_stopped(), "{jobs:?}");
    }
}

/// A command whose child, `ping`, runs with a hidden console of its own, so
/// closing the session's pseudoconsole leaves it running in the session job;
/// the command writes the child's PID to `child` and waits.
const WITH_CHILD: &[&str] = &[
    "powershell",
    "-NoLogo",
    "-NoProfile",
    "-Command",
    "$start = New-Object System.Diagnostics.ProcessStartInfo 'ping.exe', '-n 300 127.0.0.1'; \
     $start.UseShellExecute = $false; $start.CreateNoWindow = $true; \
     $child = [System.Diagnostics.Process]::Start($start); \
     Set-Content child $child.Id; while ($true) { Start-Sleep 1 }",
];

#[test]
fn kill_hangs_up_waits_the_full_grace_ends_the_job_and_exits_0_once_cleaned_up() {
    let (install, home) = (Install::new(&[]), Home::new());
    let id = session_id(&home.exec(&install, WITH_CHILD));
    let child = home.written_process("child");
    let [session] = home.sessions().try_into().unwrap();

    let started = SystemTime::now();
    let killed = home.kill(&install, &id[..8]);

    assert_eq!(
        killed.status.code(),
        Some(0),
        "{killed:?}\n{}",
        home.session_logs(&id)
    );
    assert!(
        killed.stdout.is_empty() && killed.stderr.is_empty(),
        "{killed:?}"
    );
    assert!(home.sessions().is_empty(), "kill confirmed before cleanup");
    assert!(!std::path::Path::new(session.socket_path.as_str()).exists());
    // The pseudoconsole's close spares the child; the job's end does not.
    assert_eq!(child.exit_code(Duration::ZERO), Some(0xC000_013A));
    let grace = child.exit_time().duration_since(started).unwrap();
    assert!(
        grace >= Duration::from_secs(5),
        "the job ended after {grace:?}"
    );
    assert!(home.list(&install).contains("No climon2 sessions running."));
}

/// Until the client has its console rules (#321), the session's own client,
/// started directly, refuses to attach before any connection.
#[test]
fn the_client_refuses_to_attach_on_windows_yet() {
    let (install, home) = (Install::new(&[]), Home::new());
    let id = session_id(&home.exec(&install, HELD));
    let metadata = home.home().join("sessions").join(format!("{id}.pty"));
    let attached = Command::new(install.version().join("climon2-client.exe"))
        .arg("attach")
        .arg("--metadata")
        .arg(&metadata)
        .env("CLIMON2_HOME", home.home())
        .env_remove("CLIMON2_CLIENT_STARTUP")
        .output()
        .unwrap();
    assert_eq!(attached.status.code(), Some(125), "{attached:?}");
    let stderr = String::from_utf8_lossy(&attached.stderr);
    assert!(
        stderr.starts_with("climon2: error: attach is not supported on Windows yet"),
        "{stderr}"
    );
    assert!(
        home.list(&install).contains(&id),
        "attach ended the session"
    );
    fs::write(home.work().join("release"), b"").unwrap();
    home.wait_until_ended(&install);
}
