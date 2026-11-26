//! #133 / ADR 0011: one climon2 home, one host. A command that uses a home
//! another machine or container owns exits 125 with the refusal before it
//! starts, attaches, lists or kills anything; help and version still answer.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

/// A record no host running this test can match: another machine ID, and a
/// PID namespace no real process has.
const FOREIGN: &str = r#"{"machine_id":"another-machine","pid_namespace":1}"#;

fn cli(root: &Path, arguments: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_climon2-cli"));
    command
        .args(arguments)
        .env("CLIMON2_HOME", root.join("home"))
        .env_remove("CLIMON2_LOG");
    #[cfg(windows)]
    let _local = {
        let local = climon2_test_term::local_app_data();
        command.env("LOCALAPPDATA", local.path());
        local
    };
    command.output().unwrap()
}

#[test]
fn issue133_a_home_another_host_owns_is_refused_before_anything_else() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("owner"), FOREIGN).unwrap();
    let refusal = "climon2: error: this climon2 home is in use by another machine or container \
                   (machine another-machine, PID namespace 1). To work across machines, use \
                   `c2 relay`. To move this home here, run `c2 home release` on the owning host. \
                   (logs: ";
    for arguments in [
        &["list"][..],
        &["list", "--json"],
        &["exec", "--", "climon2-no-such-command"],
        &["exec", "--headless", "--", "climon2-no-such-command"],
        &["attach", "1"],
        &["kill", "1"],
        &["monitor"],
    ] {
        let output = cli(root.path(), arguments);
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(output.status.code(), Some(125), "{arguments:?}: {stderr}");
        assert!(stderr.starts_with(refusal), "{arguments:?}: {stderr}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert!(!home.join("sessions").exists(), "{arguments:?}");
    }
    for arguments in [&["--version"][..], &["--help"]] {
        let output = cli(root.path(), arguments);
        assert_eq!(output.status.code(), Some(0), "{arguments:?}");
    }
    assert_eq!(fs::read_to_string(home.join("owner")).unwrap(), FOREIGN);
}

/// The first command against a new home claims it for this host, and this
/// host goes on using it.
#[test]
fn issue133_the_first_command_claims_the_home_for_this_host() {
    let root = tempfile::tempdir().unwrap();
    for _ in 0..2 {
        let output = cli(root.path(), &["list", "--json"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"[]\n");
    }
    let record = fs::read(root.path().join("home").join("owner")).unwrap();
    let owner = climon2_core::owner::from_json(&record).unwrap();
    assert_eq!(owner, climon2_platform::origin::current().unwrap());
}
