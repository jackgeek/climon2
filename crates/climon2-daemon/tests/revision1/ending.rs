//! Daemon lifecycle over revision 1 (#103), exit and termination: early
//! exits, the final drain, unexpected departures and the hang-up grace,
//! Kill in its harder cases, PTY write failure and job-control cleanup.
use super::{
    assert_closed, assert_killed, clocked, command_final, committed_kill, counter, exited,
    interactive::{Attached, geometry, ready_attachment},
    kill, message, monitor, open, peer, raw_frame, request, signalled, status,
    status::Watcher,
    terminating,
};
use crate::{ID, Session};
use climon2_core::{
    hello::Lifecycle,
    protocol::HANGUP_GRACE,
    wire::{self, CommandResult, Input, Leave, Mode, ServerMessage, StatusState},
};
use climon2_platform::process;
use std::{
    fs,
    io::Write,
    os::unix::net::UnixStream,
    thread,
    time::{Duration, Instant},
};

/// COM-06 (Contract, known-final result) and COM-05 (Contract, Initial metadata
/// case).
/// #20 over revision 1, an early exit: the command prints its last bytes
/// and exits 43 before anything attaches, as passive Opens' Initial
/// metadata shows once the daemon knows it. An interactive Open made after
/// that carries the result in its Initial metadata, and its restoration
/// holds the last bytes once. The session still counts it as its
/// controller and ends only once it confirms Ready: it then receives the
/// held result as its one Command final (#259), its control ends with the
/// session, and its stream closes.
#[test]
fn an_early_exit_reaches_a_later_attachment_through_its_restoration() {
    let mut session = Session::start("early");
    result_known(&session, &exited(43));
    let mut attached = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    assert_eq!(attached.initial_result, Some(exited(43)));
    assert_eq!(crate::count(&attached.display, b"early-final"), 1);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );
    attached.confirm_ready();
    attached.ended(&exited(43));
    session.finish();
}

/// COM-06 (Contract, every opened connection gets exactly one Command
/// final), #259: the command exits 43 before anything attaches. A monitor
/// open at the exit receives the Command final at once; a monitor and an
/// interactive attachment that open after it learn the result from their
/// Initial metadata. When the attachment confirms Ready the session ends,
/// and each of those two receives the held result as Command final, the
/// attachment before its control ends with the session. The first monitor
/// receives no second one, and every stream ends.
#[test]
fn an_attachment_confirming_ready_publishes_a_held_result_to_every_connection_once() {
    let mut session = Session::start("script");
    let (mut earlier, _) = monitor(&session, &[]);
    crate::issue(&session, "exit 43");
    assert_eq!(message(&mut earlier), command_final(&exited(43)));
    let mut later = late_monitor(&session, &exited(43));
    let mut attached = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    assert_eq!(attached.initial_result, Some(exited(43)));
    attached.confirm_ready();
    attached.ended(&exited(43));
    assert_eq!(message(&mut later), command_final(&exited(43)));
    assert_closed(&mut later);
    assert_closed(&mut earlier);
    session.finish();
}

/// COM-06 (Contract, every opened connection gets exactly one Command
/// final) and COM-03 (Contract, Kill cases), #259: the command exits 43
/// while only passive monitor connections watch, and a standalone Kill ends
/// the session while it still holds the exit for its initial attachment.
/// The monitor open at the exit received the Command final at once and
/// receives no second one; the monitor that opened after the exit, and
/// learned the result from its Initial metadata, receives it as Command
/// final when the session ends. The Kill confirms that result, and every
/// stream ends.
#[test]
fn a_kill_during_the_hold_publishes_the_held_result_to_every_monitor_once() {
    let mut session = Session::start("script");
    let (mut earlier, _) = monitor(&session, &[]);
    crate::issue(&session, "exit 43");
    assert_eq!(message(&mut earlier), command_final(&exited(43)));
    let mut later = late_monitor(&session, &exited(43));
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Starting, 0, false)
    );
    let mut killer = committed_kill(&session);
    assert_killed(&mut killer, &exited(43));
    assert_eq!(message(&mut later), command_final(&exited(43)));
    assert_closed(&mut later);
    assert_closed(&mut earlier);
    session.finish();
}

