//! #321: the Windows smoke test (docs/for-humans/windows/qualification.md, "Existing
//! scenarios"): the packaged product under the harness's Windows Terminal
//! model. From `pwsh`, `c2 exec pwsh` runs a command, detaches, `c2 list`
//! shows the session, `c2 attach` restores its screen, and `exit 7` in the
//! session reaches the outer shell.
//!
//! #324: `WIN-03`, the three job shapes (qualification.md, "Host models").
//! From a shell in a job that allows breakaway, `exec` starts a session that
//! survives the host. From one in a job that refuses it, and through
//! `Start-Process -Wait` from each PowerShell, `exec` exits 125 and leaves no
//! session. Also the Windows forms of two Unix scenarios: nested `pwsh`
//! sessions, whose inner daemon leaves the outer session's job, and the
//! refusal of a session attaching to itself.
//!
//! #323: closing the host and killing the session, by `WIN-01` and `WIN-02`
//! (qualification.md, "Windows-only scenarios"). Each asserts outcomes only,
//! never the order of events, because real hosts deliver close events in
//! different orders.
//!
//! #326: Ctrl+Break, by `WIN-09`, under the Windows Terminal model only: the
//! VS Code terminal delivers nothing for it.
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
use climon2_test_term::{
    ConPty, HostModel, HostSettings, JobShape, Key, Modifiers, Options, Terminal, controlled_pwsh,
    local_app_data, rerun_outside,
};
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
use windows_sys::Win32::System::{
    JobObjects::IsProcessInJob,
    Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, TerminateProcess,
    },
};

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
struct Home(tempfile::TempDir, tempfile::TempDir);

