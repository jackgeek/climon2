//! Command final and standalone Kill (#88): how the end of a session reaches
//! revision-1 connections, beside the private protocol's results.
use super::{
    assert_closed, assert_final_then_closed, assert_rejected, assert_rejected_knowing,
    command_final, counter, exited, frame,
    interactive::{Attached, geometry, ready_attachment},
    kill, message, monitor, open, peer, request, signalled, terminating, title_changed,
};
use crate::Session;
use climon2_core::{
    protocol::{ClientMessage, HANGUP_GRACE, ServerMessage as PrivateMessage},
    wire::{
        Acquire, AuthorityIds, Cleanup, CommandResult, Focus, Input, KnownResult, Leave, Mode,
        Outcome, PartialCleanup, PartialTermination, Ready, Resize, Response, ServerMessage,
        Success,
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

/// The next record fails Kill request 1 with `cleanup_failed`: the
/// termination's cleanup is incomplete, and it carries `result` when the
/// daemon knew one. The stream then ends.
fn assert_kill_failed(stream: &mut UnixStream, result: Option<CommandResult>) {
    let failure = match message(stream) {
        ServerMessage::Response(Response {
            request_id,
            outcome: Outcome::Failed(failure),
        }) if request_id == counter(1) => failure,
        other => panic!("expected the Kill to fail, got {other:?}"),
    };
    assert_eq!(failure.error().code.as_str(), "cleanup_failed");
    let incomplete = KnownResult::Termination(PartialTermination {
        command_result: result,
        cleanup: PartialCleanup::Incomplete,
    });
    assert_eq!(failure.known(), Some(&incomplete));
    assert_closed(stream);
}

/// The command fixture's process ID, which it records as it starts.
fn fixture_pid(session: &Session) -> u32 {
    let pid = fs::read_to_string(session.directory.path().join("fixture-pid")).unwrap();
    pid.parse().unwrap()
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
/// the real five-second hang-up grace, with its session metadata and socket
/// removed and the command gone. A second standalone Kill and a private
/// kill, sent late in the grace, join the same termination without
/// restarting it, and all three are answered together. On the Kill's
/// connection, a later Kill or Open is `invalid_state`, as are Leave and
/// Acquire, with nothing opened; Input, Resize and Ready are `not_attached`,
/// and Focus, never selected, is `unsupported_capability`. The command
/// ignores SIGHUP, so SIGKILL ends it: the Kills and every Command final say
/// signal 9, while the private kill gets `Terminated` 137.
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
    let committed = Instant::now();
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
    let authority = AuthorityIds {
        period: counter(1),
        restoration_id: counter(1),
    };
    let input = Input {
        request_id: counter(6),
        authority,
        payload: b"x".to_vec(),
    };
    super::send(&mut first, &input.to_frame());
    assert_rejected(&mut first, 6, "not_attached");
    let resize = Resize {
        request_id: counter(7),
        authority,
        geometry: geometry(100, 30),
    };
    request(&mut first, &resize);
    assert_rejected(&mut first, 7, "not_attached");
    let ready = Ready {
        request_id: counter(8),
        authority,
    };
    request(&mut first, &ready);
    assert_rejected(&mut first, 8, "not_attached");
    let focus = Focus {
        request_id: counter(9),
        authority,
        focused: true,
    };
    request(&mut first, &focus);
    assert_rejected(&mut first, 9, "unsupported_capability");
    // Connected before the grace can end, so both are served after cleanup
    // removes the socket too.
    let mut second = peer(&session);
    let mut private = session.connect();
    // The joining Kills go out late in the grace, which leaves the most
    // room the five-second grace allows between a restarted grace and the
    // end-of-grace work of a loaded host; not an ordering step.
    thread::sleep((committed + Duration::from_secs(4)).saturating_duration_since(Instant::now()));
    let joined = Instant::now();
    request(&mut second, &kill(1, 1, &["daemon-wire-v1"]));
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
    assert_rejected_knowing(&mut refused, 1, "unsupported_revision", Some(&result));
    assert_closed(&mut refused);

    let mut killer = peer(&session);
    request(&mut killer, &kill(1, 1, &[]));
    terminating(&session);
    let mut refused = peer(&session);
    request(
        &mut refused,
        &open(1, &[], &[], Mode::Interactive(geometry(91, 37))),
    );
    assert_rejected_knowing(&mut refused, 1, "session_terminating", Some(&result));
    assert_closed(&mut refused);
    let mut refused = peer(&session);
    request(&mut refused, &kill(1, 1, &["future-v9"]));
    assert_rejected_knowing(&mut refused, 1, "unsupported_capability", Some(&result));
    assert_closed(&mut refused);
    assert_eq!(message(&mut killer), terminated(1, &result));
    assert_closed(&mut killer);
    assert_closed(&mut early);
    assert_closed(&mut late);
    attached.ended(&result);
    session.finish();
}

/// COM-06 (Fault, through the `end-hangup` test hook: a session ending
/// without final evidence) and COM-03 (Contract, Kill cases: a partial
/// result): when the daemon cannot confirm how the command ended, here
/// because the hook fails the end of its hang-up, no connection gets a
/// Command final. The monitor's stream just ends, and the attachment's
/// control ends with the session before its stream closes. The standalone
/// Kill fails with `cleanup_failed`, reporting no result and incomplete
/// cleanup, while the private kill keeps its mapping: `Terminated` 125.
#[test]
fn a_session_ending_without_final_evidence_sends_no_command_final() {
    let mut session = Session::configured("ignore-hup", |command, _| {
        command.env("CLIMON2_DAEMON_FAULT", "end-hangup");
    });
    let mut attached = ready_attachment(&session, &[]);
    let (mut watching, _) = monitor(&session, &[]);
    let mut killer = peer(&session);
    request(&mut killer, &kill(1, 1, &[]));
    terminating(&session);
    let mut private = session.connect();
    crate::send(&mut private, ClientMessage::Kill);
    assert_kill_failed(&mut killer, None);
    assert_eq!(
        crate::receive(&mut private),
        PrivateMessage::Terminated { code: 125 }
    );
    assert_closed(&mut watching);
    attached.session_ended();
    session.finish();
}

/// COM-06 (Fault, through the `end-hangup` test hook) and COM-03 (Contract,
/// Kill cases: a partial result): a Kill whose termination the daemon
/// cannot confirm still reports the result it already knew. Here the
/// command exits 7 while its only controller withholds Ready, a Kill
/// follows, and the hook fails the final kill of what remained of the
/// command's terminal session. The Kill fails with `cleanup_failed` and
/// incomplete cleanup, but with exit 7, as the monitor's Command final
/// reported it; the private kill keeps `Terminated` 125.
#[test]
fn a_failed_kill_keeps_the_result_the_daemon_knew() {
    let mut session = Session::configured("script", |command, _| {
        command.env("CLIMON2_DAEMON_FAULT", "end-hangup");
    });
    let mut attached = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    let (mut watching, _) = monitor(&session, &[]);
    crate::issue(&session, "exit 7");
    let result = exited(7);
    assert_eq!(message(&mut watching), command_final(&result));
    let mut killer = peer(&session);
    request(&mut killer, &kill(1, 1, &[]));
    terminating(&session);
    let mut private = session.connect();
    crate::send(&mut private, ClientMessage::Kill);
    assert_kill_failed(&mut killer, Some(result.clone()));
    assert_eq!(
        crate::receive(&mut private),
        PrivateMessage::Terminated { code: 125 }
    );
    assert_closed(&mut watching);
    attached.ended(&result);
    session.finish();
}

/// COM-03 (Contract, Kill cases: a partial result) and COM-06 (Fault,
/// through the `cleanup` test hook): when the daemon's cleanup fails after
/// it confirmed how the command ended, a Kill fails with `cleanup_failed`
/// and incomplete cleanup, but carries that confirmed result, signal 9, as
/// every Command final did. The private kill gets `Error`, as after any
/// failed cleanup.
#[test]
fn a_failed_cleanup_fails_the_kill_but_keeps_the_confirmed_result() {
    let mut session = Session::configured("ignore-hup", |command, _| {
        command.env("CLIMON2_DAEMON_FAULT", "cleanup");
    });
    let mut attached = ready_attachment(&session, &[]);
    let (mut watching, _) = monitor(&session, &[]);
    let mut killer = peer(&session);
    request(&mut killer, &kill(1, 1, &[]));
    terminating(&session);
    let mut private = session.connect();
    crate::send(&mut private, ClientMessage::Kill);
    let killed = signalled(9);
    assert_kill_failed(&mut killer, Some(killed.clone()));
    assert!(matches!(
        crate::receive(&mut private),
        PrivateMessage::Error { .. }
    ));
    assert_final_then_closed(&mut watching, &killed);
    attached.ended(&killed);
    session.finish();
}

/// COM-03 (Contract, Kill cases) and COM-06 (Fault, through the
/// `pty-write` test hook): an authoritative failure of the session, here a
/// failed write of the controller's Input to the command's terminal, ends
/// it but does not by itself fail a Kill. Once the daemon confirmed how the
/// command ended (it ignores SIGHUP, so SIGKILL after the grace) and its
/// cleanup completed, a standalone Kill sent during the grace succeeds with
/// signal 9, the monitor's Command final says the same, and only the
/// private protocol's attachment and kill report `Error`.
#[test]
fn a_kill_succeeds_after_a_session_failure_once_termination_is_confirmed() {
    let mut session = Session::configured("ignore-hup", |command, _| {
        command.env("CLIMON2_DAEMON_FAULT", "pty-write");
    });
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    let (mut watching, _) = monitor(&session, &[]);
    crate::send(&mut controlling, ClientMessage::Input(b"go\n".to_vec()));
    terminating(&session);
    let mut killer = peer(&session);
    request(&mut killer, &kill(1, 1, &[]));
    let mut private = session.connect();
    crate::send(&mut private, ClientMessage::Kill);
    let killed = signalled(9);
    assert_eq!(message(&mut killer), terminated(1, &killed));
    assert_closed(&mut killer);
    assert!(matches!(
        crate::receive(&mut private),
        PrivateMessage::Error { .. }
    ));
    let (_, result) = crate::until_terminal(&mut controlling);
    assert!(matches!(result, PrivateMessage::Error { .. }), "{result:?}");
    assert_final_then_closed(&mut watching, &killed);
    session.finish();
}

/// COM-06 (Fault: the end of the command's output while it still runs):
/// the end of the command's terminal output is not final evidence. The
/// command closes its terminal and keeps running; once the daemon has read
/// the end of its output, a monitor that opens reports no command result
/// in its Initial metadata, and the monitor opened before has received
/// nothing. Only the command's exit, 3, reaches both, as one Command final,
/// and the private controller gets `Exit` 3.
#[test]
fn the_end_of_the_commands_output_while_it_runs_is_no_final_result() {
    let mut session = Session::start("close-terminal");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    let (mut early, _) = monitor(&session, &[]);
    let closed = session.directory.path().join("terminal-closed");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !closed.exists() {
        assert!(Instant::now() < deadline, "the command kept its terminal");
        thread::sleep(Duration::from_millis(5));
    }
    // The daemon reads the command's terminal right after it answers this
    // hello, so the Open below is served after it read the end of output.
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
            r#""progress":{"available":false,"value":null},"command_result":null}"#
        )
    );
    fs::write(session.directory.path().join("finish"), b"").unwrap();
    let result = exited(3);
    assert_final_then_closed(&mut early, &result);
    assert_final_then_closed(&mut late, &result);
    assert_eq!(
        crate::until_terminal(&mut controlling).1,
        PrivateMessage::Exit { code: 3 }
    );
    session.finish();
}

