/// Only the Library-layer suites use it.
#[allow(dead_code)]
pub mod library;
/// Only the passive-connection suites use it (#93).
#[allow(dead_code)]
pub mod passive;

use climon2_install::{Payload, install};
use climon2_test_term::{Isolation, Modes, Terminal, bin_path, process_command};
use portable_pty::CommandBuilder;
use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex, Weak},
    thread,
    time::{Duration, Instant},
};

// A failed handoff can spend both the five-second startup and leave budgets.
pub const TIMEOUT: Duration = Duration::from_secs(20);
/// The escape prefix, Ctrl+], then `d`.
#[allow(dead_code)]
pub const DETACH: &[u8] = b"\x1dd";
/// The client library's file name in every version folder (library embedding).
#[cfg(target_os = "macos")]
pub const LIBRARY: &str = "libclimon2_client.dylib";
#[cfg(target_os = "linux")]
pub const LIBRARY: &str = "libclimon2_client.so";
const VERSION: &str = env!("CARGO_PKG_VERSION");
static INSTALLED: Mutex<Weak<Installed>> = Mutex::new(Weak::new());

struct Installed {
    directory: tempfile::TempDir,
    archive: bool,
}

impl Installed {
    fn new() -> Self {
        let installed = Self::empty();
        if let Some(archive) = env::var_os("CLIMON2_E2E_ARCHIVE") {
            let (payload, version) = extract_payload(&archive, installed.directory.path());
            assert_eq!(version, VERSION, "archive and workspace versions differ");
            run_installer(&payload, &installed.home());
        } else {
            let binary = bin_path("climon2");
            let payload = Payload::from_siblings(binary.parent().unwrap(), VERSION).unwrap();
            install(&payload, &installed.home()).unwrap();
        }
        assert_eq!(
            fs::read_to_string(installed.home().join("current")).unwrap(),
            format!("{VERSION}\n")
        );
        installed
    }

    fn empty() -> Self {
        Self {
            directory: tempfile::Builder::new()
                .prefix("packaged-")
                .tempdir_in(env!("CARGO_MANIFEST_DIR"))
                .unwrap(),
            archive: env::var_os("CLIMON2_E2E_ARCHIVE").is_some(),
        }
    }

    fn shared() -> Arc<Self> {
        let mut installed = INSTALLED.lock().unwrap();
        if let Some(installed) = installed.upgrade() {
            return installed;
        }
        let new = Arc::new(Self::new());
        *installed = Arc::downgrade(&new);
        new
    }

    fn home(&self) -> PathBuf {
        self.directory.path().join("executable home")
    }
}

pub struct Package {
    pub isolation: Isolation,
    installed: Arc<Installed>,
}

impl Package {
    pub fn new() -> Self {
        Self {
            isolation: Isolation::new(),
            installed: Installed::shared(),
        }
    }

    pub fn previous_archive() -> Option<Self> {
        let previous = env::var_os("CLIMON2_E2E_PREVIOUS_ARCHIVE")?;
        assert!(
            env::var_os("CLIMON2_E2E_ARCHIVE").is_some(),
            "previous-release compatibility requires the exact current release archive too"
        );
        let installed = Installed::empty();
        let (payload, version) = extract_payload(&previous, installed.directory.path());
        assert_ne!(
            version, VERSION,
            "previous archive must be a different release"
        );
        run_installer(&payload, &installed.home());
        Some(Self {
            isolation: Isolation::new(),
            installed: Arc::new(installed),
        })
    }

    pub fn install_current_release(&self) {
        let archive = env::var_os("CLIMON2_E2E_ARCHIVE").unwrap();
        let (payload, version) = extract_payload(&archive, self.installed.directory.path());
        assert_eq!(version, VERSION);
        run_installer(&payload, &self.installed.home());
    }

    pub fn private() -> Self {
        Self {
            isolation: Isolation::new(),
            installed: Arc::new(Installed::new()),
        }
    }

    pub fn executable_home(&self) -> PathBuf {
        self.installed.home()
    }