/// A passive monitor connection opened after the command's result was
/// final: its Initial metadata carries `result`.
fn late_monitor(session: &Session, result: &CommandResult) -> UnixStream {
    let mut stream = peer(session);
    request(&mut stream, &open(1, &[], &[], Mode::Monitor));
    assert!(matches!(message(&mut stream), ServerMessage::Response(_)));
    let ServerMessage::InitialMetadata(metadata) = message(&mut stream) else {
        panic!("no Initial metadata after the Open response");
    };
    assert_eq!(metadata.command_result(), Some(result));
    stream
}

/// COM-06 (Contract, every opened connection gets exactly one Command
/// final) and CTL-01 (Contract, watching is not attaching), #259: the
/// command exits 43 while only passive monitor connections watch, so the
/// session holds its exit for the initial attachment and stays `starting`.
/// The monitor that watched before the exit receives the Command final at
/// once. One that opens after it learns the result in its Initial metadata
/// (`status-v1`'s final `failed` status) and receives its own Command final,
/// with that final status, only when the five-second initial deadline ends
/// the session. No monitor receives a second one, and both streams end.
#[test]
fn the_initial_deadline_publishes_a_held_result_to_every_monitor_once() {
    let mut session = clocked("script");
    let started = Instant::now();
    let (mut earlier, _) = monitor(&session, &[]);
    crate::issue(&session, "exit 43");
    assert_eq!(message(&mut earlier), command_final(&exited(43)));
    let (mut later, initial) = Watcher::open(&session);
    assert_eq!(initial, status(StatusState::Failed, 0, None));
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Starting, 0, false)
    );
    later.ended(&exited(43), 0);
    assert!(started.elapsed() >= Duration::from_millis(4500));
    assert_closed(&mut earlier);
    session.finish();
}

