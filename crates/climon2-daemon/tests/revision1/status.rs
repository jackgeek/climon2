//! Heuristic activity and attention reported through `status-v1` (#91):
//! the effective status in Initial metadata, Status changed and Command
//! final, Set viewing and Acknowledge, driven by the `test-hooks`
//! controllable clock. Time stands still until a test advances it, so every
//! sample and deadline below happens at an exact, hand-computed instant:
//! the first sample is due at most 1,500 ms after start and each later one
//! at most 1,500 ms after the last, clamped to an armed agent deadline.
use super::{
    advance, assert_closed, assert_rejected, clocked, clocked_with, command_final,
    command_final_at, counter, exited,
    interactive::{Attached, geometry, ready_attachment, rejected},
    message, open, peer, request, send, status, status_changed,
};
use crate::Session;
use climon2_core::wire::{
    Acknowledge, AuthorityIds, AuthorityResult, Available, ControlRecord, Focus, FocusBytes, Input,
    Mode, Outcome, Ready, ServerMessage, SetViewing, Status, StatusState, Success,
};
use std::{num::NonZeroU32, os::unix::net::UnixStream};

use StatusState::{Acknowledged, Idle, NeedsAttention, Running};

/// The last attention episode identifier a counter can hold.
const LAST_EPISODE: u64 = u64::MAX;

/// A passive monitor connection that selected `status-v1` and `title-v1`.
struct Watcher {
    stream: UnixStream,
    request: u64,
}

impl Watcher {
    /// Opens with request 1, and returns the status its Initial metadata
    /// reports, which must be available.
    fn open(session: &Session) -> (Self, Status) {
        let mut stream = peer(session);
        let optional = ["status-v1", "title-v1"];
        request(&mut stream, &open(1, &[], &optional, Mode::Monitor));
        let response = message(&mut stream);
        assert!(
            matches!(
                &response,
                ServerMessage::Response(response)
                    if matches!(response.outcome, Outcome::Ok(Success::Opened { .. }))
            ),
            "{response:?}"
        );
        let ServerMessage::InitialMetadata(metadata) = message(&mut stream) else {
            panic!("no Initial metadata after the Open response");
        };
        let Available::Available(status) = metadata.status().clone() else {
            panic!("status-v1 selected but its status unavailable");
        };
        (Self { stream, request: 1 }, status)
    }

    /// The next record is the Status changed of `expected`.
    fn changed(&mut self, expected: &Status) {
        assert_eq!(message(&mut self.stream), status_changed(expected.clone()));
    }

    /// Nothing is queued for this connection: a rejected request sent now
    /// is answered next.
    fn quiet(&mut self) {
        self.request += 1;
        let authority = AuthorityIds {
            period: counter(1),
            restoration_id: counter(1),
        };
        let ready = Ready {
            request_id: counter(self.request),
            authority,
        };
        request(&mut self.stream, &ready);
        assert_rejected(&mut self.stream, self.request, "not_attached");
    }

    /// The command ended with `result` at `ms`: its Command final, with
    /// the final status, is the next record, and the stream then ends.
    fn ended(&mut self, result: &climon2_core::wire::CommandResult, ms: u64) {
        assert_eq!(message(&mut self.stream), command_final_at(result, ms));
        assert_closed(&mut self.stream);
    }
}

/// The next record after any Display is the Status changed of `expected`.
fn changed(attached: &mut Attached, expected: &Status) {
    assert_eq!(
        attached.next_after_display(),
        status_changed(expected.clone())
    );
}

/// Every connection in `watchers` and the attachment receive `expected`.
fn all_changed(attached: &mut Attached, watchers: &mut [&mut Watcher], expected: &Status) {
    changed(attached, expected);
    for watcher in watchers {
        watcher.changed(expected);
    }
}

/// Nothing is queued for the ready controller: Ready again is an explicit
/// no-op answered next, after any Display.
fn quiet(attached: &mut Attached) {
    let authority = attached.authority();
    let again = Success::Ready(AuthorityResult {
        changed: false,
        authority,
    });
    assert_eq!(attached.ready(), Outcome::Ok(again));
}

