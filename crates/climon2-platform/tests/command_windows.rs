//! #322: how the daemon finds and starts a session's command on Windows
//! (docs/for-humans/windows/lifecycle.md, "Command resolution and arguments").
//! Commands resolve through `PATH` × `PATHEXT` and never the current
//! directory; `.bat` and `.cmd` run through `cmd.exe` with the standard
//! library's batch escaping, and an argument `cmd.exe` cannot represent is
//! refused.
#![cfg(windows)]
use climon2_platform::{
    command::{self, Refusal},
    pty::{self, ConPty},
};
use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

/// Folders `a`, `b` and `work` in a scratch folder.
struct Folders(tempfile::TempDir);

impl Folders {
    fn new() -> Self {
        let folders = Self(tempfile::tempdir().unwrap());
        for name in ["a", "b", "work"] {
            fs::create_dir(folders.at(name)).unwrap();
        }
        folders
    }

    fn at(&self, relative: &str) -> PathBuf {
        self.0.path().join(relative)
    }

    fn file(&self, relative: &str) -> PathBuf {
        let path = self.at(relative);
        fs::write(&path, b"").unwrap();
        path
    }

    /// A `PATH` value naming `folders`, in order.
    fn path(&self, folders: &[&str]) -> OsString {
        std::env::join_paths(folders.iter().map(|folder| self.at(folder))).unwrap()
    }

    fn search(&self, command: &str, path: &OsStr, pathext: Option<&str>) -> Option<PathBuf> {
        command::search(
            OsStr::new(command),
            &self.at("work"),
            Some(path),
            pathext.map(OsStr::new),
        )
    }
}

/// Windows compares names without case, and the extension found is spelled
/// as `PATHEXT` spells it.
#[track_caller]
fn assert_found(found: Option<PathBuf>, expected: &Path) {
    let found = found.unwrap_or_else(|| panic!("{} was not found", expected.display()));
    assert!(
        found
            .to_string_lossy()
            .eq_ignore_ascii_case(&expected.to_string_lossy()),
        "found {}, expected {}",
        found.display(),
        expected.display()
    );
}

#[test]
fn each_path_folder_is_searched_in_order_and_in_each_the_pathext_extensions_in_order() {
    let folders = Folders::new();
    let first = folders.file(r"a\tool.cmd");
    folders.file(r"a\tool.bat");
    folders.file(r"b\tool.exe");
    let path = folders.path(&["a", "b"]);
    // The earlier folder wins over an earlier extension.
    assert_found(
        folders.search("tool", &path, Some(".EXE;.CMD;.BAT")),
        &first,
    );
    assert_found(
        folders.search("tool", &path, Some(".BAT;.CMD")),
        &folders.at(r"a\tool.bat"),
    );
    assert_found(
        folders.search("tool", &path, Some(".EXE;;.COM")),
        &folders.at(r"b\tool.exe"),
    );
    assert_eq!(folders.search("tool", &path, Some(".COM;.PS1")), None);
}

#[test]
fn the_current_directory_and_relative_path_entries_are_never_searched() {
    let folders = Folders::new();
    folders.file(r"work\tool.exe");
    for path in ["", ".", "work", r".\work", ";;"] {
        assert_eq!(
            folders.search("tool", OsStr::new(path), Some(".EXE")),
            None,
            "PATH={path}"
        );
    }
    assert_eq!(
        command::search(OsStr::new("tool"), &folders.at("work"), None, None),
        None
    );
}

#[test]
fn a_name_with_an_extension_is_tried_as_typed_first_and_one_without_never_is() {
    let folders = Folders::new();
    folders.file(r"a\npm");
    let script = folders.file(r"a\npm.cmd");
    folders.file(r"a\run.exe");
    folders.file(r"a\run.exe.cmd");
    let path = folders.path(&["a"]);
    assert_found(folders.search("npm", &path, Some(".EXE;.CMD")), &script);
    assert_found(
        folders.search("run.exe", &path, Some(".CMD")),
        &folders.at(r"a\run.exe"),
    );
    folders.file(r"a\notes.txt");
    assert_found(
        folders.search("notes.txt", &path, Some(".EXE")),
        &folders.at(r"a\notes.txt"),
    );
}

