//! Daemon lifecycle over revision 1 (#103), terminal behaviour, control
//! continuity and delivery: restoration before readiness, Leave and
//! reattachment, takeover with release, the controller's size, PTY write
//! ordering, resizes, retention and eviction, synchronized updates and a
//! saturating command.
use super::{
    assert_final_then_closed, encoded, end_by_kill, exited,
    interactive::{Attached, geometry, ready_attachment, rejected},
};
use crate::Session;
use climon2_core::{
    hello::Lifecycle,
    wire::{
        ActualGeometry, ControlRecord, GrantReason, Outcome, Resize, Response, RevocationReason,
        ServerMessage, Success,
    },
};
use std::{
    fs,
    io::Write,
    num::NonZeroU16,
    thread,
    time::{Duration, Instant},
};

/// CTL-05 (Contract: input only after the complete restoration is confirmed).
/// Section 6.2 over revision 1: an attachment to an application waiting in
/// its alternate screen receives a restoration that already holds the
/// screen and its modes (alternate screen, mouse, bracketed paste and
/// application cursor keys), drawn once. Input before Ready is `not_ready`;
/// after it, input reaches the waiting application, whose reply and exit
/// follow.
#[test]
fn a_waiting_alternate_screen_is_restored_before_input_is_admitted() {
    let mut session = Session::start("tui-restore");
    crate::wait_file(&session.directory.path().join("tui-ready"));
    let mut attached = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    for sequence in [
        b"RESTORED-DISPLAY".as_slice(),
        b"\x1b[?1049h",
        b"\x1b[?1000h",
        b"\x1b[?2004h",
        b"\x1b[?1h",
    ] {
        assert!(
            crate::count(&attached.display, sequence) > 0,
            "restoration omitted {sequence:?}"
        );
    }
    let draws = session.directory.path().join("draw-count");
    assert_eq!(fs::read_to_string(draws).unwrap(), "1");
    rejected(attached.input(b"x"), "not_ready");
    attached.confirm_ready();
    attached.typed(b"x");
    let accepted = session.directory.path().join("input-accepted");
    crate::wait_contents(&accepted, |value| value == b"x");
    attached.ended(&exited(42));
    assert_eq!(crate::count(&attached.display, b"INPUT-ACCEPTED"), 1);
    session.finish();
}

/// COM-03 (Contract, Leave).
/// Section 6.2 release ordering over revision 1: Leave releases the
/// attachment's presentation exactly, then reports the detached
/// attachment; the session outlives the Leave, advertises it, and a new
/// attachment's restoration resumes it with the fixture's output once. The
/// command's last bytes reach the new attachment once, and its release
/// ends its output.
#[test]
fn leave_releases_then_reports_and_a_reattachment_resumes() {
    let mut session = Session::start("exit");
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Starting, 0, false)
    );
    let mut first = ready_attachment(&session, &[]);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );
    let mark = first.display.len();
    first.leave();
    assert_eq!(first.display[mark..], crate::release_at(3, 14));
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );

    let mut second = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    assert_eq!(crate::count(&second.display, b"fixture-ready"), 1);
    second.confirm_ready();
    let mark = second.display.len();
    second.typed(b"go\n");
    second.ended(&exited(42));
    let output = &second.display[mark..];
    assert!(output.ends_with(&crate::release_at(4, 12)), "{output:?}");
    assert_eq!(crate::count(output, b"final-bytes"), 1);
    session.finish();
}

/// A terminal of the attachment's 91x37 host.
fn host() -> vt100::Parser {
    let size = |value| NonZeroU16::new(value).unwrap();
    vt100::Parser::new(size(37), size(91), 0)
}