/// Neither the attachment nor any of `watchers` has anything queued.
fn all_quiet(attached: &mut Attached, watchers: &mut [&mut Watcher]) {
    quiet(attached);
    for watcher in watchers {
        watcher.quiet();
    }
}

/// Input of `payload` is accepted, and its acceptance is preceded by the
/// Status changed of `status` when it changes the effective status.
fn typed(attached: &mut Attached, payload: &[u8], status: Option<&Status>) {
    let request_id = attached.next();
    let input = Input {
        request_id,
        authority: attached.authority(),
        payload: payload.to_vec(),
    };
    send(&mut attached.stream, &input.to_frame());
    if let Some(status) = status {
        changed(attached, status);
    }
    let accepted_bytes = NonZeroU32::new(payload.len() as u32).unwrap();
    assert_eq!(
        attached.response(),
        Outcome::Ok(Success::InputAccepted { accepted_bytes })
    );
}

fn viewing_as(attached: &mut Attached, authority: AuthorityIds, viewing: bool) -> Outcome {
    attached.ask(authority, |request_id, authority| {
        SetViewing {
            request_id,
            authority,
            viewing,
        }
        .to_frame()
    })
}

fn viewing(attached: &mut Attached, viewing: bool) -> Outcome {
    let authority = attached.authority();
    viewing_as(attached, authority, viewing)
}

fn viewing_ok(changed: bool, viewing: bool) -> Outcome {
    Outcome::Ok(Success::Viewing { changed, viewing })
}

fn acknowledge_as(attached: &mut Attached, authority: AuthorityIds) -> Outcome {
    attached.ask(authority, |request_id, authority| {
        Acknowledge {
            request_id,
            authority,
        }
        .to_frame()
    })
}

fn acknowledge(attached: &mut Attached) -> Outcome {
    let authority = attached.authority();
    acknowledge_as(attached, authority)
}

fn acknowledgement(changed: bool) -> Outcome {
    Outcome::Ok(Success::Acknowledgement { changed })
}

/// Acknowledge changes the status to `status`, whose Status changed
/// precedes the changed success.
fn acknowledged(attached: &mut Attached, status: &Status) {
    let request_id = attached.next();
    let acknowledge = Acknowledge {
        request_id,
        authority: attached.authority(),
    };
    send(&mut attached.stream, &acknowledge.to_frame());
    changed(attached, status);
    assert_eq!(attached.response(), acknowledgement(true));
}

/// Prints `text` as a line and waits until the controller displayed it, so
/// the daemon has read it before the clock moves.
fn shown(session: &Session, attached: &mut Attached, text: &str) {
    crate::script(session, &format!("text {text}"));
    attached.until_displayed(text.as_bytes(), 1);
}

/// Prints `bytes` (`\e` for ESC) followed by a colour change that alters no
/// character, `ESC [ 38;5;N m`, and waits until the controller displayed
/// that marker.
fn printed(session: &Session, attached: &mut Attached, bytes: &str, marker: u8) {
    crate::script(session, &format!("print {bytes}\\e[38;5;{marker}m"));
    attached.until_displayed(format!("\x1b[38;5;{marker}m").as_bytes(), 1);
}

/// A clocked `script` session with a ready interactive attachment and a
/// watching monitor, both selecting `status-v1`; both start idle at the
/// daemon's initialization time.
fn watched(session: &Session) -> (Attached, Watcher) {
    let attached = ready_attachment(session, &["status-v1"]);
    let (watcher, initial) = Watcher::open(session);
    assert_eq!(initial, status(Idle, 0, None));
    (attached, watcher)
}

