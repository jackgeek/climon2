use climon2_test_term::{Isolation, Terminal, bash_profiles};
use std::{fs, time::Duration};

/// #21/#31: controlled bash 3.2/5.x ignores user startup files and uses isolated env.
#[test]
fn controlled_bash_ignores_startup_files_and_runs_the_fixture() {
    for profile in bash_profiles() {
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
            .send(
                b"climon2-fixture report shell.json exit 7; printf 'status=%s\\n' \"$?\"; exit 0\n",
            )
            .unwrap();
        terminal.wait_for_text("status=7", second).unwrap();
        assert_eq!(terminal.wait_exit(second).unwrap(), 0);
        assert!(!terminal.screen().contains("UNCONTROLLED_STARTUP"));
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(isolation.home().join("shell.json")).unwrap())
                .unwrap();
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
}

/// #21: missing profiles are explicit local skips and mandatory CI failures.
#[test]
fn profile_selection_enforces_ci_and_reports_local_skips() {
    use std::{env, process::Command};
    let isolation = Isolation::new();
    let missing = isolation.home().join("missing-bash");
    let run = |ci: bool, path: &std::path::Path| {
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
    };
    let local = run(false, &missing);
    assert!(local.status.success());
    assert!(String::from_utf8_lossy(&local.stderr).contains("SKIP:"));
    let ci = run(true, &missing);
    assert!(!ci.status.success());
    assert!(String::from_utf8_lossy(&ci.stderr).contains("required bash missing"));
    let invalid = run(false, std::path::Path::new("/bin/echo"));
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unsupported bash"));
    isolation.assert_clean();
}
