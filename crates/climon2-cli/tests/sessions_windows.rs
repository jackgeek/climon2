//! #328: the Windows forms of the packaged suite's killed-process and
//! log-privacy scenarios (docs/for-humans/windows/qualification.md,
//! "Existing scenarios"), against the packaged product under the Windows
//! Terminal model.
//!
//! - A command ended with `TerminateProcess(…, 1)` reports status 1, and the
//!   session's job object leaves no descendant.
//! - A client ended with `TerminateProcess` falls back to the daemon's leave
//!   policy: the ordinary session hangs up, waits the full grace and ends
//!   whatever is left in its job.
//! - A killed daemon: the client reports the lost connection once with the
//!   session's log folder, the command dies with the job object, and
//!   `c2 list` reclaims the stale session.
//! - Log privacy, driven through `pwsh`: no environment, argument or
//!   typed-input secret appears in any log, and every log file, metadata
//!   file and managed folder the run produced carries the private
//!   descriptor and is not a reparse point.
//!
//! The session's command in the killed scenarios is [`OWNED`]: a shell that
//! starts a child on a console of its own, so the child outlives the
//! session's console closing and only the session's job can end it.
//!
//! cargo runs tests in a job that refuses breakaway, and so does the CI
//! runner, so each test runs again outside every job
//! (`climon2_test_term::rerun_outside`).
#![cfg(windows)]
// Each test binary uses part of it.
#[allow(dead_code)]
mod support;

use climon2_core::metadata::SessionMetadata;
use climon2_test_term::{Terminal, controlled_pwsh, end_process, rerun_outside};
use portable_pty::CommandBuilder;
use std::{
    env,
    ffi::OsStr,
    fs,
    os::windows::{
        fs::MetadataExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use support::{Home, Package, TIMEOUT, wait};
use windows_sys::Win32::{
    Foundation::{INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
    Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT,
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            QueryFullProcessImageNameW, WaitForSingleObject,
        },
    },
};

climon2_test_term::outside_relay!();

/// How long one whole test may take outside every job.
const OUTSIDE: Duration = Duration::from_secs(300);
/// The hang-up grace the daemon waits in full before it ends the job.
const GRACE: Duration = Duration::from_secs(5);

/// The killed scenarios' command: it starts `ping` on a console of its own
/// with no window, so the session's console closing does not reach it,
/// writes its own PID and the child's into `$env:OWNED_DIR`, says
/// `owned-command-ready` and waits.
const OWNED: &str = r#"$info = [Diagnostics.ProcessStartInfo]::new("$env:SystemRoot\System32\ping.exe", '-n 600 127.0.0.1')
$info.UseShellExecute = $false
$info.CreateNoWindow = $true
$child = [Diagnostics.Process]::Start($info)
Set-Content -LiteralPath (Join-Path $env:OWNED_DIR 'child.pid') $child.Id
Set-Content -LiteralPath (Join-Path $env:OWNED_DIR 'command.pid') $PID
'owned-command-' + 'ready'
while ($true) { Start-Sleep 1 }
"#;

/// The session's shell: the controlled `pwsh`, or locally, where it is
/// missing, Windows PowerShell 5.1 in its place. CI requires the MSI `pwsh`.
fn session_shell() -> PathBuf {
    controlled_pwsh().unwrap_or_else(|| {
        Path::new(&env::var_os("SystemRoot").unwrap())
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
    })
}

/// `c2 <arguments>` from the package with this home.
fn c2(package: &Package, home: &Home, arguments: &[&OsStr]) -> CommandBuilder {
    let mut command = CommandBuilder::new(package.launcher());
    climon2_test_term::outside_any_session(&mut command);
    command.args(arguments);
    command.env("CLIMON2_HOME", home.home());
    command.env("LOCALAPPDATA", home.local());
    command
}

/// The folder [`OWNED`] writes its PIDs into.
fn owned_dir(home: &Home) -> PathBuf {
    home.0.path().join("owned")
}

/// The process the PID file `name` in [`owned_dir`] names, open to wait on.
fn owned(home: &Home, name: &str) -> Process {
    Process::open(owned_pid(home, name))
}

/// The PID the file `name` in [`owned_dir`] holds.
fn owned_pid(home: &Home, name: &str) -> u32 {
    let text = fs::read_to_string(owned_dir(home).join(name)).unwrap();
    text.trim().parse().unwrap()
}

/// `c2 exec` of [`OWNED`] hosted under the Windows Terminal model, once it is
/// ready; with the command and its child, open before anything is killed.
fn exec_owned(package: &Package, home: &Home) -> (Terminal, Process, Process) {
    fs::create_dir_all(owned_dir(home)).unwrap();
    let shell = session_shell();
    let mut command = c2(
        package,
        home,
        &[
            "exec".as_ref(),
            shell.as_os_str(),
            "-NoLogo".as_ref(),
            "-NoProfile".as_ref(),
            "-Command".as_ref(),
            OWNED.as_ref(),
        ],
    );
    command.env("OWNED_DIR", owned_dir(home));
    let mut terminal = Terminal::spawn(command, 30, 120).unwrap();
    wait(&mut terminal, home, "owned-command-ready");
    let leader = owned(home, "command.pid");
    let child = owned(home, "child.pid");
    (terminal, leader, child)
}

/// A process opened by PID while it runs, to see it end.
struct Process(OwnedHandle);

impl Process {
    fn open(pid: u32) -> Self {
        // Opening by PID has no preconditions.
        let raw = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            )
        };
        assert!(!raw.is_null(), "process {pid} is not running");
        // A new handle nothing else owns.
        Self(unsafe { OwnedHandle::from_raw_handle(raw) })
    }

    /// Whether the process ends within `timeout`.
    fn ends_within(&self, timeout: Duration) -> bool {
        let milliseconds = u32::try_from(timeout.as_millis()).unwrap();
        // The handle is open with SYNCHRONIZE.
        unsafe { WaitForSingleObject(self.0.as_raw_handle(), milliseconds) == WAIT_OBJECT_0 }
    }
}