    pub fn command(&self, executable: impl AsRef<std::ffi::OsStr>) -> CommandBuilder {
        let mut command = self.isolation.command(executable);
        command.env(
            "PATH",
            format!(
                "{}:{}:/usr/bin:/bin",
                self.installed.home().display(),
                bin_path("climon2-fixture").parent().unwrap().display()
            ),
        );
        command
    }

    pub fn metadata(&self) -> climon2_core::metadata::SessionMetadata {
        let paths: Vec<_> = fs::read_dir(self.isolation.climon2_home().join("sessions"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "pty"))
            .collect();
        assert_eq!(paths.len(), 1);
        climon2_core::metadata::SessionMetadata::from_json(&paths[0], &fs::read(&paths[0]).unwrap())
            .unwrap()
    }

    pub fn client_pid(&self) -> i32 {
        self.pids()
            .into_iter()
            .find(|pid| {
                let output = Command::new("/bin/ps")
                    .args(["-p", &pid.to_string(), "-o", "comm="])
                    .output()
                    .unwrap();
                Path::new(String::from_utf8_lossy(&output.stdout).trim())
                    .file_name()
                    .is_some_and(|name| name == "climon2-client")
            })
            .expect("installed client owns the outer PTY")
    }

    pub fn bash(&self, bash: &Path) -> CommandBuilder {
        let mut command = self.command(bash);
        command.args(["--noprofile", "--norc", "-i"]);
        command
    }

    pub fn list(&self) -> Vec<Value> {
        self.list_with_program("climon2")
    }

    pub fn list_with_version(&self, version: &str) -> Vec<Value> {
        self.list_with_program(
            self.installed
                .home()
                .join(version)
                .join("climon2-cli")
                .to_str()
                .unwrap(),
        )
    }

    fn list_with_program(&self, program: &str) -> Vec<Value> {
        let report = self.isolation.home().join("list.json");
        let mut command = self.command("/bin/bash");
        command.args(["--noprofile", "--norc", "-c"]);
        command.arg(format!(
            "{} list && {} list --json > {}",
            quote(Path::new(program)),
            quote(Path::new(program)),
            quote(&report)
        ));
        let mut terminal = Terminal::spawn(command, 40, 240).unwrap();
        assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
        let sessions: Vec<Value> = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
        if let Some(session) = sessions.first() {
            assert!(
                terminal
                    .screen()
                    .contains(session["session_id"].as_str().unwrap()),
                "table must show the live session's full GUID"
            );
        } else {
            assert!(
                terminal.screen().contains("No climon2 sessions running."),
                "{}",
                terminal.screen()
            );
        }
        sessions
    }

    pub fn assert_clean(&self) {
        let deadline = Instant::now() + TIMEOUT;
        while !self.pids().is_empty() {
            assert!(Instant::now() < deadline, "owned processes survived");
            thread::sleep(Duration::from_millis(10));
        }
        self.isolation.assert_clean();
    }

    pub fn assert_archive_default_levels(&self) {
        if self.installed.archive {
            for path in self.logs() {
                let log = fs::read_to_string(path).unwrap();
                for line in log.lines() {
                    assert!(
                        !line
                            .split_whitespace()
                            .any(|word| matches!(word, "TRACE" | "DEBUG")),
                        "archive default must be INFO or above: {line}"
                    );
                }
            }
        }
    }

    pub fn logs(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        collect_logs(&self.isolation.climon2_home().join("logs"), &mut files);
        files
    }

    fn pids(&self) -> Vec<i32> {
        self.isolation.owned_pids()
    }
}

impl Drop for Package {
    fn drop(&mut self) {
        if !thread::panicking() {
            return;
        }
        for pid in self.pids() {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
        let deadline = Instant::now() + Duration::from_secs(6);
        while !self.pids().is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        for pid in self.pids() {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        let mut command = self.command("climon2");
        command.arg("list");
        if let Ok(mut terminal) = Terminal::spawn(command, 30, 120) {
            let _ = terminal.wait_exit(TIMEOUT);
        }
    }
}

fn collect_logs(directory: &Path, files: &mut Vec<PathBuf>) {
    if !directory.exists() {
        return;
    }
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_logs(&path, files);
        } else {
            files.push(path);
        }
    }
}

pub fn quote(path: &Path) -> String {
    format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"))
}

