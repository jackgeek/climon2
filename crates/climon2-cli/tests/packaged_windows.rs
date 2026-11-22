//! #321: the Windows smoke test (docs/for-humans/windows/qualification.md, "Existing
//! scenarios"): the packaged product under the harness's Windows Terminal
//! model. From `pwsh`, `c2 exec pwsh` runs a command, detaches, `c2 list`
//! shows the session, `c2 attach` restores its screen, and `exit 7` in the
//! session reaches the outer shell.
//!
//! #323: closing the host and killing the session, by `WIN-01` and `WIN-02`
//! (qualification.md, "Windows-only scenarios"). Each asserts outcomes only,
//! never the order of events, because real hosts deliver close events in
//! different orders.
//!
//! The product comes from the release zip named by `CLIMON2_E2E_ARCHIVE`,
//! as the Windows Test job builds it, or else from this build's binaries in
//! the same layout. It writes the `current` file the launcher reads itself;
//! installing with `install.ps1` is the Windows Archive job's (#329).
//!
//! cargo runs tests in a job that refuses breakaway, and so does the CI
//! runner, so each test runs again outside every job
//! (`climon2_test_term::rerun_outside`), where `c2 exec`'s daemon can leave
//! its host as it does under a real terminal.
#![cfg(windows)]
use climon2_core::metadata::SessionMetadata;
use climon2_platform::process;
use climon2_test_term::{ConPty, Terminal, controlled_pwsh, rerun_outside};
use portable_pty::CommandBuilder;
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};

climon2_test_term::outside_relay!();

/// Every step waits this long at most; a failed hand-off can spend the
/// five-second start-up and leave budgets.
const TIMEOUT: Duration = Duration::from_secs(30);
/// How long the whole test may take outside every job.
const OUTSIDE: Duration = Duration::from_secs(240);
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The escape prefix, Ctrl+], then `d`.
const DETACH: &[u8] = b"\x1dd";

/// The installed product: `c2.exe`, `current` and the version folder, from
/// the release zip or from this build.
struct Package {
    _root: tempfile::TempDir,
    folder: PathBuf,
}

impl Package {
    fn new() -> Self {
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

    fn launcher(&self) -> PathBuf {
        self.folder.join("c2.exe")
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
        cli.with_file_name("deps").join("climon2_client.dll"),
        conpty.join("conpty.dll"),
        conpty.join("OpenConsole.exe"),
    ];
    for file in [&launcher].into_iter().chain(&shipped) {
        assert!(
            file.is_file(),
            "missing {}; build the launcher, daemon and client first \
             (cargo test -p climon2-launcher -p climon2-daemon -p climon2-client)",
            file.display()
        );
    }
    fs::copy(&launcher, root.join("c2.exe")).unwrap();
    for file in shipped {
        fs::copy(&file, version.join(file.file_name().unwrap())).unwrap();
    }
    root.to_owned()
}

/// A climon2 home and runtime root of its own. Dropping it ends every daemon
/// its sessions name, so a failed test leaves nothing running.
struct Home(tempfile::TempDir);

impl Home {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }

    fn home(&self) -> PathBuf {
        self.0.path().join("home")
    }

    fn local(&self) -> PathBuf {
        self.0.path().join("local")
    }

    /// Every log the home holds, for a failure message.
    fn logs(&self) -> String {
        let mut logs = String::new();
        for folder in fs::read_dir(self.home().join("logs")).into_iter().flatten() {
            for log in fs::read_dir(folder.unwrap().path()).into_iter().flatten() {
                let path = log.unwrap().path();
                let text = fs::read_to_string(&path).unwrap_or_default();
                logs.push_str(&format!(
                    "{}:
{text}
",
                    path.display()
                ));
            }
        }
        logs
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

    /// `pwsh` with the shell profile's start-up, prompt `prompt`, no line
    /// predictions and no saved history, with the package on `PATH`.
    fn pwsh(&self, package: &Package, pwsh: &Path, prompt: &str) -> CommandBuilder {
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
                terminate(session.daemon_pid);
            }
        }
    }
}

fn terminate(pid: u32) {
    // Opening by PID has no preconditions.
    let process = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if !process.is_null() {
        // A new handle, open with PROCESS_TERMINATE, closed here.
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        unsafe { TerminateProcess(process.as_raw_handle(), 1) };
    }
}

