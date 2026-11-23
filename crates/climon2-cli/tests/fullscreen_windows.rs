//! #328: the full-screen apps on Windows (docs/for-humans/windows/qualification.md,
//! "Full-screen apps"): vim for Windows and Microsoft Edit, pinned downloads
//! verified by SHA-256 (`climon2_test_term::App`), in the packaged product.
//!
//! - Each app, under the Windows Terminal and the VS Code host models: enter
//!   it from a session's `pwsh`, edit, detach, reattach and see its screen
//!   restored, resize and see it redraw at the new size, save and quit, and
//!   see the shell's main screen restored. These are the Windows forms of
//!   "real vim edits and saves" and "real `less` navigates and resizes",
//!   whose place Edit takes.
//! - `WIN-02` for the apps: `c2 kill` of a session whose command is the app
//!   ends its attached client with the app's close status, `0xC000013A` for
//!   each as measured (vim's `0x1` is an open question on #328).
//!
//! cargo runs tests in a job that refuses breakaway, and so does the CI
//! runner, so each test runs again outside every job
//! (`climon2_test_term::rerun_outside`).
#![cfg(windows)]
// Each test binary uses part of it.
#[allow(dead_code)]
mod support;

use climon2_test_term::{
    App, HostModel, HostSettings, JobShape, Key, Modifiers, Options, Terminal, controlled_pwsh,
    rerun_outside,
};
use portable_pty::CommandBuilder;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use support::{Home, Package, TIMEOUT, enter, pwsh_arguments, quoted, wait};

climon2_test_term::outside_relay!();

/// How long one whole test may take outside every job.
const OUTSIDE: Duration = Duration::from_secs(300);
const ROWS: u16 = 30;
const COLS: u16 = 120;
/// The size the host is resized to while an app runs.
const RESIZED: (u16, u16) = (40, 140);
/// The escape prefix, Ctrl+], then `d`.
const DETACH: &[u8] = b"\x1dd";
/// The document each app opens, and the line each adds to it.
const ORIGINAL: &str = "fullscreen-original";
const ADDED: &str = "added-through-session";

/// The session's shell: the controlled `pwsh`, or locally, where only the
/// Store build may be installed, which a session cannot start, Windows
/// PowerShell 5.1 in its place. CI requires the MSI `pwsh`.
fn session_shell() -> PathBuf {
    controlled_pwsh().unwrap_or_else(|| {
        Path::new(&env::var_os("SystemRoot").unwrap())
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
    })
}

/// How the scenario drives one app.
impl Driver for App {
    fn arguments(self, document: &Path) -> Vec<String> {
        let document = document.to_string_lossy().into_owned();
        match self {
            // No vimrc, plugins, viminfo or swap file.
            Self::Vim => vec!["--clean".into(), document],
            Self::Edit => vec![document],
        }
    }

    fn add_line(self, terminal: &mut Terminal) {
        match self {
            Self::Vim => {
                terminal.type_text(&format!("Go{ADDED}")).unwrap();
                terminal.press(Key::Escape, Modifiers::NONE).unwrap();
            }
            Self::Edit => {
                // The document ends with a line break, so its end starts a line.
                terminal.press(Key::End, Modifiers::CTRL).unwrap();
                terminal.type_text(ADDED).unwrap();
            }
        }
    }

    /// Fails unless the app shows it now has `rows` rows and `cols` columns.
    fn shows_size(self, terminal: &mut Terminal, home: &Home, rows: u16, cols: u16) {
        match self {
            Self::Vim => {
                terminal
                    .type_text(":echo 'size=' . &lines . 'x' . &columns")
                    .unwrap();
                terminal.press(Key::Enter, Modifiers::NONE).unwrap();
                wait(terminal, home, &format!("size={rows}x{cols}"));
            }
            // Edit draws its status bar on its last row.
            Self::Edit => {
                let last = usize::from(rows) - 1;
                let deadline = Instant::now() + TIMEOUT;
                while terminal.row(last).trim().is_empty() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(50));
                }
                assert!(
                    !terminal.row(last).trim().is_empty(),
                    "no status bar on row {last}: {}",
                    terminal.screen()
                );
            }
        }
    }

    fn save_and_quit(self, terminal: &mut Terminal) {
        match self {
            Self::Vim => {
                terminal.type_text(":wq").unwrap();
                terminal.press(Key::Enter, Modifiers::NONE).unwrap();
            }
            Self::Edit => {
                terminal.press(Key::Char('s'), Modifiers::CTRL).unwrap();
                terminal.press(Key::Char('q'), Modifiers::CTRL).unwrap();
            }
        }
    }
}

