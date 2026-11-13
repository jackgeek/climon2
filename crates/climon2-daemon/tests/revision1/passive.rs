//! Passive monitor connections (#86): monitor Open, core metadata, Leave and
//! validation, beside the private protocol that still serves attach and kill.
use super::{
    assert_closed, assert_fatal, assert_rejected, counter, encoded, families, frame, leave,
    message, monitor, open, peer, raw_frame, request, send, title_changed,
};
use crate::{ID, Session};
use climon2_core::{
    hello::Lifecycle,
    protocol::{self, ClientMessage, ServerMessage as PrivateMessage, TerminalSize},
    wire::{
        Acknowledge, Acquire, AuthorityIds, Available, ControlRecord, FatalError, Focus, Frame,
        Geometry, Input, Kill, Leave, Mode, Outcome, Ready, Resize, Response, ServerMessage,
        SetViewing, Success,
    },
};
use std::{
    fs, io::Write, net::Shutdown, num::NonZeroU32, os::unix::net::UnixStream, thread,
    time::Duration,
};

/// The exact Initial metadata of a session that has never set a title, for a
/// monitor that selected `title-v1` and for one that did not.
const UNSET_TITLE: &str = concat!(
    r#"{"title":{"available":true,"value":null},"#,
    r#""status":{"available":false,"value":null},"#,
    r#""progress":{"available":false,"value":null},"command_result":null}"#
);
const NO_TITLE: &str = concat!(
    r#"{"title":{"available":false,"value":null},"#,
    r#""status":{"available":false,"value":null},"#,
    r#""progress":{"available":false,"value":null},"command_result":null}"#
);

/// The selected tokens and connection ID of a successful Open's Response.
fn opened(frame: &Frame, request_id: &str) -> (String, Vec<String>) {
    let ServerMessage::Response(Response {
        request_id: answered,
        outcome:
            Outcome::Ok(Success::Opened {
                connection_id,
                selected_capabilities,
            }),
    }) = ServerMessage::from_frame(frame).unwrap()
    else {
        panic!("not an opened Response: {frame:?}");
    };
    assert_eq!(answered.to_string(), request_id);
    (
        connection_id.to_string(),
        selected_capabilities.tokens().to_vec(),
    )
}

/// The exact body of a server frame of type `code`.
fn body(frame: &Frame, code: u8) -> &str {
    assert_eq!(frame.code(), code, "{frame:?}");
    std::str::from_utf8(frame.body()).unwrap()
}

/// Starts an `exit` fixture session with a ready private controller, so the
/// session outlives its initial-attachment deadline however slow the test.
fn controlled() -> (Session, UnixStream) {
    let session = Session::start("exit");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    (session, controlling)
}

/// Ends an `exit` fixture session through its controller.
fn end_controlled(mut session: Session, mut controlling: UnixStream) {
    crate::send(&mut controlling, ClientMessage::Input(b"go\n".to_vec()));
    assert_eq!(
        crate::until_terminal(&mut controlling).1,
        PrivateMessage::Exit { code: 42 }
    );
    session.finish();
}

/// Ends an `exit` fixture session through an ordinary private attachment.
fn end_through_attachment(session: Session) {
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    end_controlled(session, controlling);
}

/// COM-01 (Contract case): hello advertises revision 1 the way the published
/// complete reply does. The frozen tokens and `state` keep their places and
/// meanings, the implemented families follow the tokens, and
/// `daemon_wire_v1` carries the 1,048,576-byte release-constant limits.
#[test]
fn hello_advertises_revision1_beside_the_frozen_tokens_and_state() {
    let session = Session::start("exit");
    let mut probe = UnixStream::connect(&session.socket).unwrap();
    probe
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    crate::send(&mut probe, ClientMessage::Hello { protocol: 1 });
    let reply = protocol::read_frame(&mut probe).unwrap();
    assert_eq!(reply.type_code(), 0x01);
    let expected = format!(
        concat!(
            r#"{{"session_id":"{}","climon2_version":"0.1.0","protocol":1,"#,
            r#""capabilities":["attach","kill","state-v1","client-startup-v1","#,
            r#""daemon-wire-v1","monitor-v1","title-v1","focus-v1"],"#,
            r#""state":{{"lifecycle":"starting","attachments":0,"controlled":false}},"#,
            r#""daemon_wire_v1":{{"max_request_frame_bytes":1048576,"max_server_frame_bytes":1048576}}}}"#
        ),
        ID
    );
    assert_eq!(std::str::from_utf8(reply.body()).unwrap(), expected);
    end_through_attachment(session);
}