/// The shell profile's start-up, with the prompt `<name>> `, no line
/// predictions (which would show earlier commands) and no saved history.
/// The prompt is put together when shown, so a typed command that names it
/// never shows it.
fn pwsh_arguments(name: &str) -> Vec<String> {
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
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Waits until `text` shows; a failure names every log.
fn wait(terminal: &mut Terminal, home: &Home, text: &str) {
    if let Err(error) = terminal.wait_for_text(text, TIMEOUT) {
        panic!(
            "{error}
LOGS:
{}",
            home.logs()
        );
    }
}

/// Clears the outer shell's screen, so what shows next was drawn anew.
fn clear(terminal: &mut Terminal) {
    enter(terminal, "Clear-Host");
    terminal
        .wait_for_screen(TIMEOUT, |screen| !screen.contains("42-ok"))
        .unwrap();
}

/// Types `line` and Enter.
fn enter(terminal: &mut Terminal, line: &str) {
    terminal.send(format!("{line}\r")).unwrap();
}

#[test]
fn pwsh_from_pwsh_detaches_lists_reattaches_restored_and_exits_seven() {
    if rerun_outside(
        "pwsh_from_pwsh_detaches_lists_reattaches_restored_and_exits_seven",
        OUTSIDE,
    ) {
        return;
    }
    let Some(pwsh) = controlled_pwsh() else {
        return;
    };
    let (package, home) = (Package::new(), Home::new());
    let mut terminal = Terminal::spawn(home.pwsh(&package, &pwsh, "OUTER"), 30, 120).unwrap();
    wait(&mut terminal, &home, "OUTER> ");

    let inner: Vec<String> = pwsh_arguments("INNER")
        .iter()
        .map(|argument| quoted(argument))
        .collect();
    enter(
        &mut terminal,
        &format!(
            "c2 exec {} {}; \"OUTER-STATUS=$LASTEXITCODE\"",
            quoted(&pwsh.to_string_lossy()),
            inner.join(" ")
        ),
    );
    wait(&mut terminal, &home, "INNER> ");
    enter(&mut terminal, "\"$(6*7)-ok\"");
    wait(&mut terminal, &home, "42-ok");
    let [session] = home.sessions().try_into().unwrap_or_else(|_| {
        panic!(
            "{:?} {}",
            fs::read_dir(home.0.path()).map(|d| d.map(|e| e.unwrap().path()).collect::<Vec<_>>()),
            terminal.screen()
        )
    });
    let id = session.session_id.to_string();

    terminal.send(DETACH).unwrap();
    wait(&mut terminal, &home, "OUTER-STATUS=0");
    // The notice is wider than the screen, so it wraps.
    let unwrapped = terminal.screen().replace('\n', "");
    assert!(
        unwrapped.contains(&format!(
            "detached from session {id}; reattach with: c2 attach {id}"
        )),
        "{}",
        terminal.screen()
    );
    clear(&mut terminal);
    enter(&mut terminal, "c2 list");
    wait(&mut terminal, &home, &id[..8]);

    clear(&mut terminal);
    enter(
        &mut terminal,
        &format!(
            "c2 attach {}; \"OUTER-STATUS=$LASTEXITCODE\"; exit $LASTEXITCODE",
            &id[..8]
        ),
    );
    wait(&mut terminal, &home, "42-ok");
    wait(&mut terminal, &home, "INNER> ");
    enter(&mut terminal, "exit 7");
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 7);
    assert!(
        terminal.screen().contains("OUTER-STATUS=7"),
        "{}",
        terminal.screen()
    );
    drop(terminal);
    assert!(home.sessions().is_empty());
}

/// The shell a session runs, by its qualified profile
/// (docs/for-humans/windows/qualification.md, "Shell profiles").
#[derive(Clone, Copy, Debug)]
enum Profile {
    Pwsh,
    PowerShell,
    Cmd,
}

impl Profile {
    const ALL: [Self; 3] = [Self::Pwsh, Self::PowerShell, Self::Cmd];

    /// The shell by absolute path; `None` is a reported local skip.
    fn path(self) -> Option<PathBuf> {
        let system = Path::new(&env::var_os("SystemRoot").unwrap()).join("System32");
        match self {
            Self::Pwsh => controlled_pwsh(),
            Self::PowerShell => Some(system.join(r"WindowsPowerShell\v1.0\powershell.exe")),
            Self::Cmd => Some(system.join("cmd.exe")),
        }
    }

