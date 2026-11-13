//! Command final and standalone Kill (#88): how the end of a session reaches
//! revision-1 connections, beside the private protocol's results.
use super::{
    assert_closed, assert_final_then_closed, assert_rejected, command_final, counter, exited,
    frame,
    interactive::{Attached, geometry, ready_attachment},
    kill, message, monitor, open, peer, request, signalled,
};
use crate::Session;
use climon2_core::{
    hello::Lifecycle,
    protocol::{ClientMessage, HANGUP_GRACE, ServerMessage as PrivateMessage},
    wire::{
        Acquire, AuthorityIds, Cleanup, CommandResult, Input, Leave, Mode, Outcome, Response,
        ServerMessage, Success,
    },
};
use climon2_platform::process;
use std::{
    fs,
    os::unix::net::UnixStream,
    thread,
    time::{Duration, Instant},
};

/// A standalone Kill's success: termination confirmed with `result` after
/// complete daemon cleanup.
fn terminated(request_id: u64, result: &CommandResult) -> ServerMessage {
    let termination = Success::Termination {
        command_result: result.clone(),
        cleanup: Cleanup::Complete,
    };
    ServerMessage::Response(Response {
        request_id: counter(request_id),
        outcome: Outcome::Ok(termination),
    })
}

/// The next record rejects first-role request 1 with `code`, carrying the
/// command's known result, and the stream then ends.
fn assert_refused_knowing(stream: &mut UnixStream, code: &str, result: &CommandResult) {
    let rejection = match message(stream) {
        ServerMessage::Response(Response {
            request_id,
            outcome: Outcome::Rejected(rejection),
        }) if request_id == counter(1) => rejection,
        other => panic!("expected the rejection of request 1, got {other:?}"),
    };
    assert_eq!(rejection.error.code.as_str(), code);
    assert_eq!(rejection.known_final.as_ref(), Some(result));
    assert_closed(stream);
}

/// The command fixture's process ID, which it records as it starts.
fn fixture_pid(session: &Session) -> u32 {
    let pid = fs::read_to_string(session.directory.path().join("fixture-pid")).unwrap();
    pid.parse().unwrap()
}

/// Waits until the hello `state` reports the session terminating.
fn terminating(session: &Session) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while crate::advertised_state(session).0 != Lifecycle::Terminating {
        assert!(Instant::now() < deadline, "termination never committed");
        thread::sleep(Duration::from_millis(5));
    }
}

/// COM-06 (Contract): exit zero, an arbitrary nonzero exit and a signal each
/// reach every opened revision-1 connection as exactly one Command final
/// with that result, and exit 143 is not signal 15. A passive monitor
/// connection's stream then ends; an interactive attachment's control ends
/// with the session (`session_ending`) and its stream closes. The private
/// protocol keeps its mapping: its attachment gets `Exit` with 0, 143 and
/// 143.
#[test]
fn command_final_reports_each_exit_and_signal_once() {
    let cases = [
        ("exit", exited(0), 0),
        ("exit 143", exited(143), 143),
        ("signal 15", signalled(15), 143),
    ];
    for (script, result, private_code) in cases {
        let mut session = Session::start("script");
        let mut private = session.attach();
        session.wait_marker(&mut private);
        let (mut watching, _) = monitor(&session, &[]);
        let mut attached = ready_attachment(&session, &[]);
        assert_eq!(
            crate::until_terminal(&mut private).1,
            PrivateMessage::Revoked
        );
        crate::issue(&session, script);
        assert_final_then_closed(&mut watching, &result);
        attached.ended(&result);
        assert_eq!(
            crate::until_terminal(&mut private).1,
            PrivateMessage::Exit { code: private_code },
            "{script}"
        );
        session.finish();
    }
}

