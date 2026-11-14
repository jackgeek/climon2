//! Daemon wire revision 1 over the session socket: raw-socket peers against
//! the real daemon binary, the Contract layer of spec #73's Testing Decisions.
//! Peers encode and decode with `climon2-core`'s codec, which has its own
//! tests; expectations are written out by hand from the wire reference.
mod interactive;
mod passive;
mod status;
mod termination;

use super::Session;
use climon2_core::{
    hello::Lifecycle,
    protocol::ClientMessage,
    wire::{
        self, Available, CommandFinal, CommandResult, ControlRecord, Counter, Families, FatalError,
        Frame, FrameLimit, Kill, Leave, Mode, Open, Outcome, Requested, Response, ServerMessage,
        Status, StatusChanged, StatusState, Success, Utc,
    },
};
use std::{
    fs,
    io::Write,
    num::NonZeroU32,
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};

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

/// A standalone Kill request.
fn kill(request_id: u64, revision: u32, required: &[&str]) -> Kill {
    Kill {
        request_id: counter(request_id),
        revision,
        required_capabilities: families(required),
    }
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
    assert_rejected_knowing(stream, request_id, code, None);
}

/// The next record is the rejection of `request_id` with `code`, carrying
/// `known_final`.
fn assert_rejected_knowing(
    stream: &mut UnixStream,
    request_id: u64,
    code: &str,
    known_final: Option<&CommandResult>,
) {
    let (answered, rejection) = match message(stream) {
        ServerMessage::Response(Response {
            request_id: answered,
            outcome: Outcome::Rejected(rejection),
        }) => (answered, rejection),
        other => panic!("expected a rejection of request {request_id}, got {other:?}"),
    };
    assert_eq!(answered, counter(request_id));
    assert_eq!(rejection.error.code.as_str(), code);
    assert_eq!(rejection.known_final.as_ref(), known_final);
}

fn exited(code: u32) -> CommandResult {
    CommandResult::Exit { code }
}

fn signalled(number: u32) -> CommandResult {
    let number = NonZeroU32::new(number).unwrap();
    CommandResult::Signal { number, name: None }
}

/// The Command final of `result` for a connection that did not select
/// `status-v1`: its status is null.
fn command_final(result: &CommandResult) -> ServerMessage {
    ServerMessage::CommandFinal(CommandFinal::new(result.clone(), None).unwrap())
}

/// The Command final of `result` for a connection that selected
/// `status-v1`: it carries the final status, changed at `ms` on the
/// controllable clock.
fn command_final_at(result: &CommandResult, ms: u64) -> ServerMessage {
    let final_status = status(result.final_state(), ms, None);
    ServerMessage::CommandFinal(CommandFinal::new(result.clone(), Some(final_status)).unwrap())
}

/// The controllable clock's starting reading, in milliseconds since the
/// Unix epoch: 2027-01-15T08:00:00.000Z, and monotonic time 0 for tests.
const T0: u64 = 1_800_000_000_000;

/// The controllable clock file of a session that [`clocked`] started.
fn clock_file(session: &Session) -> std::path::PathBuf {
    fs::canonicalize(session.directory.path())
        .unwrap()
        .join("clock")
}

/// Starts a session whose status detector reads the `test-hooks`
/// controllable clock, set to [`T0`]. Time stands still until the test
/// [`advance`]s it, so no sample is taken and no deadline passes unless a
/// test says so; every other daemon deadline stays real.
fn clocked(mode: &str) -> Session {
    clocked_with(mode, |_| {})
}

fn clocked_with(mode: &str, configure: impl FnOnce(&mut std::process::Command)) -> Session {
    Session::configured(mode, |command, home| {
        fs::write(home.join("clock"), T0.to_string()).unwrap();
        command.env("CLIMON2_DAEMON_CLOCK", home.join("clock"));
        configure(command);
    })
}

/// Sets the controllable clock `ms` milliseconds after [`T0`] and waits,
/// bounded, until a daemon turn that read it has ended: the daemon then
/// acknowledges the value, and that turn's records are queued before the
/// answer to any later request.
fn advance(session: &Session, ms: u64) {
    let value = (T0 + ms).to_string();
    let path = clock_file(session);
    let staged = path.with_extension("tmp");
    fs::write(&staged, &value).unwrap();
    fs::rename(&staged, &path).unwrap();
    let acknowledged = path.with_extension("ack");
    let deadline = Instant::now() + Duration::from_secs(20);
    while fs::read_to_string(&acknowledged).unwrap_or_default() != value {
        assert!(Instant::now() < deadline, "the clock never reached {ms}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The status `state`, changed `ms` after [`T0`], with `episode`.
fn status(state: StatusState, ms: u64, episode: Option<u64>) -> Status {
    let changed_at = Utc::from_unix_millis(T0 + ms).unwrap();
    Status::new(state, changed_at, episode.map(counter)).unwrap()
}

/// The Status changed record of `status`.
fn status_changed(status: Status) -> ServerMessage {
    ServerMessage::StatusChanged(StatusChanged::new(status).unwrap())
}

/// The next record is the Command final of `result`, and the stream then
/// ends with nothing more: a passive monitor connection's, or a revoked
/// attachment's.
fn assert_final_then_closed(stream: &mut UnixStream, result: &CommandResult) {
    assert_eq!(message(stream), command_final(result));
    assert_closed(stream);
}

/// The next record is a Fatal error with `code`, and the stream then ends.
fn assert_fatal(stream: &mut UnixStream, code: &str) {
    match message(stream) {
        ServerMessage::FatalError(FatalError(error)) => assert_eq!(error.code.as_str(), code),
        other => panic!("expected Fatal error {code}, got {other:?}"),
    }
    assert_closed(stream);
}

/// The bytes of a frame of `code` with `body` exactly as given, valid or not.
fn raw_frame(code: u8, body: &[u8]) -> Vec<u8> {
    let length = u32::try_from(body.len() + 1).unwrap();
    [&length.to_be_bytes()[..], &[code], body].concat()
}

/// The bytes of one encoded revision-1 frame.
fn encoded(frame: &Frame) -> Vec<u8> {
    raw_frame(frame.code(), frame.body())
}

/// Writes `bytes` in three-byte pieces. A hello answered on a newer
/// connection proves the daemon serviced this one, and read the piece, in
/// that or an earlier pass, before the next piece is written.
fn write_in_pieces(session: &Session, stream: &mut UnixStream, bytes: &[u8]) {
    for piece in bytes.chunks(3) {
        stream.write_all(piece).unwrap();
        drop(session.connect());
    }
}

/// A private kill requester whose Kill the session has committed to: the
/// hello `state` reports it terminating. Its Terminated arrives later.
fn committed_kill(session: &Session) -> UnixStream {
    let mut killer = session.connect();
    killer
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    crate::send(&mut killer, ClientMessage::Kill);
    terminating(session);
    killer
}

/// Waits until the hello `state` reports the session terminating.
fn terminating(session: &Session) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while crate::advertised_state(session).0 != Lifecycle::Terminating {
        assert!(Instant::now() < deadline, "termination never committed");
        std::thread::sleep(Duration::from_millis(5));
    }
}
