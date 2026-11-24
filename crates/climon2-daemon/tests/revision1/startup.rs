//! Daemon lifecycle over revision 1 (#103), startup: readiness and its
//! files, long paths, the daemon's ignored signals, probes, the hello
//! deadline and traffic that never completes a hello.
use super::{
    assert_final_then_closed, encoded, exited,
    interactive::{Attached, geometry, ready_attachment, rejected},
    kill, message, open, raw_frame, request,
};
use crate::{ID, Session};
use climon2_core::{
    metadata::SessionMetadata,
    wire::{ControlRecord, Mode, Outcome, Response, RevocationReason, ServerMessage, Success},
};
use climon2_platform::process;
use std::{
    fs,
    io::{ErrorKind, Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    thread,
    time::{Duration, Instant},
};

/// COM-06 (Contract: Command final is the last record).
/// #15/#19/#26 over revision 1: the real binary publishes private (0600)
/// session metadata naming its PID, start time and release in a private
/// sessions directory, then serves a revision-1 controller on the real PTY.
/// The command's last bytes reach it before its Command final with the
/// exact exit, after which control ends with the session and the stream
/// closes: nothing follows the result. Every session file is cleaned up.
#[test]
fn readiness_publishes_session_metadata_and_command_final_is_last() {
    let mut session = Session::start("exit");
    let metadata =
        SessionMetadata::from_json(&session.metadata, &fs::read(&session.metadata).unwrap())
            .unwrap();
    assert_eq!(metadata.daemon_pid, session.pid);
    assert_eq!(
        Some(metadata.daemon_start_time),
        process::start_time(session.pid).unwrap()
    );
    assert_eq!(metadata.climon2_version.as_str(), crate::VERSION);
    let sessions = session.directory.path().join("sessions");
    assert_eq!(fs::read_dir(&sessions).unwrap().count(), 1);
    for (path, mode) in [(&session.metadata, 0o600), (&sessions, 0o700)] {
        let permissions = fs::metadata(path).unwrap().permissions();
        assert_eq!(permissions.mode() & 0o777, mode);
    }
    let mut attached = ready_attachment(&session, &[]);
    attached.typed(b"go\n");
    attached.ended(&exited(42));
    assert_eq!(crate::count(&attached.display, b"final-bytes"), 1);
    session.finish();
}

/// Daemon lifecycle (#103), startup; no matrix row.
/// #18/#35 over revision 1: a home and executable path longer than 103
/// bytes and a session socket of exactly 103 bytes start the session
/// without enlarging the helper's startup socket, which is gone once the
/// session is ready; a revision-1 controller is served through that socket
/// to the command's exact exit, and every owned file and process is cleaned
/// up.
#[test]
fn long_paths_and_a_maximum_length_session_socket_serve_a_controller() {
    let mut session = Session::start("long-paths");
    let home = session.directory.path().as_os_str().as_encoded_bytes();
    assert!(home.len() > 103);
    assert_eq!(session.socket.as_os_str().as_encoded_bytes().len(), 103);
    assert!(
        !session.startup_socket.exists(),
        "startup socket outlived readiness"
    );
    let mut attached = ready_attachment(&session, &[]);
    attached.typed(b"go\n");
    attached.ended(&exited(42));
    session.finish();
}

/// Daemon lifecycle (#103), startup; no matrix row.
/// #19 over revision 1: SIGHUP, SIGINT and SIGPIPE sent to the daemon are
/// ignored, not treated as departures: the daemon still answers a hello,
/// and its controller's input still reaches the command, which reports its
/// own exit.
#[test]
fn the_daemon_ignores_hup_int_and_pipe_and_its_controller_goes_on() {
    let mut session = Session::start("exit");
    let mut attached = ready_attachment(&session, &[]);
    for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGPIPE] {
        assert_eq!(unsafe { libc::kill(session.pid as i32, signal) }, 0);
    }
    drop(session.connect());
    attached.typed(b"go\n");
    attached.ended(&exited(42));
    session.finish();
}

/// Reads `stream` to its end, which must come without a byte: the daemon
/// answers traffic that never completes a hello by closing the connection,
/// never with a hello reply or any other frame. It may close with the
/// peer's bytes unread, which resets the connection.
fn assert_closed_without_hello(stream: &mut UnixStream) {
    let mut bytes = Vec::new();
    if let Err(error) = stream.read_to_end(&mut bytes) {
        assert_eq!(error.kind(), ErrorKind::ConnectionReset, "{error}");
    }
    assert_eq!(bytes, b"", "the daemon answered");
}

