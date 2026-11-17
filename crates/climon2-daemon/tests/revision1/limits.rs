//! Bounded delivery and counters that never wrap (#90): the daemon's
//! connection, queue, no-progress and Ready-deadline bounds on revision-1
//! connections, its frame and text bounds, and the control period,
//! restoration ID and request ID at the end of their range.
use super::{
    assert_closed, assert_fatal, assert_rejected, counter, end_by_kill, frame,
    interactive::{Attached, detached, geometry, ready_attachment, rejected},
    leave, limit, message, monitor, open,
    passive::end_script,
    peer, raw_frame, request, title_changed,
};
use crate::Session;
use climon2_core::{
    hello::Lifecycle,
    protocol::{self, ClientMessage},
    wire::{
        self, Acquire, AuthorityIds, AuthorityResult, Available, ControlRecord, FatalCode,
        GrantReason, Leave, Mode, Outcome, Response, RevocationReason, ServerMessage, Success,
        WireError,
    },
};
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// The largest counter value; no counter follows it.
pub(super) const LAST: u64 = u64::MAX;

/// A `lines` session whose last issued control period and restoration ID
/// are `period` and `restoration`, through the `CLIMON2_DAEMON_ISSUED`
/// test hook.
pub(super) fn issued(period: u64, restoration: u64) -> Session {
    Session::configured("lines", |command, _| {
        command.env("CLIMON2_DAEMON_ISSUED", format!("{period},{restoration}"));
    })
}

pub(super) fn ids(period: u64, restoration: u64) -> AuthorityIds {
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
/// which ends its stream, and an Acquire by a
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
    end_by_kill(session);
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
    end_by_kill(session);
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
    end_by_kill(session);
}

/// The title `long-titles` sets for `number`: `t-<number>-` padded with `x`
/// to 4,000 bytes.
fn long_title(number: usize) -> String {
    let head = format!("t-{number}-");
    format!("{head}{}", "x".repeat(4000 - head.len()))
}

/// The titles `long-titles 1 <count>` sets, in order.
fn long_titles(count: usize) -> Vec<String> {
    (1..=count).map(long_title).collect()
}

/// The fixture acknowledged its first `issued` script commands: it wrote
/// all their output, so the daemon read the PTY on.
fn acknowledged(session: &Session, issued: usize) {
    crate::wait_contents(&session.directory.path().join("acks"), |acks| {
        String::from_utf8_lossy(acks).lines().count() == issued
    });
}

/// Reads `count` Title changed records from `stream` on another thread, as
/// fast as they arrive.
fn reader(mut stream: UnixStream, count: usize) -> JoinHandle<(UnixStream, Vec<String>)> {
    thread::spawn(move || {
        let titles = (0..count)
            .map(|_| title_changed(&mut stream).unwrap())
            .collect();
        (stream, titles)
    })
}

/// Reads up to `count` Title changed records from `stream` on another
/// thread, pausing 250 ms after each while `paced` holds: the daemon sees
/// progress several times a second, far from its five seconds without any,
/// yet the reader drains about 16 KB a second, far slower than a flood of
/// long titles arrives. It stops early when the stream ends, between frames
/// or inside one, as a failed connection's may.
fn paced_reader(
    mut stream: UnixStream,
    count: usize,
    paced: Arc<AtomicBool>,
) -> JoinHandle<Vec<String>> {
    thread::spawn(move || {
        let mut titles = Vec::new();
        while titles.len() < count {
            let frame = match wire::read_frame(&mut stream, limit()) {
                Ok(Some(frame)) => frame,
                Ok(None) | Err(WireError::Fatal(FatalCode::TruncatedFrame)) => break,
                Err(error) => panic!("the paced reader failed: {error:?}"),
            };
            match ServerMessage::from_frame(&frame).unwrap() {
                ServerMessage::TitleChanged(changed) => titles.push(changed.title.unwrap()),
                other => panic!("expected Title changed, got {other:?}"),
            }
            if paced.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(250));
            }
        }
        titles
    })
}

