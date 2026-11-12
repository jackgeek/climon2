use climon2_test_term::{Isolation, Terminal, controlled_bash};
use std::{
    env, fs,
    path::Path,
    process::{Command, Output},
    time::Duration,
};

/// #21/#31: controlled bash 5.x ignores user startup files and uses isolated env.
#[test]
fn controlled_bash_ignores_startup_files_and_runs_the_fixture() {
    let Some(profile) = controlled_bash() else {
        return;
    };
    let isolation = Isolation::new();
    for file in [".bashrc", ".bash_profile", ".profile"] {
        fs::write(isolation.home().join(file), "echo UNCONTROLLED_STARTUP\n").unwrap();
    }

    let mut terminal = Terminal::spawn(isolation.bash(&profile), 24, 160).unwrap();
    let second = Duration::from_secs(1);
    terminal.wait_for_text("CLIMON2-PROMPT> ", second).unwrap();
    terminal.send(b"echo $((6*7))-ok\n").unwrap();
    terminal.wait_for_text("42-ok", second).unwrap();
    terminal
        .send(b"climon2-fixture report shell.json exit 7; printf 'status=%s\\n' \"$?\"; exit 0\n")
        .unwrap();
    terminal.wait_for_text("status=7", second).unwrap();
    assert_eq!(terminal.wait_exit(second).unwrap(), 0);
    assert!(!terminal.screen().contains("UNCONTROLLED_STARTUP"));
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(isolation.home().join("shell.json")).unwrap()).unwrap();
    assert_eq!(report["env"]["HOME"], isolation.home().to_str().unwrap());
    assert_eq!(
        report["env"]["CLIMON2_HOME"],
        isolation.climon2_home().to_str().unwrap()
    );
    assert_eq!(report["env"]["TERM"], "xterm-256color");
    assert_eq!(report["env"]["LANG"], "en_US.UTF-8");
    assert_eq!(report["env"]["PS1"], "CLIMON2-PROMPT> ");
    assert_eq!(report["env"]["CLIMON2_TEST_ID"], isolation.id());
    assert!(report["env"].get("RUSTUP_HOME").is_none());
    drop(terminal);
    isolation.assert_clean();
}

/// #21: missing profiles are explicit local skips and mandatory CI failures.
#[test]
fn profile_selection_enforces_ci_and_reports_local_skips() {
    let isolation = Isolation::new();
    let missing = isolation.home().join("missing-bash");
    let local = select_profile(false, &missing);
    assert!(local.status.success());
    assert!(String::from_utf8_lossy(&local.stderr).contains("SKIP:"));
    let ci = select_profile(true, &missing);
    assert!(!ci.status.success());
    assert!(String::from_utf8_lossy(&ci.stderr).contains("required bash missing"));
    let invalid = select_profile(false, Path::new("/bin/echo"));
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unsupported bash"));
    isolation.assert_clean();
}

/// Interactive qualification is GNU bash 5.x only: macOS `/bin/bash` (3.2) is
/// rejected even when requested explicitly, locally as well as in CI.
#[test]
#[cfg(target_os = "macos")]
fn macos_bash_3_2_is_not_an_interactive_profile() {
    let rejected = select_profile(false, Path::new("/bin/bash"));
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("unsupported bash /bin/bash"));
}

#[test]
#[cfg(target_os = "linux")]
fn linux_system_bash_is_a_qualified_interactive_profile() {
    let accepted = select_profile(false, Path::new("/bin/bash"));
    assert!(accepted.status.success(), "{accepted:?}");
}

fn select_profile(ci: bool, path: &Path) -> Output {
    let mut command = Command::new(env::current_exe().unwrap());
    command.args([
        "--exact",
        "controlled_bash_ignores_startup_files_and_runs_the_fixture",
        "--nocapture",
    ]);
    command.env("CLIMON2_TEST_BASH", path);
    command.env_remove("CI");
    if ci {
        command.env("CI", "1");
    }
    command.output().unwrap()
}