    /// `pwsh`; or locally, where only the Store build may be installed, which
    /// a session cannot start, Windows PowerShell in its place.
    fn session() -> (Self, PathBuf) {
        [Self::Pwsh, Self::PowerShell]
            .into_iter()
            .find_map(|shell| Some((shell, shell.path()?)))
            .unwrap()
    }

    /// Interactive start-up with the prompt `<name>> `.
    fn arguments(self, name: &str) -> Vec<String> {
        match self {
            Self::Pwsh | Self::PowerShell => pwsh_arguments(name),
            Self::Cmd => vec!["/D".into(), "/K".into(), format!("prompt {name}$G$S")],
        }
    }
}

impl Home {
    /// `c2 <arguments>` from the package, with this home.
    fn c2(&self, package: &Package, arguments: &[&OsStr]) -> CommandBuilder {
        let mut command = CommandBuilder::new(package.launcher());
        command.args(arguments);
        command.env("CLIMON2_HOME", self.home());
        command.env("LOCALAPPDATA", self.local());
        command
    }

    /// `c2 <arguments>` run to its end without a console, with this home.
    fn run(&self, package: &Package, arguments: &[&OsStr]) -> std::process::Output {
        Command::new(package.launcher())
            .args(arguments)
            .env("CLIMON2_HOME", self.home())
            .env("LOCALAPPDATA", self.local())
            .output()
            .unwrap()
    }