/// The running processes whose executable is `path`.
fn processes_of(path: &Path) -> Vec<u32> {
    let wanted = comparable(&fs::canonicalize(path).unwrap().to_string_lossy());
    // Taking a snapshot has no preconditions.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    assert_ne!(raw, INVALID_HANDLE_VALUE);
    // A new handle nothing else owns.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut entry = PROCESSENTRY32W {
        dwSize: u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).unwrap(),
        ..PROCESSENTRY32W::default()
    };
    let mut pids = Vec::new();
    // The snapshot is open and `entry` names its own size.
    let mut more = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut entry) } != 0;
    while more {
        if image(entry.th32ProcessID).is_some_and(|image| image == wanted) {
            pids.push(entry.th32ProcessID);
        }
        // As for the first entry.
        more = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut entry) } != 0;
    }
    pids
}

/// The path of process `pid`'s executable, if it can be read.
fn image(pid: u32) -> Option<String> {
    // Opening by PID has no preconditions.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw.is_null() {
        return None;
    }
    // A new handle nothing else owns.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut buffer = vec![0u16; 32_768];
    let mut length = u32::try_from(buffer.len()).unwrap();
    // The handle is open with query access; `length` is the buffer's size.
    let read = unsafe {
        QueryFullProcessImageNameW(process.as_raw_handle(), 0, buffer.as_mut_ptr(), &mut length)
    };
    (read != 0).then(|| comparable(&String::from_utf16_lossy(&buffer[..length as usize])))
}