/// LIM-01 (Contract, queue bound): a passive monitor that keeps reading,
/// but far slower than 5,000 long titles (about 20 MB of Title changed)
/// arrive, falls behind until its output queue would exceed its 16 MiB
/// bound, and is then explicitly failed: its stream ends after a gap-free
/// prefix of the titles. It made progress throughout, so its five seconds
/// without progress never ran out: the queue bound failed it. Meanwhile
/// another monitor receives every title in order, the daemon reads the
/// command's whole output (the fixture acknowledges the flood only once it
/// is written) and still answers a probe: the congested connection stalls
/// no other, nor the PTY, nor control.
#[test]
fn a_monitor_falling_behind_beyond_its_queue_bound_is_failed_alone() {
    const TITLES: usize = 5000;
    let session = Session::start("script");
    detached(&session);
    let paced = Arc::new(AtomicBool::new(true));
    let (slow, _) = monitor(&session, &["title-v1"]);
    let slow = paced_reader(slow, TITLES, paced.clone());
    let (fast, _) = monitor(&session, &["title-v1"]);
    let fast = reader(fast, TITLES);
    let issued = crate::issue(&session, &format!("long-titles 1 {TITLES}"));
    let (mut fast, titles) = fast.join().unwrap();
    assert_eq!(titles, long_titles(TITLES));
    acknowledged(&session, issued);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );
    // Every title was queued for it before the fast monitor read the last.
    paced.store(false, Ordering::SeqCst);
    let received = slow.join().unwrap();
    assert!(received.len() < TITLES, "the slow monitor kept up");
    assert_eq!(received, long_titles(received.len()));
    leave(&mut fast, 2);
    end_script(session);
}

/// LIM-01 (Contract, queue bound, below it): a passive monitor reading far
/// slower than 3,000 long titles (about 12 MB of Title changed, within its
/// 16 MiB bound) arrive falls behind but is never failed. While it is
/// behind, another monitor receives every title and a later one, the
/// command's output is read, and a probe is answered; once it reads at full
/// speed it receives every title too, in order, without a gap.
#[test]
fn a_monitor_behind_within_its_queue_bound_receives_every_title() {
    const TITLES: usize = 3000;
    let session = Session::start("script");
    detached(&session);
    let paced = Arc::new(AtomicBool::new(true));
    let (slow, _) = monitor(&session, &["title-v1"]);
    let slow = paced_reader(slow, TITLES + 1, paced.clone());
    let (fast, _) = monitor(&session, &["title-v1"]);
    let fast = reader(fast, TITLES);
    let issued = crate::issue(&session, &format!("long-titles 1 {TITLES}"));
    let (mut fast, titles) = fast.join().unwrap();
    assert_eq!(titles, long_titles(TITLES));
    acknowledged(&session, issued);
    crate::script(&session, "title after");
    assert_eq!(title_changed(&mut fast).as_deref(), Some("after"));
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );
    assert!(!slow.is_finished(), "the slow monitor caught up too soon");
    paced.store(false, Ordering::SeqCst);
    let mut expected = long_titles(TITLES);
    expected.push("after".into());
    assert_eq!(slow.join().unwrap(), expected);
    leave(&mut fast, 2);
    end_script(session);
}

