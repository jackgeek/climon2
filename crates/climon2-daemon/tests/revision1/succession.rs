//! Monitors acquire and inherit control (#89): Acquire by a passive monitor
//! connection, succession on a controller's departure, Leave and Kill racing
//! a promotion, and the canonical presentation of monitor-mode connections.
use super::{
    assert_closed, assert_final_then_closed, assert_rejected, command_final, counter, exited,
    interactive::{Attached, geometry, ready_attachment, rejected},
    kill, leave, message, monitor, open, peer, request, signalled, terminating,
    termination::terminated,
};
use crate::Session;
use climon2_core::{
    hello::Lifecycle,
    protocol::{ClientMessage, ServerMessage as PrivateMessage},
    wire::{
        Acquire, ActualGeometry, AuthorityResult, CommandResult, ControlRecord, GrantReason, Leave,
        Mode, Outcome, RevocationReason, ServerMessage, Success,
    },
};
use std::{
    fs,
    num::NonZeroU16,
    os::unix::net::UnixStream,
    path::Path,
    time::{Duration, Instant},
};

/// The `lines` fixture's record: each line it read, with its terminal size.
fn recorded(session: &Session) -> String {
    fs::read_to_string(session.directory.path().join("lines")).unwrap()
}

/// A standalone Kill whose termination the session has committed to.
fn committed_standalone_kill(session: &Session) -> UnixStream {
    let mut killer = peer(session);
    request(&mut killer, &kill(1, 1, &[]));
    terminating(session);
    killer
}

/// The standalone Kill's success arrives once the session ended with
/// `result`, then its stream ends.
fn assert_killed(killer: &mut UnixStream, result: &CommandResult) {
    assert_eq!(message(killer), terminated(1, result));
    assert_closed(killer);
}

/// The lines a fixture command has printed once it is ready: the test
/// harness's header, a blank line and `running 1 test`, then the fixture's
/// marker.
fn started() -> Vec<String> {
    ["", "running 1 test", "fixture-ready"]
        .map(String::from)
        .to_vec()
}

/// What a fresh terminal of `cols` x `rows` holds after `bytes`: its
/// history, oldest first, and its screen, row by row.
fn parsed(cols: u16, rows: u16, bytes: &[u8]) -> (Vec<String>, Vec<String>) {
    let size = |value| NonZeroU16::new(value).unwrap();
    let mut parser = vt100::Parser::new(size(rows), size(cols), 10_000);
    parser.process(bytes);
    let screen = parser.screen_mut();
    let history = (1..=screen.scrollback_len())
        .rev()
        .map(|offset| {
            screen.set_scrollback(offset);
            screen.rows(0, cols).next().unwrap()
        })
        .collect();
    screen.set_scrollback(0);
    (history, screen.rows(0, cols).collect())
}

/// A fresh terminal of `cols` x `rows` holds exactly `lines` after `bytes`:
/// each once and in order, its history then its screen, without a blank
/// row, and the cursor's blank row last.
fn assert_canonical(bytes: &[u8], cols: u16, rows: u16, lines: &[String]) {
    let (history, mut screen) = parsed(cols, rows, bytes);
    assert_eq!(screen.len(), usize::from(rows));
    assert_eq!(screen.pop().as_deref(), Some(""), "the cursor's row");
    let held: Vec<String> = history.into_iter().chain(screen).collect();
    assert_eq!(held, lines);
}