/// Waits until the daemon knows the command ended with `result`: a passive
/// monitor connection's Initial metadata reports it. Each monitor that
/// finds no result yet leaves at once.
fn result_known(session: &Session, result: &CommandResult) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut stream = peer(session);
        request(&mut stream, &open(1, &[], &[], Mode::Monitor));
        assert!(matches!(message(&mut stream), ServerMessage::Response(_)));
        let ServerMessage::InitialMetadata(metadata) = message(&mut stream) else {
            panic!("no Initial metadata after the Open response");
        };
        let mut known = metadata.command_result().cloned();
        request(
            &mut stream,
            &Leave {
                request_id: counter(2),
            },
        );
        // The result may become known after the Initial metadata, before
        // the Leave: its Command final then precedes the Leave's Response.
        let mut answer = message(&mut stream);
        if answer == command_final(result) {
            known = Some(result.clone());
            answer = message(&mut stream);
        }
        assert!(matches!(answer, ServerMessage::Response(_)), "{answer:?}");
        assert_closed(&mut stream);
        if let Some(known) = known {
            assert_eq!(&known, result);
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the command's result stayed unknown"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// COM-06 (Contract) and CTL-05 (Contract: Ready under the current
/// restoration).
/// #20 over revision 1: an exit after the controller's restoration reaches
/// it as its last bytes, then its Command final, while it still withholds
/// Ready, but the session lives on: its Ready is still answered as the
/// first under its authority, and only then does its control end with the
/// session and its stream close.
#[test]
fn the_session_ends_only_after_its_controller_confirms_ready() {
    let mut session = Session::start("early-after-restore");
    let mut attached = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    fs::write(session.directory.path().join("finish-early"), b"").unwrap();
    assert_eq!(attached.next_after_display(), command_final(&exited(43)));
    assert_eq!(crate::count(&attached.display, b"early-final"), 1);
    attached.confirm_ready();
    attached.session_ended();
    session.finish();
}

/// COM-06 (Contract: Command final follows the command's last output).
/// #20/#19 over revision 1: once the command's session leader exits 23, the
/// final drain ends at the end of its output or at its one-second cap,
/// whether a descendant holding the terminal stays idle or keeps writing.
/// The leader's last bytes reach the controller before its Command final,
/// well within the cap's bound, and no member of the command's group
/// survives the session.
#[test]
fn the_final_drain_is_bounded_and_leaves_no_descendants() {
    for mode in ["drain-idle", "drain-cap"] {
        let mut session = Session::start(mode);
        let mut attached = ready_attachment(&session, &[]);
        attached.typed(b"go\n");
        let started = Instant::now();
        attached.ended(&exited(23));
        assert!(
            crate::count(&attached.display, b"leader-final") > 0,
            "{mode}"
        );
        assert!(started.elapsed() < Duration::from_millis(2800), "{mode}");
        session.finish();
    }
}

/// How a revision-1 controller departs without Leave.
#[derive(Clone, Copy, Debug)]
enum Departure {
    /// Its stream ends.
    Closed,
    /// A body that is not JSON, holding text that must stay out of the log.
    InvalidJson,
    /// A hello repeated on its revision-1 stream.
    RepeatedHello,
    /// The daemon is asked to terminate (SIGTERM).
    Terminated,
    /// Input the command never reads, then its stream ends.
    UnreadInput,
}

impl Departure {
    /// Departs `attached` from `session` this way.
    fn depart(self, session: &Session, mut attached: Attached) {
        match self {
            Self::Closed => {}
            Self::InvalidJson => {
                let body = b"{\"untrusted secret\nvalue";
                fatal(&mut attached, &raw_frame(0x37, body), "invalid_json");
            }
            Self::RepeatedHello => {
                fatal(&mut attached, &crate::hello_bytes(), "unknown_message_type");
            }
            Self::Terminated => unsafe {
                libc::kill(session.pid as i32, libc::SIGTERM);
            },
            Self::UnreadInput => {
                let authority = attached.authority();
                for _ in 0..3 {
                    attached.send(|request_id| {
                        Input {
                            request_id,
                            authority,
                            payload: vec![b'x'; 65_535],
                        }
                        .to_frame()
                    });
                }
            }
        }
        drop(attached);
    }
}

/// Writes `bytes` on `attached`'s stream; after any Display the daemon
/// answers with a Fatal error `code`.
fn fatal(attached: &mut Attached, bytes: &[u8], code: &str) {
    attached.stream.write_all(bytes).unwrap();
    match attached.next_after_display() {
        ServerMessage::FatalError(wire::FatalError(error)) => {
            assert_eq!(error.code.as_str(), code);
        }
        other => panic!("expected Fatal error {code}, got {other:?}"),
    }
}

/// #19/#27 over revision 1, and HDL-01 (Contract, ordinary case): every way
/// the final revision-1 controller departs without Leave, its stream
/// closing, a connection-fatal error, the daemon's SIGTERM or input left
/// unread, is an unexpected departure of an ordinary session. The session
/// commits to terminating, hangs the command up, keeps it through the real
/// five-second grace (it ignores SIGHUP), then kills it and ends. Text the
/// peer sent never reaches the daemon log.
#[test]
fn every_unexpected_departure_hangs_up_then_kills_after_the_real_grace() {
    for departure in [
        Departure::Closed,
        Departure::InvalidJson,
        Departure::RepeatedHello,
        Departure::Terminated,
        Departure::UnreadInput,
    ] {
        let mut session = Session::start("ignore-hup");
        let attached = ready_attachment(&session, &[]);
        let pid = fixture_pid(&session);
        let started = Instant::now();
        departure.depart(&session, attached);
        terminating(&session);
        assert!(
            process::start_time(pid).unwrap().is_some(),
            "{departure:?}: grace skipped"
        );
        session.finish();
        let elapsed = started.elapsed();
        assert!(elapsed >= HANGUP_GRACE, "{departure:?}: grace skipped");
        assert!(
            elapsed < HANGUP_GRACE + Duration::from_secs(4),
            "{departure:?}: grace exceeded: {elapsed:?}"
        );
        let log = session
            .directory
            .path()
            .join(format!("logs/{ID}/daemon.log"));
        assert!(
            !fs::read_to_string(log)
                .unwrap()
                .contains("untrusted secret")
        );
    }
}

/// The command fixture's process ID, which it records as it starts.
fn fixture_pid(session: &Session) -> u32 {
    let pid = fs::read_to_string(session.directory.path().join("fixture-pid")).unwrap();
    pid.parse().unwrap()
}

/// COM-03 (Contract, Kill cases).
/// Section 6.2 over revision 1, unfinished token: the command's pending
/// `ESC [` does not hold a restoration, which completes, so the attachment
/// confirms Ready and leaves. A standalone Kill then ends the command,
/// and confirms it once the daemon cleaned up.
#[test]
fn an_unfinished_token_still_restores_and_a_kill_confirms_the_result() {
    let session = Session::start("partial");
    crate::wait_file(&session.directory.path().join("partial-ready"));
    let mut attached = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    attached.confirm_ready();
    attached.leave();
    super::end_by_kill(session);
}

/// COM-03 (Contract, Kill cases).
/// Section 3.3 over revision 1: hanging up closes the command's terminal
/// without typing into it. A standalone Kill commits the termination while
/// ptrace records every write the daemon makes to the terminal's master
/// until it closes its last handle on it: there are none, so no newline or
/// end-of-file key reaches the command as if a client had typed it. The
/// command ignores SIGHUP, so SIGKILL ends it after the grace, and both the
/// Kill and the controller's Command final say so.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn hanging_up_types_nothing_into_the_command_terminal() {
    let mut session = Session::start("ignore-hup");
    let mut attached = ready_attachment(&session, &[]);
    let mut killer = peer(&session);
    // The hang-up scans every process's procfs entry; each call stops here.
    let tracer = climon2_test_term::ptrace::Tracer::seize(session.pid, Duration::from_secs(60));
    request(&mut killer, &kill(1, 1, &[]));
    let master = std::path::Path::new("/dev/ptmx");
    let (write, close) = (libc::SYS_write as u64, libc::SYS_close as u64);
    let mut typed = Vec::new();
    loop {
        let call = tracer.next_entry();
        if ![write, close].contains(&call.number)
            || tracer.descriptor(call.arguments[0]).as_deref() != Some(master)
        {
            continue;
        }
        if call.number == write {
            typed.push(tracer.memory(call.arguments[1], call.arguments[2]));
            continue;
        }
        tracer.finish_syscall();
        if !crate::holds(session.pid, master) {
            break;
        }
    }
    tracer.detach();
    assert!(
        typed.is_empty(),
        "hang-up typed {typed:?} into the terminal"
    );
    attached.ended(&signalled(9));
    assert_killed(&mut killer, &signalled(9));
    session.finish();
}