/// COM-03 (Contract, Acquire) and CTL-05 (Contract: a revoked controller).
/// Section 6.2 over revision 1: a newer interactive Open takes control. The
/// old controller's release leaves the alternate screen it entered, once,
/// before its Control revoked (`takeover`), and its host's cursor is no
/// longer where the application left it. Its Acquire takes control back,
/// revoking the other attachment after its own release, and its
/// restoration puts its host's cursor back. The application's exit from
/// the alternate screen releases it once more; the revoked attachment
/// receives no output, only the Command final.
#[test]
fn takeover_releases_the_alternate_screen_and_the_exit_leaves_it() {
    let mut session = Session::start("tui-restore");
    crate::wait_file(&session.directory.path().join("tui-ready"));
    let mut first = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    let mut terminal = host();
    terminal.process(&first.display);
    first.confirm_ready();
    let mark = first.display.len();
    let mut second = Attached::open(&session, geometry(80, 24), &[], geometry(80, 24));
    first.revoked(RevocationReason::Takeover);
    let released = &first.display[mark..];
    assert_eq!(crate::count(released, b"\x1b[?1049l"), 1);
    terminal.process(released);
    assert_ne!(terminal.screen().cursor_position(), (3, 20));
    assert_eq!(crate::count(&second.display, b"RESTORED-DISPLAY"), 1);
    second.confirm_ready();

    let (first_mark, second_mark) = (first.display.len(), second.display.len());
    first.acquire(geometry(91, 37));
    second.revoked(RevocationReason::Takeover);
    assert_eq!(
        crate::count(&second.display[second_mark..], b"\x1b[?1049l"),
        1
    );
    terminal.process(&first.display[first_mark..]);
    assert_eq!(terminal.screen().cursor_position(), (3, 20));
    first.confirm_ready();
    let mark = first.display.len();
    first.typed(b"x");
    first.ended(&exited(42));
    let output = &first.display[mark..];
    let accepted = output
        .windows(14)
        .position(|bytes| bytes == b"INPUT-ACCEPTED")
        .unwrap();
    assert_eq!(crate::count(&output[accepted..], b"\x1b[?1049l"), 1);
    assert_final_then_closed(&mut second.stream, &exited(42));
    session.finish();
}

/// CTL-05 (Contract: only the controller's input and size apply) and CTL-04
/// (Contract: hand-back).
/// Section 1.5/1.6 over revision 1: the controller's size drives the PTY.
/// A newer Open takes control at its own 80x24; the revoked attachment's
/// Input and Resize are `not_controller` and reach nothing. Its Acquire
/// applies its own host size, and its Leave hands control back to the
/// other attachment (`hand_back`), whose size the command's terminal takes
/// again. The command sees each line at its controller's size, and no
/// other.
#[test]
fn the_controller_drives_the_size_and_a_hand_back_restores_the_remaining_size() {
    let mut session = Session::start("lines");
    let mut first = ready_attachment(&session, &[]);
    first.typed(b"a1\n");
    crate::wait_line(&session, "a1 91x37");

    let mut second = Attached::open(&session, geometry(80, 24), &[], geometry(80, 24));
    first.revoked(RevocationReason::Takeover);
    rejected(first.input(b"stale\n"), "not_controller");
    rejected(first.resize(100, 30), "not_controller");
    second.confirm_ready();
    second.typed(b"b1\n");
    crate::wait_line(&session, "b1 80x24");

    first.acquire(geometry(91, 37));
    second.revoked(RevocationReason::Takeover);
    rejected(first.input(b"early\n"), "not_ready");
    first.confirm_ready();
    first.typed(b"a2\n");
    crate::wait_line(&session, "a2 91x37");

    first.leave();
    second.granted(GrantReason::HandBack, geometry(80, 24));
    second.confirm_ready();
    second.typed(b"b2\nend\n");
    second.ended(&exited(0));
    session.finish();
    assert_eq!(
        fs::read_to_string(session.directory.path().join("lines")).unwrap(),
        "a1 91x37\nb1 80x24\na2 91x37\nb2 80x24\n"
    );
}

