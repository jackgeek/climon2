//! What the Windows end-to-end tests share (#321, #327): the installed
//! product, a home of its own, and the `pwsh` shell profile they drive it
//! from.
//!
//! The product comes from the release zip named by `CLIMON2_E2E_ARCHIVE`,
//! as the Windows Test job builds it, or else from this build's binaries in
//! the same layout. It writes the `current` file the launcher reads itself;
//! installing with `install.ps1` is the Windows Archive job's (#329).
use climon2_core::metadata::SessionMetadata;
use climon2_platform::process;
use climon2_test_term::{ConPty, Terminal, end_process, local_app_data};
use portable_pty::CommandBuilder;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

/// Every step waits this long at most; a failed hand-off can spend the
/// five-second start-up and leave budgets.
pub const TIMEOUT: Duration = Duration::from_secs(30);
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The installed product: `c2.exe`, `current` and the version folder, from
/// the release zip or from this build.
pub struct Package {
    _root: tempfile::TempDir,
    pub folder: PathBuf,
}

impl Package {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let folder = match env::var_os("CLIMON2_E2E_ARCHIVE") {
            Some(archive) => extracted(Path::new(&archive), root.path()),
            None => built(root.path()),
        };
        assert!(folder.join(VERSION).join("climon2-cli.exe").is_file());
        fs::write(folder.join("current"), format!("{VERSION}\n")).unwrap();
        Self {
            _root: root,
            folder,
        }
    }

    pub fn launcher(&self) -> PathBuf {
        self.folder.join("c2.exe")
    }

    /// The version folder.
    pub fn version(&self) -> PathBuf {
        self.folder.join(VERSION)
    }
}

/// The zip's one folder, unpacked into `root` with Windows' own `tar`.
fn extracted(archive: &Path, root: &Path) -> PathBuf {
    assert!(
        archive.is_absolute(),
        "CLIMON2_E2E_ARCHIVE must be absolute: {}",
        archive.display()
    );
    let tar = Path::new(&env::var_os("SystemRoot").unwrap()).join(r"System32\tar.exe");
    let status = Command::new(tar)
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(root)
        .status()
        .unwrap();
    assert!(status.success(), "cannot unpack {}", archive.display());
    let folder = root.join(format!("climon2-{VERSION}-windows-x86_64-msvc"));
    assert!(
        folder.is_dir(),
        "{} holds no {}",
        archive.display(),
        folder.display()
    );
    folder
}

/// The release layout from this build's binaries and the pinned ConPTY.
fn built(root: &Path) -> PathBuf {
    let cli = PathBuf::from(env!("CARGO_BIN_EXE_climon2-cli"));
    let version = root.join(VERSION);
    fs::create_dir_all(&version).unwrap();
    let launcher = cli.with_file_name("c2.exe");
    let conpty = ConPty::pinned().unwrap().folder();
    // `cargo test` leaves the client library in deps.
    let shipped = [
        cli.clone(),
        cli.with_file_name("climon2-daemon.exe"),
        cli.with_file_name("climon2-client.exe"),
        cli.with_file_name("climon2-monitor.exe"),
        cli.with_file_name("deps").join("climon2_client.dll"),
        conpty.join("conpty.dll"),
        conpty.join("OpenConsole.exe"),
    ];
    for file in [&launcher].into_iter().chain(&shipped) {
        assert!(
            file.is_file(),
            "missing {}; build the launcher, daemon, client and monitor first \
             (cargo test -p climon2-launcher -p climon2-daemon -p climon2-client \
             -p climon2-monitor)",
            file.display()
        );
    }
    place(&launcher, &root.join("c2.exe"));
    for file in shipped {
        place(&file, &version.join(file.file_name().unwrap()));
    }
    root.to_owned()
}

/// Puts a built file into a test install as a hard link, so Windows Defender
/// scans it once, not on each install's first launch, which under the coverage
/// build pushed daemon start-up past its five-second deadline. A copy where a
/// link cannot be made, such as across volumes.
fn place(from: &Path, to: &Path) {
    if fs::hard_link(from, to).is_err() {
        fs::copy(from, to).unwrap();
    }
}

/// A climon2 home of its own, and a runtime root short enough for a socket
/// path. Dropping it ends every daemon its sessions name, so a failed test
/// leaves nothing running.
pub struct Home(pub tempfile::TempDir, climon2_test_term::LocalAppData);

impl Home {
    pub fn new() -> Self {
        Self(tempfile::tempdir().unwrap(), local_app_data())
    }

    pub fn home(&self) -> PathBuf {
        self.0.path().join("home")
    }

    pub fn local(&self) -> PathBuf {
        self.1.path().to_owned()
    }

    /// Every log the home holds, for a failure message.
    pub fn logs(&self) -> String {
        let mut logs = String::new();
        for folder in fs::read_dir(self.home().join("logs")).into_iter().flatten() {
            for log in fs::read_dir(folder.unwrap().path()).into_iter().flatten() {
                let path = log.unwrap().path();
                let text = fs::read_to_string(&path).unwrap_or_default();
                logs.push_str(&format!("{}:\n{text}\n", path.display()));
            }
        }
        logs
    }

    pub fn sessions(&self) -> Vec<SessionMetadata> {
        let Ok(entries) = fs::read_dir(self.home().join("sessions")) else {
            return Vec::new();
        };
        entries
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "pty"))
            .filter_map(|path| SessionMetadata::from_json(&path, &fs::read(&path).ok()?).ok())
            .collect()
    }

    /// `pwsh` with the shell profile's start-up, prompt `prompt`, no line
    /// predictions and no saved history, with the package on `PATH`.
    pub fn pwsh(&self, package: &Package, pwsh: &Path, prompt: &str) -> CommandBuilder {
        let mut command = CommandBuilder::new(pwsh);
        command.args(pwsh_arguments(prompt));
        let path = env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![package.folder.clone()];
        paths.extend(env::split_paths(&path));
        command.env("PATH", env::join_paths(paths).unwrap());
        command.env("CLIMON2_HOME", self.home());
        command.env("LOCALAPPDATA", self.local());
        command
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        for session in self.sessions() {
            if process::is_same_daemon(&session).unwrap_or(false) {
                end_process(session.daemon_pid);
            }
        }
    }
}

/// The shell profile's start-up, with the prompt `<name>> `, no line
/// predictions (which would show earlier commands) and no saved history.
/// The prompt is put together when shown, so a typed command that names it
/// never shows it.
pub fn pwsh_arguments(name: &str) -> Vec<String> {
    vec![
        "-NoLogo".into(),
        "-NoProfile".into(),
        "-NoExit".into(),
        "-Command".into(),
        format!(
            "Set-PSReadLineOption -HistorySaveStyle SaveNothing; \
             try {{ Set-PSReadLineOption -PredictionSource None }} catch {{}}; \
             function global:prompt {{ '{name}' + '> ' }}"
        ),
    ]
}

/// `text` quoted for a `pwsh` command line: in single quotes, each doubled.
pub fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Waits until `text` shows; a failure names every log.
pub fn wait(terminal: &mut Terminal, home: &Home, text: &str) {
    if let Err(error) = terminal.wait_for_text(text, TIMEOUT) {
        panic!("{error}\nLOGS:\n{}", home.logs());
    }
}

/// Types `line` and Enter.
pub fn enter(terminal: &mut Terminal, line: &str) {
    terminal.send(format!("{line}\r")).unwrap();
}
