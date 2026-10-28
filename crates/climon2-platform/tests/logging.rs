use climon2_core::SessionId;
use climon2_platform::logging::{LogFile, Role, prune, timestamp};
use std::{fs, io::Write, os::unix::fs::PermissionsExt, time::SystemTime};

fn id() -> SessionId {
    "12345678-1234-4234-8234-123456789abc".parse().unwrap()
}

/// #25: separate private role files, headers and links survive process handoff.
#[test]
fn diagnostic_roles_have_private_files_and_required_headers() {
    let home = tempfile::tempdir().unwrap();
    let general = LogFile::general(home.path()).unwrap();
    let cli = LogFile::session(home.path(), &id(), Role::Cli, None, Some(general.path())).unwrap();
    let daemon = LogFile::session(home.path(), &id(), Role::Daemon, Some(123), None).unwrap();
    let client = LogFile::session(home.path(), &id(), Role::Client, None, None).unwrap();
    for (log, role) in [(&cli, "cli"), (&daemon, "daemon"), (&client, "client")] {
        let text = fs::read_to_string(log.path()).unwrap();
        assert!(text.contains(&format!("role={role}")));
        assert!(text.contains(&format!("pid={}", std::process::id())));
        assert!(text.contains("version=0.1.0"));
        assert!(text.contains(id().as_str()));
        assert_eq!(
            fs::metadata(log.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let text = fs::read_to_string(cli.path()).unwrap();
    assert!(text.contains(general.path().to_str().unwrap()));
    assert!(
        fs::read_to_string(daemon.path())
            .unwrap()
            .contains("cli_pid=123")
    );
    let mut writer = general.writer();
    writeln!(
        writer,
        "session {} started; continuing in {}",
        id(),
        cli.path().display()
    )
    .unwrap();
    assert!(
        fs::read_to_string(general.path())
            .unwrap()
            .contains("continuing in")
    );
    let now = timestamp().unwrap();
    assert_eq!(now.as_str().len(), 24);
}

/// #25: prune only old non-live session logs and old general logs, never metadata.
#[test]
fn pruning_retains_live_recent_and_unknown_artifacts() {
    let home = tempfile::tempdir().unwrap();
    let old = LogFile::session(home.path(), &id(), Role::Cli, None, None).unwrap();
    let general = LogFile::general(home.path()).unwrap();
    let ancient = filetime::FileTime::from_unix_time(1, 0);
    filetime::set_file_mtime(old.path(), ancient).unwrap();
    filetime::set_file_mtime(general.path(), ancient).unwrap();
    fs::create_dir(home.path().join("sessions")).unwrap();
    let metadata = home.path().join("sessions").join(format!("{}.pty", id()));
    fs::write(&metadata, b"preserve even invalid metadata").unwrap();
    let unknown = home.path().join("logs").join("not-a-session");
    fs::create_dir(&unknown).unwrap();
    fs::write(unknown.join("old"), b"keep").unwrap();
    prune(home.path(), SystemTime::now());
    assert!(old.path().exists());
    assert!(metadata.exists());
    assert!(!general.path().exists());
    assert!(unknown.exists());
    fs::remove_file(metadata).unwrap();
    let recent = LogFile::session(home.path(), &id(), Role::Client, None, None).unwrap();
    prune(home.path(), SystemTime::now());
    assert!(
        old.path().exists(),
        "newest role file protects the entire session"
    );
    filetime::set_file_mtime(recent.path(), ancient).unwrap();
    prune(home.path(), SystemTime::now());
    assert!(!old.path().parent().unwrap().exists());
}

/// #25/#26: invalid metadata entries still protect their logs from pruning.
#[test]
fn dangling_metadata_entry_is_not_absence_of_metadata() {
    let home = tempfile::tempdir().unwrap();
    let log = LogFile::session(home.path(), &id(), Role::Cli, None, None).unwrap();
    filetime::set_file_mtime(log.path(), filetime::FileTime::from_unix_time(1, 0)).unwrap();
    fs::create_dir(home.path().join("sessions")).unwrap();
    let metadata = home.path().join("sessions").join(format!("{}.pty", id()));
    std::os::unix::fs::symlink(home.path().join("missing-metadata"), metadata).unwrap();
    prune(home.path(), SystemTime::now());
    assert!(log.path().exists());
}

/// #25: symlinks must not redirect log writes or pruning outside climon2 home.
#[test]
fn logging_refuses_symlinked_directories_and_files() {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), home.path().join("logs")).unwrap();
    assert!(LogFile::general(home.path()).is_err());
    prune(home.path(), SystemTime::now());
    assert!(outside.path().read_dir().unwrap().next().is_none());
}

/// #25: daemon stderr (including panic diagnostics) shares the capped role log.
#[test]
fn daemon_stderr_and_panics_are_captured() {
    const CHILD: &str = "CLIMON2_LOG_CAPTURE_CHILD";
    if let Some(home) = std::env::var_os(CHILD) {
        let log = LogFile::session(
            std::path::Path::new(&home),
            &id(),
            Role::Daemon,
            Some(123),
            None,
        )
        .unwrap();
        {
            let _capture = log.capture_stderr().unwrap();
            eprintln!("daemon stderr diagnostic");
            let _ = std::panic::catch_unwind(|| panic!("controlled daemon panic"));
        }
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "daemon_stderr_and_panics_are_captured",
            "--nocapture",
        ])
        .env(CHILD, home.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = fs::read_to_string(
        home.path()
            .join("logs")
            .join(id().as_str())
            .join("daemon.log"),
    )
    .unwrap();
    assert!(text.contains("daemon stderr diagnostic"));
    assert!(text.contains("controlled daemon panic"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("controlled daemon panic"));
}
