use climon2_test_term::{Isolation, Terminal, bin_path};
use std::{env, fs, time::Duration};

/// #33/#38: controlled PTYs preserve the explicit coverage sink for installed binaries.
#[test]
fn controlled_terminal_preserves_the_coverage_profile_sink() {
    const CHILD: &str = "CLIMON2_PROFILE_TEST_CHILD";
    if env::var_os(CHILD).is_some() {
        let isolation = Isolation::new();
        let report = isolation.home().join("profile-env.json");
        let mut command = isolation.command(bin_path("climon2-fixture"));
        command.arg("report");
        command.arg(&report);
        command.args(["exit", "0"]);
        let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
        assert_eq!(terminal.wait_exit(Duration::from_secs(3)).unwrap(), 0);
        let value: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
        assert_eq!(
            value["env"]["LLVM_PROFILE_FILE"],
            env::var("LLVM_PROFILE_FILE").unwrap()
        );
        drop(terminal);
        isolation.assert_clean();
        return;
    }
    let sink = Isolation::new();
    let output = std::process::Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "controlled_terminal_preserves_the_coverage_profile_sink",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("LLVM_PROFILE_FILE", sink.home().join("%p-%m.profraw"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}
