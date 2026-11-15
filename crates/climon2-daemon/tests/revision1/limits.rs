//! Bounded delivery and counters that never wrap (#90): the daemon's
//! connection, queue, no-progress and Ready-deadline bounds on revision-1
//! connections, its frame and text bounds, and the control period,
//! restoration ID and request ID at the end of their range.
use super::{
    assert_closed, assert_fatal, assert_rejected, counter,
    interactive::{Attached, geometry, ready_attachment, rejected},
    leave, message, monitor, open, peer, raw_frame, request,
};
use crate::Session;
use climon2_core::{
    hello::Lifecycle,
    protocol::{ClientMessage, ServerMessage as PrivateMessage, TerminalSize},
    wire::{
        Acquire, AuthorityIds, AuthorityResult, ControlRecord, Leave, Mode, Outcome, Response,
        RevocationReason, ServerMessage, Success,
    },
};
use std::io::Write;

/// The largest counter value; no counter follows it.
const LAST: u64 = u64::MAX;

/// A `lines` session whose last issued control period and restoration ID
/// are `period` and `restoration`, through the `CLIMON2_DAEMON_ISSUED`
/// test hook.
fn issued(period: u64, restoration: u64) -> Session {
    Session::configured("lines", |command, _| {
        command.env("CLIMON2_DAEMON_ISSUED", format!("{period},{restoration}"));
    })
}

fn ids(period: u64, restoration: u64) -> AuthorityIds {
    AuthorityIds {
        period: counter(period),
        restoration_id: counter(restoration),
    }
}

/// An interactive Open at 91x37 is rejected with `code`, and its stream
/// ends.
fn open_rejected(session: &Session, code: &str) {
    let mut stream = peer(session);
    let interactive = Mode::Interactive(geometry(91, 37));
    request(&mut stream, &open(1, &[], &[], interactive));
    assert_rejected(&mut stream, 1, code);
    assert_closed(&mut stream);
}

/// Acquire by `attached`, rejected with `code` without effects: its
/// Response is the next record.
fn acquire_rejected(attached: &mut Attached, code: &str) {
    attached.send(|request_id| Acquire { request_id }.to_frame());
    rejected(attached.response(), code);
}

/// LIM-05 (Contract and Fault, through the `CLIMON2_DAEMON_ISSUED` test
/// hook): the session issues the last control period and restoration ID,
/// 18,446,744,073,709,551,615, and then refuses, with `resource_limit` and
/// without effects, all work that needs a new one: an interactive Open,
/// which ends its stream, a private attach, and an Acquire by a
/// non-controlling attachment or a registration, whose streams go on. The
/// controller keeps its authority and its own Acquire stays a no-op. Its
/// Leave hands control back to nobody: no attachment can control again, so
/// the other attachment is explicitly failed (its stream ends), the
/// registration is not promoted, and the session stays detached. No
/// identifier wraps or is reused.
#[test]
fn the_last_control_period_is_issued_then_new_control_is_refused_without_effects() {
    let session = issued(LAST - 2, LAST - 2);
    let mut first = ready_attachment(&session, &[]);
    assert_eq!(first.authority(), ids(LAST - 1, LAST - 1));
    let mut watching = Attached::registered(&session);
    let mut last = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    first.revoked(RevocationReason::Takeover);
    assert_eq!(last.authority(), ids(LAST, LAST));
    last.confirm_ready();
    last.typed(b"last\n");
    crate::wait_line(&session, "last 91x37");

    open_rejected(&session, "resource_limit");
    let mut private = session.connect();
    let size = TerminalSize { cols: 91, rows: 37 };
    crate::send(&mut private, ClientMessage::Attach(size));
    let refusal = PrivateMessage::Error {
        message: "attach refused: resource_limit".into(),
    };
    assert_eq!(crate::receive(&mut private), refusal);
    acquire_rejected(&mut first, "resource_limit");
    acquire_rejected(&mut watching, "resource_limit");
    last.send(|request_id| Acquire { request_id }.to_frame());
    let kept = Success::Acquired(AuthorityResult {
        changed: false,
        authority: ids(LAST, LAST),
    });
    assert_eq!(last.response(), Outcome::Ok(kept));
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 2, true)
    );
    last.typed(b"kept\n");
    crate::wait_line(&session, "kept 91x37");

    last.leave();
    first.closed();
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );
    acquire_rejected(&mut watching, "resource_limit");
    watching.send(|request_id| Leave { request_id }.to_frame());
    let left = Success::Left {
        attachment_detached: false,
    };
    assert_eq!(watching.response(), Outcome::Ok(left));
    watching.closed();
    crate::kill(session);
}

/// LIM-05 (Contract and Fault, through the `CLIMON2_DAEMON_ISSUED` test
/// hook, at the limit and one past): a monitor-mode controller's Resize to
/// a new geometry takes the last restoration ID for its replacement
/// restoration; the next such Resize is refused with `resource_limit`
/// without effects, so the session keeps its size. A grant needs a new
/// restoration ID as well as a new period, so an interactive Open is
/// refused too, though periods remain.
#[test]
fn the_last_restoration_id_is_issued_then_a_resize_needing_another_is_refused() {
    let session = issued(5, LAST - 2);
    let mut watching = Attached::registered(&session);
    watching.acquire(geometry(91, 37));
    assert_eq!(watching.authority(), ids(6, LAST - 1));
    watching.confirm_ready();
    let accepted = Outcome::Ok(Success::ResizeAccepted {});
    assert_eq!(watching.resize(100, 30), accepted);
    watching.regeometried(geometry(100, 30));
    assert_eq!(watching.authority(), ids(6, LAST));
    watching.confirm_ready();

    rejected(watching.resize(110, 35), "resource_limit");
    watching.typed(b"unchanged\n");
    crate::wait_line(&session, "unchanged 100x30");
    open_rejected(&session, "resource_limit");
    watching.leave();
    crate::kill(session);
}

/// LIM-05 (Contract, request correlation): a request may carry the last
/// request ID, 18,446,744,073,709,551,615, and is served. No ID can follow
/// it, so a later request, even one with ID 1, is connection-fatal
/// `invalid_request_id`: the sequence never wraps. One past the last value
/// is not a counter, and is `invalid_request_id` too.
#[test]
fn the_last_request_id_is_served_and_no_request_follows_it() {
    let session = Session::start("lines");
    let mut stream = peer(&session);
    request(&mut stream, &open(LAST, &[], &[], Mode::Monitor));
    match message(&mut stream) {
        ServerMessage::Response(Response {
            request_id,
            outcome: Outcome::Ok(Success::Opened { .. }),
        }) => assert_eq!(request_id, counter(LAST)),
        other => panic!("expected the Open's success, got {other:?}"),
    }
    assert!(matches!(
        message(&mut stream),
        ServerMessage::InitialMetadata(_)
    ));
    request(
        &mut stream,
        &Leave {
            request_id: counter(1),
        },
    );
    assert_fatal(&mut stream, "invalid_request_id");

    let (mut stream, _) = monitor(&session, &[]);
    let past = br#"{"request_id":"18446744073709551616"}"#;
    stream.write_all(&raw_frame(0x37, past)).unwrap();
    assert_fatal(&mut stream, "invalid_request_id");
    let (mut stream, _) = monitor(&session, &[]);
    leave(&mut stream, 2);
    crate::kill(session);
}