/// COM-03 (Contract, Kill cases) and INP-03 (Contract: an open paste).
/// Section 1.5 over revision 1: a Kill while the controller's application
/// paste is open queues the paste's closing bytes, which can no longer
/// reach the hung-up terminal; termination still completes, and the Kill
/// and the controller's Command final report the command's end by SIGHUP.
#[test]
fn a_kill_with_an_open_application_paste_still_terminates() {
    let mut session = Session::start("paste");
    let mut attached = ready_attachment(&session, &[]);
    attached.typed(b"\x1b[200~abc");
    let mut killer = committed_kill(&session);
    assert_killed(&mut killer, &signalled(1));
    attached.ended(&signalled(1));
    session.finish();
}

/// COM-06 (Fault, through the `pty-write` test hook).
/// Section 6.2 over revision 1, authoritative failure: when the
/// controller's accepted Input cannot be written to the command's terminal,
/// the session fails. It hangs the command up, which SIGHUP ends, reports
/// that result to the controller as its Command final, releases its
/// presentation exactly, ends its control with the session, and cleans up.
#[test]
fn a_pty_write_failure_releases_then_ends_control_with_the_session() {
    let mut session = Session::configured("paste", |command, _| {
        command.env("CLIMON2_DAEMON_FAULT", "pty-write");
    });
    let mut attached = ready_attachment(&session, &[]);
    attached.typed(b"go\n");
    assert_eq!(attached.next_after_display(), command_final(&signalled(1)));
    let mark = attached.display.len();
    attached.session_ended();
    assert_eq!(attached.display[mark..], crate::release_at(3, 14));
    session.finish();
}