#[test]
fn a_folder_is_not_a_command() {
    let folders = Folders::new();
    fs::create_dir(folders.at(r"a\tool.exe")).unwrap();
    let program = folders.file(r"b\tool.exe");
    assert_found(
        folders.search("tool", &folders.path(&["a", "b"]), Some(".EXE")),
        &program,
    );
}

#[test]
fn a_command_with_a_folder_is_found_from_the_working_directory_and_not_on_path() {
    let folders = Folders::new();
    fs::create_dir(folders.at(r"work\sub")).unwrap();
    let relative = folders.file(r"work\sub\tool.cmd");
    let absolute = folders.file(r"b\other.exe");
    folders.file(r"a\tool.cmd");
    let path = folders.path(&["a"]);
    for typed in [r"sub\tool", "sub/tool", r".\sub\tool.cmd"] {
        assert_found(folders.search(typed, &path, Some(".CMD")), &relative);
    }
    assert_found(
        folders.search(
            &folders.at(r"b\other").to_string_lossy(),
            &path,
            Some(".EXE"),
        ),
        &absolute,
    );
    assert_eq!(folders.search(r"..\a\missing", &path, Some(".CMD")), None);
}

#[test]
fn without_pathext_the_extensions_are_com_exe_bat_and_cmd() {
    let folders = Folders::new();
    let path = folders.path(&["a"]);
    folders.file(r"a\tool.ps1");
    for pathext in [None, Some("")] {
        assert_eq!(folders.search("tool", &path, pathext), None);
    }
    for extension in ["com", "exe", "bat", "cmd"] {
        let program = folders.file(&format!(r"a\tool.{extension}"));
        assert_found(folders.search("tool", &path, None), &program);
        fs::remove_file(program).unwrap();
    }
}

#[test]
fn quoted_path_entries_are_searched_without_their_quotes() {
    let folders = Folders::new();
    fs::create_dir(folders.at("with space")).unwrap();
    let program = folders.file(r"with space\tool.exe");
    let path = format!("\"{}\"", folders.at("with space").display());
    assert_found(
        folders.search("tool", OsStr::new(&path), Some(".EXE")),
        &program,
    );
}

#[test]
fn a_missing_command_is_not_found_and_a_batch_argument_cmd_cannot_represent_is_refused() {
    let folders = Folders::new();
    let work = folders.at("work");
    let missing = command::launch(OsStr::new(r".\climon2-missing"), &[], &work);
    assert_eq!(missing.err(), Some(Refusal::NotFound));
    folders.file(r"work\script.cmd");
    folders.file(r"work\script.bat");
    folders.file(r"work\program.exe");
    for refused in ["line\nfeed", "carriage\rreturn", "nul\0"] {
        let arguments = [OsString::from("fine"), OsString::from(refused)];
        for script in [r".\script.cmd", r".\script.BAT"] {
            let launch = command::launch(OsStr::new(script), &arguments, &work);
            assert_eq!(launch.err(), Some(Refusal::Unrepresentable), "{refused:?}");
        }
        // A program takes any argument the C runtime can parse.
        let launch = command::launch(OsStr::new(r".\program.exe"), &arguments[..1], &work);
        assert!(launch.is_ok());
    }
    let launch = command::launch(OsStr::new(r".\program"), &[OsString::from("a\nb")], &work);
    assert!(launch.is_ok());
}

/// Every character `cmd.exe` treats specially, a trailing backslash,
/// spaces, quotes and an empty argument.
const CORPUS: &[&str] = &[
    "plain",
    "%",
    "%PATH%",
    "%%",
    "^",
    "^^",
    "&",
    "a&b",
    "|",
    "!",
    "!PATH!",
    "\"",
    "\"\"",
    "a\"b",
    "<",
    ">",
    "(",
    ")",
    "(x)",
    r"trailing\",
    r"two\\",
    r"\\server\share\",
    r#"back\"quote"#,
    "with space",
    "  ",
    "",
    "tab\there",
    "#$*+-./:?@\\_",
    "雪",
    "& echo injected",
    "\" & echo injected & \"",
];