/// COM-03 (Contract, Kill cases) and COM-06 (Contract, signal): a
/// standalone Kill is a role of its own connection, not an attachment. It
/// commits the session's termination at once and is answered only after
/// the command's termination is confirmed and the daemon cleaned up: after
/// the real five-second hang-up grace, with the session's files removed
/// and the command gone. A second standalone Kill and a private kill, sent
/// a second later, join the same termination without restarting the grace,
/// and all three are answered together. On the Kill's connection, a later
/// Kill or Open is `invalid_state`, as are Leave and Acquire, with nothing
/// opened, and Input is `not_attached`. The command ignores SIGHUP, so
/// SIGKILL ends it: the Kills and every Command final say signal 9, while
/// the private kill gets `Terminated` 137.
#[test]
fn a_standalone_kill_is_answered_after_termination_and_cleanup_and_later_kills_join_it() {
    let mut session = Session::start("ignore-hup");
    let mut attached = ready_attachment(&session, &[]);
    let (mut watching, _) = monitor(&session, &[]);
    let command = fixture_pid(&session);
    let mut first = peer(&session);
    let started = Instant::now();
    request(&mut first, &kill(1, 1, &[]));
    terminating(&session);
    request(&mut first, &kill(2, 1, &[]));
    assert_rejected(&mut first, 2, "invalid_state");
    request(&mut first, &open(3, &[], &[], Mode::Monitor));
    assert_rejected(&mut first, 3, "invalid_state");
    request(
        &mut first,
        &Leave {
            request_id: counter(4),
        },
    );
    assert_rejected(&mut first, 4, "invalid_state");
    request(
        &mut first,
        &Acquire {
            request_id: counter(5),
        },
    );
    assert_rejected(&mut first, 5, "invalid_state");
    let input = Input {
        request_id: counter(6),
        authority: AuthorityIds {
            period: counter(1),
            restoration_id: counter(1),
        },
        payload: b"x".to_vec(),
    };
    super::send(&mut first, &input.to_frame());
    assert_rejected(&mut first, 6, "not_attached");
    // A gap a restarted grace would show, not an ordering step.
    thread::sleep(Duration::from_secs(1));
    let mut second = peer(&session);
    let joined = Instant::now();
    request(&mut second, &kill(1, 1, &["daemon-wire-v1"]));
    let mut private = session.connect();
    crate::send(&mut private, ClientMessage::Kill);

    let killed = signalled(9);
    assert_eq!(message(&mut first), terminated(1, &killed));
    assert!(started.elapsed() >= HANGUP_GRACE, "{:?}", started.elapsed());
    assert!(joined.elapsed() < HANGUP_GRACE, "the grace restarted");
    assert!(!session.metadata.exists() && !session.socket.exists());
    assert_eq!(process::start_time(command).unwrap(), None);
    assert_closed(&mut first);
    assert_eq!(message(&mut second), terminated(1, &killed));
    assert_closed(&mut second);
    assert_eq!(
        crate::receive(&mut private),
        PrivateMessage::Terminated { code: 137 }
    );
    assert_final_then_closed(&mut watching, &killed);
    attached.ended(&killed);
    session.finish();
}

/// COM-06 (Contract, known-final result) and COM-05 (Contract, Initial
/// metadata case): once the daemon knows the command's result, a later
/// Open's Initial metadata carries it and no Command final follows, and a
/// rejected first role carries it as `known_final`. Here the command exits
/// while its only controller withholds Ready, so the session lives on.
/// During the grace of a Kill that follows, an interactive Open is
/// `session_terminating` and a Kill requiring an unknown family
/// `unsupported_capability`, each with the result. The Kill then confirms
/// the same result, and no connection gets a second Command final.
#[test]
fn a_known_result_reaches_later_opens_and_rejected_first_roles() {
    let mut session = Session::start("script");
    let mut attached = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    let (mut early, _) = monitor(&session, &[]);
    crate::issue(&session, "exit 7");
    let result = exited(7);
    assert_eq!(message(&mut early), command_final(&result));

    let mut late = peer(&session);
    request(&mut late, &open(1, &[], &[], Mode::Monitor));
    assert!(matches!(
        message(&mut late),
        ServerMessage::Response(Response {
            outcome: Outcome::Ok(Success::Opened { .. }),
            ..
        })
    ));
    let metadata = frame(&mut late);
    assert_eq!(metadata.code(), 0x41);
    assert_eq!(
        std::str::from_utf8(metadata.body()).unwrap(),
        concat!(
            r#"{"title":{"available":false,"value":null},"#,
            r#""status":{"available":false,"value":null},"#,
            r#""progress":{"available":false,"value":null},"#,
            r#""command_result":{"kind":"exit","code":7}}"#
        )
    );
    let mut later_revision = open(1, &[], &[], Mode::Monitor);
    later_revision.revision = 2;
    let mut refused = peer(&session);
    request(&mut refused, &later_revision);
    assert_refused_knowing(&mut refused, "unsupported_revision", &result);

    let mut killer = peer(&session);
    request(&mut killer, &kill(1, 1, &[]));
    terminating(&session);
    let mut refused = peer(&session);
    request(
        &mut refused,
        &open(1, &[], &[], Mode::Interactive(geometry(91, 37))),
    );
    assert_refused_knowing(&mut refused, "session_terminating", &result);
    let mut refused = peer(&session);
    request(&mut refused, &kill(1, 1, &["future-v9"]));
    assert_refused_knowing(&mut refused, "unsupported_capability", &result);
    assert_eq!(message(&mut killer), terminated(1, &result));
    assert_closed(&mut killer);
    assert_closed(&mut early);
    assert_closed(&mut late);
    attached.ended(&result);
    session.finish();
}