/// CTL-01 (Contract) and COM-03 (Contract, passive Acquire): registering
/// alone never acquires a detached session; only the registration's own
/// Acquire does, and before any Open there is nothing to convert, so
/// Acquire is `invalid_state`. On a registration Acquire converts the
/// passive monitor connection into an attachment that takes control at
/// once: Control granted (`acquire`) at the session's geometry and
/// Restoration begin precede the changed success, and its restoration
/// follows. Acquiring changes no geometry and sends no input. The hello
/// `state` then counts the attachment, and its own Acquire is a no-op. It
/// stays an attachment when another registration's Acquire takes control
/// from it, and its Leave reports a detached attachment and hands control
/// back; the final Leave leaves the session running, detached.
#[test]
fn a_registration_takes_control_only_by_its_own_acquire() {
    let mut session = Session::start("lines");
    let mut first = session.attach();
    session.wait_marker(&mut first);
    crate::send(&mut first, ClientMessage::Detach);
    assert_eq!(
        crate::until_terminal(&mut first).1,
        PrivateMessage::Detached
    );
    let mut unopened = peer(&session);
    request(
        &mut unopened,
        &Acquire {
            request_id: counter(1),
        },
    );
    assert_rejected(&mut unopened, 1, "invalid_state");
    let mut watching = Attached::registered(&session);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );

    watching.acquire(geometry(91, 37));
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );
    let authority = watching.authority();
    watching.send(|request_id| Acquire { request_id }.to_frame());
    let unchanged = Success::Acquired(AuthorityResult {
        changed: false,
        authority,
    });
    assert_eq!(watching.response(), Outcome::Ok(unchanged));
    watching.confirm_ready();
    watching.typed(b"first\n");
    crate::wait_line(&session, "first 91x37");

    let mut other = Attached::registered(&session);
    other.acquire(geometry(91, 37));
    watching.revoked(RevocationReason::Takeover);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 2, true)
    );
    watching.acquire(geometry(91, 37));
    other.revoked(RevocationReason::Takeover);
    watching.leave();
    other.granted(GrantReason::HandBack, geometry(91, 37));
    other.confirm_ready();
    other.typed(b"other\n");
    crate::wait_line(&session, "other 91x37");
    other.leave();
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );

    let mut last = session.attach();
    session.wait_marker(&mut last);
    crate::send(&mut last, ClientMessage::Input(b"end\n".to_vec()));
    assert_eq!(
        crate::until_terminal(&mut last).1,
        PrivateMessage::Exit { code: 0 }
    );
    session.finish();
    assert_eq!(recorded(&session), "first 91x37\nother 91x37\n");
}

/// CTL-01 (Contract, a registration's Acquire of a starting session):
/// before any attachment, a registration's Acquire converts it into the
/// session's first attachment, which takes control at once at the session's
/// geometry: Control granted (`acquire`), its restoration, then the changed
/// success. The session then counts it as attached, and its input reaches
/// the command once it is ready.
#[test]
fn a_registration_acquires_a_starting_session() {
    let mut session = Session::start("lines");
    let mut watching = Attached::registered(&session);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Starting, 0, false)
    );
    watching.acquire(geometry(91, 37));
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );
    watching.confirm_ready();
    watching.typed(b"acquired\n");
    crate::wait_line(&session, "acquired 91x37");
    watching.typed(b"end\n");
    watching.ended(&exited(0));
    session.finish();
}

/// CTL-01 (Contract, watching is not attaching): a registration does not
/// stop an ordinary session's five-second initial attach rule. A session
/// that only a passive monitor connection watches stays `starting`, ends
/// at its initial deadline, and the registration receives the Command
/// final of the hung-up command, then its stream ends.
#[test]
fn watching_does_not_stop_the_initial_attach_rule() {
    let mut session = Session::start("lines");
    let started = Instant::now();
    let (mut watching, _) = monitor(&session, &[]);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Starting, 0, false)
    );
    assert_final_then_closed(&mut watching, &signalled(1));
    assert!(started.elapsed() >= Duration::from_millis(4500));
    session.finish();
}