/// INP-03 (Contract) and CTL-05 (Fault, through the `pty-write-partial` hook).
/// Section 1.5 paste cleanup over revision 1, through the
/// `pty-write-partial` test hook: a paste cut short by a takeover is closed
/// with exactly its end marker, written whole across one-byte PTY writes,
/// before the next controller's input. Only part of the paste's 400 bytes
/// reached the application.
#[test]
fn a_revoked_paste_is_closed_whole_across_one_byte_writes() {
    let mut session = Session::configured("paste-record", |command, _| {
        command.env("CLIMON2_DAEMON_FAULT", "pty-write-partial");
    });
    let mut first = ready_attachment(&session, &[]);
    let mut paste = b"\x1b[200~".to_vec();
    paste.extend([b'p'; 400]);
    first.typed(&paste);
    let path = session.directory.path().join("pasted");
    crate::wait_contents(&path, |contents| contents.len() >= 8);

    let mut second = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    first.revoked(RevocationReason::Takeover);
    second.confirm_ready();
    second.typed(b"after-END");
    second.ended(&exited(0));
    assert_final_then_closed(&mut first.stream, &exited(0));
    session.finish();

    let pasted = fs::read(&path).unwrap();
    let payload = pasted
        .strip_prefix(b"\x1b[200~".as_slice())
        .and_then(|rest| rest.strip_suffix(b"\x1b[201~after-END".as_slice()))
        .unwrap_or_else(|| panic!("{:?}", String::from_utf8_lossy(&pasted)));
    assert!(payload.iter().all(|byte| *byte == b'p'), "{payload:?}");
    assert!((1..400).contains(&payload.len()), "{}", payload.len());
}

/// TRM-02 (Contract: the daemon answers session queries).
/// Section 3.3 one PTY writer over revision 1, through the
/// `pty-write-partial` test hook: the daemon's reply to a terminal query
/// that the application makes while the controller's input is being written
/// one byte at a time follows that input whole and never splits it.
#[test]
fn a_terminal_reply_never_interleaves_with_input_being_written() {
    let mut session = Session::configured("reply-record", |command, _| {
        command.env("CLIMON2_DAEMON_FAULT", "pty-write-partial");
    });
    let mut attached = ready_attachment(&session, &[]);
    attached.typed(&[b'i'; 100]);
    let path = session.directory.path().join("recorded");
    crate::wait_contents(&path, |contents| contents.contains(&b'c'));
    attached.typed(b"END");
    attached.ended(&exited(0));
    session.finish();

    let recorded = fs::read(&path).unwrap();
    let reply = recorded
        .strip_prefix([b'i'; 100].as_slice())
        .and_then(|rest| rest.strip_suffix(b"END".as_slice()))
        .unwrap_or_else(|| panic!("{:?}", String::from_utf8_lossy(&recorded)));
    assert!(
        reply.starts_with(b"\x1b[?") && reply.ends_with(b"c") && !reply.contains(&b'i'),
        "{:?}",
        String::from_utf8_lossy(reply)
    );
}

/// CTL-05 (Contract: superseded restoration) and COM-05 (Contract).
/// Section 6.2 replacement restoration over revision 1: a Resize before
/// Ready replaces the restoration within its period, at the new size. The
/// replacement first leaves the alternate screen the first restoration
/// entered, exactly once, then restores the application's screen once.
/// Ready under the replaced restoration is `stale_restoration`; under the
/// current one it admits input.
#[test]
fn a_resize_before_ready_replaces_the_restoration_and_leaves_the_alternate_screen_once() {
    let mut session = Session::start("tui-restore");
    crate::wait_file(&session.directory.path().join("tui-ready"));
    let mut attached = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    assert_eq!(crate::count(&attached.display, b"\x1b[?1049h"), 1);
    let replaced = attached.authority();
    assert_eq!(
        attached.resize(100, 40),
        Outcome::Ok(Success::ResizeAccepted {})
    );
    let mark = attached.display.len();
    attached.restored(attached.restoration + 1, geometry(100, 40));
    let replacement = &attached.display[mark..];
    assert!(replacement.starts_with(b"\x1b[?1049l"), "{replacement:?}");
    assert_eq!(crate::count(replacement, b"\x1b[?1049l"), 1);
    assert_eq!(crate::count(replacement, b"RESTORED-DISPLAY"), 1);
    rejected(attached.ready_as(replaced), "stale_restoration");
    attached.confirm_ready();
    attached.typed(b"x");
    attached.ended(&exited(42));
    session.finish();
}