/// A path as compared above: without the `\\?\` prefix, in lower case.
fn comparable(path: &str) -> String {
    path.trim_start_matches(r"\\?\").to_lowercase()
}

/// Waits until the home lists no session; a failure names every log.
fn no_sessions(home: &Home) {
    let deadline = Instant::now() + TIMEOUT;
    while !home.sessions().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(home.sessions().is_empty(), "{}", home.logs());
}

/// Runs `c2 list` with this home and fails unless it lists no session.
fn lists_none(package: &Package, home: &Home) {
    let listed = Command::new(package.launcher())
        .arg("list")
        .env("CLIMON2_HOME", home.home())
        .env("LOCALAPPDATA", home.local())
        .output()
        .unwrap();
    assert_eq!(listed.status.code(), Some(0), "{listed:?}");
    assert_eq!(
        listed.stdout, b"No climon2 sessions running.\n",
        "{listed:?}"
    );
}

/// The Windows form of "a killed command returns 137 and its descendants are
/// cleaned up": the command ended with `TerminateProcess(…, 1)` reports 1
/// through the client, the CLI and the launcher, and the session's job
/// leaves no descendant of it running.
#[test]
fn a_command_ended_by_terminate_process_reports_1_and_leaves_no_descendant() {
    let name = "a_command_ended_by_terminate_process_reports_1_and_leaves_no_descendant";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    let (package, home) = (Package::new(), Home::new());
    let (mut terminal, leader, child) = exec_owned(&package, &home);
    end_process(owned_pid(&home, "command.pid"));
    assert_eq!(
        terminal.wait_exit(TIMEOUT).unwrap(),
        1,
        "{}\nLOGS:\n{}",
        terminal.screen(),
        home.logs()
    );
    assert!(leader.ends_within(Duration::ZERO));
    assert!(child.ends_within(TIMEOUT), "{}", home.logs());
    no_sessions(&home);
}

/// The Windows form of "a killed client falls back to the daemon's leave
/// policy": the client ended by `TerminateProcess` cannot ask for anything,
/// so the daemon only loses it. The ordinary session hangs up, waits the
/// full grace, and ends the child its console's closing did not reach.
#[test]
fn a_client_ended_by_terminate_process_leaves_the_session_to_the_leave_policy() {
    let name = "a_client_ended_by_terminate_process_leaves_the_session_to_the_leave_policy";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    let (package, home) = (Package::new(), Home::new());
    let (mut terminal, leader, child) = exec_owned(&package, &home);
    let [client] = processes_of(&package.version().join("climon2-client.exe"))
        .try_into()
        .unwrap_or_else(|pids| panic!("not one client: {pids:?}"));
    let killed = Instant::now();
    end_process(client);
    // The CLI and the launcher pass the client's status on unchanged.
    assert_eq!(
        terminal.wait_exit(TIMEOUT).unwrap(),
        1,
        "{}",
        terminal.screen()
    );
    assert!(leader.ends_within(TIMEOUT), "{}", home.logs());
    assert!(child.ends_within(TIMEOUT), "{}", home.logs());
    assert!(
        killed.elapsed() >= GRACE,
        "the job was ended before the grace: {:?}",
        killed.elapsed()
    );
    no_sessions(&home);
    lists_none(&package, &home);
}

/// The Windows form of "a killed daemon: logs are reported and `list`
/// reclaims the stale session": the client reports the lost connection
/// once, naming the session's log folder, and exits 125; the command and
/// its child die with the job object; `c2 list` removes the stale session.
#[test]
fn a_killed_daemon_is_reported_with_its_logs_its_command_dies_and_list_reclaims_it() {
    let name = "a_killed_daemon_is_reported_with_its_logs_its_command_dies_and_list_reclaims_it";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    let (package, home) = (Package::new(), Home::new());
    let (mut terminal, leader, child) = exec_owned(&package, &home);
    let [session]: [SessionMetadata; 1] = home.sessions().try_into().unwrap();
    end_process(session.daemon_pid);
    assert_eq!(
        terminal.wait_exit(TIMEOUT).unwrap(),
        125,
        "{}",
        terminal.screen()
    );
    let screen = terminal.screen().replace('\n', "");
    assert_eq!(screen.matches("climon2: error:").count(), 1, "{screen}");
    assert!(screen.contains("lost connection to session"), "{screen}");
    // The client names the folder by its canonical path, as on Unix.
    let logs = home
        .home()
        .canonicalize()
        .unwrap()
        .join("logs")
        .join(session.session_id.to_string());
    assert!(
        screen.contains(&format!("(logs: {})", logs.display())),
        "{screen}"
    );
    assert!(leader.ends_within(TIMEOUT), "{}", home.logs());
    assert!(child.ends_within(TIMEOUT), "{}", home.logs());

    assert_eq!(
        home.sessions().len(),
        1,
        "the stale session is left for list"
    );
    lists_none(&package, &home);
    assert!(home.sessions().is_empty(), "{}", home.logs());
}

/// The session's script in the log-privacy scenario: it shows the lengths of
/// the argument and environment secrets it got, reads a typed line and
/// shows its length, then exits 23.
const PRIVACY: &str = r#"param($Secret)
"argv-length=$($Secret.Length)"
"env-length=$($env:E2E_SECRET.Length)"
$typed = Read-Host 'input-ready'
"input-received=$($typed.Length)"
exit 23
"#;
const ARGV_SECRET: &str = "argv-secret ' literal spaces -- -78";
const ENV_SECRET: &str = "env-secret-not-for-logs-91b";
const INPUT_SECRET: &str = "input-secret-not-for-logs-72c";

/// Log privacy, driven through `pwsh`: with `CLIMON2_LOG=trace`, a session's
/// environment, argument and typed-input secrets reach its command and
/// appear in no log. Every log file, metadata file and managed folder the
/// run produced carries the private descriptor and is not a reparse point
/// (ADR 0003, "Private files and folders").
#[test]
fn no_secret_reaches_a_log_and_every_file_and_folder_the_run_made_is_private() {
    let name = "no_secret_reaches_a_log_and_every_file_and_folder_the_run_made_is_private";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    let (package, home) = (Package::new(), Home::new());
    let script = home.0.path().join("privacy.ps1");
    fs::write(&script, PRIVACY).unwrap();
    let shell = session_shell();
    let mut command = c2(
        &package,
        &home,
        &[
            "exec".as_ref(),
            shell.as_os_str(),
            "-NoLogo".as_ref(),
            "-NoProfile".as_ref(),
            "-ExecutionPolicy".as_ref(),
            "Bypass".as_ref(),
            "-File".as_ref(),
            script.as_os_str(),
            ARGV_SECRET.as_ref(),
        ],
    );
    command.env("CLIMON2_LOG", "trace");
    command.env("E2E_SECRET", ENV_SECRET);
    let mut terminal = Terminal::spawn(command, 30, 120).unwrap();
    wait(&mut terminal, &home, "input-ready");
    let screen = terminal.screen();
    assert!(
        screen.contains(&format!("argv-length={}", ARGV_SECRET.len())),
        "{screen}"
    );
    assert!(
        screen.contains(&format!("env-length={}", ENV_SECRET.len())),
        "{screen}"
    );
    // While the session runs: its metadata, the runtime directory and the
    // logs so far.
    let mut checked = produced(&home);
    assert!(
        checked
            .iter()
            .any(|path| path.extension().is_some_and(|extension| extension == "pty")),
        "no metadata among {checked:?}"
    );
    assert_private(&checked);

    terminal.send(format!("{INPUT_SECRET}\r")).unwrap();
    wait(
        &mut terminal,
        &home,
        &format!("input-received={}", INPUT_SECRET.len()),
    );
    assert_eq!(
        terminal.wait_exit(TIMEOUT).unwrap(),
        23,
        "{}",
        terminal.screen()
    );
    drop(terminal);
    no_sessions(&home);

    let logs = log_files(&home);
    assert!(!logs.is_empty());
    for path in &logs {
        let log = fs::read_to_string(path).unwrap();
        for secret in ["env-secret-not-for-logs", "argv-secret", INPUT_SECRET] {
            assert!(
                !log.contains(secret),
                "{}: secret leaked: {log}",
                path.display()
            );
        }
    }
    // After it ended: the logs it wrote since.
    checked = produced(&home);
    assert_private(&checked);
}

/// Every file under the home's `logs` folder.
fn log_files(home: &Home) -> Vec<PathBuf> {
    walk(&home.home().join("logs"))
        .into_iter()
        .filter(|path| path.is_file())
        .collect()
}

/// What climon2 manages in the home and the runtime root: the `logs` and
/// `sessions` folders and everything in them, and each runtime directory
/// with its contents but the sockets, which are reparse points of their own
/// kind (ADR 0003).
fn produced(home: &Home) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for folder in ["logs", "sessions"] {
        let folder = home.home().join(folder);
        if folder.exists() {
            paths.push(folder.clone());
            paths.extend(walk(&folder));
        }
    }
    let runtime = fs::read_dir(home.local().join("c2")).unwrap();
    for directory in runtime {
        let directory = directory.unwrap().path();
        paths.push(directory.clone());
        paths.extend(
            walk(&directory)
                .into_iter()
                .filter(|path| path.extension().is_none_or(|extension| extension != "sock")),
        );
    }
    paths
}