    /// The one session, once its metadata is written.
    fn session(&self) -> SessionMetadata {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Ok([session]) = <[_; 1]>::try_from(self.sessions()) {
                return session;
            }
            assert!(Instant::now() < deadline, "{}", self.logs());
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Waits until no session is left and `session`'s daemon has ended.
    fn wait_until_ended(&self, session: &SessionMetadata) {
        let deadline = Instant::now() + TIMEOUT;
        while !self.sessions().is_empty() || process::is_same_daemon(session).unwrap() {
            assert!(Instant::now() < deadline, "{}", self.logs());
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// `exec <shell> <its start-up>`, the shell's prompt `INNER> `.
fn exec_arguments(shell: Profile, path: &Path) -> Vec<OsString> {
    let mut arguments: Vec<OsString> = vec!["exec".into(), path.into()];
    arguments.extend(shell.arguments("INNER").into_iter().map(OsString::from));
    arguments
}

fn borrowed(arguments: &[OsString]) -> Vec<&OsStr> {
    arguments.iter().map(OsString::as_os_str).collect()
}

/// `c2 <arguments>` hosted in a terminal of its own, once `text` shows.
fn hosted(package: &Package, home: &Home, arguments: &[OsString], text: &str) -> Terminal {
    let command = home.c2(package, &borrowed(arguments));
    let mut terminal = Terminal::spawn(command, 30, 120).unwrap();
    wait(&mut terminal, home, text);
    terminal
}

fn attach_arguments(id: &str) -> Vec<OsString> {
    vec!["attach".into(), id[..8].into()]
}

/// `0xC000013A`, `STATUS_CONTROL_C_EXIT`: how each shell ends when its
/// console closes.
const CONTROL_C_EXIT: u32 = 0xC000_013A;
/// Longer than the hang-up grace, with room for a session that ends late.
const PAST_GRACE: Duration = Duration::from_secs(8);

/// WIN-02 and #323: `c2 kill` hangs a session up, and every client attached
/// to it exits with the command's status: the shell's status when its
/// console closes, through the client, the CLI and the launcher unchanged.
#[test]
fn kill_ends_every_attached_client_with_the_shells_close_status() {
    if rerun_outside(
        "kill_ends_every_attached_client_with_the_shells_close_status",
        OUTSIDE,
    ) {
        return;
    }
    for shell in Profile::ALL {
        let Some(path) = shell.path() else {
            continue;
        };
        let (package, home) = (Package::new(), Home::new());
        let mut first = hosted(&package, &home, &exec_arguments(shell, &path), "INNER> ");
        let id = home.session().session_id.to_string();
        let mut second = hosted(&package, &home, &attach_arguments(&id), "INNER> ");

        let killed = home.run(&package, &borrowed(&["kill".into(), id[..8].into()]));
        assert_eq!(killed.status.code(), Some(0), "{shell:?} {killed:?}");
        for client in [&mut first, &mut second] {
            assert_eq!(
                client.wait_exit(TIMEOUT).unwrap(),
                CONTROL_C_EXIT,
                "{shell:?}: {}",
                client.screen()
            );
        }
        assert!(home.sessions().is_empty(), "{shell:?}");
    }
}

/// WIN-01, Windows Terminal model: closing the host while attached ends the
/// client with 129, and the ordinary session, whose last client is lost, is
/// hung up and ends.
#[test]
fn closing_the_host_ends_the_client_with_129_and_the_ordinary_session_with_it() {
    if rerun_outside(
        "closing_the_host_ends_the_client_with_129_and_the_ordinary_session_with_it",
        OUTSIDE,
    ) {
        return;
    }
    let (shell, path) = Profile::session();
    let (package, home) = (Package::new(), Home::new());
    let mut host = hosted(&package, &home, &exec_arguments(shell, &path), "INNER> ");
    let session = home.session();

    host.close();
    assert_eq!(host.wait_exit(TIMEOUT).unwrap(), 129, "{}", host.screen());
    home.wait_until_ended(&session);
}

/// WIN-01, Windows Terminal model: closing the host of a client attached to a
/// headless session ends the client with 129 and leaves the session running,
/// to be attached again.
#[test]
fn closing_the_host_of_a_headless_sessions_client_leaves_the_session_running() {
    if rerun_outside(
        "closing_the_host_of_a_headless_sessions_client_leaves_the_session_running",
        OUTSIDE,
    ) {
        return;
    }
    let (shell, path) = Profile::session();
    let (package, home) = (Package::new(), Home::new());
    let mut arguments = exec_arguments(shell, &path);
    arguments.splice(1..1, ["--headless".into(), "--".into()]);
    let started = home.run(&package, &borrowed(&arguments));
    assert_eq!(started.status.code(), Some(0), "{started:?}");
    let session = home.session();
    let id = session.session_id.to_string();

    let mut host = hosted(&package, &home, &attach_arguments(&id), "INNER> ");
    host.close();
    assert_eq!(host.wait_exit(TIMEOUT).unwrap(), 129, "{}", host.screen());
    std::thread::sleep(PAST_GRACE);
    assert!(process::is_same_daemon(&home.session()).unwrap());

    let mut again = hosted(&package, &home, &attach_arguments(&id), "INNER> ");
    enter(&mut again, "exit 7");
    assert_eq!(again.wait_exit(TIMEOUT).unwrap(), 7, "{}", again.screen());
    home.wait_until_ended(&session);
}

/// WIN-01, Windows Terminal model: a session detached from its host survives
/// the host closing, and attaching again restores its screen.
#[test]
fn a_detached_session_survives_its_host_closing_and_reattaches() {
    if rerun_outside(
        "a_detached_session_survives_its_host_closing_and_reattaches",
        OUTSIDE,
    ) {
        return;
    }
    let (shell, path) = Profile::session();
    let (package, home) = (Package::new(), Home::new());
    let mut host = Terminal::spawn(home.pwsh(&package, &path, "OUTER"), 30, 120).unwrap();
    wait(&mut host, &home, "OUTER> ");
    let inner: Vec<String> = shell
        .arguments("INNER")
        .iter()
        .map(|argument| quoted(argument))
        .collect();
    enter(
        &mut host,
        &format!(
            "c2 exec {} {}",
            quoted(&path.to_string_lossy()),
            inner.join(" ")
        ),
    );
    wait(&mut host, &home, "INNER> ");
    enter(&mut host, "\"$(6*7)-ok\"");
    wait(&mut host, &home, "42-ok");
    let session = home.session();
    let id = session.session_id.to_string();
    host.send(DETACH).unwrap();
    wait(&mut host, &home, "OUTER> ");

    host.close();
    host.wait_exit(TIMEOUT).unwrap();
    drop(host);
    std::thread::sleep(PAST_GRACE);
    assert!(process::is_same_daemon(&home.session()).unwrap());

    let mut again = hosted(&package, &home, &attach_arguments(&id), "42-ok");
    enter(&mut again, "exit 7");
    assert_eq!(again.wait_exit(TIMEOUT).unwrap(), 7, "{}", again.screen());
    home.wait_until_ended(&session);
}