/// CTL-04 (Contract, succession): when a controller departs, control goes
/// first to the most recently controlling remaining attachment, and only
/// when no attachment remains to the newest passive registration, whether
/// the final attachment departs by Leave or by loss. The promoted
/// connection gets Control granted (`successor`) at the session's geometry
/// and a restoration; the promotion resizes nothing and sends no input, and
/// an older registration receives nothing until its own promotion. The
/// leave policy applies only after succession: the session outlives its
/// final controller's loss while a monitor is registered, and ends at the
/// final attachment's loss once no registration is left.
#[test]
fn departures_hand_control_back_first_then_promote_the_newest_registration() {
    let mut session = Session::start("lines");
    let mut first = ready_attachment(&session, &[]);
    assert_eq!(
        first.resize(100, 30),
        Outcome::Ok(Success::ResizeAccepted {})
    );
    let resized = ActualGeometry {
        period: counter(1),
        geometry: geometry(100, 30),
    };
    assert_eq!(first.next_after_display(), ServerMessage::Geometry(resized));
    let mut older = Attached::registered(&session);
    let mut newer = Attached::registered(&session);
    let mut second = Attached::open(&session, geometry(100, 30), &[], geometry(100, 30));
    first.revoked(RevocationReason::Takeover);
    second.confirm_ready();
    drop(second);
    first.granted(GrantReason::HandBack, geometry(100, 30));
    first.confirm_ready();
    first.typed(b"first\n");
    crate::wait_line(&session, "first 100x30");

    first.leave();
    newer.granted(GrantReason::Successor, geometry(100, 30));
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );
    newer.confirm_ready();
    newer.typed(b"newer\n");
    crate::wait_line(&session, "newer 100x30");
    drop(newer);
    older.granted(GrantReason::Successor, geometry(100, 30));
    older.confirm_ready();
    older.typed(b"older\n");
    crate::wait_line(&session, "older 100x30");
    drop(older);
    session.finish();
    assert_eq!(
        recorded(&session),
        "first 100x30\nnewer 100x30\nolder 100x30\n"
    );
}

/// CTL-04 (Fault, Leave racing a promotion): Leave removes a registration
/// and any attachment in one step and reports which it removed. A
/// registration whose Leave the daemon reads after a departure promoted
/// it, before it read the grant, leaves as an attachment: it receives the
/// grant and its restoration, then the release and `left` with
/// `attachment_detached: true`, and with no registration left the session
/// stays detached. A registration whose Leave is answered first
/// (`attachment_detached: false`) is never promoted: the final controller's
/// later loss ends the session.
#[test]
fn leave_racing_a_promotion_removes_the_registration_or_the_attachment_atomically() {
    let mut session = Session::start("exit");
    let mut controller = ready_attachment(&session, &[]);
    let mut watching = Attached::registered(&session);
    controller.leave();
    watching.send(|request_id| Leave { request_id }.to_frame());
    watching.granted(GrantReason::Successor, geometry(91, 37));
    let left = Success::Left {
        attachment_detached: true,
    };
    assert_eq!(watching.response(), Outcome::Ok(left));
    watching.closed();
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );

    let controller = ready_attachment(&session, &[]);
    let (mut leaving, _) = monitor(&session, &[]);
    leave(&mut leaving, 2);
    drop(controller);
    session.finish();
}