/// Ends a `script` session at clock `ms` through its command's exit zero:
/// completed, reported by Command final, never as Status changed.
fn completed(
    mut session: Session,
    attached: &mut Attached,
    watchers: &mut [&mut Watcher],
    ms: u64,
) {
    crate::issue(&session, "exit");
    assert_eq!(
        attached.next_after_display(),
        command_final_at(&exited(0), ms)
    );
    attached.session_ended();
    for watcher in watchers {
        watcher.ended(&exited(0), ms);
    }
    session.finish();
}

/// STA-01, NOT-01 and STA-07 (Contract, controllable clock): changed output
/// is agent work, reported as running at the sample that saw it, at most
/// 1,500 ms after the change. Ten seconds without change after that sample
/// requests attention in one step: needs_attention arrives with episode 1,
/// with no idle reported before it, and nothing one millisecond earlier.
/// Accepted Input from the ready controller then clears pending attention,
/// its Status changed preceding its acceptance; the user window ends idle
/// two seconds later. The next inactivity starts episode 2, which explicit
/// acknowledgement keeps, and accepted Input clears that acknowledged
/// attention too. The monitor receives every change as the attachment
/// does, and tied changes keep the same exact time.
#[test]
fn inactivity_requests_attention_atomically_and_input_clears_both_attention_paths() {
    let session = clocked("script");
    let (mut attached, mut watcher) = watched(&session);
    shown(&session, &mut attached, "busy");
    advance(&session, 1_500);
    all_changed(
        &mut attached,
        &mut [&mut watcher],
        &status(Running, 1_500, None),
    );
    advance(&session, 11_499);
    all_quiet(&mut attached, &mut [&mut watcher]);
    advance(&session, 11_500);
    let pending = status(NeedsAttention, 11_500, Some(1));
    all_changed(&mut attached, &mut [&mut watcher], &pending);

    let running = status(Running, 11_500, None);
    typed(&mut attached, b"x", Some(&running));
    watcher.changed(&running);
    advance(&session, 13_499);
    all_quiet(&mut attached, &mut [&mut watcher]);
    advance(&session, 13_500);
    all_changed(
        &mut attached,
        &mut [&mut watcher],
        &status(Idle, 13_500, None),
    );

    shown(&session, &mut attached, "again");
    advance(&session, 15_000);
    all_changed(
        &mut attached,
        &mut [&mut watcher],
        &status(Running, 15_000, None),
    );
    advance(&session, 25_000);
    let pending = status(NeedsAttention, 25_000, Some(2));
    all_changed(&mut attached, &mut [&mut watcher], &pending);
    assert_eq!(viewing(&mut attached, true), viewing_ok(true, true));
    let acknowledged_status = status(Acknowledged, 25_000, Some(2));
    acknowledged(&mut attached, &acknowledged_status);
    watcher.changed(&acknowledged_status);
    let running = status(Running, 25_000, None);
    typed(&mut attached, b"y", Some(&running));
    watcher.changed(&running);
    completed(session, &mut attached, &mut [&mut watcher], 25_000);
}

/// STA-01 (Contract, controllable clock): each accepted Input re-arms the
/// two-second user window. Input at 0 is running at once; Input at 1,999
/// re-arms it, so the first window's end at 2,000 is ignored, and idle
/// follows exactly two seconds after the last Input, not one millisecond
/// earlier. Input during user work changes no effective status.
#[test]
fn repeated_input_re_arms_the_two_second_user_window() {
    let session = clocked("script");
    let (mut attached, mut watcher) = watched(&session);
    let running = status(Running, 0, None);
    typed(&mut attached, b"a", Some(&running));
    watcher.changed(&running);
    advance(&session, 1_999);
    all_quiet(&mut attached, &mut [&mut watcher]);
    typed(&mut attached, b"b", None);
    advance(&session, 2_000);
    all_quiet(&mut attached, &mut [&mut watcher]);
    advance(&session, 3_998);
    all_quiet(&mut attached, &mut [&mut watcher]);
    advance(&session, 3_999);
    all_changed(
        &mut attached,
        &mut [&mut watcher],
        &status(Idle, 3_999, None),
    );
    completed(session, &mut attached, &mut [&mut watcher], 3_999);
}

