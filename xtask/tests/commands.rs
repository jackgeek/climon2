use std::process::Command;

#[test]
fn help_reports_the_incremental_gate_scope() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("--help")
        .output()
        .expect("run xtask help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");
    assert!(help.contains("fmt, clippy and workspace tests"));
    assert!(help.contains("not yet a full release gate"));
}

#[test]
fn invalid_arguments_fail_before_running_checks() {
    for args in [vec!["dist"], vec!["check", "unexpected"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(args)
            .output()
            .expect("run xtask");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("unknown arguments"));
    }
}

#[test]
fn check_propagates_preflight_failure_without_starting_cargo() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("check")
        .env("CLIMON2_TEST_BASH", env!("CARGO_BIN_EXE_xtask"))
        .output()
        .expect("run failing xtask check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--version failed"));
    assert!(!stderr.contains("> cargo"));
}