/// Waits until the hello `state` reports `expected`.
fn advertises(session: &Session, expected: (Lifecycle, u32, bool)) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while crate::advertised_state(session) != expected {
        assert!(
            Instant::now() < deadline,
            "the session never became {expected:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// CTL-04 and HDL-01 (Contract, headless succession over revision-1
/// attachments): in a headless session the unexpected loss of the last
/// attachment promotes the registered monitor, which gets Control granted
/// (`successor`) at the session's geometry and controls the command. The
/// promoted monitor's own loss then leaves the session running, detached
/// with zero clients, and its command still reaches a later attachment.
#[test]
fn a_headless_session_promotes_a_monitor_and_outlives_its_loss() {
    let mut session = crate::headless("lines");
    let controller = ready_attachment(&session, &[]);
    let mut watching = Attached::registered(&session);
    drop(controller);
    watching.granted(GrantReason::Successor, geometry(91, 37));
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );
    watching.confirm_ready();
    watching.typed(b"inherited\n");
    crate::wait_line(&session, "inherited 91x37");
    drop(watching);
    advertises(&session, (Lifecycle::Detached, 0, false));

    let mut last = session.attach();
    session.wait_marker(&mut last);
    crate::send(&mut last, ClientMessage::Input(b"end\n".to_vec()));
    assert_eq!(
        crate::until_terminal(&mut last).1,
        PrivateMessage::Exit { code: 0 }
    );
    session.finish();
    assert_eq!(recorded(&session), "inherited 91x37\n");
}

/// Holds the session's incoming frames through the `CLIMON2_DAEMON_HOLD`
/// hook at `hold`, runs `queue`, then releases them, so the daemon reads
/// every frame `queue` wrote in one pass over its connections.
fn in_one_pass(hold: &Path, queue: impl FnOnce()) {
    fs::write(hold, b"").unwrap();
    let held = hold.with_extension("held");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !held.exists() {
        assert!(Instant::now() < deadline, "the daemon never held its input");
        std::thread::sleep(Duration::from_millis(5));
    }
    queue();
    fs::remove_file(hold).unwrap();
}

/// CTL-04 (Fault, through the `monitor-overflow` and `CLIMON2_DAEMON_HOLD`
/// test hooks: a registration lost to its own queue's overflow racing a
/// promotion). The daemon reads, in one pass, a registration's request
/// whose Response overflows its output queue, then the final attachment's
/// Leave. The departing registration is removed before succession, so the
/// Leave promotes nobody: the registration's stream just ends, the
/// attachment leaves as an accepted detach, and the session stays
/// detached rather than ending at the lost registration's departure.
#[test]
fn a_registration_lost_to_an_overflow_is_never_promoted() {
    let mut hold = None;
    let mut session = Session::configured("lines", |command, home| {
        command.env("CLIMON2_DAEMON_FAULT", "monitor-overflow");
        hold = Some(home.join("hold"));
        command.env("CLIMON2_DAEMON_HOLD", home.join("hold"));
    });
    let mut watching = Attached::registered(&session);
    let mut controller = ready_attachment(&session, &[]);
    in_one_pass(&hold.unwrap(), || {
        watching.send(|request_id| {
            let mut again = open(1, &[], &[], Mode::Monitor);
            again.request_id = request_id;
            again.to_frame()
        });
        controller.send(|request_id| Leave { request_id }.to_frame());
    });
    let left = Success::Left {
        attachment_detached: true,
    };
    assert_eq!(controller.response(), Outcome::Ok(left));
    controller.closed();
    watching.closed();
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );

    let mut last = session.attach();
    session.wait_marker(&mut last);
    crate::send(&mut last, ClientMessage::Input(b"end\n".to_vec()));
    assert_eq!(
        crate::until_terminal(&mut last).1,
        PrivateMessage::Exit { code: 0 }
    );
    session.finish();
}

/// CTL-04 (Fault, a promotion racing a committed Kill) and COM-03
/// (Contract, committed termination): committed termination overrides
/// succession. Once a standalone Kill has committed the session's
/// termination, a registration's Acquire is `session_terminating` and the
/// controller's Leave promotes nobody: the registration receives only the
/// Command final, then its stream ends, and the Kill succeeds.
#[test]
fn a_committed_kill_overrides_succession() {
    let mut session = Session::start("ignore-hup");
    let mut controller = ready_attachment(&session, &[]);
    let mut watching = Attached::registered(&session);
    let mut killer = committed_standalone_kill(&session);
    watching.send(|request_id| Acquire { request_id }.to_frame());
    rejected(watching.response(), "session_terminating");
    controller.leave();
    assert_eq!(watching.next_after_display(), command_final(&signalled(9)));
    watching.closed();
    assert_killed(&mut killer, &signalled(9));
    session.finish();
}