/// COM-05 (Contract: Geometry follows a Resize's acceptance).
/// Section 6.2 resize burst over revision 1: three Resize requests of a
/// ready controller in one write are each accepted, and the session reports
/// one Geometry, at the last size: the command's terminal is resized once,
/// to it.
#[test]
fn a_resize_burst_is_one_pty_resize_to_the_last_size() {
    let mut session = Session::start("winch");
    let mut attached = ready_attachment(&session, &[]);
    let authority = attached.authority();
    let mut burst = Vec::new();
    let mut expected = Vec::new();
    for cols in [100, 101, 102] {
        let request_id = attached.next();
        let resize = Resize {
            request_id,
            authority,
            geometry: geometry(cols, 30),
        };
        burst.extend(encoded(&resize.to_frame()));
        expected.push(ServerMessage::Response(Response {
            request_id,
            outcome: Outcome::Ok(Success::ResizeAccepted {}),
        }));
    }
    expected.push(ServerMessage::Geometry(ActualGeometry {
        period: authority.period,
        geometry: geometry(102, 30),
    }));
    attached.stream.write_all(&burst).unwrap();
    let received: Vec<_> = expected
        .iter()
        .map(|_| attached.next_after_display())
        .collect();
    assert_eq!(received, expected);
    attached.typed(b"go\n");
    attached.ended(&exited(0));
    assert_eq!(
        fs::read_to_string(session.directory.path().join("resized")).unwrap(),
        "1 102x30"
    );
    session.finish();
}

/// Takes over with fresh 91x37 interactive attachments, each left without
/// Ready, until one's restoration includes `marker`, and returns that
/// restoration. The fixture has already written it, so only the daemon's
/// read of its terminal is awaited.
fn restoration_through(session: &Session, clients: &mut Vec<Attached>, marker: &str) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let client = Attached::open(session, geometry(91, 37), &[], geometry(91, 37));
        let restored = client.display.clone();
        clients.push(client);
        if crate::count(&restored, marker.as_bytes()) > 0 {
            return restored;
        }
        assert!(Instant::now() < deadline, "{marker} never restored");
    }
}

/// LIM-03 (Contract: retained history).
/// Section 6.2 retention with an unfinished restoration, over revision 1:
/// scrolling while the controller has not confirmed its restoration still
/// honours `max_rows`, so a takeover's restoration has no history for 0 and
/// only the newest 2 rows above the screen for 2.
#[test]
fn retention_with_an_unconfirmed_restoration_keeps_only_the_newest_rows() {
    for max_rows in [0, 2] {
        let session = Session::start(&format!("retain-{max_rows}"));
        crate::script(&session, "rows 1 100");
        let mut clients = vec![Attached::open(
            &session,
            geometry(91, 37),
            &[],
            geometry(91, 37),
        )];
        crate::script(&session, "rows 101 200");
        let restored = restoration_through(&session, &mut clients, "row-0200");
        // 37 rows show rows 165..=200 above the empty cursor row.
        let expected: Vec<u32> = (165 - max_rows..=200).collect();
        assert_eq!(crate::numbered_rows(&restored), expected, "{max_rows}");
        end_by_kill(session);
    }
}