/// An outer non-interactive `bash` on a `rows` x `cols` terminal that records
/// termios around `command`, reports its status as `OUTER-DONE=<status>`,
/// then echoes one ordinary typed line as `TYPED=<line>`. The continuity and
/// monitor scenarios run climon2 from it; packaged.rs does not.
#[allow(dead_code)]
pub fn outer(package: &Package, bash: &Path, command: &str, rows: u16, cols: u16) -> Terminal {
    let mut shell = package.command(bash);
    shell.args(["--noprofile", "--norc", "-c"]);
    shell.arg(format!(
        "stty -g > ~/before; {command}; s=$?; stty -g > ~/after; \
         printf 'OUTER-DONE=%s\\n' \"$s\"; IFS= read -r line; printf 'TYPED=%s\\n' \"$line\""
    ));
    Terminal::spawn(shell, rows, cols).unwrap()
}

/// After the [`outer`] shell's `command` returned `status`: the documented
/// normal-shell baseline, exact termios and ordinary shell input.
#[allow(dead_code)]
pub fn assert_released(package: &Package, terminal: &mut Terminal, status: i32) {
    terminal
        .wait_for_text(&format!("OUTER-DONE={status}"), TIMEOUT)
        .unwrap();
    assert_eq!(terminal.modes(), Modes::BASELINE, "{}", terminal.screen());
    let home = package.isolation.home();
    assert_eq!(
        termios(&home.join("after")),
        termios(&home.join("before")),
        "termios restored"
    );
    terminal.send("plain words\n").unwrap();
    terminal
        .wait_for_text("TYPED=plain words", TIMEOUT)
        .unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
}