/// COM-03 (Contract, Kill cases) and COM-06 (Contract, known-final result).
/// Kill after an unreported exit, over revision 1: the command exits 43
/// before anything attaches, as passive Opens' Initial metadata shows once
/// the daemon knows it. A standalone Kill then still ends the session and
/// confirms that exit.
#[test]
fn a_kill_after_the_command_exited_confirms_its_result() {
    let mut session = Session::start("early");
    result_known(&session, &exited(43));
    let mut killer = peer(&session);
    request(&mut killer, &kill(1, 1, &[]));
    assert_killed(&mut killer, &exited(43));
    session.finish();
}

/// Daemon lifecycle (#103), job-control cleanup; no matrix row.
/// #19/#27 over revision 1: on the command's natural exit, the shell's
/// HUP-ignoring background job, orphaned in the command's terminal session
/// once the shell (and so the terminal) is gone, is killed once the final
/// result is sent.
#[test]
fn natural_exit_kills_jobs_left_in_the_command_session() {
    let mut session = crate::job_control_session();
    let mut attached = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    attached.confirm_ready();
    attached.typed(crate::job("background", "&").as_bytes());
    let background = crate::job_pid(session.directory.path(), "background");
    // The daemon records job groups every 100 ms while the shell lives; allow several looks.
    thread::sleep(Duration::from_millis(500));
    attached.typed(b"exit\n");
    attached.ended(&exited(0));
    session.finish();
    let gone = crate::wait_gone(background);
    if !gone {
        unsafe { libc::kill(background as i32, libc::SIGKILL) };
    }
    assert!(gone, "background job survived natural exit");
}

/// Daemon lifecycle (#103), job-control cleanup; no matrix row.
/// #19/#27 (owner decision) over revision 1: the loss of the final
/// controller hangs up, waits the real grace, then kills every process
/// still in the command's terminal session, across job-control process
/// groups. A process that started its own session is not part of it and
/// survives. The jobs ignore SIGHUP, so only the daemon's end-of-grace kill
/// ends them: they are gone by the time the daemon exits. How long that
/// kill takes after the grace is host load, not policy, so the daemon's
/// exit is bounded only by `FINISH_BOUND`;
/// `every_unexpected_departure_hangs_up_then_kills_after_the_real_grace`
/// bounds the grace itself.
#[test]
fn losing_the_controller_kills_every_job_of_the_command_session_but_not_new_sessions() {
    let mut session = crate::job_control_session();
    let mut attached = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    attached.confirm_ready();
    let script = crate::job("background", "&")
        + &crate::job("daemonize", "&")
        + &crate::job("foreground", "");
    attached.typed(script.as_bytes());
    let home = session.directory.path().to_path_buf();
    let foreground = crate::job_pid(&home, "foreground");
    let background = crate::job_pid(&home, "background");
    let outsider = crate::job_pid(&home, "outsider");
    let shell = unsafe { libc::getsid(foreground as i32) };
    for pid in [foreground, background] {
        assert_eq!(
            unsafe { libc::getsid(pid as i32) },
            shell,
            "{pid} left the session"
        );
        assert_eq!(
            unsafe { libc::getpgid(pid as i32) },
            pid as i32,
            "{pid} is not its own job"
        );
    }
    assert_eq!(unsafe { libc::getsid(outsider as i32) }, outsider as i32);
    let started = Instant::now();
    drop(attached);
    terminating(&session);
    assert!(
        process::start_time(foreground).unwrap().is_some(),
        "grace skipped"
    );
    session.finish();
    assert!(started.elapsed() >= HANGUP_GRACE, "grace skipped");
    let survivors = [("foreground", foreground), ("background", background)]
        .into_iter()
        .filter(|(_, pid)| process::start_time(*pid).unwrap().is_some())
        .collect::<Vec<_>>();
    let outsider_alive = process::start_time(outsider).unwrap().is_some();
    for (_, pid) in &survivors {
        unsafe { libc::kill(*pid as i32, libc::SIGKILL) };
    }
    unsafe { libc::kill(outsider as i32, libc::SIGKILL) };
    assert!(survivors.is_empty(), "session jobs survived: {survivors:?}");
    assert!(outsider_alive, "a process in its own session was killed");
}