/// LIM-03 (Contract: evicted history).
/// Section 6.2 identical-row eviction over revision 1: identical rows still
/// evict history and leave the discard notice once; clearing the history
/// clears the notice too, so a later restoration has none.
#[test]
fn identical_rows_evict_and_cleared_history_has_no_stale_notice() {
    let session = Session::start("retain-2");
    let mut clients = Vec::new();
    crate::script(&session, "same 100");
    crate::script(&session, "rows 1 1");
    let restored = restoration_through(&session, &mut clients, "row-0001");
    let notice = b"[climon2: older session scrollback was discarded]";
    assert_eq!(crate::count(&restored, notice), 1);
    crate::script(&session, "clear");
    crate::script(&session, "rows 2 2");
    let restored = restoration_through(&session, &mut clients, "row-0002");
    assert_eq!(crate::count(&restored, notice), 0);
    assert_eq!(crate::numbered_rows(&restored), [1, 2]);
    end_by_kill(session);
}

/// COM-05 (Contract: a grant is never held by synchronized updates).
/// Section 6.2 sync latch over revision 1: an application that begins a
/// synchronized update every 20 ms and never ends one does not hold a
/// grant: the interactive Open is granted and its restoration completes
/// through the latch, so it confirms Ready. Its loss then ends the session.
#[test]
fn repeated_sync_begins_do_not_hold_a_grant() {
    let mut session = Session::start("sync");
    crate::wait_file(&session.directory.path().join("sync-ready"));
    let mut attached = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    attached.confirm_ready();
    drop(attached);
    session.finish();
}

/// COM-05 (Contract: synchronized updates hold output back).
/// Section 6.2 sync latch over revision 1: an open synchronized update is
/// held back from the ready controller until its 150 ms latch expires, then
/// released once, with its begin once, each of three times.
#[test]
fn an_open_synchronized_update_is_not_released_before_its_deadline() {
    let session = Session::start("retain-10");
    let mut attached = ready_attachment(&session, &[]);
    for _ in 0..3 {
        let mark = attached.display.len();
        let released = crate::count(&attached.display, b"\x1b[?2026l");
        let started = Instant::now();
        crate::script(&session, "sync");
        attached.until_displayed(b"\x1b[?2026l", released + 1);
        assert_eq!(crate::count(&attached.display[mark..], b"\x1b[?2026h"), 1);
        assert!(
            started.elapsed() >= Duration::from_millis(150),
            "synchronized update ended early: {:?}",
            started.elapsed()
        );
    }
    end_by_kill(session);
}

/// LIM-01 (Contract: one connection's backlog stalls nothing else) and COM-04
/// (Contract).
/// #20 over revision 1, saturation: an 8 MiB burst of output to a ready
/// controller that is not reading yet arrives exactly, in Display chunks
/// of at most 65,535 bytes, followed by the release, within 15 s, which a
/// pump of one read per tick cannot meet, while probes stay serviceable.
/// The whole stream is compared, so terminal noise cannot pass as data.
#[test]
fn saturation_backpressures_the_pty_without_blocking_probes_or_losing_bytes() {
    let mut session = Session::start("flood");
    let mut attached = ready_attachment(&session, &[]);
    let start = attached.display.len();
    attached.typed(b"go\n");
    let started = Instant::now();
    thread::sleep(Duration::from_millis(300));
    assert!(!session.directory.path().join("flood-finished").exists());
    drop(session.connect());
    attached.ended(&exited(17));
    let elapsed = started.elapsed();
    let output = &attached.display[start..];
    let mut expected = b"go\r\n".to_vec();
    expected.resize(expected.len() + 8 * 1024 * 1024, b'x');
    let release = crate::release_at(37, 47);
    let expected_len = expected.len();
    assert!(
        output.len() == expected_len + release.len()
            && output[..expected_len] == expected
            && output[expected_len..] == release,
        "flood output differs ({} bytes, prefix {:?}, suffix {:?})",
        output.len(),
        &output[..output.len().min(160)],
        &output[output.len().saturating_sub(160)..]
    );
    assert!(elapsed < Duration::from_secs(15), "flood took {elapsed:?}");
    session.finish();
}
