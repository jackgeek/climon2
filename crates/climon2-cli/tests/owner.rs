//! #133 / ADR 0011: one climon2 home, one host. A command that uses a home
//! another machine or container owns exits 125 with the refusal before it
//! starts, attaches, lists or kills anything; help and version still answer.
//! `c2 home` shows the owner without checking or claiming, and `c2 home
//! release` removes the record on any host, unless sessions run here.
use std::{
    fs,
    path::{Path, PathBuf},
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
                   (machine another-machine, PID namespace 1). climon2 does not work across \
                   machines yet (`c2 relay` is planned). To move this home here, run `c2 home \
                   release`, or `c2 home release --force` if the host that owns it is gone. \
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

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// What `c2 home` prints for the home in `root`, given its owner line and the
/// verdict on this host.
fn shown(root: &Path, owner: &str, verdict: &str) -> String {
    let home = fs::canonicalize(root.join("home")).unwrap();
    let here = climon2_platform::origin::current().unwrap();
    format!(
        "climon2 home: {}\nowner: {owner}\nthis host: {here} ({verdict})\n",
        home.display()
    )
}

/// `c2 home` shows the home, its owner and whether this host is the owner,
/// with exit 0 whoever owns it, and claims nothing.
#[test]
fn issue133_home_shows_the_owner_and_whether_this_host_is_it() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    for name in ["home", "h"] {
        let output = cli(root.path(), &[name]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let none = "none (the next command claims this home)";
        assert_eq!(stdout(&output), shown(root.path(), none, "no owner yet"));
        assert!(!home.join("owner").exists(), "c2 {name} claimed the home");
    }
    assert_eq!(cli(root.path(), &["list"]).status.code(), Some(0));
    let here = climon2_platform::origin::current().unwrap().to_string();
    let output = cli(root.path(), &["home"]);
    assert_eq!(stdout(&output), shown(root.path(), &here, "the owner"));

    fs::write(home.join("owner"), FOREIGN).unwrap();
    let output = cli(root.path(), &["home"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let foreign = "machine another-machine, PID namespace 1";
    assert_eq!(
        stdout(&output),
        shown(root.path(), foreign, "not the owner")
    );

    fs::write(home.join("owner"), "not json").unwrap();
    let output = cli(root.path(), &["home"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let text = stdout(&output);
    assert!(text.contains("\nowner: unreadable ("), "{text}");
    assert!(text.ends_with(" (owner unknown)\n"), "{text}");
    assert_eq!(fs::read_to_string(home.join("owner")).unwrap(), "not json");
}

/// `c2 home release` removes the record on the owner and on any other host,
/// naming whose it was, so the next command claims the home for this host. A
/// home without a record says so.
#[test]
fn issue133_home_release_removes_any_owner_so_the_next_command_claims_the_home() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let output = cli(root.path(), &["home", "release"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(stdout(&output), "this climon2 home has no owner record\n");

    let path = fs::canonicalize(&home).unwrap().join("owner");
    let here = climon2_platform::origin::current().unwrap();
    for (record, owner) in [
        (None, here.to_string()),
        (
            Some(FOREIGN),
            "machine another-machine, PID namespace 1".to_owned(),
        ),
    ] {
        match record {
            Some(record) => fs::write(home.join("owner"), record).unwrap(),
            None => assert_eq!(cli(root.path(), &["list"]).status.code(), Some(0)),
        }
        let output = cli(root.path(), &["h", "release"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            stdout(&output),
            format!(
                "removed the owner record of {owner} from {}; the next command claims this home\n",
                path.display()
            )
        );
        assert!(!home.join("owner").exists());
        assert_eq!(cli(root.path(), &["list"]).status.code(), Some(0));
        let claimed = climon2_core::owner::from_json(&fs::read(home.join("owner")).unwrap());
        assert_eq!(claimed.unwrap(), here);
        fs::remove_file(home.join("owner")).unwrap();
    }
}

/// A corrupt record, or a link at the record's name, is removed itself, and a
/// link's target is never touched.
#[test]
fn issue133_home_release_removes_a_bad_or_linked_record_itself() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir(&home).unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let target = elsewhere.path().join("owner");
    fs::write(&target, FOREIGN).unwrap();
    for linked in [false, true] {
        let link = home.join("owner");
        if linked {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &link).unwrap();
            #[cfg(windows)]
            std::os::windows::fs::symlink_file(&target, &link).unwrap();
        } else {
            fs::write(&link, "not json").unwrap();
        }
        let output = cli(root.path(), &["home", "release"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let text = stdout(&output);
        assert!(
            text.starts_with("removed an unreadable owner record ("),
            "{text}"
        );
        assert!(fs::symlink_metadata(&link).is_err(), "{text}");
        assert_eq!(fs::read_to_string(&target).unwrap(), FOREIGN);
    }
}

/// The home in `root`, created for a test.
fn home_in(root: &Path) -> PathBuf {
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    fs::canonicalize(home).unwrap()
}

/// Writes session record `suffix` into the home in `root`, its socket refusing
/// connections: its daemon is this test process, still running when `alive`
/// and gone otherwise, and `origin` is where it was created.
fn session(root: &Path, suffix: u8, alive: bool, origin: Option<&str>) -> PathBuf {
    use climon2_platform::{process, runtime};
    let sessions = home_in(root).join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let id = format!("12345678-1234-4234-8234-123456789a{suffix:02x}");
    let socket = root.join(format!("{suffix}.sock"));
    drop(runtime::bind(&socket).unwrap());
    let pid = std::process::id();
    let start = match alive {
        true => process::start_time(pid).unwrap().unwrap(),
        false => 1,
    };
    let mut record = serde_json::json!({
        "format_version": 1, "session_id": id, "socket_path": socket,
        "daemon_pid": pid, "daemon_start_time": start,
        "command": "bash", "args": [], "cwd": root, "depth": 1,
        "created_at": "2026-11-26T12:00:00.000Z", "climon2_version": "0.1.0",
    });
    if let Some(identity) = process::linux_identity(pid).unwrap() {
        let mut identity = serde_json::to_value(identity).unwrap();
        if !alive {
            identity["start_ticks"] = serde_json::json!(0);
        }
        record["daemon_linux_identity"] = identity;
    }
    if let Some(origin) = origin {
        record["daemon_origin"] = serde_json::from_str(origin).unwrap();
    }
    let path = sessions.join(format!("{id}.pty"));
    fs::write(&path, record.to_string()).unwrap();
    path
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

/// On a host that is not the owner, `c2 home release` refuses with 125 while
/// the home holds any session record, valid or not, since the owning host may
/// still run them; `--force` releases it anyway, and every record stays. A
/// home whose record cannot be read is treated so too.
#[test]
fn issue133_a_non_owner_releases_a_home_with_sessions_only_with_force() {
    for (record, sessions) in [(FOREIGN, 1), (FOREIGN, 2), ("not json", 1)] {
        let root = tempfile::tempdir().unwrap();
        let home = home_in(root.path());
        fs::write(home.join("owner"), record).unwrap();
        let mut kept = vec![session(root.path(), 1, false, None)];
        if sessions == 2 {
            let invalid = home.join("sessions/12345678-1234-4234-8234-123456789a02.pty");
            fs::write(&invalid, "not metadata").unwrap();
            kept.push(invalid);
        }
        let output = cli(root.path(), &["home", "release"]);
        assert_eq!(output.status.code(), Some(125), "{output:?}");
        let refusal = format!(
            "climon2: error: this climon2 home holds {sessions} session record(s), which the \
             host that owns it may still be running; if that host is gone, run `c2 home \
             release --force` (logs: "
        );
        assert!(stderr(&output).starts_with(&refusal), "{output:?}");
        assert_eq!(fs::read_to_string(home.join("owner")).unwrap(), record);

        let output = cli(root.path(), &["home", "release", "--force"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(stdout(&output).starts_with("removed "), "{output:?}");
        assert!(!home.join("owner").exists());
        assert!(kept.iter().all(|path| path.exists()));
    }
}

/// On the owner, `c2 home release` refuses with 125, even with `--force`,
/// while a session of its own may still run: its daemon is alive here. It
/// releases the home once its sessions are proven ended, or were created on
/// another host, whose records stay even when the next command claims it.
#[test]
fn issue133_the_owner_releases_only_once_its_sessions_have_ended() {
    let other = r#"{"machine_id":"another-machine","pid_namespace":1}"#;
    for (alive, origin) in [(true, None), (false, None), (false, Some(other))] {
        let root = tempfile::tempdir().unwrap();
        let home = home_in(root.path());
        assert_eq!(cli(root.path(), &["list"]).status.code(), Some(0));
        let path = session(root.path(), 1, alive, origin);
        for force in [&["h", "release"][..], &["h", "release", "--force"]] {
            let output = cli(root.path(), force);
            if alive {
                assert_eq!(output.status.code(), Some(125), "{output:?}");
                let refusal = "climon2: error: this host still has 1 climon2 session(s) in \
                               this home that may be running; end them before releasing it \
                               (logs: ";
                assert!(stderr(&output).starts_with(refusal), "{output:?}");
                assert!(home.join("owner").exists());
            } else {
                assert_eq!(output.status.code(), Some(0), "{output:?}");
                assert_eq!(cli(root.path(), &["list"]).status.code(), Some(0));
            }
        }
        // A stale record of this host's own is cleaned up by the next command.
        assert_eq!(
            path.exists(),
            alive || origin.is_some(),
            "{alive} {origin:?}"
        );
    }
}

/// On the owner, `c2 home release` refuses with 125, even with `--force`,
/// while a session of its own answers here; nothing is removed.
#[cfg(unix)]
#[test]
fn issue133_home_release_is_refused_while_sessions_run_here() {
    let root = tempfile::tempdir().unwrap();
    let home = home_in(root.path());
    assert_eq!(cli(root.path(), &["list"]).status.code(), Some(0));
    let owner = fs::read_to_string(home.join("owner")).unwrap();
    let path = session(root.path(), 1, false, None);
    let id = "12345678-1234-4234-8234-123456789a01";
    let socket = root.path().join("1.sock");
    fs::remove_file(&socket).unwrap();
    let reply = format!(r#"{{"session_id":"{id}","climon2_version":"0.1.0","protocol":1}}"#);
    let peer = climon2_test_term::hello_peer(&socket, &reply, 2);
    for arguments in [&["home", "release"][..], &["home", "release", "--force"]] {
        let output = cli(root.path(), arguments);
        assert_eq!(output.status.code(), Some(125), "{output:?}");
        let refusal = "climon2: error: this host still has 1 climon2 session(s) in this home \
                       that may be running; end them before releasing it (logs: ";
        assert!(stderr(&output).starts_with(refusal), "{output:?}");
        assert!(output.stdout.is_empty());
    }
    peer.join().unwrap();
    assert_eq!(fs::read_to_string(home.join("owner")).unwrap(), owner);
    assert!(path.exists());
}
