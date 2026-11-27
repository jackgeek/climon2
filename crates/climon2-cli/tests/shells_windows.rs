//! #328: the shell subset on Windows (docs/for-humans/windows/qualification.md,
//! "Coverage"): Windows PowerShell 5.1 and `cmd.exe`, each both the outer
//! shell that runs `c2` and the shell inside the session, under the Windows
//! Terminal model, against the packaged product.
//!
//! For each shell: it starts and shows its prompt in a session; a command's
//! exit status reaches it; Ctrl+C interrupts a running command; it detaches,
//! and reattaching restores its screen; `exit 7` reaches the outer shell; and
//! `c2 kill` closes it with its measured close status, `0xC000013A`, which the
//! outer shell sees.
//!
//! `pwsh` runs the full suite in the other test files.
//!
//! cargo runs tests in a job that refuses breakaway, and so does the CI
//! runner, so each test runs again outside every job
//! (`climon2_test_term::rerun_outside`).
#![cfg(windows)]
// Each test binary uses part of it.
#[allow(dead_code)]
mod support;

use climon2_test_term::{Key, Modifiers, Terminal, rerun_outside};
use portable_pty::CommandBuilder;
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use support::{Home, Package, enter, pwsh_arguments, quoted, wait};

climon2_test_term::outside_relay!();

/// How long one whole test may take outside every job.
const OUTSIDE: Duration = Duration::from_secs(300);
/// The escape prefix, Ctrl+], then `d`.
const DETACH: &[u8] = b"\x1dd";
/// `0xC000013A`, `STATUS_CONTROL_C_EXIT`, as both shells show a status: a
/// signed 32-bit number.
const CLOSED: &str = "-1073741510";
/// A program that runs until interrupted, the same line in both shells. It
/// shows `Reply from` (on an English Windows, as CI runs) once it is
/// running, so Ctrl+C reaches it rather than the shell starting it.
const LONG_RUNNING: &str = "ping -n 600 127.0.0.1";

/// A shell of the subset, by its qualified profile
/// (qualification.md, "Shell profiles").
#[derive(Clone, Copy, Debug)]
enum Shell {
    PowerShell,
    Cmd,
}

impl Shell {
    fn path(self) -> PathBuf {
        let system = Path::new(&env::var_os("SystemRoot").unwrap()).join("System32");
        match self {
            Self::PowerShell => system.join(r"WindowsPowerShell\v1.0\powershell.exe"),
            Self::Cmd => system.join("cmd.exe"),
        }
    }

    /// Interactive start-up with the prompt `<name>> `.
    fn arguments(self, name: &str) -> Vec<String> {
        match self {
            Self::PowerShell => pwsh_arguments(name),
            Self::Cmd => vec!["/D".into(), "/K".into(), format!("prompt {name}$G$S")],
        }
    }

    /// `line`, then the status it left shown as `<label>=<status>`.
    fn with_status(self, line: &str, label: &str) -> String {
        match self {
            Self::PowerShell => format!("{line}; \"{label}=$LASTEXITCODE\""),
            // `call` expands the status once the line before it has run.
            Self::Cmd => format!("{line} & call echo {label}=%^ERRORLEVEL%"),
        }
    }

    /// A line that shows 1000042, which the line itself does not contain;
    /// with `+1` after it, 1000043.
    fn arithmetic(self) -> &'static str {
        match self {
            Self::PowerShell => "1000000 + 6 * 7",
            Self::Cmd => "set /a 1000000+6*7",
        }
    }

    /// Clears the screen, then runs `line`.
    fn cleared(self, line: &str) -> String {
        match self {
            Self::PowerShell => format!("Clear-Host; {line}"),
            Self::Cmd => format!("cls & {line}"),
        }
    }

    /// `c2 exec` of this shell at the prompt `INNER> `, typed into itself.
    fn exec_line(self) -> String {
        let arguments: Vec<String> = self
            .arguments("INNER")
            .iter()
            .map(|argument| self.quote(argument))
            .collect();
        format!(
            "c2 exec {} {}",
            self.quote(&self.path().to_string_lossy()),
            arguments.join(" ")
        )
    }

    /// `text` as one argument on this shell's command line.
    fn quote(self, text: &str) -> String {
        match self {
            Self::PowerShell => quoted(text),
            // None of the arguments holds a double quote.
            Self::Cmd => format!("\"{text}\""),
        }
    }
}

/// `shell` at the prompt `OUTER> `, hosted under the Windows Terminal model,
/// with the package on `PATH`.
fn outer(package: &Package, home: &Home, shell: Shell) -> Terminal {
    let mut command = CommandBuilder::new(shell.path());
    climon2_test_term::outside_any_session(&mut command);
    command.args(shell.arguments("OUTER"));
    let path = env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![package.folder.clone()];
    paths.extend(env::split_paths(&path));
    command.env("PATH", env::join_paths(paths).unwrap());
    command.env("CLIMON2_HOME", home.home());
    command.env("LOCALAPPDATA", home.local());
    let mut terminal = Terminal::spawn(command, 30, 120).unwrap();
    wait(&mut terminal, home, "OUTER> ");
    terminal
}