/// STA-01, NOT-01, STA-07 and DSC-01 (Contract, controllable clock): while
/// attention is pending, further agent work and its inactivity stay
/// hidden and join episode 1: no report and no new change time. A monitor
/// opened later receives the stored status, time and episode, not its
/// connection time. Acknowledgement keeps episode 1; acknowledging again
/// is an explicit no-change. The next inactivity after acknowledgement
/// starts episode 2, and viewing the session does not acknowledge it.
#[test]
fn requests_coalesce_while_pending_and_re_arm_after_acknowledgement() {
    let session = clocked("script");
    let (mut attached, mut watcher) = watched(&session);
    shown(&session, &mut attached, "busy");
    advance(&session, 1_500);
    all_changed(
        &mut attached,
        &mut [&mut watcher],
        &status(Running, 1_500, None),
    );
    advance(&session, 11_500);
    let first = status(NeedsAttention, 11_500, Some(1));
    all_changed(&mut attached, &mut [&mut watcher], &first);

    shown(&session, &mut attached, "more");
    advance(&session, 13_000);
    all_quiet(&mut attached, &mut [&mut watcher]);
    advance(&session, 23_000);
    all_quiet(&mut attached, &mut [&mut watcher]);
    let (mut later, initial) = Watcher::open(&session);
    assert_eq!(initial, first);

    assert_eq!(viewing(&mut attached, true), viewing_ok(true, true));
    let acknowledged_status = status(Acknowledged, 23_000, Some(1));
    acknowledged(&mut attached, &acknowledged_status);
    watcher.changed(&acknowledged_status);
    later.changed(&acknowledged_status);
    assert_eq!(acknowledge(&mut attached), acknowledgement(false));
    all_quiet(&mut attached, &mut [&mut watcher, &mut later]);

    shown(&session, &mut attached, "third");
    advance(&session, 24_500);
    all_quiet(&mut attached, &mut [&mut watcher, &mut later]);
    advance(&session, 34_500);
    let second = status(NeedsAttention, 34_500, Some(2));
    all_changed(&mut attached, &mut [&mut watcher, &mut later], &second);
    all_quiet(&mut attached, &mut [&mut watcher, &mut later]);
    completed(
        session,
        &mut attached,
        &mut [&mut watcher, &mut later],
        34_500,
    );
}

/// STA-01 (Contract, final states): the command's exit is completed, with
/// the time of its result, carried by Command final and never by Status
/// changed. The exit is held while the session's controller has not sent
/// Ready, so the daemon keeps serving: a deadline that would have expired,
/// a later Set viewing and an Acknowledge change nothing, and no Status
/// changed follows the Command final on any connection.
#[test]
fn a_final_status_rejects_late_deadlines_viewing_and_acknowledgement() {
    let mut session = clocked("script");
    let (mut watcher, initial) = Watcher::open(&session);
    assert_eq!(initial, status(Idle, 0, None));
    let mut attached = Attached::open(&session, geometry(91, 37), &["status-v1"], geometry(91, 37));
    crate::script(&session, "title busy");
    assert_eq!(
        super::title_changed(&mut watcher.stream).as_deref(),
        Some("busy")
    );
    advance(&session, 1_500);
    let running = status(Running, 1_500, None);
    watcher.changed(&running);
    changed(&mut attached, &running);
    crate::issue(&session, "exit");
    assert_eq!(
        message(&mut watcher.stream),
        command_final_at(&exited(0), 1_500)
    );
    advance(&session, 11_500);
    watcher.quiet();

    let authority = attached.authority();
    let ready = Ready {
        request_id: counter(2),
        authority,
    };
    let viewing = SetViewing {
        request_id: counter(3),
        authority,
        viewing: true,
    };
    let acknowledge = Acknowledge {
        request_id: counter(4),
        authority,
    };
    let bytes = [ready.to_frame(), viewing.to_frame(), acknowledge.to_frame()]
        .iter()
        .flat_map(super::encoded)
        .collect::<Vec<u8>>();
    std::io::Write::write_all(&mut attached.stream, &bytes).unwrap();
    assert_eq!(
        attached.next_after_display(),
        command_final_at(&exited(0), 1_500)
    );
    let ready = Success::Ready(AuthorityResult {
        changed: true,
        authority,
    });
    for (request_id, outcome) in [
        (2, Outcome::Ok(ready)),
        (3, viewing_ok(true, true)),
        (4, acknowledgement(false)),
    ] {
        attached.request = request_id;
        assert_eq!(attached.response(), outcome);
    }
    attached.session_ended();
    assert_closed(&mut watcher.stream);
    session.finish();
}