/// Run as an ordinary test it does nothing. The round-trip script runs this
/// binary as this test, with `CLIMON2_ECHO_ARGUMENTS` naming a file, which it
/// writes with every argument after `--`, one JSON string each.
#[test]
fn climon2_echo_arguments() {
    let Some(file) = std::env::var_os("CLIMON2_ECHO_ARGUMENTS") else {
        return;
    };
    let arguments: Vec<String> = std::env::args()
        .skip_while(|argument| argument != "--")
        .skip(1)
        .collect();
    fs::write(file, serde_json::to_vec(&arguments).unwrap()).unwrap();
}

/// A batch file that records the command line `cmd.exe` was given and runs
/// this test binary with its arguments, as `climon2_echo_arguments`.
fn round_trip_script(folder: &Path) -> PathBuf {
    let script = folder.join("round trip.cmd");
    let echo = std::env::current_exe().unwrap();
    let text = format!(
        "@setlocal EnableDelayedExpansion\r\n\
         @>\"%~dp0line.txt\" echo(!CMDCMDLINE!\r\n\
         @endlocal\r\n\
         @set \"CLIMON2_ECHO_ARGUMENTS=%~dp0echoed.json\"\r\n\
         @\"{}\" --exact climon2_echo_arguments --nocapture --test-threads=1 -- %*\r\n",
        echo.display()
    );
    fs::write(&script, text).unwrap();
    script
}

fn conpty() -> ConPty {
    ConPty::load(climon2_test_term::ConPty::pinned().unwrap().folder()).unwrap()
}

/// Runs `launch` in a pseudoconsole, as the daemon does, until it exits.
fn run(launch: &command::Launch, cwd: &Path) -> u32 {
    let hosted = pty::open(conpty(), 24, 80)
        .unwrap()
        .start(launch, cwd, &[])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        // The pseudoconsole's output must keep flowing for the script to end.
        let _ = hosted.read(&mut [0u8; 4096]);
        if let Some(status) = hosted.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "the script did not end");
        hosted.wait_readable(Duration::from_millis(20));
    }
}

#[test]
fn a_batch_files_arguments_round_trip_with_the_standard_librarys_escaping() {
    let folders = Folders::new();
    let script = round_trip_script(&folders.at("work"));
    let arguments: Vec<OsString> = CORPUS.iter().map(OsString::from).collect();

    // The command line the standard library gives `cmd.exe`.
    let status = Command::new(&script).args(&arguments).status().unwrap();
    assert!(status.success());
    let by_std = fs::read_to_string(folders.at(r"work\line.txt")).unwrap();
    fs::remove_file(folders.at(r"work\echoed.json")).unwrap();

    let launch = command::launch(script.as_os_str(), &arguments, &folders.at("a")).unwrap();
    assert_eq!(run(&launch, &folders.at("a")), 0);
    let line = fs::read_to_string(folders.at(r"work\line.txt")).unwrap();
    assert_eq!(line, by_std);
    let echoed: Vec<String> =
        serde_json::from_slice(&fs::read(folders.at(r"work\echoed.json")).unwrap()).unwrap();
    assert_eq!(echoed, CORPUS);
}

#[test]
fn a_program_is_started_by_its_resolved_path_and_sees_its_first_argument_as_typed() {
    let folders = Folders::new();
    let typed = "PowerShell";
    let arguments = [
        "-NoLogo",
        "-NoProfile",
        "-Command",
        "[Environment]::CommandLine | Set-Content seen.txt",
    ]
    .map(OsString::from);
    let launch = command::launch(OsStr::new(typed), &arguments, &folders.at("work")).unwrap();
    assert_eq!(run(&launch, &folders.at("work")), 0);
    let seen = fs::read_to_string(folders.at(r"work\seen.txt")).unwrap();
    assert!(
        seen.starts_with("PowerShell -NoLogo -NoProfile -Command \"["),
        "{seen}"
    );
}