/// Waits until the command `pid` exited while its parent, the daemon, has
/// not reaped it yet.
fn exited_unreaped(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let output = std::process::Command::new("/bin/ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        if String::from_utf8_lossy(&output.stdout)
            .trim_start()
            .starts_with('Z')
        {
            return;
        }
        assert!(Instant::now() < deadline, "the command never exited");
        thread::sleep(Duration::from_millis(5));
    }
}

/// COM-06 (Contract: Command final means no more output follows): the
/// command's last output reaches every opened connection before its Command
/// final, even when the daemon learns of the exit before it reads that
/// output. The daemon is stopped while the command sets a title, prints its
/// last line and exits 3, so all three are waiting for it as it resumes.
/// The ready controller receives every Display holding the last line, and
/// a passive monitor that selected `title-v1` the new title, before their
/// Command final.
#[test]
fn command_final_follows_the_commands_last_output() {
    let mut session = Session::start("script");
    let mut attached = ready_attachment(&session, &[]);
    let (mut watching, _) = monitor(&session, &["title-v1"]);
    let command = fixture_pid(&session);
    let daemon = i32::try_from(session.pid).unwrap();
    assert_eq!(unsafe { libc::kill(daemon, libc::SIGSTOP) }, 0);
    crate::issue(&session, "title done");
    crate::issue(&session, "text last line");
    crate::issue(&session, "exit 3");
    exited_unreaped(command);
    assert_eq!(unsafe { libc::kill(daemon, libc::SIGCONT) }, 0);
    let result = exited(3);
    assert_eq!(title_changed(&mut watching), Some("done".into()));
    assert_final_then_closed(&mut watching, &result);
    assert_eq!(attached.next_after_display(), command_final(&result));
    let before_final = attached.display.clone();
    attached.session_ended();
    assert_eq!(crate::count(&before_final, b"last line"), 1);
    session.finish();
}