/// STA-02 (Contract, controllable clock): only changed characters or a
/// changed title are activity. After the user window ends with a fresh
/// baseline, restyled characters, a cursor-only move, an identical redraw,
/// and a switch to the alternate screen showing the same characters and
/// back change nothing; neither does a resize, which replaces the baseline,
/// nor the sample after it. Changed output is agent work. A change seen at
/// the very deadline is checked first and restarts it, as does a resize
/// during agent work. The last sample before the deadline is clamped to
/// it, so attention arrives exactly ten seconds after the last restart and
/// not a millisecond earlier, after which uninterrupted silence reports
/// nothing.
#[test]
fn only_changed_characters_or_title_are_activity_at_controlled_deadlines() {
    let session = clocked("script");
    let (mut attached, mut watcher) = watched(&session);
    let running = status(Running, 0, None);
    typed(&mut attached, b"x", Some(&running));
    watcher.changed(&running);
    printed(&session, &mut attached, r"\e[2J\e[Hsampled", 1);
    advance(&session, 2_000);
    all_changed(
        &mut attached,
        &mut [&mut watcher],
        &status(Idle, 2_000, None),
    );

    let unchanged = [
        r"\e[H\e[1;31msampled\e[0m",
        r"\e[9;9H",
        r"\e[2J\e[Hsampled",
        r"\e[?1049h\e[2J\e[Hsampled",
        r"\e[?1049l",
    ];
    let mut now = 2_000;
    for (marker, bytes) in (2..).zip(unchanged) {
        printed(&session, &mut attached, bytes, marker);
        now += 1_500;
        advance(&session, now);
        all_quiet(&mut attached, &mut [&mut watcher]);
    }
    resized(&mut attached, 100, 30);
    for now in [11_000, 12_500] {
        advance(&session, now);
        all_quiet(&mut attached, &mut [&mut watcher]);
    }

    shown(&session, &mut attached, "working-now");
    advance(&session, 14_000);
    all_changed(
        &mut attached,
        &mut [&mut watcher],
        &status(Running, 14_000, None),
    );
    shown(&session, &mut attached, "still-working");
    advance(&session, 24_000);
    all_quiet(&mut attached, &mut [&mut watcher]);
    resized(&mut attached, 91, 37);
    advance(&session, 34_000);
    all_quiet(&mut attached, &mut [&mut watcher]);
    advance(&session, 43_999);
    all_quiet(&mut attached, &mut [&mut watcher]);
    advance(&session, 44_000);
    let pending = status(NeedsAttention, 44_000, Some(1));
    all_changed(&mut attached, &mut [&mut watcher], &pending);
    for now in (45_500..=60_500).step_by(1_500) {
        advance(&session, now);
        all_quiet(&mut attached, &mut [&mut watcher]);
    }
    completed(session, &mut attached, &mut [&mut watcher], 60_500);
}

/// A ready controller's Resize, accepted, then its actual geometry.
fn resized(attached: &mut Attached, cols: u32, rows: u32) {
    assert_eq!(
        attached.resize(cols, rows),
        Outcome::Ok(Success::ResizeAccepted {})
    );
    let actual = ServerMessage::Geometry(climon2_core::wire::ActualGeometry {
        period: counter(attached.period),
        geometry: geometry(cols, rows),
    });
    assert_eq!(attached.next_after_display(), actual);
}