/// COM-01 and COM-03 (Contract case, passive Open): a monitor Open selects
/// the requested families this daemon supports and drops unknown and
/// unsupported optional ones, under a fresh connection ID. Its success
/// Response comes first, then one Initial metadata record: the title by
/// value when `title-v1` is selected and unavailable otherwise, status and
/// progress explicitly unavailable, and no command result.
#[test]
fn monitor_open_selects_supported_families_then_reports_initial_metadata() {
    let (session, controlling) = controlled();
    let mut first = peer(&session);
    let optional = ["monitor-v1", "status-v1", "focus-v1", "future-v9"];
    request(
        &mut first,
        &open(1, &["title-v1"], &optional, Mode::Monitor),
    );
    let (first_id, mut selected) = opened(&frame(&mut first), "1");
    selected.sort();
    assert_eq!(selected, ["focus-v1", "monitor-v1", "title-v1"]);
    assert_eq!(body(&frame(&mut first), 0x41), UNSET_TITLE);

    let mut second = peer(&session);
    request(&mut second, &open(7, &[], &["progress-v1"], Mode::Monitor));
    let (second_id, selected) = opened(&frame(&mut second), "7");
    assert!(selected.is_empty(), "{selected:?}");
    assert_eq!(body(&frame(&mut second), 0x41), NO_TITLE);
    assert_ne!(first_id, second_id);
    end_controlled(session, controlling);
}

/// Attaches and detaches through the private protocol, so the session has
/// no attachment and outlives its initial-attachment deadline.
fn detached(session: &Session) {
    let mut client = session.attach();
    session.wait_marker(&mut client);
    crate::send(&mut client, ClientMessage::Detach);
    assert_eq!(
        crate::until_terminal(&mut client).1,
        PrivateMessage::Detached
    );
}

/// Ends a `script` fixture session through its command's own exit.
fn end_script(mut session: Session) {
    crate::issue(&session, "exit");
    session.finish();
}