/// The subset for `shell`, as the outer shell and the session's.
fn the_shell_subset(shell: Shell) {
    let (package, home) = (Package::new(), Home::new());
    let mut terminal = outer(&package, &home, shell);

    // Start and prompt.
    enter(
        &mut terminal,
        &shell.with_status(&shell.exec_line(), "EXEC-STATUS"),
    );
    wait(&mut terminal, &home, "INNER> ");
    let [session] = home.sessions().try_into().unwrap();
    let id = session.session_id.to_string();
    enter(&mut terminal, shell.arithmetic());
    wait(&mut terminal, &home, "1000042");

    // A command's status reaches the session's shell.
    enter(
        &mut terminal,
        &shell.with_status("cmd /d /c exit 3", "COMMAND-STATUS"),
    );
    wait(&mut terminal, &home, "COMMAND-STATUS=3");

    // Ctrl+C interrupts a running command and the shell takes input again.
    enter(&mut terminal, LONG_RUNNING);
    wait(&mut terminal, &home, "Reply from");
    terminal.press(Key::Char('c'), Modifiers::CTRL).unwrap();
    // Windows PowerShell drops what was typed before its prompt is back.
    terminal
        .wait_for_screen(support::TIMEOUT, |screen| {
            screen
                .lines()
                .map(str::trim)
                .rev()
                .find(|line| !line.is_empty())
                == Some("INNER>")
        })
        .unwrap_or_else(|error| panic!("{error}"));
    enter(&mut terminal, &format!("{}+1", shell.arithmetic()));
    wait(&mut terminal, &home, "1000043");

    // Detach, then reattach with the screen restored.
    terminal.send(DETACH).unwrap();
    wait(&mut terminal, &home, "EXEC-STATUS=0");
    enter(
        &mut terminal,
        &shell.with_status(&shell.cleared(&format!("c2 attach {id}")), "ATTACH-STATUS"),
    );
    wait(&mut terminal, &home, "1000042");
    input_reaches_the_session(&mut terminal, shell);

    // `exit 7` reaches the outer shell.
    enter(&mut terminal, "exit 7");
    wait(&mut terminal, &home, "ATTACH-STATUS=7");

    // A kill closes the shell with its measured status, which the outer
    // shell sees.
    enter(
        &mut terminal,
        &shell.with_status(&shell.cleared(&shell.exec_line()), "KILLED-STATUS"),
    );
    let session = one_session(&home);
    wait(&mut terminal, &home, "INNER> ");
    let killed =
        climon2_test_term::process_outside_any_session(&mut Command::new(package.launcher()))
            .args(["kill", session.session_id.as_str()])
            .env("CLIMON2_HOME", home.home())
            .env("LOCALAPPDATA", home.local())
            .output()
            .unwrap();
    assert_eq!(killed.status.code(), Some(0), "{killed:?}");
    wait(&mut terminal, &home, &format!("KILLED-STATUS={CLOSED}"));
    assert!(home.sessions().is_empty(), "{}", home.logs());
}

#[test]
fn windows_powershell_passes_the_shell_subset_as_outer_and_session_shell() {
    let name = "windows_powershell_passes_the_shell_subset_as_outer_and_session_shell";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    the_shell_subset(Shell::PowerShell);
}

#[test]
fn cmd_passes_the_shell_subset_as_outer_and_session_shell() {
    let name = "cmd_passes_the_shell_subset_as_outer_and_session_shell";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    the_shell_subset(Shell::Cmd);
}

/// The home's one session, once its metadata is written.
fn one_session(home: &Home) -> climon2_core::metadata::SessionMetadata {
    let deadline = std::time::Instant::now() + support::TIMEOUT;
    loop {
        if let Ok([session]) = <[_; 1]>::try_from(home.sessions()) {
            return session;
        }
        assert!(std::time::Instant::now() < deadline, "{}", home.logs());
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Types a line that shows a number until the session shows it: keys typed
/// before the attaching client reads its console do not reach the session.
fn input_reaches_the_session(terminal: &mut Terminal, shell: Shell) {
    for attempt in 1..=10 {
        let line = match shell {
            Shell::PowerShell => format!("3000000 + {attempt}"),
            Shell::Cmd => format!("set /a 3000000+{attempt}"),
        };
        enter(terminal, &line);
        let shown = (3_000_000 + attempt).to_string();
        if terminal
            .wait_for_text(&shown, Duration::from_secs(2))
            .is_ok()
        {
            return;
        }
    }
    panic!(
        "typed input never reached the session: {}",
        terminal.screen()
    );
}