/// CTL-03 (Contract, status case) and DSC-01 (Contract): status changes
/// reach a monitor that never had control, the controller and an
/// attachment whose control was revoked, whatever the control; a monitor
/// that did not select `status-v1` receives none, and its Initial metadata
/// and Command final carry no status. A monitor opened later receives the
/// stored change time, not its connection time.
#[test]
fn status_reaches_every_status_v1_connection_whatever_the_control() {
    let mut session = clocked("script");
    let (mut watcher, initial) = Watcher::open(&session);
    assert_eq!(initial, status(Idle, 0, None));
    let mut revoked = ready_attachment(&session, &["status-v1"]);
    let mut controller = ready_attachment(&session, &["status-v1"]);
    revoked.revoked(climon2_core::wire::RevocationReason::Takeover);
    let (plain, _) = super::monitor(&session, &["title-v1"]);
    let mut plain = Watcher {
        stream: plain,
        request: 1,
    };
    shown(&session, &mut controller, "busy");
    advance(&session, 1_500);
    let running = status(Running, 1_500, None);
    changed(&mut controller, &running);
    watcher.changed(&running);
    assert_eq!(
        message(&mut revoked.stream),
        status_changed(running.clone())
    );
    advance(&session, 5_000);
    plain.quiet();
    let (mut later, initial) = Watcher::open(&session);
    assert_eq!(initial, running);

    crate::issue(&session, "exit");
    assert_eq!(
        controller.next_after_display(),
        command_final_at(&exited(0), 5_000)
    );
    controller.session_ended();
    assert_eq!(
        message(&mut revoked.stream),
        command_final_at(&exited(0), 5_000)
    );
    assert_closed(&mut revoked.stream);
    watcher.ended(&exited(0), 5_000);
    later.ended(&exited(0), 5_000);
    assert_eq!(message(&mut plain.stream), command_final(&exited(0)));
    assert_closed(&mut plain.stream);
    session.finish();
}

/// STA-08 (Contract and Fault, through a raw peer sending stale
/// authority): Set viewing checks the current controller, period and
/// restoration; viewing true also needs readiness, while viewing false is
/// accepted during restoration. Acknowledge also needs readiness and the
/// latest reported viewing true, which a revocation clears. With nothing
/// pending Acknowledge is an explicit no-change. With attention pending,
/// viewing the session, Focus and the daemon's answer to an application's
/// terminal query leave it pending; only Acknowledge acknowledges it.
#[test]
fn set_viewing_and_acknowledge_check_authority_readiness_and_viewing() {
    let mut session = clocked("script");
    let optional = ["status-v1", "focus-v1"];
    let mut attached = Attached::open(&session, geometry(91, 37), &optional, geometry(91, 37));
    rejected(viewing(&mut attached, true), "not_ready");
    assert_eq!(viewing(&mut attached, false), viewing_ok(false, false));
    rejected(acknowledge(&mut attached), "not_ready");
    attached.confirm_ready();
    rejected(acknowledge(&mut attached), "not_viewing");
    assert_eq!(viewing(&mut attached, true), viewing_ok(true, true));
    assert_eq!(viewing(&mut attached, true), viewing_ok(false, true));
    assert_eq!(acknowledge(&mut attached), acknowledgement(false));
    assert_eq!(viewing(&mut attached, false), viewing_ok(true, false));
    rejected(acknowledge(&mut attached), "not_viewing");
    assert_eq!(viewing(&mut attached, true), viewing_ok(true, true));

    let first = attached.authority();
    let mut other = ready_attachment(&session, &[]);
    attached.revoked(climon2_core::wire::RevocationReason::Takeover);
    rejected(viewing_as(&mut attached, first, false), "not_controller");
    rejected(acknowledge_as(&mut attached, first), "not_controller");
    attached.acquire(geometry(91, 37));
    other.revoked(climon2_core::wire::RevocationReason::Takeover);
    rejected(viewing_as(&mut attached, first, false), "stale_period");
    rejected(acknowledge_as(&mut attached, first), "stale_period");
    let superseded = attached.authority();
    assert_eq!(
        attached.resize(100, 30),
        Outcome::Ok(Success::ResizeAccepted {})
    );
    attached.restored(superseded.restoration_id.get() + 1, geometry(100, 30));
    rejected(
        viewing_as(&mut attached, superseded, false),
        "stale_restoration",
    );
    rejected(
        acknowledge_as(&mut attached, superseded),
        "stale_restoration",
    );
    attached.confirm_ready();
    rejected(acknowledge(&mut attached), "not_viewing");

    advance(&session, 1_500);
    quiet(&mut attached);
    shown(&session, &mut attached, "busy");
    advance(&session, 3_000);
    changed(&mut attached, &status(Running, 3_000, None));
    advance(&session, 13_000);
    changed(&mut attached, &status(NeedsAttention, 13_000, Some(1)));
    assert_eq!(viewing(&mut attached, true), viewing_ok(true, true));
    quiet(&mut attached);
    let focused = Outcome::Ok(Success::Focus {
        changed: true,
        focused: true,
        accepted_bytes: FocusBytes::None,
    });
    let authority = attached.authority();
    let focus = attached.ask(authority, |request_id, authority| {
        Focus {
            request_id,
            authority,
            focused: true,
        }
        .to_frame()
    });
    assert_eq!(focus, focused);
    printed(&session, &mut attached, r"\e[c", 2);
    quiet(&mut attached);
    acknowledged(&mut attached, &status(Acknowledged, 13_000, Some(1)));
    crate::issue(&session, "exit");
    assert_eq!(
        attached.next_after_display(),
        command_final_at(&exited(0), 13_000)
    );
    attached.session_ended();
    super::assert_final_then_closed(&mut other.stream, &exited(0));
    session.finish();
}

