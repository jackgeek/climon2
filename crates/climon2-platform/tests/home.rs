//! #155: how every climon2 executable finds its climon2 home from its
//! environment and its install layout from its own path. Each case runs in a
//! child of this test binary, with the environment and path under test, and
//! the child writes what it found to a file for the parent to check.
use climon2_core::layout::VersionBinary;
use climon2_platform::home;
use std::{
    ffi::OsStr,
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

/// Runs `test` alone in `program`, a copy of this binary, with `HOME` and
/// `CLIMON2_HOME` set or removed, and returns what the child found.
fn child(program: &Path, test: &str, user: Option<&OsStr>, climon2: Option<&OsStr>) -> String {
    let scratch = tempfile::tempdir().unwrap();
    let report = scratch.path().join("report");
    let mut command = Command::new(program);
    command.args(["--exact", test]).env(CHILD, &report);
    for (name, value) in [("HOME", user), ("CLIMON2_HOME", climon2)] {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{test}: {output:?}");
    fs::read_to_string(report).unwrap()
}

fn this_binary() -> PathBuf {
    std::env::current_exe().unwrap()
}

/// #155: `CLIMON2_HOME` names the climon2 home, whatever `HOME` is; without
/// it, the home is `.climon2` in `HOME`. The home must be absolute, so a
/// relative override or a missing `HOME` is refused. Nothing is created.
#[test]
fn issue155_the_environment_names_the_climon2_home() {
    let found = || home::named().map(|layout| layout.root().to_owned());
    if as_child(found) {
        return;
    }
    let test = "issue155_the_environment_names_the_climon2_home";
    let named = |user: Option<&str>, climon2: Option<&str>| {
        child(
            &this_binary(),
            test,
            user.map(OsStr::new),
            climon2.map(OsStr::new),
        )
    };
    assert_eq!(
        named(Some("/user"), Some("/custom home")),
        r#"Ok("/custom home")"#
    );
    assert_eq!(named(Some("/user"), None), r#"Ok("/user/.climon2")"#);
    let refused = r#"Err(Custom { kind: Other, error: ContractError("path must be nonempty and absolute") })"#;
    assert_eq!(named(Some("/user"), Some("relative")), refused);
    assert_eq!(named(Some("/user"), Some("")), refused);
    assert_eq!(named(None, None), refused);
    assert!(!Path::new("/user/.climon2").exists());
}

/// #313: without `--dir`, an installer's executable home is
/// `.local/share/climon2` in `HOME`, which must be absolute. `CLIMON2_HOME`
/// does not move it. Nothing is created.
#[test]
fn issue313_the_environment_names_the_default_executable_home() {
    if as_child(home::default_executable_home) {
        return;
    }
    let test = "issue313_the_environment_names_the_default_executable_home";
    let named = |user: Option<&str>| {
        child(
            &this_binary(),
            test,
            user.map(OsStr::new),
            Some(OsStr::new("/custom home")),
        )
    };
    assert_eq!(named(Some("/user")), r#"Ok("/user/.local/share/climon2")"#);
    let refused =
        r#"Err(Custom { kind: InvalidInput, error: "HOME must name an absolute directory" })"#;
    assert_eq!(named(Some("relative")), refused);
    assert_eq!(named(Some("")), refused);
    assert_eq!(named(None), refused);
    assert!(!Path::new("/user/.local").exists());
}

/// #155: the canonical climon2 home is created when missing, and is the real
/// path of the named home: symbolic links in it are resolved.
#[test]
fn issue155_the_canonical_climon2_home_is_created_and_resolved() {
    if as_child(home::canonical) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let real = root.path().canonicalize().unwrap().join("real");
    fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, root.path().join("link")).unwrap();
    let found = child(
        &this_binary(),
        "issue155_the_canonical_climon2_home_is_created_and_resolved",
        Some(root.path().join("link").as_os_str()),
        None,
    );
    let expected = real.join(".climon2");
    assert_eq!(found, format!("Ok({expected:?})"));
    assert!(expected.is_dir());
}

/// #155: an executable in a version folder finds its install layout, the
/// executable home and version around its own real path.
#[test]
fn issue155_an_executable_finds_its_layout_from_its_own_path() {
    let found = || {
        home::executable_layout(VersionBinary::Monitor).map(|layout| {
            layout.map(|layout| (layout.root().to_owned(), layout.version().to_string()))
        })
    };
    if as_child(found) {
        return;
    }
    let root = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let executable_home = root.path().canonicalize().unwrap();
    fs::create_dir(executable_home.join("1.2.3")).unwrap();
    let monitor = executable_home.join("1.2.3/climon2-monitor");
    fs::hard_link(this_binary(), &monitor).unwrap();
    let found = child(
        &monitor,
        "issue155_an_executable_finds_its_layout_from_its_own_path",
        None,
        None,
    );
    assert_eq!(found, format!("Ok(Ok(({executable_home:?}, \"1.2.3\")))"));
}

/// #155: an executable outside a version folder, such as this test binary,
/// has no install layout, and the layout rule says why.
#[test]
fn issue155_an_executable_outside_a_version_folder_has_no_layout() {
    let refusal = home::executable_layout(VersionBinary::Cli)
        .unwrap()
        .unwrap_err();
    assert_eq!(refusal.to_string(), "expected canonical climon2-cli path");
}