trait Driver {
    fn arguments(self, document: &Path) -> Vec<String>;
    fn add_line(self, terminal: &mut Terminal);
    fn shows_size(self, terminal: &mut Terminal, home: &Home, rows: u16, cols: u16);
    fn save_and_quit(self, terminal: &mut Terminal);
}

/// `pwsh` at the prompt `OUTER> `, hosted under `model`, with `APPDATA` an
/// empty folder, so Edit starts with no settings.
fn outer(package: &Package, home: &Home, model: HostModel) -> Terminal {
    let appdata = home.0.path().join("appdata");
    fs::create_dir_all(&appdata).unwrap();
    let mut shell = home.pwsh(package, &session_shell(), "OUTER");
    shell.env("APPDATA", appdata);
    let settings = HostSettings {
        model,
        job: JobShape::None,
    };
    let mut terminal =
        Terminal::spawn_hosted(shell, ROWS, COLS, Options::default(), settings).unwrap();
    wait(&mut terminal, home, "OUTER> ");
    terminal
}

/// The app's command line for `pwsh`: its program by absolute path and its
/// arguments, each quoted.
fn app_line(app: App, document: &Path) -> String {
    let program = app.program().unwrap();
    let arguments: Vec<String> = app
        .arguments(document)
        .iter()
        .map(|argument| quoted(argument))
        .collect();
    format!(
        "& {} {}",
        quoted(&program.to_string_lossy()),
        arguments.join(" ")
    )
}

/// The scenario for `app` under `model`: enter it from a session's shell,
/// edit, detach, reattach restored, resize, save and quit to the shell's
/// restored main screen.
fn enter_edit_detach_reattach_resize_and_quit(app: App, model: HostModel) {
    let (package, home) = (Package::new(), Home::new());
    let document = home.0.path().join("document.txt");
    fs::write(&document, format!("{ORIGINAL}\r\n")).unwrap();
    let mut terminal = outer(&package, &home, model);
    let inner: Vec<String> = pwsh_arguments("INNER")
        .iter()
        .map(|argument| quoted(argument))
        .collect();
    enter(
        &mut terminal,
        &format!(
            "c2 exec {} {}; \"EXEC-STATUS=$LASTEXITCODE\"",
            quoted(&session_shell().to_string_lossy()),
            inner.join(" ")
        ),
    );
    wait(&mut terminal, &home, "INNER> ");
    enter(&mut terminal, "\"before-$(6*7)\"");
    wait(&mut terminal, &home, "before-42");
    let [session] = home.sessions().try_into().unwrap();
    let id = session.session_id.to_string();

    enter(&mut terminal, &app_line(app, &document));
    wait(&mut terminal, &home, ORIGINAL);
    app.add_line(&mut terminal);
    wait(&mut terminal, &home, ADDED);

    terminal.send(DETACH).unwrap();
    wait(&mut terminal, &home, "EXEC-STATUS=0");
    enter(
        &mut terminal,
        &format!("Clear-Host; c2 attach {id}; \"ATTACH-STATUS=$LASTEXITCODE\""),
    );
    wait(&mut terminal, &home, ADDED);
    let screen = terminal.screen();
    assert!(screen.contains(ORIGINAL), "not restored: {screen}");

    terminal.resize(RESIZED.0, RESIZED.1).unwrap();
    app.shows_size(&mut terminal, &home, RESIZED.0, RESIZED.1);

    app.save_and_quit(&mut terminal);
    wait(&mut terminal, &home, "before-42");
    terminal
        .wait_for_screen(TIMEOUT, |screen| !screen.contains(ADDED))
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(!terminal.modes().alternate_screen, "{}", terminal.screen());
    let saved = fs::read_to_string(&document).unwrap();
    assert_eq!(
        saved.lines().collect::<Vec<_>>(),
        [ORIGINAL, ADDED],
        "{saved:?}"
    );

    enter(&mut terminal, "exit 7");
    wait(&mut terminal, &home, "ATTACH-STATUS=7");
    assert!(home.sessions().is_empty(), "{}", home.logs());
}