/// CTL-03 (Contract, title case) and COM-05 (Contract, Initial metadata
/// case): Initial metadata captures the current title by value, and Title
/// changed records follow gap-free and in order, whatever the control: with a
/// controller elsewhere and with none. Unset, empty and a value stay distinct,
/// an unchanged title is not reported again, and a title is inert text: never
/// percent-encoded or interpreted as terminal presentation.
#[test]
fn title_changes_follow_initial_metadata_by_value_whatever_the_control() {
    let session = Session::start("script");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    let (mut first, title) = monitor(&session, &["title-v1"]);
    assert_eq!(title, Available::Available(None));
    crate::script(&session, "title one");
    assert_eq!(title_changed(&mut first).as_deref(), Some("one"));
    crate::script(&session, "title one");
    crate::script(&session, "title two");
    assert_eq!(title_changed(&mut first).as_deref(), Some("two"));

    crate::send(&mut controlling, ClientMessage::Detach);
    assert_eq!(
        crate::until_terminal(&mut controlling).1,
        PrivateMessage::Detached
    );
    crate::script(&session, "title-empty");
    assert_eq!(title_changed(&mut first).as_deref(), Some(""));
    let (mut second, title) = monitor(&session, &["title-v1"]);
    assert_eq!(title, Available::Available(Some(String::new())));
    crate::script(&session, "reset");
    assert_eq!(title_changed(&mut first), None);
    assert_eq!(title_changed(&mut second), None);

    let inert = "café — \u{9b}[31m%41";
    crate::script(&session, &format!("title {inert}"));
    assert_eq!(title_changed(&mut first).as_deref(), Some(inert));
    assert_eq!(
        body(&frame(&mut second), 0x42),
        format!(r#"{{"title":"{inert}"}}"#)
    );
    leave(&mut first, 2);
    leave(&mut second, 2);
    end_script(session);
}

/// COM-01 (Contract, optional families) and CTL-03 (Contract, title case): a
/// monitor that did not select `title-v1` receives nothing for it. Its
/// Initial metadata reports the title unavailable, never a fabricated unset or
/// empty title, and no Title changed reaches it before its Leave response.
#[test]
fn a_monitor_without_title_v1_receives_nothing_for_titles() {
    let session = Session::start("script");
    detached(&session);
    crate::script(&session, "title before");
    let (mut subscribed, _) = monitor(&session, &["title-v1"]);
    let (mut unsubscribed, title) = monitor(&session, &[]);
    assert_eq!(title, Available::Unavailable);
    crate::script(&session, "title after");
    assert_eq!(title_changed(&mut subscribed).as_deref(), Some("after"));
    leave(&mut unsubscribed, 2);
    leave(&mut subscribed, 2);
    end_script(session);
}

/// COM-05 (Contract, Initial metadata case) and CTL-03 (Contract, title
/// case): titles set many times in one write reach a monitor as every change,
/// in order. A monitor opening inside a burst, after a first monitor has
/// seen its first write's last title and before its second write, reports
/// that title atomically, then exactly the second write's changes, with no
/// gap or repeat.
#[test]
fn a_title_burst_is_reported_gap_free_around_an_atomic_initial_metadata() {
    let session = Session::start("script");
    detached(&session);
    let (mut early, title) = monitor(&session, &["title-v1"]);
    assert_eq!(title, Available::Available(None));
    crate::script(&session, "titles 1 150");
    for number in 1..=150 {
        assert_eq!(title_changed(&mut early), Some(format!("t-{number}")));
    }
    let (mut late, title) = monitor(&session, &["title-v1"]);
    assert_eq!(title, Available::Available(Some("t-150".into())));
    crate::script(&session, "titles 151 300");
    for number in 151..=300 {
        assert_eq!(title_changed(&mut early), Some(format!("t-{number}")));
        assert_eq!(title_changed(&mut late), Some(format!("t-{number}")));
    }
    leave(&mut early, 2);
    leave(&mut late, 2);
    end_script(session);
}

/// CTL-03 (Contract, title case) and the derived release-constant title
/// bound: the longest title a title sequence can carry arrives whole: 4,094
/// UTF-8 bytes, because the existing 4,096-byte token body includes the
/// sequence's `2;`. A longer one, at or one past the documented 4,096 bytes,
/// is ignored, never truncated.
#[test]
fn an_oversized_title_is_ignored_never_truncated() {
    let session = Session::start("script");
    detached(&session);
    let (mut monitor, _) = monitor(&session, &["title-v1"]);
    crate::script(&session, "title-bytes 4094");
    assert_eq!(title_changed(&mut monitor), Some("é".repeat(2047)));
    for bytes in [4095, 4096, 4097] {
        crate::script(&session, &format!("title-bytes {bytes}"));
    }
    crate::script(&session, "title after");
    assert_eq!(title_changed(&mut monitor).as_deref(), Some("after"));
    leave(&mut monitor, 2);
    end_script(session);
}

/// A standalone Kill request.
fn kill(request_id: u64, revision: u32, required: &[&str]) -> Kill {
    Kill {
        request_id: counter(request_id),
        revision,
        required_capabilities: families(required),
    }
}

/// COM-03 (Contract, passive cases): each passive request gets its
/// documented outcome. Leave before Open is `invalid_state` and the stream
/// goes on. A rejected first role, Open or Kill, has no effects and ends its
/// stream: an unsupported revision is `unsupported_revision`, and an unknown
/// or unsupported required family is `unsupported_capability`. A later Open
/// or Kill never repurposes a registration: `invalid_state`, and the
/// registration stays until Leave, which reports that no attachment was
/// detached.
#[test]
fn passive_requests_get_their_documented_outcomes() {
    let (session, controlling) = controlled();
    let mut stream = peer(&session);
    request(
        &mut stream,
        &Leave {
            request_id: counter(3),
        },
    );
    assert_rejected(&mut stream, 3, "invalid_state");
    request(&mut stream, &open(4, &[], &["title-v1"], Mode::Monitor));
    opened(&frame(&mut stream), "4");
    assert_eq!(body(&frame(&mut stream), 0x41), UNSET_TITLE);
    request(&mut stream, &open(5, &[], &[], Mode::Monitor));
    assert_rejected(&mut stream, 5, "invalid_state");
    request(&mut stream, &kill(6, 1, &[]));
    assert_rejected(&mut stream, 6, "invalid_state");
    leave(&mut stream, 7);

    let mut later_revision = open(1, &[], &[], Mode::Monitor);
    later_revision.revision = 2;
    let roles = [
        (later_revision.to_frame(), "unsupported_revision"),
        (
            open(1, &["status-v1"], &[], Mode::Monitor).to_frame(),
            "unsupported_capability",
        ),
        (
            open(1, &["future-v9"], &["title-v1"], Mode::Monitor).to_frame(),
            "unsupported_capability",
        ),
        (kill(1, 2, &[]).to_frame(), "unsupported_revision"),
        (
            kill(1, 1, &["future-v9"]).to_frame(),
            "unsupported_capability",
        ),
    ];
    for (role, code) in roles {
        let mut stream = peer(&session);
        send(&mut stream, &role);
        assert_rejected(&mut stream, 1, code);
        assert_closed(&mut stream);
    }
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );
    end_controlled(session, controlling);
}