/// DSC-04 (Contract: the frozen two-second hello deadline with bounded probes)
/// and COM-03 (Contract: an interactive Open takes control).
/// #20/#27 over revision 1: probes and a hello that never finishes never
/// hold the session's revision-1 controller. Eight probes are answered
/// while a hello trickles a byte every 50 ms, and a second interactive
/// Open takes control at once. The trickling hello is closed without a
/// byte no sooner than two seconds after it connected, its deadline
/// running from accept however recently it wrote. The first attachment,
/// revoked by the takeover, can no longer type (`not_controller`); the
/// second's input reaches the command, and both get its Command final.
#[test]
fn probes_and_an_unfinished_hello_never_hold_a_revision1_controller() {
    let mut session = Session::start("exit");
    drop(session.connect());
    assert!(session.metadata.exists());
    let mut first = ready_attachment(&session, &[]);
    let mut slow = UnixStream::connect(&session.socket).unwrap();
    let accepted = Instant::now();
    slow.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let trickle = crate::trickle(slow.try_clone().unwrap());
    for _ in 0..8 {
        drop(session.connect());
    }
    let mut second = ready_attachment(&session, &[]);
    first.revoked(RevocationReason::Takeover);
    assert_closed_without_hello(&mut slow);
    assert!(accepted.elapsed() >= Duration::from_secs(2));
    trickle.join().unwrap();
    rejected(first.input(b"ignored\n"), "not_controller");
    second.typed(b"go\n");
    second.ended(&exited(42));
    assert_final_then_closed(&mut first.stream, &exited(42));
    session.finish();
}

