//! Notifications and progress (#98): the session daemon interprets OSC 9
//! text, OSC 99 chunks, OSC 777 `notify` and OSC 9;4 progress in the
//! command's output, before filtering, and reports them through
//! `notification-v1` (with `status-v1`) and `progress-v1` to every
//! subscribed connection, whatever its control, driven by the `test-hooks`
//! controllable clock. The fixture prints each sequence with `print`, `\e`
//! standing for ESC and `\e\` for ST. Expected records are written out by
//! hand from the wire reference and the source normalization.
use super::{
    advance, assert_closed, assert_rejected, clocked, clocked_with, command_final,
    command_final_at, exited,
    interactive::{Attached, geometry, ready_attachment},
    message, open, peer, request, status,
    status::{all_quiet, completed, printed, shown},
    status_changed,
};
use crate::Session;
use base64::Engine;
use climon2_core::wire::{
    Available, InitialMetadata, Mode, Notification, NotificationSource, Outcome, Percent, Progress,
    ProgressChanged, ProgressState, Response, RevocationReason, ServerMessage,
    StatusState::{NeedsAttention, Running},
    Success,
};
use std::os::unix::net::UnixStream;

use NotificationSource::{Osc9, Osc99, Osc777};

/// Status, notifications and progress.
const FAMILIES: [&str; 3] = ["status-v1", "notification-v1", "progress-v1"];

/// A passive monitor connection opened with request 1 selecting
/// `optional`: its stream and Initial metadata.
fn watcher(session: &Session, optional: &[&str]) -> (UnixStream, InitialMetadata) {
    let mut stream = peer(session);
    request(&mut stream, &open(1, &[], optional, Mode::Monitor));
    let response = message(&mut stream);
    assert!(
        matches!(
            &response,
            ServerMessage::Response(Response {
                outcome: Outcome::Ok(Success::Opened { .. }),
                ..
            })
        ),
        "{response:?}"
    );
    let ServerMessage::InitialMetadata(metadata) = message(&mut stream) else {
        panic!("no Initial metadata after the Open response");
    };
    (stream, metadata)
}

fn notification(
    source: NotificationSource,
    source_id: Option<&str>,
    title: Option<&str>,
    body: Option<&str>,
    episode: Option<u64>,
) -> ServerMessage {
    ServerMessage::Notification(Notification {
        source,
        source_id: source_id.map(str::to_owned),
        title: title.map(str::to_owned),
        body: body.map(str::to_owned),
        episode_id: episode.map(super::counter),
    })
}

fn progress_value(state: ProgressState, percent: Option<u8>) -> Progress {
    Progress::new(state, percent.map(|value| Percent::new(value).unwrap())).unwrap()
}

fn progress(state: ProgressState, percent: Option<u8>) -> ServerMessage {
    ServerMessage::ProgressChanged(ProgressChanged {
        progress: progress_value(state, percent),
    })
}

/// No observed notification or progress sequence reached the controller's
/// host output.
fn nothing_observed_displayed(attached: &Attached) {
    for sequence in [&b"\x1b]9;"[..], b"\x1b]99;", b"\x1b]777;"] {
        assert_eq!(
            crate::count(&attached.display, sequence),
            0,
            "{:?}",
            String::from_utf8_lossy(&attached.display)
        );
    }
}