/// CTL-01 (Contract): a passive registration is not an attachment and never
/// acquires control, not even briefly: not of a detached session, and not of
/// one controlled elsewhere. Hello `state`, which `climon2 list` shows as
/// STATE and CLIENTS, stays as it was, and core metadata flows before any
/// control.
#[test]
fn a_passive_registration_never_acquires_control_and_is_not_an_attachment() {
    let mut session = Session::start("exit");
    let mut first = session.attach();
    session.wait_marker(&mut first);
    crate::send(&mut first, ClientMessage::Detach);
    assert_eq!(
        crate::until_terminal(&mut first).1,
        PrivateMessage::Detached
    );
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );
    let (mut watching_detached, title) = monitor(&session, &["title-v1"]);
    assert_eq!(title, Available::Available(None));
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );

    let mut second = crate::attach_at(&session, TerminalSize { cols: 91, rows: 37 });
    let (restoration, _) = crate::restoration(&mut second);
    assert_eq!(restoration, 2, "only the two attachments were ever granted");
    crate::send(
        &mut second,
        ClientMessage::Ready {
            period: restoration,
        },
    );
    let (mut watching_controlled, _) = monitor(&session, &[]);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );
    leave(&mut watching_detached, 2);
    leave(&mut watching_controlled, 2);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );
    crate::send(&mut second, ClientMessage::Input(b"go\n".to_vec()));
    let (output, end) = crate::until_terminal(&mut second);
    assert_eq!(end, PrivateMessage::Exit { code: 42 });
    assert_eq!(crate::count(&output, b"final-bytes"), 1);
    session.finish();
}

/// COM-03 (Contract, passive cases) and CTL-01 (Contract): committed
/// termination rejects new interactive work only. A monitor can still
/// register, as passive outcome recovery needs, and Leave; neither changes
/// the terminating `state`.
#[test]
fn a_monitor_registers_and_leaves_during_committed_termination() {
    let mut session = Session::start("ignore-hup");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    let mut killer = super::committed_kill(&session);
    let (mut watching, title) = monitor(&session, &["title-v1"]);
    assert_eq!(title, Available::Available(None));
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Terminating, 1, true)
    );
    leave(&mut watching, 2);
    assert!(matches!(
        crate::receive(&mut killer),
        PrivateMessage::Terminated { .. }
    ));
    assert!(matches!(
        crate::until_terminal(&mut controlling).1,
        PrivateMessage::Exit { .. }
    ));
    session.finish();
}