/// DSC-04, COM-01 (Contract after retirement) and COM-02 (Contract:
/// framing failures before hello, connection effects).
/// #20 over revision 1: traffic that never completes a hello closes only
/// its own connection. A revision-1 Open or Kill or a retired private
/// Attach sent instead of a hello, a hello whose JSON is malformed or whose
/// protocol is not 1, and a first frame of zero length or over the
/// 1,048,576-byte hello limit are each refused: the connection closes
/// without a byte. The session and its revision-1 controller go on.
#[test]
fn traffic_instead_of_a_hello_closes_only_its_connection() {
    let mut session = Session::start("exit");
    let mut attached = ready_attachment(&session, &[]);
    let refused = [
        encoded(&open(1, &[], &[], Mode::Monitor).to_frame()),
        encoded(&kill(1, 1, &[]).to_frame()),
        raw_frame(0x10, br#"{"cols":80,"rows":24}"#),
        raw_frame(0x01, b"["),
        raw_frame(0x01, br#"{"protocol":2}"#),
        vec![0, 0, 0, 0],
        vec![0, 0x10, 0, 1],
    ];
    for bytes in refused {
        let mut probe = UnixStream::connect(&session.socket).unwrap();
        probe
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        probe.write_all(&bytes).unwrap();
        assert_closed_without_hello(&mut probe);
    }
    attached.typed(b"go\n");
    attached.ended(&exited(42));
    session.finish();
}

/// DSC-04 and COM-01 (Contract: the frozen hello limit, at the limit): a
/// hello whose frame is exactly 1,048,576 bytes, its type and a body padded
/// with JSON whitespace, is answered, and revision 1 follows it on the same
/// connection. One byte more is refused
/// (`traffic_instead_of_a_hello_closes_only_its_connection`).
#[test]
fn a_hello_at_the_frozen_limit_is_answered() {
    let mut session = Session::start("exit");
    let mut body = br#"{"protocol":1"#.to_vec();
    body.resize(1_048_574, b' ');
    body.push(b'}');
    let mut stream = UnixStream::connect(&session.socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    stream.write_all(&raw_frame(0x01, &body)).unwrap();
    crate::receive_hello(&mut stream);
    request(&mut stream, &open(1, &[], &[], Mode::Monitor));
    assert!(
        matches!(
            message(&mut stream),
            ServerMessage::Response(Response {
                outcome: Outcome::Ok(Success::Opened { .. }),
                ..
            })
        ),
        "the Open after the hello was not answered"
    );
    let mut attached = ready_attachment(&session, &[]);
    attached.typed(b"go\n");
    attached.ended(&exited(42));
    session.finish();
}

/// Daemon lifecycle (#103), startup; no matrix row.
/// #15/#29 over revision 1: the helper restores an absent, empty or
/// non-UTF-8 SHELL and the exact literal OS arguments, cwd and inherited
/// environment, an explicit logging filter applies, and the daemon log holds
/// none of them. A revision-1 controller serves the session to its end.
#[cfg(feature = "test-hooks")]
#[test]
fn runtime_argv_cwd_and_inherited_environment_are_exact_not_display_metadata() {
    use std::{
        collections::BTreeMap,
        ffi::OsString,
        os::unix::ffi::{OsStrExt, OsStringExt},
    };
    for shell in [
        None,
        Some(OsString::new()),
        Some(OsString::from_vec(b"shell-\xff".to_vec())),
    ] {
        let mut session = Session::configured("literal", |command, _| {
            command.env(
                "SECRET_ENVIRONMENT",
                OsString::from_vec(b"private-\xfe".to_vec()),
            );
            command.env("CLIMON2_LOG", "trace");
            command.args(["--log", "warn"]);
            if let Some(shell) = &shell {
                command.env("SHELL", shell);
            }
        });
        let mut attached = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
        attached.confirm_ready();
        let home = fs::canonicalize(session.directory.path()).unwrap();
        let report = home.join("report");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !report.with_extension("ready").exists() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        let bytes = fs::read(&report).unwrap();
        let mut input = bytes.as_slice();
        let mut fields = Vec::new();
        while !input.is_empty() {
            let mut size = [0; 4];
            input.read_exact(&mut size).unwrap();
            let mut field = vec![0; u32::from_be_bytes(size) as usize];
            input.read_exact(&mut field).unwrap();
            fields.push(field);
        }
        let cwd = home.join("cwd space --literal é");
        assert_eq!(fields[0], cwd.as_os_str().as_bytes());
        assert_eq!(fields[1], crate::daemon_fixture().as_os_str().as_bytes());
        assert_eq!(
            &fields[2..6],
            &[
                b"".to_vec(),
                b"space argument".to_vec(),
                b"--looks-like-option".to_vec(),
                b"invalid-\xff".to_vec()
            ]
        );
        assert_eq!(fields[6], b"ENVIRONMENT");
        let environment: BTreeMap<_, _> = fields[7..]
            .chunks_exact(2)
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect();
        assert_eq!(
            environment.get(b"SHELL".as_slice()),
            shell
                .as_ref()
                .map(|value| value.as_os_str().as_bytes().to_vec())
                .as_ref()
        );
        assert_eq!(
            environment.get(b"SECRET_ENVIRONMENT".as_slice()).unwrap(),
            b"private-\xfe"
        );
        let mut expected: BTreeMap<Vec<u8>, Vec<u8>> = std::env::vars_os()
            .map(|(key, value)| {
                (
                    key.as_os_str().as_bytes().to_vec(),
                    value.as_os_str().as_bytes().to_vec(),
                )
            })
            .collect();
        expected.remove(b"SHELL".as_slice());
        expected.insert(b"DAEMON_FIXTURE_MODE".to_vec(), b"literal".to_vec());
        expected.insert(
            b"DAEMON_REPORT".to_vec(),
            report.as_os_str().as_bytes().to_vec(),
        );
        expected.insert(b"SECRET_ENVIRONMENT".to_vec(), b"private-\xfe".to_vec());
        expected.insert(b"CLIMON2_LOG".to_vec(), b"trace".to_vec());
        if let Some(shell) = &shell {
            expected.insert(b"SHELL".to_vec(), shell.as_os_str().as_bytes().to_vec());
        }
        assert!(environment == expected, "runtime environment was changed");
        attached.typed(b"go\n");
        attached.ended(&exited(0));
        session.finish();
        let log = fs::read(
            session
                .directory
                .path()
                .join(format!("logs/{ID}/daemon.log")),
        )
        .unwrap();
        assert!(
            !log.windows(b"session ready".len())
                .any(|bytes| bytes == b"session ready"),
            "daemon ignored explicit warn filter"
        );
        for secret in [
            b"private-".as_slice(),
            b"space argument".as_slice(),
            b"invalid-".as_slice(),
        ] {
            assert!(!log.windows(secret.len()).any(|bytes| bytes == secret));
        }
    }
}