/// Whether a new connection to `session` is served: its hello is answered
/// rather than the connection closed unanswered, which may also fail the
/// hello's write or reset the stream.
fn served(session: &Session) -> bool {
    let mut stream = UnixStream::connect(&session.socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let hello = ClientMessage::Hello { protocol: 1 }.to_frame().unwrap();
    let mut bytes = Vec::new();
    protocol::write_frame(&mut bytes, &hello).unwrap();
    stream.write_all(&bytes).is_ok() && matches!(stream.read(&mut [0]), Ok(1))
}

/// LIM-01 (Contract, existing connection and no-progress bounds on
/// revision 1): 64 revision-1 connections, passive monitors, are served and
/// a 65th connection is closed unanswered. One monitor stops reading while
/// 1,000 long titles (about 4 MB, within its queue bound) arrive; the
/// others keep their slots. Only after five seconds without progress is
/// the stalled monitor lost, which frees its slot for a new connection,
/// and its stream ends after a gap-free prefix of the titles.
#[test]
fn revision1_connections_share_the_cap_and_a_stalled_monitor_is_lost_after_five_seconds() {
    const TITLES: usize = 1000;
    let session = Session::start("script");
    detached(&session);
    let (stalled, _) = monitor(&session, &["title-v1"]);
    let (fast, _) = monitor(&session, &["title-v1"]);
    let others: Vec<_> = (0..62).map(|_| monitor(&session, &[]).0).collect();
    assert!(!served(&session), "a 65th connection was served");
    let fast = reader(fast, TITLES);
    let flooded = Instant::now();
    let issued = crate::issue(&session, &format!("long-titles 1 {TITLES}"));
    let (mut fast, titles) = fast.join().unwrap();
    assert_eq!(titles, long_titles(TITLES));
    acknowledged(&session, issued);
    let deadline = flooded + Duration::from_secs(60);
    while !served(&session) {
        assert!(
            Instant::now() < deadline,
            "the stalled monitor kept its slot"
        );
        thread::sleep(Duration::from_millis(50));
    }
    assert!(flooded.elapsed() >= Duration::from_secs(5));
    let received = paced_reader(stalled.try_clone().unwrap(), TITLES, Arc::default())
        .join()
        .unwrap();
    assert!(received.len() < TITLES, "the stalled monitor was not lost");
    assert_eq!(received, long_titles(received.len()));
    drop((stalled, others));
    leave(&mut fast, 2);
    end_script(session);
}

/// LIM-01 (Contract, existing Ready deadline on revision 1): an interactive
/// controller that receives its whole restoration but never confirms Ready
/// is lost when the restoration deadline, 30 s for a restoration this
/// small, passes: its stream ends, and as an unexpected departure it
/// promotes the registered monitor (`successor`). The size-scaled part of
/// the deadline, a second per 4 MiB, is the unit test
/// `session::delivery::tests::the_deadline_allows_a_second_per_four_mebibytes_of_prefix`.
#[test]
fn a_revision1_controller_withholding_ready_is_lost_at_the_restoration_deadline() {
    let session = Session::start("lines");
    let opened = Instant::now();
    let mut withholding = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    let (watching, _) = monitor(&session, &[]);
    watching
        .set_read_timeout(Some(Duration::from_secs(90)))
        .unwrap();
    let mut watching = Attached::passive(watching);
    watching.granted(GrantReason::Successor, geometry(91, 37));
    assert!(opened.elapsed() >= Duration::from_secs(30));
    withholding.lost();
    watching.confirm_ready();
    watching.leave();
    end_by_kill(session);
}

/// LIM-01 (Contract, derived release-constant bounds): the most escaped
/// title the 4,096-byte title bound lets through, 4,096 `"` that JSON
/// doubles, reaches a monitor whole, as Title changed and in a later
/// Open's Initial metadata, and each of those records fits the 16,384-byte
/// metadata record bound.
#[test]
fn the_most_escaped_title_fits_a_metadata_record() {
    let title = "\"".repeat(4096);
    let session = Session::start("script");
    detached(&session);
    let (mut watching, _) = monitor(&session, &["title-v1"]);
    crate::script(&session, &format!("title {title}"));
    // Each record's frame, its type byte and body, fits 16,384 bytes.
    let changed = frame(&mut watching);
    assert!(changed.body().len() < 16_384, "{}", changed.body().len());
    match ServerMessage::from_frame(&changed).unwrap() {
        ServerMessage::TitleChanged(changed) => assert_eq!(changed.title, Some(title.clone())),
        other => panic!("expected Title changed, got {other:?}"),
    }
    let mut later = peer(&session);
    request(&mut later, &open(1, &[], &["title-v1"], Mode::Monitor));
    assert!(matches!(message(&mut later), ServerMessage::Response(_)));
    let initial = frame(&mut later);
    assert!(initial.body().len() < 16_384, "{}", initial.body().len());
    match ServerMessage::from_frame(&initial).unwrap() {
        ServerMessage::InitialMetadata(metadata) => {
            assert_eq!(metadata.title(), &Available::Available(Some(title)));
        }
        other => panic!("expected Initial metadata, got {other:?}"),
    }
    leave(&mut watching, 2);
    leave(&mut later, 2);
    end_script(session);
}
