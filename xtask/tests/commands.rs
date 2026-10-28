use std::process::Command;

#[test]
fn help_reports_local_and_distribution_gate_scope() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("--help")
        .output()
        .expect("run xtask help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");
    assert!(help.contains("fmt, clippy and workspace tests"));
    assert!(help.contains("including packaged acceptance"));
    assert!(help.contains("dist builds verified universal macOS release artifacts"));
}

#[test]
fn invalid_arguments_fail_before_running_checks() {
    for args in [vec!["dist", "unexpected"], vec!["check", "unexpected"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(args)
            .output()
            .expect("run xtask");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("unknown arguments"));
    }
}
