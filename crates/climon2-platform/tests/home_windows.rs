//! #313: how a climon2 executable on Windows finds its climon2 home and its
//! default executable home from the environment. Each case runs in a child of
//! this test binary, with the environment under test, and the child writes
//! what it found to a file for the parent to check.
#![cfg(windows)]
use climon2_platform::home;
use std::{
    fmt::Debug,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Names the file a child writes its finding to.
const CHILD: &str = "CLIMON2_HOME_TEST_CHILD";

/// In a child, writes what `find` returned and says so; in the parent, false.
fn as_child<T: Debug>(find: impl FnOnce() -> T) -> bool {
    let Some(report) = std::env::var_os(CHILD) else {
        return false;
    };
    fs::write(report, format!("{:?}", find())).unwrap();
    true
}

/// Runs `test` alone in a child of this binary with each named variable set
/// or removed, and `HOME` set to a directory that must never be used.
fn child(test: &str, variables: &[(&str, Option<&str>)]) -> String {
    let scratch = tempfile::tempdir().unwrap();
    let report = scratch.path().join("report");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test])
        .env(CHILD, &report)
        .env("HOME", r"C:\not-the-profile");
    for (name, value) in variables {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{test}: {output:?}");
    fs::read_to_string(report).unwrap()
}

fn found(path: &str) -> String {
    format!("Ok({:?})", PathBuf::from(path))
}

/// #313: `CLIMON2_HOME` names the climon2 home; without it, the home is
/// `.climon2` in `USERPROFILE`, never in `HOME`. The home must be absolute,
/// so a relative override or a missing profile is refused. Nothing is created.
#[test]
fn issue313_the_profile_names_the_climon2_home_on_windows() {
    let resolve = || home::named().map(|layout| layout.root().to_owned());
    if as_child(resolve) {
        return;
    }
    let test = "issue313_the_profile_names_the_climon2_home_on_windows";
    let named = |profile: Option<&str>, climon2: Option<&str>| {
        child(test, &[("USERPROFILE", profile), ("CLIMON2_HOME", climon2)])
    };
    assert_eq!(
        named(Some(r"C:\Users\someone"), Some(r"D:\custom home")),
        found(r"D:\custom home")
    );
    assert_eq!(
        named(Some(r"C:\Users\someone"), None),
        found(r"C:\Users\someone\.climon2")
    );
    let refused = r#"Err(Custom { kind: Other, error: ContractError("path must be nonempty and absolute") })"#;
    assert_eq!(named(Some(r"C:\Users\someone"), Some("relative")), refused);
    assert_eq!(named(Some(r"C:\Users\someone"), Some("")), refused);
    assert_eq!(named(None, None), refused);
    assert!(!Path::new(r"C:\Users\someone\.climon2").exists());
}

/// #313: without `--dir`, an installer's executable home is `climon2` in
/// `LOCALAPPDATA`, which must be absolute. Neither `CLIMON2_HOME` nor the
/// profile moves it. Nothing is created.
#[test]
fn issue313_local_application_data_names_the_default_executable_home_on_windows() {
    if as_child(home::default_executable_home) {
        return;
    }
    let test = "issue313_local_application_data_names_the_default_executable_home_on_windows";
    let named = |local: Option<&str>| {
        child(
            test,
            &[
                ("LOCALAPPDATA", local),
                ("USERPROFILE", Some(r"C:\Users\someone")),
                ("CLIMON2_HOME", Some(r"D:\custom home")),
            ],
        )
    };
    assert_eq!(
        named(Some(r"C:\Users\someone\AppData\Local")),
        found(r"C:\Users\someone\AppData\Local\climon2")
    );
    let refused = r#"Err(Custom { kind: InvalidInput, error: "LOCALAPPDATA must name an absolute directory" })"#;
    assert_eq!(named(Some("relative")), refused);
    assert_eq!(named(Some("")), refused);
    assert_eq!(named(None), refused);
    assert!(!Path::new(r"C:\Users\someone\AppData\Local\climon2").exists());
}