/// LIM-05 (Contract and Fault, episode case, through the `test-hooks`
/// `CLIMON2_DAEMON_FIRST_EPISODE` near-exhaustion hook): the last episode
/// identifier, 18446744073709551615, is issued and acknowledged; a later
/// request that would need another changes nothing rather than wrap or
/// reuse one, while activity goes on and is reported.
#[test]
fn attention_episodes_never_wrap() {
    let session = clocked_with("script", |command| {
        command.env("CLIMON2_DAEMON_FIRST_EPISODE", LAST_EPISODE.to_string());
    });
    let (mut attached, mut watcher) = watched(&session);
    shown(&session, &mut attached, "busy");
    advance(&session, 1_500);
    all_changed(
        &mut attached,
        &mut [&mut watcher],
        &status(Running, 1_500, None),
    );
    advance(&session, 11_500);
    let last = status(NeedsAttention, 11_500, Some(LAST_EPISODE));
    all_changed(&mut attached, &mut [&mut watcher], &last);
    assert_eq!(viewing(&mut attached, true), viewing_ok(true, true));
    let acknowledged_status = status(Acknowledged, 11_500, Some(LAST_EPISODE));
    acknowledged(&mut attached, &acknowledged_status);
    watcher.changed(&acknowledged_status);

    shown(&session, &mut attached, "more");
    advance(&session, 13_000);
    advance(&session, 23_000);
    all_quiet(&mut attached, &mut [&mut watcher]);
    let running = status(Running, 23_000, None);
    typed(&mut attached, b"x", Some(&running));
    watcher.changed(&running);
    advance(&session, 25_000);
    all_changed(
        &mut attached,
        &mut [&mut watcher],
        &status(Idle, 25_000, None),
    );
    shown(&session, &mut attached, "again");
    advance(&session, 26_500);
    all_changed(
        &mut attached,
        &mut [&mut watcher],
        &status(Running, 26_500, None),
    );
    advance(&session, 36_500);
    all_changed(
        &mut attached,
        &mut [&mut watcher],
        &status(Idle, 36_500, None),
    );
    completed(session, &mut attached, &mut [&mut watcher], 36_500);
}