/// `stty -g` settings without macOS's transient `PENDIN`, which the kernel
/// sets whenever a terminal returns to canonical mode.
#[allow(dead_code)]
fn termios(path: &Path) -> String {
    const PENDIN: u64 = 0x2000_0000;
    fs::read_to_string(path)
        .unwrap()
        .trim_end()
        .split(':')
        .map(|field| match field.strip_prefix("lflag=") {
            Some(flags) => {
                let flags = u64::from_str_radix(flags, 16).unwrap() & !PENDIN;
                format!("lflag={flags:x}")
            }
            None => field.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(":")
}

/// `<launcher> <args>` as a script runs it: stdin from /dev/null, stdout and
/// stderr into pipes, so none of them is a terminal.
#[allow(dead_code)]
pub fn scripted(package: &Package, launcher: &str, args: &[&str]) -> Output {
    scripted_to(package, launcher, args, Stdio::piped())
}

/// Like [`scripted`], with stdout going to `stdout`. A CLI still running at
/// the deadline is killed and reaped, and the test fails.
#[allow(dead_code)]
pub fn scripted_to(package: &Package, launcher: &str, args: &[&str], stdout: Stdio) -> Output {
    let isolated = package.command(launcher);
    let mut cli = process_command(package.executable_home().join(launcher))
        .args(args)
        .env_clear()
        .envs(isolated.iter_full_env_as_str())
        .current_dir(package.isolation.home())
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while cli.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            cli.kill().unwrap();
            cli.wait().unwrap();
            panic!("{launcher} {args:?} was still running after {TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
    cli.wait_with_output().unwrap()
}

/// The contents of `path` once it exists.
#[allow(dead_code)]
pub fn wait_file(path: &Path) -> String {
    let deadline = Instant::now() + TIMEOUT;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
    fs::read_to_string(path).unwrap()
}

/// Types the harmless `command` until `output` appears, waiting for input
/// readiness before each try. A client discards keys typed before it confirmed
/// its restoration and nothing on the screen marks that, so only the session's
/// answer to typed input proves the client takes input; `command` must be
/// harmless to repeat, and `output` must differ from its echo.
pub fn type_until(terminal: &mut Terminal, command: &str, output: &str) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        terminal.wait_for_input_ready(TIMEOUT).unwrap();
        terminal.send(command).unwrap();
        if terminal
            .wait_for_text(output, Duration::from_secs(1))
            .is_ok()
        {
            return;
        }
        assert!(Instant::now() < deadline, "{output:?} never appeared");
    }
}

/// Explicit detach: [`DETACH`] sent exactly once, then status 0 and the
/// reattach hint on stderr. The client must already have confirmed its
/// restoration, or it discards the keys: a just-attached client proves it
/// with [`type_until`] first.
#[allow(dead_code)]
pub fn detach(terminal: &mut Terminal, id: &str) {
    terminal.wait_for_input_ready(TIMEOUT).unwrap();
    terminal.send(DETACH).unwrap();
    assert_eq!(
        terminal.wait_exit(TIMEOUT).unwrap(),
        0,
        "{}",
        terminal.screen()
    );
    terminal
        .wait_for_bytes(
            format!("climon2: detached from session {id}; reattach with: climon2 attach {id}")
                .as_bytes(),
            TIMEOUT,
        )
        .unwrap();
}

/// `climon2 kill <id>` from a script: silent and status 0, which it returns
/// only once the session has ended and is cleaned up.
#[allow(dead_code)]
pub fn kill_session(package: &Package, id: &str) {
    let output = scripted(package, "climon2", &["kill", id]);
    assert_eq!(
        (
            output.status.code(),
            output.stdout.as_slice(),
            output.stderr.as_slice()
        ),
        (Some(0), &b""[..], &b""[..])
    );
}

fn extract_payload(archive: &std::ffi::OsStr, directory: &Path) -> (PathBuf, String) {
    let archive = PathBuf::from(archive).canonicalize().unwrap();
    assert!(
        archive.to_string_lossy().ends_with(".tar.gz"),
        "CLIMON2_E2E_ARCHIVE must name the exact .tar.gz release archive"
    );
    let listing = Command::new("/usr/bin/tar")
        .args(["-tzf"])
        .arg(&archive)
        .output()
        .unwrap();
    assert!(listing.status.success(), "{listing:?}");
    let listing = String::from_utf8(listing.stdout).unwrap();
    let root = listing.lines().next().unwrap().split('/').next().unwrap();
    let suffix = if cfg!(target_os = "linux") {
        "-linux-x86_64-gnu"
    } else {
        "-macos-universal"
    };
    let version = root
        .strip_prefix("climon2-")
        .and_then(|root| root.strip_suffix(suffix))
        .expect("release archive's versioned root");
    version.parse::<climon2_core::VersionName>().unwrap();
    let expected = [
        format!("{root}/install"),
        format!("{root}/climon2"),
        format!("{root}/{version}/climon2-cli"),
        format!("{root}/{version}/climon2-daemon"),
        format!("{root}/{version}/climon2-client"),
        format!("{root}/{version}/climon2-monitor"),
        format!("{root}/{version}/{LIBRARY}"),
        format!("{root}/LICENSE"),
        format!("{root}/README.md"),
    ];
    let mut files: Vec<_> = listing
        .lines()
        .filter(|entry| !entry.ends_with('/'))
        .collect();
    for entry in listing.lines().filter(|entry| entry.ends_with('/')) {
        assert!(
            entry == format!("{root}/") || entry == format!("{root}/{version}/"),
            "unexpected archive directory: {entry}"
        );
    }
    files.sort_unstable();
    let mut expected: Vec<_> = expected.iter().map(String::as_str).collect();
    expected.sort_unstable();
    assert_eq!(
        files, expected,
        "release archive contains exactly the payload"
    );
    let extracted = Command::new("/usr/bin/tar")
        .args(["-xzf"])
        .arg(&archive)
        .arg("-C")
        .arg(directory)
        .output()
        .unwrap();
    assert!(extracted.status.success(), "{extracted:?}");
    (directory.join(root), version.to_owned())
}

fn run_installer(payload: &Path, destination: &Path) {
    let mut installer = Command::new(payload.join("install"));
    installer
        .env_clear()
        .env("HOME", payload)
        .env("PATH", "/usr/bin:/bin")
        .arg("--dir")
        .arg(destination);
    if let Some(profile) = env::var_os("LLVM_PROFILE_FILE") {
        installer.env("LLVM_PROFILE_FILE", profile);
    }
    let output = installer.output().unwrap();
    assert!(output.status.success(), "{output:?}");
}
