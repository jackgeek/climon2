//! Daemon wire revision 1 over the session socket: raw-socket peers against
//! the real daemon binary, the Contract layer of spec #73's Testing Decisions.
//! Peers encode and decode with `climon2-core`'s codec, which has its own
//! tests; expectations are written out by hand from the wire reference.
mod passive;

use super::Session;
use climon2_core::wire::{
    self, Available, ControlRecord, Counter, Families, FatalError, Frame, FrameLimit, Leave, Mode,
    Open, Outcome, Requested, Response, ServerMessage, Success,
};
use std::{os::unix::net::UnixStream, time::Duration};

/// The daemon's release-constant request and server frame limit.
const FRAME_BYTES: u32 = 1_048_576;

fn limit() -> FrameLimit {
    FrameLimit::new(FRAME_BYTES).unwrap()
}

/// A peer that completed the frozen hello: its next frame selects a protocol.
fn peer(session: &Session) -> UnixStream {
    let stream = session.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    stream
}

fn send(stream: &mut UnixStream, frame: &Frame) {
    wire::write_frame(stream, frame, limit()).unwrap();
}

fn request(stream: &mut UnixStream, record: &impl ControlRecord) {
    send(stream, &record.to_frame());
}

/// The next frame the daemon sent, with its exact type and body.
fn frame(stream: &mut UnixStream) -> Frame {
    wire::read_frame(stream, limit())
        .unwrap()
        .expect("a frame before the daemon closed the connection")
}

fn message(stream: &mut UnixStream) -> ServerMessage {
    ServerMessage::from_frame(&frame(stream)).unwrap()
}

/// The daemon closed the connection exactly between frames.
fn assert_closed(stream: &mut UnixStream) {
    assert!(wire::read_frame(stream, limit()).unwrap().is_none());
}

fn counter(value: u64) -> Counter {
    Counter::new(value).unwrap()
}

fn families(tokens: &[&str]) -> Families {
    Families::new(tokens.iter().map(|token| (*token).to_owned()).collect()).unwrap()
}

fn open(request_id: u64, required: &[&str], optional: &[&str], mode: Mode) -> Open {
    Open {
        request_id: counter(request_id),
        revision: 1,
        capabilities: Requested::new(families(required), families(optional)).unwrap(),
        mode,
    }
}

/// A passive monitor connection opened with request 1: its success
/// Response, then the title its Initial metadata reports.
fn monitor(session: &Session, optional: &[&str]) -> (UnixStream, Available<Option<String>>) {
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
    (stream, metadata.title().clone())
}

/// The title of the next record, which must be a Title changed.
fn title_changed(stream: &mut UnixStream) -> Option<String> {
    match message(stream) {
        ServerMessage::TitleChanged(changed) => changed.title,
        other => panic!("expected Title changed, got {other:?}"),
    }
}

/// Leaves a passive registration. Its Response is the next frame, so no
/// other record was pending, and the stream then ends.
fn leave(stream: &mut UnixStream, request_id: u64) {
    request(
        stream,
        &Leave {
            request_id: counter(request_id),
        },
    );
    let left = Response {
        request_id: counter(request_id),
        outcome: Outcome::Ok(Success::Left {
            attachment_detached: false,
        }),
    };
    assert_eq!(message(stream), ServerMessage::Response(left));
    assert_closed(stream);
}

/// The next record is the rejection of `request_id` with `code`, carrying no
/// result.
fn assert_rejected(stream: &mut UnixStream, request_id: u64, code: &str) {
    let (answered, rejection) = match message(stream) {
        ServerMessage::Response(Response {
            request_id: answered,
            outcome: Outcome::Rejected(rejection),
        }) => (answered, rejection),
        other => panic!("expected a rejection of request {request_id}, got {other:?}"),
    };
    assert_eq!(answered, counter(request_id));
    assert_eq!(rejection.error.code.as_str(), code);
    assert_eq!(rejection.known_final, None);
}

/// The next record is a Fatal error with `code`, and the stream then ends.
fn assert_fatal(stream: &mut UnixStream, code: &str) {
    match message(stream) {
        ServerMessage::FatalError(FatalError(error)) => assert_eq!(error.code.as_str(), code),
        other => panic!("expected Fatal error {code}, got {other:?}"),
    }
    assert_closed(stream);
}
