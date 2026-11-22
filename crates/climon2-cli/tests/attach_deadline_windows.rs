//! #323: on Windows an ordinary session that nobody attaches to within five
//! seconds of its start ends, command and all (docs/for-humans/windows/lifecycle.md,
//! "Session job and hang-up"). `exec` is held before it starts the client by
//! the `CLIMON2_CLI_HOLD` test hook.
//!
//! This scenario keeps a CLI, a daemon and two pseudoconsoles busy for about
//! ten seconds, so it has a test binary of its own: cargo runs test binaries
//! one at a time, and beside `exec_windows.rs`'s kill tests, whose deadline is
//! ten seconds, it made them miss it under the coverage build.
//!
//! cargo runs tests in a job that refuses breakaway, and so does the CI
//! runner, so the test runs again outside every job.
#![cfg(windows)]
use climon2_core::metadata::SessionMetadata;
use climon2_platform::process;
use climon2_test_term::{ConPty, Terminal, rerun_outside};
use portable_pty::CommandBuilder;
use std::{
    fs,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};
use windows_sys::Win32::{
    Foundation::{FILETIME, WAIT_OBJECT_0},
    System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
    },
};

climon2_test_term::outside_relay!();

const TIMEOUT: Duration = Duration::from_secs(30);
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A command that writes its PID to `pid` in its working directory, then
/// runs until it is ended.
const WRITES_ITS_PID: &[&str] = &[
    "powershell",
    "-NoLogo",
    "-NoProfile",
    "-Command",
    "Set-Content pid $PID; while ($true) { Start-Sleep 1 }",
];

/// A version folder of this build's CLI (with its test hooks), daemon,
/// client and library and the pinned ConPTY, a home and a working directory,
/// all outside the crate folder. Dropping it ends every daemon its sessions
/// name.
struct Install(tempfile::TempDir);

impl Install {
    fn new() -> Self {
        let install = Self(tempfile::tempdir().unwrap());
        fs::create_dir_all(install.version()).unwrap();
        fs::create_dir_all(install.work()).unwrap();
        let cli = PathBuf::from(env!("CARGO_BIN_EXE_climon2-cli"));
        let conpty = ConPty::pinned().unwrap().folder();
        // `cargo test` leaves the cdylib in deps.
        for from in [
            cli.clone(),
            cli.with_file_name("climon2-daemon.exe"),
            cli.with_file_name("climon2-client.exe"),
            cli.with_file_name("deps").join("climon2_client.dll"),
            conpty.join("conpty.dll"),
            conpty.join("OpenConsole.exe"),
        ] {
            assert!(
                from.is_file(),
                "missing {}; build the daemon and client first",
                from.display()
            );
            fs::copy(&from, install.version().join(from.file_name().unwrap())).unwrap();
        }
        install
    }

    fn version(&self) -> PathBuf {
        self.0.path().join("exe").join(VERSION)
    }

    fn home(&self) -> PathBuf {
        self.0.path().join("home")
    }

    fn work(&self) -> PathBuf {
        self.0.path().join("work")
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
}

impl Drop for Install {
    fn drop(&mut self) {
        for session in self.sessions() {
            if process::is_same_daemon(&session).unwrap_or(false) {
                drop(Held::open(session.daemon_pid));
            }
        }
    }
}

/// A process handle that outlives the process, so its exit can be read.
/// Dropping it ends the process if it still runs.
struct Held(OwnedHandle);

impl Held {
    fn open(pid: u32) -> Self {
        let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE;
        // Opening by PID has no preconditions.
        let raw = unsafe { OpenProcess(access, 0, pid) };
        assert!(!raw.is_null(), "cannot open process {pid}");
        // A new handle nothing else owns.
        Self(unsafe { OwnedHandle::from_raw_handle(raw) })
    }

    /// The process `path` names once it is written.
    fn written(path: &Path) -> Self {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Ok(pid) = fs::read_to_string(path).unwrap_or_default().trim().parse() {
                return Self::open(pid);
            }
            assert!(
                Instant::now() < deadline,
                "{} never written",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn ended_within(&self, timeout: Duration) -> bool {
        let milliseconds = u32::try_from(timeout.as_millis()).unwrap();
        // The handle is open with SYNCHRONIZE.
        unsafe { WaitForSingleObject(self.0.as_raw_handle(), milliseconds) == WAIT_OBJECT_0 }
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

#[test]
fn an_ordinary_session_nobody_attaches_to_within_five_seconds_ends() {
    if rerun_outside(
        "an_ordinary_session_nobody_attaches_to_within_five_seconds_ends",
        Duration::from_secs(120),
    ) {
        return;
    }
    let install = Install::new();
    let hold = install.work().join("release-client");
    let mut command = CommandBuilder::new(install.version().join("climon2-cli.exe"));
    command.args(["exec", "--"]);
    command.args(WRITES_ITS_PID);
    command.env("CLIMON2_HOME", install.home());
    command.env("LOCALAPPDATA", install.0.path().join("local"));
    command.env("CLIMON2_CLI_HOLD", &hold);
    command.cwd(install.work());
    let mut execing = Terminal::spawn(command, 24, 100).unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while !hold.with_extension("held").exists() {
        assert!(Instant::now() < deadline, "{}", execing.screen());
        std::thread::sleep(Duration::from_millis(10));
    }
    let [session] = install.sessions().try_into().unwrap();
    let command = Held::written(&install.work().join("pid"));

    assert!(command.ended_within(TIMEOUT), "the command still runs");
    let deadline = Instant::now() + TIMEOUT;
    while !install.sessions().is_empty() || process::is_same_daemon(&session).unwrap() {
        assert!(Instant::now() < deadline, "the session did not end");
        std::thread::sleep(Duration::from_millis(50));
    }
    let started = SystemTime::UNIX_EPOCH + Duration::from_micros(session.daemon_start_time);
    let lived = command.exit_time().duration_since(started).unwrap();
    assert!(lived >= Duration::from_secs(5), "ended after {lived:?}");

    fs::write(&hold, b"").unwrap();
    let status = execing.wait_exit(TIMEOUT).unwrap();
    assert_eq!(status, 125, "{}", execing.screen());
}