impl Home {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap(), local_app_data())
    }

    fn home(&self) -> PathBuf {
        self.0.path().join("home")
    }

    fn local(&self) -> PathBuf {
        self.1.path().to_owned()
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

/// Whether the running process `pid` is in any job.
fn in_any_job(pid: u32) -> bool {
    // Opening by PID has no preconditions.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    assert!(!raw.is_null(), "process {pid} is not running");
    // A new handle, owned by nothing else.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut in_job = 1;
    // The handle is open with query access; the call writes one BOOL.
    assert_ne!(
        unsafe { IsProcessInJob(process.as_raw_handle(), std::ptr::null_mut(), &mut in_job) },
        0
    );
    in_job != 0
}

/// Windows PowerShell 5.1, which every Windows 11 has, by absolute path.
fn windows_powershell() -> PathBuf {
    Path::new(&env::var_os("SystemRoot").unwrap())
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
}

/// `shell` at the prompt `OUTER> `, hosted under the Windows Terminal model
/// in a job of `job`'s shape.
fn host(package: &Package, home: &Home, shell: &Path, job: JobShape) -> Terminal {
    let settings = HostSettings {
        model: HostModel::WindowsTerminal,
        job,
    };
    let command = home.pwsh(package, shell, "OUTER");
    let mut terminal =
        Terminal::spawn_hosted(command, 30, 120, Options::default(), settings).unwrap();
    wait(&mut terminal, home, "OUTER> ");
    terminal
}

/// Typed in a hosted shell: a headless session running `cmd`, which waits
/// for input that never comes, then the status `exec` exited with.
const HEADLESS_EXEC: &str = "c2 exec --headless -- cmd /d; \"EXEC-STATUS=$LASTEXITCODE\"";

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

/// `WIN-03`: from a shell in a job that allows breakaway, `exec` succeeds,
/// and the session survives the host ending every process in that job.
#[test]
fn from_a_job_that_allows_breakaway_exec_starts_a_session_that_survives_the_host() {
    if rerun_outside(
        "from_a_job_that_allows_breakaway_exec_starts_a_session_that_survives_the_host",
        OUTSIDE,
    ) {
        return;
    }
    let Some(pwsh) = controlled_pwsh() else {
        return;
    };
    let (package, home) = (Package::new(), Home::new());
    let mut terminal = host(&package, &home, &pwsh, JobShape::AllowsBreakaway);
    enter(&mut terminal, HEADLESS_EXEC);
    wait(&mut terminal, &home, "EXEC-STATUS=0");
    // Dropping the host closes its job, which ends every process in it.
    drop(terminal);
    let [session] = home.sessions().try_into().unwrap();
    assert_eq!(process::is_same_daemon(&session).ok(), Some(true));
    assert!(!in_any_job(session.daemon_pid));
}

/// `WIN-03`: from a shell in a job that refuses breakaway, `exec` exits 125
/// and leaves no session.
#[test]
fn from_a_job_that_refuses_breakaway_exec_exits_125() {
    if rerun_outside("from_a_job_that_refuses_breakaway_exec_exits_125", OUTSIDE) {
        return;
    }
    let Some(pwsh) = controlled_pwsh() else {
        return;
    };
    let (package, home) = (Package::new(), Home::new());
    let mut terminal = host(&package, &home, &pwsh, JobShape::RefusesBreakaway);
    enter(&mut terminal, HEADLESS_EXEC);
    wait(&mut terminal, &home, "EXEC-STATUS=125");
    assert!(
        terminal
            .screen()
            .replace('\n', "")
            .contains("cannot leave the job"),
        "{}",
        terminal.screen()
    );
    assert!(home.sessions().is_empty());
}

/// `WIN-03`: `c2` run through `Start-Process -Wait`, which puts it in a job
/// that refuses breakaway, exits 125 and leaves no session, from `pwsh` and
/// from Windows PowerShell 5.1.
#[test]
fn through_start_process_wait_from_each_powershell_exec_exits_125() {
    if rerun_outside(
        "through_start_process_wait_from_each_powershell_exec_exits_125",
        OUTSIDE,
    ) {
        return;
    }
    let Some(pwsh) = controlled_pwsh() else {
        return;
    };
    for shell in [pwsh, windows_powershell()] {
        let (package, home) = (Package::new(), Home::new());
        let mut terminal = host(&package, &home, &shell, JobShape::None);
        enter(
            &mut terminal,
            "$c2 = Start-Process -Wait -PassThru -NoNewWindow c2 \
             -ArgumentList 'exec','--headless','--','cmd','/d'; \
             \"EXEC-STATUS=$($c2.ExitCode)\"",
        );
        wait(&mut terminal, &home, "EXEC-STATUS=125");
        assert!(
            terminal
                .screen()
                .replace('\n', "")
                .contains("cannot leave the job"),
            "{}: {}",
            shell.display(),
            terminal.screen()
        );
        assert!(home.sessions().is_empty(), "{}", shell.display());
    }
}

/// The Windows form of the nested bash scenario: `c2 exec pwsh` inside a
/// session starts a second session, at depth 2, whose daemon is in no job,
/// so it left the outer session's; each session's status reaches the shell
/// that started it.
#[test]
fn nested_pwsh_sessions_are_both_listed_leave_the_outer_job_and_route_exit_status() {
    if rerun_outside(
        "nested_pwsh_sessions_are_both_listed_leave_the_outer_job_and_route_exit_status",
        OUTSIDE,
    ) {
        return;
    }
    let Some(pwsh) = controlled_pwsh() else {
        return;
    };
    let (package, home) = (Package::new(), Home::new());
    let mut terminal = host(&package, &home, &pwsh, JobShape::None);
    let exec = |prompt: &str, then: &str| {
        let arguments: Vec<String> = pwsh_arguments(prompt)
            .iter()
            .map(|argument| quoted(argument))
            .collect();
        format!(
            "c2 exec {} {}; {then}",
            quoted(&pwsh.to_string_lossy()),
            arguments.join(" ")
        )
    };
    enter(&mut terminal, &exec("ENCLOSING", "exit $LASTEXITCODE"));
    wait(&mut terminal, &home, "ENCLOSING> ");
    enter(
        &mut terminal,
        &exec("NESTED", "\"NESTED-STATUS=$LASTEXITCODE\""),
    );
    wait(&mut terminal, &home, "NESTED> ");
    enter(&mut terminal, "\"depth:$env:CLIMON2_SESSION_DEPTH\"");
    wait(&mut terminal, &home, "depth:2");

    let mut sessions = home.sessions();
    sessions.sort_by_key(|session| session.depth);
    let depths: Vec<u32> = sessions.iter().map(|session| session.depth.get()).collect();
    assert_eq!(depths, [1u32, 2u32]);
    assert_ne!(sessions[0].session_id, sessions[1].session_id);
    assert!(!in_any_job(sessions[1].daemon_pid));

    enter(&mut terminal, "exit 7");
    wait(&mut terminal, &home, "NESTED-STATUS=7");
    assert_eq!(home.sessions().len(), 1);
    enter(&mut terminal, "exit 9");
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 9);
    drop(terminal);
    assert!(home.sessions().is_empty());
}