/// COM-06 (Contract: Command final means no more output follows) and
/// COM-03 (Contract, Kill cases): a Kill that arrives after the command
/// exited but before its output drained waits for the drain. The daemon is
/// stopped while the command sets a title, prints its last line and exits
/// 3, and while a hello and a standalone Kill are sent in one go, so the daemon serves the Kill
/// before it reads that output. The ready controller still receives every
/// Display holding the last line, and a passive monitor that selected
/// `title-v1` the new title, before their Command final with exit 3; the
/// Kill then confirms that result.
#[test]
fn a_kill_during_the_drain_lets_the_commands_last_output_through() {
    let mut session = Session::start("script");
    let mut attached = ready_attachment(&session, &[]);
    let (mut watching, _) = monitor(&session, &["title-v1"]);
    let command = fixture_pid(&session);
    let daemon = i32::try_from(session.pid).unwrap();
    assert_eq!(unsafe { libc::kill(daemon, libc::SIGSTOP) }, 0);
    crate::issue(&session, "title done");
    crate::issue(&session, "text last line");
    crate::issue(&session, "exit 3");
    exited_unreaped(command);
    let mut killing = UnixStream::connect(&session.socket).unwrap();
    killing
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    crate::send(&mut killing, ClientMessage::Hello { protocol: 1 });
    request(&mut killing, &kill(1, 1, &[]));
    assert_eq!(unsafe { libc::kill(daemon, libc::SIGCONT) }, 0);
    let result = exited(3);
    assert_eq!(title_changed(&mut watching), Some("done".into()));
    assert_final_then_closed(&mut watching, &result);
    assert_eq!(attached.next_after_display(), command_final(&result));
    let before_final = attached.display.clone();
    attached.session_ended();
    assert_eq!(crate::count(&before_final, b"last line"), 1);
    assert!(matches!(
        crate::receive(&mut killing),
        PrivateMessage::Hello(_)
    ));
    assert_eq!(message(&mut killing), terminated(1, &result));
    assert_closed(&mut killing);
    session.finish();
}

