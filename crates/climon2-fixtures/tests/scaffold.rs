/// #30 only: these are not yet the terminal-contract fixtures from #21.
#[test]
fn unfinished_fixtures_fail_explicitly() {
    for binary in [
        env!("CARGO_BIN_EXE_climon2-fixture"),
        env!("CARGO_BIN_EXE_climon2-tui-fixture"),
    ] {
        let output = std::process::Command::new(binary)
            .output()
            .expect("run fixture");
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("fixture is not implemented"),
            "{output:?}"
        );
    }
}