/// CTL-04 (Fault, a promotion racing a committed Kill): a registration that
/// a departure promoted before a Kill committed the session's termination is
/// one of the terminating session's attachments. It keeps control until the
/// session ends, then receives the Command final, the release and Control
/// revoked (`session_ending`), and its stream ends; the Kill succeeds.
#[test]
fn a_registration_promoted_before_a_kill_ends_as_an_attachment() {
    let mut session = Session::start("ignore-hup");
    let mut controller = ready_attachment(&session, &[]);
    let mut watching = Attached::registered(&session);
    controller.leave();
    watching.granted(GrantReason::Successor, geometry(91, 37));
    watching.confirm_ready();
    let mut killer = committed_standalone_kill(&session);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Terminating, 1, true)
    );
    watching.ended(&signalled(9));
    assert_killed(&mut killer, &signalled(9));
    session.finish();
}

/// CTL-04 (Contract, a promotion's canonical restoration) and COM-05
/// (Contract, monitor-mode geometry): every restoration of a monitor-mode
/// connection is serialized at the effective session geometry for a fresh
/// terminal of that size, whatever host size anyone reports, while an
/// ordinary attachment keeps its host-relative presentation. An interactive
/// attachment on a 600x250 host gets the clamped 500x200 session and a
/// Derived restoration whose first act scrolls its host's 250 rows into
/// native scrollback. The registration its Leave promotes gets Control
/// granted (`successor`) at 500x200, and its restoration, parsed by a fresh
/// 500x200 terminal, holds every line once and in order, history then
/// screen, with no blank row. Before its Ready a host change that keeps the
/// session's geometry keeps its restoration. Once it is ready, each Resize
/// that changes the effective geometry is followed by Geometry, then a
/// Restoration begin at that geometry before any Display at it, then a
/// restoration just as canonical, which needs a new Ready; a Resize that
/// keeps the geometry is followed by nothing.
#[test]
fn monitor_mode_restorations_are_canonical_at_the_effective_geometry_whatever_the_host() {
    let mut session = Session::start("script");
    let mut ordinary = Attached::open(&session, geometry(600, 250), &[], geometry(500, 200));
    ordinary.until_displayed(b"fixture-ready", 1);
    let (native, screen) = parsed(600, 250, &ordinary.display);
    assert_eq!(native, vec![String::new(); 250]);
    assert_eq!(screen[..4], [started(), vec![String::new()]].concat());
    ordinary.confirm_ready();
    crate::script(&session, "rows 1 250");
    ordinary.until_displayed(b"row-0250", 1);
    let mut watching = Attached::registered(&session);
    ordinary.leave();

    let rows = (1..=250).map(|row| format!("row-{row:04}"));
    let lines: Vec<String> = started().into_iter().chain(rows).collect();
    watching.granted(GrantReason::Successor, geometry(500, 200));
    assert_canonical(&watching.display, 500, 200, &lines);
    let accepted = Outcome::Ok(Success::ResizeAccepted {});
    assert_eq!(watching.resize(610, 260), accepted);
    watching.confirm_ready();
    for (cols, rows, effective) in [(300, 100, (300, 100)), (600, 250, (500, 200))] {
        assert_eq!(watching.resize(cols, rows), accepted);
        let mark = watching.display.len();
        watching.regeometried(geometry(u32::from(effective.0), u32::from(effective.1)));
        assert_canonical(&watching.display[mark..], effective.0, effective.1, &lines);
        watching.confirm_ready();
    }
    assert_eq!(watching.resize(610, 260), accepted);
    let again = Success::Ready(AuthorityResult {
        changed: false,
        authority: watching.authority(),
    });
    assert_eq!(watching.ready(), Outcome::Ok(again));
    watching.leave();
    crate::issue(&session, "exit");
    session.finish();
}