/// The Windows form of the self-attach scenario: the session's command sees
/// `CLIMON2_SESSION_ID`, so `c2 attach` to it is refused with 125 and the
/// session carries on undisturbed.
#[test]
fn a_session_attaching_to_itself_is_refused_with_125_and_carries_on() {
    if rerun_outside(
        "a_session_attaching_to_itself_is_refused_with_125_and_carries_on",
        OUTSIDE,
    ) {
        return;
    }
    let Some(pwsh) = controlled_pwsh() else {
        return;
    };
    let (package, home) = (Package::new(), Home::new());
    let mut terminal = host(&package, &home, &pwsh, JobShape::None);
    let inner: Vec<String> = pwsh_arguments("INNER")
        .iter()
        .map(|argument| quoted(argument))
        .collect();
    enter(
        &mut terminal,
        &format!(
            "c2 exec {} {}; exit $LASTEXITCODE",
            quoted(&pwsh.to_string_lossy()),
            inner.join(" ")
        ),
    );
    wait(&mut terminal, &home, "INNER> ");
    let [session] = home.sessions().try_into().unwrap();
    enter(
        &mut terminal,
        "c2 attach $env:CLIMON2_SESSION_ID; \"SELF-STATUS=$LASTEXITCODE\"",
    );
    wait(&mut terminal, &home, "SELF-STATUS=125");
    assert!(
        terminal.screen().replace('\n', "").contains(&format!(
            "cannot attach to the current session {}",
            session.session_id
        )),
        "{}",
        terminal.screen()
    );
    enter(&mut terminal, "\"still-$(1+1)\"");
    wait(&mut terminal, &home, "still-2");
    let [still] = home.sessions().try_into().unwrap();
    assert_eq!(still.session_id, session.session_id);
    enter(&mut terminal, "exit 0");
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
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

    /// `pwsh`; or locally, where the controlled `pwsh` is missing, Windows
    /// PowerShell in its place.
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
/// #322, `WIN-10`: a command's full 32-bit status, including 259, which
/// Windows also reports for a process still running, passes unchanged
/// through the client, the CLI and the launcher.
#[test]
fn full_width_statuses_pass_through_the_client_the_cli_and_the_launcher() {
    if rerun_outside(
        "full_width_statuses_pass_through_the_client_the_cli_and_the_launcher",
        OUTSIDE,
    ) {
        return;
    }
    let (package, home) = (Package::new(), Home::new());
    for status in [0xC000_013A_u32, 259, 0xFFFF_FFFF] {
        let mut command = CommandBuilder::new(package.launcher());
        // cmd.exe takes the status as a signed 32-bit number.
        let exit = format!("exit {}", status as i32);
        command.args(["exec", "cmd", "/d", "/c", &exit]);
        command.env("CLIMON2_HOME", home.home());
        command.env("LOCALAPPDATA", home.local());
        let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
        let ended = terminal
            .wait_exit(TIMEOUT)
            .unwrap_or_else(|error| panic!("{error}\nLOGS:\n{}", home.logs()));
        assert_eq!(ended, status, "{status:#x}: {}", terminal.screen());
    }
}

/// `WIN-09` (#326): Ctrl+Break pressed in the Windows Terminal model reaches
/// the program in the session. `ping -t` reports Ctrl+Break and carries on,
/// where Ctrl+C would stop it, so the key-up record carrying ETX that comes
/// with Ctrl+Break never reached the session as Ctrl+C. `timeout`, which
/// keeps the default handler, ends with `0xC000013A`, and `cmd`, the
/// session's shell, survives.
#[test]
fn ctrl_break_reaches_the_program_in_the_session_and_the_shell_survives() {
    if rerun_outside(
        "ctrl_break_reaches_the_program_in_the_session_and_the_shell_survives",
        OUTSIDE,
    ) {
        return;
    }
    let (package, home) = (Package::new(), Home::new());
    let mut command = CommandBuilder::new(package.launcher());
    command.args(["exec", "cmd", "/d", "/k", "prompt INNER$G"]);
    command.env("CLIMON2_HOME", home.home());
    command.env("LOCALAPPDATA", home.local());
    let mut terminal = Terminal::spawn(command, 30, 120).unwrap();
    wait(&mut terminal, &home, "INNER>");

    enter(&mut terminal, "ping -t 127.0.0.1");
    wait(&mut terminal, &home, "Reply from");
    terminal.press(Key::Break, Modifiers::CTRL).unwrap();
    wait(&mut terminal, &home, "Control-Break");
    terminal
        .wait_for_screen(TIMEOUT, |screen| {
            screen
                .split("Control-Break")
                .nth(1)
                .is_some_and(|after| after.contains("Reply from"))
        })
        .unwrap_or_else(|error| panic!("{error}\nLOGS:\n{}", home.logs()));
    assert!(
        !terminal.screen().contains("Control-C"),
        "{}",
        terminal.screen()
    );
    // Ctrl+C, byte 0x03, stops it.
    terminal.send([0x03]).unwrap();
    wait(&mut terminal, &home, "Control-C");

    enter(&mut terminal, "cls");
    terminal
        .wait_for_screen(TIMEOUT, |screen| !screen.contains("Control-C"))
        .unwrap();
    enter(&mut terminal, "timeout /t 60");
    wait(&mut terminal, &home, "press a key");
    terminal.press(Key::Break, Modifiers::CTRL).unwrap();
    enter(&mut terminal, "echo BREAK-STATUS=%ERRORLEVEL%");
    wait(&mut terminal, &home, "BREAK-STATUS=-1073741510");
    enter(&mut terminal, "exit 0");
    assert_eq!(
        terminal.wait_exit(TIMEOUT).unwrap(),
        0,
        "{}",
        terminal.screen()
    );
}