/// A monitor Open whose body is padded with JSON whitespace to `bytes`.
fn padded_open(bytes: usize) -> Vec<u8> {
    let open = open(1, &[], &[], Mode::Monitor).to_frame();
    let mut body = open.body().to_vec();
    let end = body.pop().unwrap();
    body.resize(bytes - 1, b' ');
    body.push(end);
    raw_frame(0x30, &body)
}

/// COM-02 (Contract): revision-1 traffic that breaks the framing or strict
/// JSON rules, sends a server, private or unknown type, or does not increase
/// its request ID fails with exactly the documented connection-fatal code,
/// then the connection closes. The first frame after hello is judged as
/// revision 1 when its type is. A stream ending inside a frame is
/// `truncated_frame`; one ending between frames just closes. Other
/// connections and the session go on.
#[test]
fn revision1_traffic_that_breaks_the_rules_is_connection_fatal() {
    let (session, controlling) = controlled();
    let valid =
        r#""revision":1,"required_capabilities":[],"optional_capabilities":[],"mode":"monitor""#;
    let first_frames: [(Vec<u8>, &str); 8] = [
        (raw_frame(0x30, b"{"), "invalid_json"),
        (
            raw_frame(0x30, b"{\"request_id\":\"1\xff\"}"),
            "invalid_json",
        ),
        (
            raw_frame(
                0x30,
                format!(r#"{{"request_id":"1",{valid},"geometry":null,"extra":1}}"#).as_bytes(),
            ),
            "invalid_record",
        ),
        (
            raw_frame(
                0x30,
                format!(r#"{{"request_id":"1",{valid},"geometry":null,"geometry":null}}"#)
                    .as_bytes(),
            ),
            "invalid_record",
        ),
        (
            raw_frame(0x30, format!(r#"{{"request_id":"1",{valid}}}"#).as_bytes()),
            "invalid_record",
        ),
        (
            raw_frame(
                0x30,
                format!(r#"{{"request_id":"01",{valid},"geometry":null}}"#).as_bytes(),
            ),
            "invalid_request_id",
        ),
        (raw_frame(0x40, b"{}"), "wrong_direction"),
        (raw_frame(0x4d, b"{}"), "wrong_direction"),
    ];
    for (bytes, code) in first_frames {
        let mut stream = peer(&session);
        stream.write_all(&bytes).unwrap();
        assert_fatal(&mut stream, code);
    }
    let hello = ClientMessage::Hello { protocol: 1 }.to_frame().unwrap();
    let mut private_hello = Vec::new();
    protocol::write_frame(&mut private_hello, &hello).unwrap();
    let after_open: [(Vec<u8>, &str); 8] = [
        (vec![0, 0, 0, 0], "invalid_frame_length"),
        (vec![0, 0x10, 0, 1], "frame_too_large"),
        (raw_frame(0x41, b"{}"), "wrong_direction"),
        (raw_frame(0x3a, b"{}"), "unknown_message_type"),
        (
            raw_frame(0x10, br#"{"cols":80,"rows":24}"#),
            "unknown_message_type",
        ),
        (private_hello, "unknown_message_type"),
        (
            raw_frame(0x37, br#"{"request_id":"1"}"#),
            "invalid_request_id",
        ),
        (vec![0, 0, 0, 19, 0x37, b'{'], "truncated_frame"),
    ];
    for (bytes, code) in after_open {
        let (mut stream, _) = monitor(&session, &[]);
        stream.write_all(&bytes).unwrap();
        if code == "truncated_frame" {
            stream.shutdown(Shutdown::Write).unwrap();
        }
        assert_fatal(&mut stream, code);
    }
    let mut stream = peer(&session);
    request(&mut stream, &open(5, &[], &[], Mode::Monitor));
    opened(&frame(&mut stream), "5");
    body(&frame(&mut stream), 0x41);
    request(
        &mut stream,
        &Leave {
            request_id: counter(4),
        },
    );
    assert_fatal(&mut stream, "invalid_request_id");
    let (mut stream, _) = monitor(&session, &[]);
    stream.shutdown(Shutdown::Write).unwrap();
    assert_closed(&mut stream);
    end_controlled(session, controlling);
}

/// COM-02 (Contract, oversized frames): a request frame exactly at the
/// advertised 1,048,576-byte limit is served; on a revision-1 stream one byte
/// more is `frame_too_large`, decided from its prefix before any body
/// arrives.
#[test]
fn a_request_at_the_frame_limit_is_served_and_one_byte_more_is_fatal() {
    let (session, controlling) = controlled();
    let mut at_limit = peer(&session);
    at_limit.write_all(&padded_open(1_048_575)).unwrap();
    opened(&frame(&mut at_limit), "1");
    assert_eq!(body(&frame(&mut at_limit), 0x41), NO_TITLE);
    leave(&mut at_limit, 2);
    let (mut over, _) = monitor(&session, &[]);
    over.write_all(&1_048_577_u32.to_be_bytes()).unwrap();
    assert_fatal(&mut over, "frame_too_large");
    end_controlled(session, controlling);
}

/// COM-02 (Contract, fragmented and coalesced I/O): partial reads and writes
/// never define messages. Hello, Open and Leave written at once, then the
/// peer's half-close, are each answered in order, the first frame after hello
/// selecting revision 1; the stream ends after the Leave response, so a frame
/// sent after Leave is not served. An Open written in pieces, each consumed
/// before the next is sent, is answered once whole.
#[test]
fn coalesced_and_fragmented_revision1_frames_are_each_answered_in_order() {
    let (session, controlling) = controlled();
    let mut coalesced = UnixStream::connect(&session.socket).unwrap();
    coalesced
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let mut bytes = Vec::new();
    let hello = ClientMessage::Hello { protocol: 1 }.to_frame().unwrap();
    protocol::write_frame(&mut bytes, &hello).unwrap();
    bytes.extend(encoded(
        &open(1, &[], &["title-v1"], Mode::Monitor).to_frame(),
    ));
    bytes.extend(raw_frame(0x37, br#"{"request_id":"2"}"#));
    bytes.extend(encoded(&open(3, &[], &[], Mode::Monitor).to_frame()));
    coalesced.write_all(&bytes).unwrap();
    coalesced.shutdown(Shutdown::Write).unwrap();
    assert_eq!(
        protocol::read_frame(&mut coalesced).unwrap().type_code(),
        0x01
    );
    opened(&frame(&mut coalesced), "1");
    assert_eq!(body(&frame(&mut coalesced), 0x41), UNSET_TITLE);
    assert_eq!(
        body(&frame(&mut coalesced), 0x40),
        r#"{"request_id":"2","outcome":"ok","result":{"kind":"left","attachment_detached":false},"error":null}"#
    );
    assert_closed(&mut coalesced);

    let mut fragmented = peer(&session);
    let opening = encoded(&open(1, &[], &[], Mode::Monitor).to_frame());
    super::write_in_pieces(&session, &mut fragmented, &opening);
    opened(&frame(&mut fragmented), "1");
    assert_eq!(body(&frame(&mut fragmented), 0x41), NO_TITLE);
    leave(&mut fragmented, 2);
    end_controlled(session, controlling);
}

/// Interim until #88: this daemon advertises `daemon-wire-v1` but does not
/// serve standalone Kill yet. It is rejected without effects
/// (`internal_error`) and ends its stream; the session goes on with its one
/// controller, not terminating.
#[test]
fn standalone_kill_is_refused_until_it_is_served() {
    let (session, controlling) = controlled();
    let mut killer = peer(&session);
    request(&mut killer, &kill(1, 1, &["daemon-wire-v1"]));
    assert_rejected(&mut killer, 1, "internal_error");
    assert_closed(&mut killer);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );
    end_controlled(session, controlling);
}

/// Each request that needs an attachment, Input `0x31`, Resize `0x32`, Ready
/// `0x34`, Set viewing `0x35`, Acknowledge `0x36` and Focus `0x39`, as
/// request `id`. Its period and restoration ID exceed every request ID these
/// tests send, and its input, size and focus would each show in the `lines`
/// fixture's record.
fn needing_an_attachment(id: u64) -> [Frame; 6] {
    let request_id = counter(id);
    let authority = AuthorityIds {
        period: counter(70),
        restoration_id: counter(90),
    };
    let geometry = Geometry {
        cols: NonZeroU32::new(100).unwrap(),
        rows: NonZeroU32::new(30).unwrap(),
    };
    [
        Input {
            request_id,
            authority,
            payload: b"leaked\n".to_vec(),
        }
        .to_frame(),
        Resize {
            request_id,
            authority,
            geometry,
        }
        .to_frame(),
        Ready {
            request_id,
            authority,
        }
        .to_frame(),
        SetViewing {
            request_id,
            authority,
            viewing: true,
        }
        .to_frame(),
        Acknowledge {
            request_id,
            authority,
        }
        .to_frame(),
        Focus {
            request_id,
            authority,
            focused: true,
        }
        .to_frame(),
    ]
}

/// COM-02 (Contract, out-of-state requests) and COM-03 (Contract, passive
/// cases): Input, Resize, Ready, Set viewing, Acknowledge and Focus need an
/// attachment, so on a connection without one each is identifiable but
/// impermissible: rejected without effects, never connection-fatal, and
/// consuming its request ID. Focus needs `focus-v1`, which is checked first:
/// a connection that did not select it gets `unsupported_capability`.
/// Otherwise, as the first frame after hello a request finds no role
/// (`invalid_state`), so a monitor Open still registers; on a passive
/// monitor connection, which is not an attachment, Input, Resize, Ready and
/// Focus are `not_attached` and the registration stays until Leave reports
/// it. Set viewing and Acknowledge stay `invalid_state` until #91 serves
/// them. No byte reaches the command, its terminal keeps the controller's
/// size, and the hello `state` is unchanged.
#[test]
fn requests_that_need_an_attachment_are_refused_and_the_stream_goes_on() {
    let mut session = Session::start("lines");
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    let without_role = [
        "invalid_state",
        "invalid_state",
        "invalid_state",
        "invalid_state",
        "invalid_state",
        "unsupported_capability",
    ];
    let passive = [
        "not_attached",
        "not_attached",
        "not_attached",
        "invalid_state",
        "invalid_state",
        "unsupported_capability",
    ];
    let cases = needing_an_attachment(1)
        .into_iter()
        .zip(needing_an_attachment(2))
        .zip(without_role.into_iter().zip(passive));
    for ((first, registered), (first_code, registered_code)) in cases {
        let mut stream = peer(&session);
        send(&mut stream, &first);
        assert_rejected(&mut stream, 1, first_code);
        request(&mut stream, &open(2, &[], &[], Mode::Monitor));
        opened(&frame(&mut stream), "2");
        assert_eq!(body(&frame(&mut stream), 0x41), NO_TITLE);
        leave(&mut stream, 3);

        let (mut stream, _) = monitor(&session, &[]);
        send(&mut stream, &registered);
        assert_rejected(&mut stream, 2, registered_code);
        leave(&mut stream, 3);
    }
    let (mut focusing, _) = monitor(&session, &["focus-v1"]);
    let [.., focus] = needing_an_attachment(2);
    send(&mut focusing, &focus);
    assert_rejected(&mut focusing, 2, "not_attached");
    leave(&mut focusing, 3);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );
    crate::send(
        &mut controlling,
        ClientMessage::Input(b"after\nend\n".to_vec()),
    );
    assert_eq!(
        crate::until_terminal(&mut controlling).1,
        PrivateMessage::Exit { code: 0 }
    );
    session.finish();
    assert_eq!(
        fs::read_to_string(session.directory.path().join("lines")).unwrap(),
        "after 91x37\n"
    );
}

/// COM-03 (Contract, passive cases), with the interim refusal until #89:
/// Acquire would convert a passive monitor connection into an attachment, so
/// on a registration it is rejected without effects (`internal_error`), the
/// detached session stays detached, and the registration stays until Leave.
/// Before any Open there is nothing to convert: like an early Leave, Acquire
/// is `invalid_state` and the stream goes on.
#[test]
fn acquire_is_refused_without_effects_until_it_is_served() {
    let session = Session::start("exit");
    detached(&session);
    let mut stream = peer(&session);
    request(
        &mut stream,
        &Acquire {
            request_id: counter(1),
        },
    );
    assert_rejected(&mut stream, 1, "invalid_state");
    request(&mut stream, &open(2, &[], &["title-v1"], Mode::Monitor));
    opened(&frame(&mut stream), "2");
    assert_eq!(body(&frame(&mut stream), 0x41), UNSET_TITLE);
    request(
        &mut stream,
        &Acquire {
            request_id: counter(3),
        },
    );
    assert_rejected(&mut stream, 3, "internal_error");
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );
    leave(&mut stream, 4);
    end_through_attachment(session);
}

/// Opens a monitor once the daemon has read `title`, a title that ends a
/// burst already written, and so has queued every Title changed before it:
/// the monitor reports that title at once, or an earlier one and then the
/// rest of the burst, gap-free, up to it.
fn monitor_after_title(session: &Session, title: &str) -> UnixStream {
    let (mut stream, reported) = monitor(session, &["title-v1"]);
    let Available::Available(mut seen) = reported else {
        panic!("title-v1 was selected");
    };
    while seen.as_deref() != Some(title) {
        seen = title_changed(&mut stream);
    }
    stream
}

/// COM-02 (Contract, connection effects): a connection-fatal failure on a
/// stream whose peer has stopped reading is reported without waiting for
/// its backlog. Frames already started still arrive whole, so every record
/// decodes, then the Fatal error replaces the output not yet started, and
/// the stream ends. Other connections receive everything. The failure is
/// sent as soon as the daemon has queued the whole backlog, without reading
/// it anywhere first, so the five seconds a stalled reader is allowed only
/// have to cover the daemon's own work; another monitor reads its copy
/// concurrently, so it never stalls.
#[test]
fn a_fatal_error_replaces_a_backlog_without_splitting_a_frame() {
    // Far more than a socket buffers as separate tiny frames.
    const TITLES: u32 = 2000;
    let session = Session::start("script");
    detached(&session);
    let (mut stalled, _) = monitor(&session, &["title-v1"]);
    let (mut reading, _) = monitor(&session, &["title-v1"]);
    let reader = thread::spawn(move || {
        let titles: Vec<_> = (0..TITLES).map(|_| title_changed(&mut reading)).collect();
        (reading, titles)
    });
    crate::script(&session, &format!("titles 1 {TITLES}"));
    let mut queued = monitor_after_title(&session, &format!("t-{TITLES}"));
    stalled.write_all(&[0, 0, 0, 0]).unwrap();
    let mut received = 0;
    loop {
        match message(&mut stalled) {
            ServerMessage::TitleChanged(changed) => {
                received += 1;
                assert_eq!(changed.title, Some(format!("t-{received}")));
            }
            ServerMessage::FatalError(FatalError(error)) => {
                assert_eq!(error.code.as_str(), "invalid_frame_length");
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(received < TITLES, "the backlog was not replaced");
    assert_closed(&mut stalled);
    let (mut reading, titles) = reader.join().unwrap();
    let expected: Vec<_> = (1..=TITLES)
        .map(|number| Some(format!("t-{number}")))
        .collect();
    assert_eq!(titles, expected);
    leave(&mut reading, 2);
    leave(&mut queued, 2);
    end_script(session);
}