/// STA-06 (Contract) and CTL-03 (Contract, notifications and progress):
/// OSC 9 text, a completed OSC 99 notification and OSC 777 `notify` are
/// notifications: the first requests attention, reported as
/// needs_attention before its record, and the others join that episode,
/// which each record carries. OSC 9;4 progress never requests attention:
/// it reports normal 50, indeterminate, then paused with the cached 50.
/// Incomplete OSC 99 chunks, an OSC 99 query, other OSC 777 commands and
/// other OSC 9 control operations report nothing. Every record reaches, in
/// the same order, a passive monitor connection, an attachment revoked by
/// takeover and a monitor controlling in the background, before its Ready;
/// a connection that selected only `status-v1` receives the status alone.
/// A monitor opened later reports the retained progress in its Initial
/// metadata, which an earlier one reported as none.
#[test]
fn notifications_and_progress_reach_every_subscribed_role_in_order() {
    let session = clocked("script");
    let (mut passive, initial) = watcher(&session, &FAMILIES);
    let none = progress_value(ProgressState::None, None);
    assert_eq!(initial.progress(), &Available::Available(none));
    let (mut status_only, initial) = watcher(&session, &["status-v1"]);
    assert_eq!(initial.progress(), &Available::Unavailable);
    let mut revoked = ready_attachment(&session, &FAMILIES);
    let mut background = Attached::passive(watcher(&session, &FAMILIES).0);
    background.acquire(geometry(91, 37));
    revoked.revoked(RevocationReason::Takeover);

    let sequences = concat!(
        r"\e]9;4;1;50\e\",
        r"\e]9;Build done\e\",
        r"\e]9;4;3\e\",
        r"\e]99;i=n:d=0;Tit\e\",
        r"\e]99;p=?;\e\",
        r"\e]777;precmd\e\",
        r"\e]9;1;100\e\",
        r"\e]99;i=n:p=body;Body\e\",
        r"\e]777;notify;T;B;c\e\",
        r"\e]9;4;4\e\",
    );
    crate::script(&session, &format!("print {sequences}"));
    let expected = [
        progress(ProgressState::Normal, Some(50)),
        status_changed(status(NeedsAttention, 0, Some(1))),
        notification(Osc9, None, None, Some("Build done"), Some(1)),
        progress(ProgressState::Indeterminate, None),
        notification(Osc99, Some("n"), Some("Tit"), Some("Body"), Some(1)),
        notification(Osc777, None, Some("T"), Some("B;c"), Some(1)),
        progress(ProgressState::Paused, Some(50)),
    ];
    for record in &expected {
        assert_eq!(&message(&mut passive), record);
        assert_eq!(&message(&mut revoked.stream), record);
        assert_eq!(&background.next_after_display(), record);
    }
    assert_eq!(message(&mut status_only), expected[1]);
    let (mut later, initial) = watcher(&session, &["progress-v1"]);
    let paused = progress_value(ProgressState::Paused, Some(50));
    assert_eq!(initial.progress(), &Available::Available(paused));

    background.confirm_ready();
    let mut session = session;
    crate::issue(&session, "exit");
    let ended = command_final_at(&exited(0), 0);
    for stream in [&mut passive, &mut status_only, &mut revoked.stream] {
        assert_eq!(message(stream), ended);
        assert_closed(stream);
    }
    assert_eq!(message(&mut later), command_final(&exited(0)));
    assert_closed(&mut later);
    assert_eq!(background.next_after_display(), ended);
    background.session_ended();
    session.finish();
}