/// Everything under `folder`, depth first, without following links.
fn walk(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(folder).into_iter().flatten() {
        let path = entry.unwrap().path();
        found.push(path.clone());
        if fs::symlink_metadata(&path).unwrap().is_dir() {
            found.extend(walk(&path));
        }
    }
    found
}

/// Fails unless each of `paths` is no reparse point and carries exactly the
/// private descriptor: owned by the user, with one ACE granting the user
/// full control, inherited by what a folder holds.
fn assert_private(paths: &[PathBuf]) {
    let expected = |folder: bool| private_descriptor(if folder { "OICI" } else { "" });
    let (file, folder) = (expected(false), expected(true));
    let found = descriptors(paths);
    for (path, found) in paths.iter().zip(found) {
        let metadata = fs::symlink_metadata(path).unwrap();
        assert_eq!(
            metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT,
            0,
            "{} is a reparse point",
            path.display()
        );
        let wanted = if metadata.is_dir() { &folder } else { &file };
        assert_eq!(&found, wanted, "{}", path.display());
    }
}

/// The private descriptor in SDDL, as `Get-Acl` writes it, with the ACE
/// flags `ace_flags`.
fn private_descriptor(ace_flags: &str) -> String {
    let [descriptor] = powershell(
        "$user = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value; \
         $sddl = \"O:${user}D:P(A;$env:CLIMON2_ACE_FLAGS;FA;;;$user)\"; \
         [Security.AccessControl.RawSecurityDescriptor]::new($sddl).GetSddlForm('Owner, Access')",
        &[("CLIMON2_ACE_FLAGS", OsStr::new(ace_flags))],
    )
    .try_into()
    .unwrap();
    descriptor
}

/// The owner and DACL of each of `paths` in SDDL, as `Get-Acl` reads them.
fn descriptors(paths: &[PathBuf]) -> Vec<String> {
    let joined = env::join_paths(paths).unwrap();
    let found = powershell(
        "foreach ($path in $env:CLIMON2_ACL_PATHS -split ';') { \
         (Get-Acl -LiteralPath $path).GetSecurityDescriptorSddlForm('Owner, Access') }",
        &[("CLIMON2_ACL_PATHS", &joined)],
    );
    assert_eq!(found.len(), paths.len(), "{found:?}");
    found
}

/// The lines Windows PowerShell 5.1 prints for `script`.
fn powershell(script: &str, environment: &[(&str, &OsStr)]) -> Vec<String> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        // A PowerShell 7 parent's module path would load its own modules here.
        .env_remove("PSModulePath")
        .envs(environment.iter().copied())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}