/// Stops the daemon process, runs `during` and lets it go on.
fn while_stopped<T>(session: &Session, during: impl FnOnce() -> T) -> T {
    let daemon = i32::try_from(session.pid).unwrap();
    assert_eq!(unsafe { libc::kill(daemon, libc::SIGSTOP) }, 0);
    let value = during();
    assert_eq!(unsafe { libc::kill(daemon, libc::SIGCONT) }, 0);
    value
}

/// A screen and history whose restoration takes many Display frames: 5,000
/// rows of 90 cells in alternating colours, about 2.7 MB, then `tail-line`.
/// A first attachment receives it all live, so the daemon has read it, and
/// leaves. The command then exits 3 while the daemon is stopped, and a
/// hello and an interactive Open at 91x37 are sent in one go, so the daemon
/// grants the Open in the same pass in which it reaps the command and reads
/// the end of its output, ending the drain. Returns the Open's stream, its
/// hello reply read.
fn opened_as_the_drain_ends(session: &Session) -> UnixStream {
    let mut first = ready_attachment(session, &[]);
    crate::issue(session, "colors 5000");
    crate::issue(session, "text tail-line");
    first.until_displayed(b"tail-line", 1);
    first.leave();
    let command = fixture_pid(session);
    let mut stream = while_stopped(session, || {
        crate::issue(session, "exit 3");
        exited_unreaped(command);
        let mut stream = UnixStream::connect(&session.socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        crate::send(&mut stream, ClientMessage::Hello { protocol: 1 });
        request(
            &mut stream,
            &open(1, &[], &[], Mode::Interactive(geometry(91, 37))),
        );
        stream
    });
    assert!(matches!(
        crate::receive(&mut stream),
        PrivateMessage::Hello(_)
    ));
    stream
}

/// COM-06 (Contract: Command final means no more output follows) and
/// COM-05 (Contract, interactive order): a restoration still in progress
/// when the command's output drained delivers all of its Display, and its
/// Restoration complete, before Command final. The restoration of a large
/// screen and history begins in the pass that ends the drain
/// ([`opened_as_the_drain_ends`]). The attachment receives the whole
/// restoration, `tail-line` included, then Command final with exit 3 while
/// it has not confirmed Ready, as does a passive monitor connection. Ready
/// then ends the session.
#[test]
fn a_restoration_in_progress_when_the_drain_ends_completes_before_command_final() {
    let mut session = Session::start("script");
    let (mut watching, _) = monitor(&session, &[]);
    let stream = opened_as_the_drain_ends(&session);
    let mut attached = Attached::opened(stream, geometry(91, 37));
    assert_eq!(crate::count(&attached.display, b"tail-line"), 1);
    let result = exited(3);
    assert_eq!(attached.next_after_display(), command_final(&result));
    attached.confirm_ready();
    attached.session_ended();
    assert_final_then_closed(&mut watching, &result);
    session.finish();
}