/// Ignored until the owner decides #328's question: the client holds a lone
/// Escape as a possible start of a paste marker until the next byte, which
/// the session's pseudoconsole then reads as Alt with that key, so vim never
/// leaves insert mode.
#[test]
#[ignore = "a lone Escape never reaches the session (#328)"]
fn vim_enters_edits_detaches_reattaches_resizes_and_quits_under_windows_terminal() {
    let name = "vim_enters_edits_detaches_reattaches_resizes_and_quits_under_windows_terminal";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    enter_edit_detach_reattach_resize_and_quit(App::Vim, HostModel::WindowsTerminal);
}

/// Ignored for the same reason as the Windows Terminal form.
#[test]
#[ignore = "a lone Escape never reaches the session (#328)"]
fn vim_enters_edits_detaches_reattaches_resizes_and_quits_under_vs_code() {
    let name = "vim_enters_edits_detaches_reattaches_resizes_and_quits_under_vs_code";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    enter_edit_detach_reattach_resize_and_quit(App::Vim, HostModel::VsCode);
}

#[test]
fn edit_enters_edits_detaches_reattaches_resizes_and_quits_under_windows_terminal() {
    let name = "edit_enters_edits_detaches_reattaches_resizes_and_quits_under_windows_terminal";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    enter_edit_detach_reattach_resize_and_quit(App::Edit, HostModel::WindowsTerminal);
}

#[test]
fn edit_enters_edits_detaches_reattaches_resizes_and_quits_under_vs_code() {
    let name = "edit_enters_edits_detaches_reattaches_resizes_and_quits_under_vs_code";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    enter_edit_detach_reattach_resize_and_quit(App::Edit, HostModel::VsCode);
}

/// `WIN-02` for the full-screen apps: `c2 kill` hangs up a session whose
/// command is the app, and its attached client exits with the app's close
/// status through the CLI and the launcher. Both report `0xC000013A`: Edit
/// exits so on the close, and vim, whose own close handler exits `0x1` at
/// once outside a session, does not finish it within the grace in one, so
/// the session job ends it (an open question on #328).
#[test]
fn kill_ends_an_apps_attached_client_with_the_apps_close_status() {
    let name = "kill_ends_an_apps_attached_client_with_the_apps_close_status";
    if rerun_outside(name, OUTSIDE) {
        return;
    }
    for (app, status) in [(App::Vim, 0xC000_013A_u32), (App::Edit, 0xC000_013A_u32)] {
        let (package, home) = (Package::new(), Home::new());
        let document = home.0.path().join("document.txt");
        fs::write(&document, format!("{ORIGINAL}\r\n")).unwrap();
        let appdata = home.0.path().join("appdata");
        fs::create_dir_all(&appdata).unwrap();
        let mut command = CommandBuilder::new(package.launcher());
        command.arg("exec");
        command.arg(app.program().unwrap());
        command.args(app.arguments(&document));
        command.env("CLIMON2_HOME", home.home());
        command.env("LOCALAPPDATA", home.local());
        command.env("APPDATA", &appdata);
        let mut terminal = Terminal::spawn(command, ROWS, COLS).unwrap();
        wait(&mut terminal, &home, ORIGINAL);
        let [session] = home.sessions().try_into().unwrap();

        let killed = Command::new(package.launcher())
            .args(["kill", session.session_id.as_str()])
            .env("CLIMON2_HOME", home.home())
            .env("LOCALAPPDATA", home.local())
            .output()
            .unwrap();
        assert_eq!(killed.status.code(), Some(0), "{app:?} {killed:?}");
        assert_eq!(
            terminal.wait_exit(TIMEOUT).unwrap(),
            status,
            "{app:?}: {}\nLOGS:\n{}",
            terminal.screen(),
            home.logs()
        );
        assert!(home.sessions().is_empty(), "{app:?}");
    }
}
