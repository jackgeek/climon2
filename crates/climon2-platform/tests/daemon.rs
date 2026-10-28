#![cfg(target_os = "macos")]

use climon2_platform::daemon::{self, Readiness, ReadinessWriter};
use std::{
    fs::File,
    io::Read,
    os::fd::FromRawFd,
    process::Command,
    time::{Duration, Instant},
};

#[test]
fn issue19_readiness_failure_writer_preserves_all_launch_codes_over_real_pipe() {
    for (code, encoded_code) in [(125, 0x7d), (126, 0x7e), (127, 0x7f)] {
        let mut descriptors = [-1; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let mut reader = unsafe { File::from_raw_fd(descriptors[0]) };
        let mut writer =
            unsafe { ReadinessWriter::from_inherited(u32::try_from(descriptors[1]).unwrap()) }
                .unwrap();
        writer
            .send(&Readiness::Failed {
                code,
                message: "cannot start".into(),
            })
            .unwrap();
        drop(writer);

        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        let mut expected = vec![b'F', 0, 0, 0, encoded_code, 0, 0, 0, 12];
        expected.extend_from_slice(b"cannot start");
        assert_eq!(bytes, expected);
    }
}

fn launch(mode: &str) -> daemon::PendingDaemon {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--ignored",
        "--exact",
        "owned_daemon_fixture",
        "--nocapture",
    ]);
    command.env("CLIMON2_DAEMON_FIXTURE", mode);
    daemon::spawn(command, |command, fd| {
        command.env("CLIMON2_READINESS_FIXTURE_FD", fd.to_string());
    })
    .unwrap()
}

#[test]
#[ignore = "owned subprocess fixture"]
fn owned_daemon_fixture() {
    let fd = std::env::var("CLIMON2_READINESS_FIXTURE_FD")
        .unwrap()
        .parse()
        .unwrap();
    let mut writer = unsafe { ReadinessWriter::from_inherited(fd) }.unwrap();
    assert_eq!(
        unsafe { libc::getsid(0) },
        i32::try_from(std::process::id()).unwrap()
    );
    match std::env::var("CLIMON2_DAEMON_FIXTURE").unwrap().as_str() {
        "ready" => writer.send(&Readiness::Ready).unwrap(),
        "fail" => writer
            .send(&Readiness::Failed {
                code: 126,
                message: "cannot run command".into(),
            })
            .unwrap(),
        "empty" => return,
        "partial" => {
            assert_eq!(
                unsafe { libc::write(i32::try_from(fd).unwrap(), b"F".as_ptr().cast(), 1) },
                1
            );
            std::mem::forget(writer);
        }
        "invalid" => {
            assert_eq!(
                unsafe { libc::write(i32::try_from(fd).unwrap(), b"X".as_ptr().cast(), 1) },
                1
            );
            std::mem::forget(writer);
        }
        _ => unreachable!(),
    }
    if std::env::var("CLIMON2_DAEMON_FIXTURE").unwrap() == "ready" {
        return;
    }
    std::thread::sleep(Duration::from_secs(30));
}

#[test]
fn issue19_daemon_exec_is_detached_and_readiness_preserves_launch_failure() {
    let daemon = launch("ready");
    let pid = daemon.id();
    assert_eq!(daemon.wait_ready(&[]).unwrap(), Readiness::Ready);
    climon2_platform::process::wait(pid).unwrap();
    assert_eq!(
        launch("fail").wait_ready(&[]).unwrap(),
        Readiness::Failed {
            code: 126,
            message: "cannot run command".into()
        }
    );
}

#[test]
fn issue19_empty_readiness_kills_daemon_and_cleans_metadata_before_socket() {
    let root = tempfile::Builder::new()
        .prefix(".dr")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    let metadata = root.path().join("metadata");
    let socket = root.path().join("socket");
    std::fs::write(&metadata, b"").unwrap();
    std::fs::write(&socket, b"").unwrap();
    assert!(launch("empty").wait_ready(&[&metadata, &socket]).is_err());
    assert!(!metadata.exists());
    assert!(!socket.exists());
}

/// #19: an unknown readiness reply is a startup failure: the daemon is killed and its files removed.
#[test]
fn issue19_invalid_readiness_reply_kills_daemon_and_cleans_files() {
    let root = tempfile::Builder::new()
        .prefix(".dr")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    let metadata = root.path().join("metadata");
    std::fs::write(&metadata, b"").unwrap();
    let daemon = launch("invalid");
    let pid = daemon.id();
    let error = daemon.wait_ready(&[&metadata]).unwrap_err();
    assert_eq!(error.to_string(), "invalid daemon readiness reply");
    assert!(!metadata.exists());
    assert!(
        climon2_platform::process::start_time(pid)
            .unwrap()
            .is_none()
    );
}

#[test]
fn issue19_readiness_uses_real_five_second_absolute_deadline() {
    let start = Instant::now();
    let daemon = launch("partial");
    let pid = daemon.id();
    let error = daemon.wait_ready(&[]).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(start.elapsed() >= Duration::from_secs(5));
    assert!(start.elapsed() < Duration::from_secs(8));
    assert!(
        climon2_platform::process::start_time(pid)
            .unwrap()
            .is_none()
    );
}