/// LIM-01 (Contract, exec-provided assembly limits): a daemon started with
/// two assemblies of 40 text bytes, each living one second, applies them
/// at the limit and one past. Two incomplete OSC 99 notifications holding
/// 40 bytes together are kept; a third one's chunk is dropped, so its
/// completion is empty and reports nothing, and one more byte discards the
/// notification it would join, whose completion reports nothing. Completed
/// 999 ms after its first chunk, a notification requests attention and
/// carries its 30 assembled bytes; one completed 1,000 ms after its first
/// chunk has expired and reports nothing.
#[test]
fn exec_provided_assembly_limits_apply_at_the_limit_and_one_past() {
    let limits = [
        "--assembly-records",
        "2",
        "--assembly-bytes",
        "40",
        "--assembly-ms",
        "1000",
    ];
    let session = clocked_with("script", |command| {
        command.args(limits);
    });
    let mut attached = ready_attachment(&session, &FAMILIES);
    shown(&session, &mut attached, "busy");
    advance(&session, 1_500);
    let running = status(Running, 1_500, None);
    assert_eq!(attached.next_after_display(), status_changed(running));

    let a = "a".repeat(30);
    let held = format!(r"\e]99;i=a:d=0;{a}\e\\e]99;i=b:d=0;bbbbbbbbbb\e\\e]99;i=c:d=0;C\e\");
    printed(&session, &mut attached, &held, 1);
    let refused = concat!(r"\e]99;i=c;\e\", r"\e]99;i=b:d=0;b\e\", r"\e]99;i=b;B\e\");
    printed(&session, &mut attached, refused, 2);
    all_quiet(&mut attached, &mut []);

    advance(&session, 2_499);
    let completing = concat!(r"\e]99;i=a;!\e\", r"\e]99;i=d:d=0;D\e\");
    printed(&session, &mut attached, completing, 3);
    let pending = status(NeedsAttention, 2_499, Some(1));
    assert_eq!(attached.next_after_display(), status_changed(pending));
    let title = format!("{a}!");
    let completed_a = notification(Osc99, Some("a"), Some(&title), None, Some(1));
    assert_eq!(attached.next_after_display(), completed_a);
    all_quiet(&mut attached, &mut []);

    advance(&session, 3_499);
    printed(&session, &mut attached, r"\e]99;i=d;\e\", 4);
    all_quiet(&mut attached, &mut []);
    nothing_observed_displayed(&attached);
    completed(session, &mut attached, &mut [], 3_499);
}

/// An OSC 99 notification `id` whose title and body are `title` and
/// `body`, sent as base64 chunks of at most 2,048 bytes each, so chunk
/// boundaries split multi-byte characters.
fn chunked(id: &str, title: &[u8], body: &[u8]) -> String {
    let mut sequences = String::new();
    for (part, text) in [("title", title), ("body", body)] {
        for piece in text.chunks(2_048) {
            let encoded = base64::engine::general_purpose::STANDARD.encode(piece);
            sequences += &format!(r"\e]99;i={id}:d=0:e=1:p={part};{encoded}\e\");
        }
    }
    sequences + &format!(r"\e]99;i={id};\e\")
}

/// `bytes` UTF-8 bytes of three-byte euro signs, padded with `e`.
fn euros(bytes: usize) -> String {
    "\u{20ac}".repeat(bytes / 3) + &"e".repeat(bytes % 3)
}

/// LIM-01 (Contract, release-constant notification bounds) and STA-06
/// (Contract, inert text): a notification title of 4,096 UTF-8 bytes and a
/// body of 8,192, multi-byte characters filling both and split across
/// OSC 99 chunks, arrive whole; a title or a body one byte longer is
/// ignored, not truncated. Invalid UTF-8 is replaced, and OSC 9 and OSC 777
/// text join the pending episode. No observed sequence reaches host output.
#[test]
fn notification_titles_and_bodies_are_bounded_and_inert() {
    let session = clocked("script");
    let mut attached = ready_attachment(&session, &FAMILIES);
    let (title, body) = (euros(4_096), euros(8_192));
    let whole = chunked("w", title.as_bytes(), body.as_bytes());
    printed(&session, &mut attached, &whole, 1);
    let pending = status(NeedsAttention, 0, Some(1));
    assert_eq!(attached.next_after_display(), status_changed(pending));
    let record = notification(Osc99, Some("w"), Some(&title), Some(&body), Some(1));
    assert_eq!(attached.next_after_display(), record);

    let long_title = chunked("t", euros(4_097).as_bytes(), b"b");
    let long_body = chunked("b", b"t", euros(8_193).as_bytes());
    printed(&session, &mut attached, &(long_title + &long_body), 2);
    all_quiet(&mut attached, &mut []);

    let others = concat!(
        r"\e]99;i=x:e=1;b2v/\e\",
        r"\e]9;nine\e\",
        r"\e]777;notify;seven\e\"
    );
    printed(&session, &mut attached, others, 3);
    let replaced = notification(Osc99, Some("x"), Some("ok\u{fffd}"), None, Some(1));
    assert_eq!(attached.next_after_display(), replaced);
    let nine = notification(Osc9, None, None, Some("nine"), Some(1));
    assert_eq!(attached.next_after_display(), nine);
    let seven = notification(Osc777, None, Some("seven"), None, Some(1));
    assert_eq!(attached.next_after_display(), seven);
    all_quiet(&mut attached, &mut []);
    nothing_observed_displayed(&attached);
    completed(session, &mut attached, &mut [], 0);
}

/// STA-06 (Contract, capability dependency): `notification-v1` needs
/// `status-v1`. Required without it, the Open is rejected with
/// `unsupported_capability` and its stream ends; optional without it, it is
/// not selected; with `status-v1` it is.
#[test]
fn notification_v1_is_selected_only_with_status_v1() {
    let mut session = Session::start("script");
    let mut rejected = peer(&session);
    let required = ["notification-v1"];
    request(&mut rejected, &open(1, &required, &[], Mode::Monitor));
    assert_rejected(&mut rejected, 1, "unsupported_capability");
    assert_closed(&mut rejected);
    for (required, optional, expected) in [
        (&[][..], &["notification-v1"][..], &[][..]),
        (
            &["notification-v1"],
            &["status-v1"],
            &["notification-v1", "status-v1"],
        ),
    ] {
        let mut stream = peer(&session);
        request(&mut stream, &open(1, required, optional, Mode::Monitor));
        let ServerMessage::Response(Response {
            outcome:
                Outcome::Ok(Success::Opened {
                    selected_capabilities,
                    ..
                }),
            ..
        }) = message(&mut stream)
        else {
            panic!("not opened");
        };
        let mut selected = selected_capabilities.tokens().to_vec();
        selected.sort();
        assert_eq!(selected, expected);
    }
    crate::issue(&session, "exit");
    session.finish();
}
