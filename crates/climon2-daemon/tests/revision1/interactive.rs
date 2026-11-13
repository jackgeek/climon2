//! Interactive attachments (#87): interactive Open, control and authority,
//! binary Input and Display, Acquire, Focus and Leave over revision 1,
//! beside the private protocol and its attachments.
use super::{assert_closed, assert_rejected, counter, limit, message, open, peer, request, send};
use crate::Session;
use climon2_core::{
    hello::Lifecycle,
    protocol::{self, ClientMessage, ServerMessage as PrivateMessage, TerminalSize},
    wire::{
        self, Acknowledge, Acquire, ControlRecord, ControlRevoked, Counter, Display, Focus,
        FocusBytes, Frame, Geometry, GrantReason, Input, Kill, Leave, Mode, Outcome, Ready, Resize,
        Response, RestorationBegin, RestorationComplete, RevocationReason, ServerMessage,
        SetViewing, Success,
    },
};
use std::{
    fs,
    io::Write,
    num::NonZeroU32,
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};

fn geometry(cols: u32, rows: u32) -> Geometry {
    Geometry {
        cols: NonZeroU32::new(cols).unwrap(),
        rows: NonZeroU32::new(rows).unwrap(),
    }
}

/// An interactive attachment: its stream, the authority of its current
/// restoration, whether it still controls, its last request ID and the
/// terminal bytes it received.
struct Attached {
    stream: UnixStream,
    /// The families its Open selected, sorted.
    selected: Vec<String>,
    period: u64,
    restoration: u64,
    /// From its Control granted until its Control revoked.
    controlling: bool,
    request: u64,
    display: Vec<u8>,
}

impl Attached {
    /// Opens an interactive attachment with request 1, selecting
    /// `optional`, and checks every record of the documented order: the
    /// Response, Initial metadata, Control granted (`attach`) at the session
    /// geometry `granted`, then the restoration.
    fn open(session: &Session, host: Geometry, optional: &[&str], granted: Geometry) -> Self {
        let mut stream = peer(session);
        request(
            &mut stream,
            &open(1, &[], optional, Mode::Interactive(host)),
        );
        Self::opened(stream, granted)
    }

    fn opened(stream: UnixStream, granted: Geometry) -> Self {
        let mut attached = Self::opened_begun(stream, granted);
        attached.completed();
        attached
    }

    /// The Response, Initial metadata, Control granted (`attach`) at the
    /// session geometry `granted` and Restoration begin of an interactive
    /// Open with request 1.
    fn opened_begun(mut stream: UnixStream, granted: Geometry) -> Self {
        let response = message(&mut stream);
        let ServerMessage::Response(Response {
            request_id,
            outcome:
                Outcome::Ok(Success::Opened {
                    selected_capabilities,
                    ..
                }),
        }) = response
        else {
            panic!("not opened: {response:?}");
        };
        assert_eq!(request_id, counter(1));
        let mut selected = selected_capabilities.tokens().to_vec();
        selected.sort();
        assert!(
            matches!(message(&mut stream), ServerMessage::InitialMetadata(_)),
            "no Initial metadata after the Open response"
        );
        let mut attached = Self {
            stream,
            selected,
            period: 0,
            restoration: 0,
            controlling: false,
            request: 1,
            display: Vec::new(),
        };
        attached.grant(GrantReason::Attach, granted);
        attached
    }

    /// The next record is Control granted for `reason` at `geometry`, under a
    /// period newer than any this attachment had; its restoration follows.
    fn granted(&mut self, reason: GrantReason, geometry: Geometry) {
        self.grant(reason, geometry);
        self.completed();
    }

    /// Control granted, then the beginning of its restoration.
    fn grant(&mut self, reason: GrantReason, geometry: Geometry) {
        let ServerMessage::ControlGranted(granted) = message(&mut self.stream) else {
            panic!("expected Control granted");
        };
        assert_eq!(
            (granted.geometry, granted.reason),
            (geometry, reason),
            "{granted:?}"
        );
        assert!(granted.period.get() > self.period, "{granted:?}");
        self.period = granted.period.get();
        self.controlling = true;
        self.begun(granted.restoration_id.get(), geometry);
    }

    /// A replacement restoration of the current period at `geometry`.
    fn restored(&mut self, restoration: u64, geometry: Geometry) {
        self.begun(restoration, geometry);
        self.completed();
    }

    /// Restoration begin for `restoration` of the current period at
    /// `geometry`.
    fn begun(&mut self, restoration: u64, geometry: Geometry) {
        let begin = RestorationBegin {
            period: counter(self.period),
            restoration_id: counter(restoration),
            geometry,
        };
        assert_eq!(
            message(&mut self.stream),
            ServerMessage::RestorationBegin(begin)
        );
        self.restoration = restoration;
    }

    /// The current restoration's Display chunks, then its Restoration
    /// complete.
    fn completed(&mut self) {
        let (period, restoration_id) = self.authority();
        let complete = RestorationComplete {
            period,
            restoration_id,
        };
        assert_eq!(
            self.next_after_display(),
            ServerMessage::RestorationComplete(complete)
        );
    }

    /// The next record that is not a Display of the current authority,
    /// whose payloads are kept in order.
    fn next_after_display(&mut self) -> ServerMessage {
        loop {
            match message(&mut self.stream) {
                ServerMessage::Display(display) => self.displayed(display),
                other => return other,
            }
        }
    }

    /// A Display reaches only the controller, carries its current authority,
    /// and holds at least one byte and at most 65,535, the private
    /// protocol's output chunk.
    fn displayed(&mut self, display: Display) {
        assert!(self.controlling, "Display after Control revoked");
        assert_eq!(
            (display.period, display.restoration_id),
            self.authority(),
            "Display under another authority"
        );
        assert!(!display.payload.is_empty(), "an empty Display");
        assert!(display.payload.len() <= 65_535, "{}", display.payload.len());
        self.display.extend(display.payload);
    }

    /// Reads Display until the bytes received hold `needle` `times` times.
    fn until_displayed(&mut self, needle: &[u8], times: usize) {
        while crate::count(&self.display, needle) < times {
            match message(&mut self.stream) {
                ServerMessage::Display(display) => self.displayed(display),
                other => panic!("expected Display holding {needle:?}, got {other:?}"),
            }
        }
    }

    /// The next request ID.
    fn next(&mut self) -> Counter {
        self.request += 1;
        counter(self.request)
    }

    fn authority(&self) -> (Counter, Counter) {
        (counter(self.period), counter(self.restoration))
    }

    /// The Response to the latest request, after any Display.
    fn response(&mut self) -> Outcome {
        match self.next_after_display() {
            ServerMessage::Response(response) => {
                assert_eq!(response.request_id, counter(self.request));
                response.outcome
            }
            other => panic!("expected the Response to {}, got {other:?}", self.request),
        }
    }

    /// Sends the request `record` builds from the next request ID and
    /// `authority`, and returns its outcome.
    fn ask(
        &mut self,
        authority: (Counter, Counter),
        record: impl FnOnce(Counter, Counter, Counter) -> Frame,
    ) -> Outcome {
        let (period, restoration_id) = authority;
        let frame = record(self.next(), period, restoration_id);
        send(&mut self.stream, &frame);
        self.response()
    }

    fn ready_as(&mut self, authority: (Counter, Counter)) -> Outcome {
        self.ask(authority, |request_id, period, restoration_id| {
            Ready {
                request_id,
                period,
                restoration_id,
            }
            .to_frame()
        })
    }

    fn ready(&mut self) -> Outcome {
        self.ready_as(self.authority())
    }

    /// Confirms readiness for the first time under this authority.
    fn confirm_ready(&mut self) {
        let (period, restoration_id) = self.authority();
        let ready = Success::Ready {
            changed: true,
            period,
            restoration_id,
        };
        assert_eq!(self.ready(), Outcome::Ok(ready));
    }

    fn input_as(&mut self, authority: (Counter, Counter), payload: &[u8]) -> Outcome {
        self.ask(authority, |request_id, period, restoration_id| {
            Input {
                request_id,
                period,
                restoration_id,
                payload: payload.to_vec(),
            }
            .to_frame()
        })
    }

    fn input(&mut self, payload: &[u8]) -> Outcome {
        self.input_as(self.authority(), payload)
    }

    /// Input accepted whole for forwarding.
    fn typed(&mut self, payload: &[u8]) {
        let accepted_bytes = NonZeroU32::new(payload.len() as u32).unwrap();
        assert_eq!(
            self.input(payload),
            Outcome::Ok(Success::InputAccepted { accepted_bytes })
        );
    }

    fn resize_as(&mut self, authority: (Counter, Counter), cols: u32, rows: u32) -> Outcome {
        self.ask(authority, |request_id, period, restoration_id| {
            Resize {
                request_id,
                period,
                restoration_id,
                geometry: geometry(cols, rows),
            }
            .to_frame()
        })
    }

    fn resize(&mut self, cols: u32, rows: u32) -> Outcome {
        self.resize_as(self.authority(), cols, rows)
    }

    fn focus_as(&mut self, authority: (Counter, Counter), focused: bool) -> Outcome {
        self.ask(authority, |request_id, period, restoration_id| {
            Focus {
                request_id,
                period,
                restoration_id,
                focused,
            }
            .to_frame()
        })
    }

    fn focus(&mut self, focused: bool) -> Outcome {
        self.focus_as(self.authority(), focused)
    }

    /// Sends Input of `payload`, then a Focus changing the desired focus to
    /// `focused`, in one write; both are accepted, the Focus with its
    /// sequence.
    fn type_then_focus(&mut self, payload: &[u8], focused: bool) {
        let (period, restoration_id) = self.authority();
        let typed = self.next();
        let reported = self.next();
        let input = Input {
            request_id: typed,
            period,
            restoration_id,
            payload: payload.to_vec(),
        };
        let focus = Focus {
            request_id: reported,
            period,
            restoration_id,
            focused,
        };
        let bytes = [
            frame_bytes(&input.to_frame()),
            frame_bytes(&focus.to_frame()),
        ]
        .concat();
        self.stream.write_all(&bytes).unwrap();
        self.request = typed.get();
        let accepted_bytes = NonZeroU32::new(payload.len() as u32).unwrap();
        assert_eq!(
            self.response(),
            Outcome::Ok(Success::InputAccepted { accepted_bytes })
        );
        self.request = reported.get();
        assert_eq!(
            self.response(),
            focus_ok(true, focused, FocusBytes::Sequence)
        );
    }

    /// Acquire by a non-controlling attachment: its grant and the beginning
    /// of its restoration precede the success, then the restoration
    /// completes.
    fn acquire(&mut self, geometry: Geometry) {
        let request_id = self.next();
        request(&mut self.stream, &Acquire { request_id });
        self.grant(GrantReason::Acquire, geometry);
        let (period, restoration_id) = self.authority();
        let acquired = Success::Acquired {
            changed: true,
            period,
            restoration_id,
        };
        assert_eq!(self.response(), Outcome::Ok(acquired));
        self.completed();
    }

    /// Any release Display, then Control revoked for `reason` under the
    /// current authority; no Display may follow it.
    fn revoked(&mut self, reason: RevocationReason) {
        let (period, restoration_id) = self.authority();
        let revoked = ControlRevoked {
            period,
            restoration_id,
            reason,
        };
        assert_eq!(
            self.next_after_display(),
            ServerMessage::ControlRevoked(revoked)
        );
        self.controlling = false;
    }

    /// Leaves as an accepted detach: its Response reports an attachment
    /// detached, and the stream then ends.
    fn leave(&mut self) {
        let request_id = self.next();
        request(&mut self.stream, &Leave { request_id });
        let left = Success::Left {
            attachment_detached: true,
        };
        assert_eq!(self.response(), Outcome::Ok(left));
        assert_closed(&mut self.stream);
    }

    /// The session ended: control ends with it, then the stream.
    fn ended(&mut self) {
        self.revoked(RevocationReason::SessionEnding);
        assert_closed(&mut self.stream);
    }
}

/// The rejection of the latest request with `code`, carrying no result.
fn rejected(outcome: Outcome, code: &str) {
    match outcome {
        Outcome::Rejected(rejection) => {
            assert_eq!(rejection.error.code.as_str(), code);
            assert_eq!(rejection.known_final, None);
        }
        other => panic!("expected a rejection with {code}, got {other:?}"),
    }
}

/// The bytes of one revision-1 frame.
fn frame_bytes(frame: &Frame) -> Vec<u8> {
    let mut bytes = Vec::new();
    wire::write_frame(&mut bytes, frame, limit()).unwrap();
    bytes
}

/// A frame of `code` with `body` as written, whether or not it is valid.
fn raw_frame(code: u8, body: &[u8]) -> Vec<u8> {
    let length = u32::try_from(body.len() + 1).unwrap();
    [&length.to_be_bytes()[..], &[code], body].concat()
}

/// A binary Input header: request ID, period and restoration ID.
fn input_header(request: u64, period: u64, restoration: u64) -> Vec<u8> {
    [request, period, restoration]
        .iter()
        .flat_map(|value| value.to_be_bytes())
        .collect()
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

/// Waits for the `zero-size` fixture, which starts at the 80x24 fallback, to
/// have reported its initial size, so a grant can resize its terminal.
fn zero_size_started(session: &Session) {
    let report = session.directory.path().join("initial-size");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !report.exists() {
        assert!(Instant::now() < deadline, "the fixture never started");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(fs::read_to_string(report).unwrap(), "80x24");
}

/// COM-05 (Contract, interactive order) and COM-03 (Contract, interactive
/// Open): an interactive Open attaches at once and is granted at the
/// requested geometry. Its records arrive in the documented order: the
/// Response, then Initial metadata, then Control granted (`attach`), then
/// Restoration begin with the same authority and geometry, then zero or more
/// Display chunks of that authority, then Restoration complete. The order
/// holds when hello and the start of Open arrive in one write and the rest
/// in pieces, and when Ready and Input arrive together: each is answered in
/// order, Input only after readiness. The command's terminal has the
/// requested size, its output follows as live Display of the same
/// authority, and the hello `state` counts the attachment. When the command
/// exits, control ends with the session (`session_ending`) and the stream
/// closes.
#[test]
fn interactive_open_is_granted_at_the_requested_geometry_in_the_documented_order() {
    let mut session = Session::start("zero-size");
    zero_size_started(&session);
    let mut stream = UnixStream::connect(&session.socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let hello = ClientMessage::Hello { protocol: 1 }.to_frame().unwrap();
    let mut bytes = Vec::new();
    protocol::write_frame(&mut bytes, &hello).unwrap();
    let opening = open(1, &[], &[], Mode::Interactive(geometry(100, 30))).to_frame();
    let opening = frame_bytes(&opening);
    let (first, rest) = opening.split_at(9);
    bytes.extend_from_slice(first);
    stream.write_all(&bytes).unwrap();
    assert_eq!(protocol::read_frame(&mut stream).unwrap().type_code(), 0x01);
    write_in_pieces(&session, &mut stream, rest);
    let mut attached = Attached::opened(stream, geometry(100, 30));
    assert_eq!((attached.period, attached.restoration), (1, 1));
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );

    let (period, restoration_id) = attached.authority();
    let mut coalesced = frame_bytes(
        &Ready {
            request_id: counter(2),
            period,
            restoration_id,
        }
        .to_frame(),
    );
    coalesced.extend(frame_bytes(
        &Input {
            request_id: counter(3),
            period,
            restoration_id,
            payload: b"go\n".to_vec(),
        }
        .to_frame(),
    ));
    attached.stream.write_all(&coalesced).unwrap();
    attached.request = 2;
    let ready = Success::Ready {
        changed: true,
        period,
        restoration_id,
    };
    assert_eq!(attached.response(), Outcome::Ok(ready));
    attached.request = 3;
    let accepted_bytes = NonZeroU32::new(3).unwrap();
    assert_eq!(
        attached.response(),
        Outcome::Ok(Success::InputAccepted { accepted_bytes })
    );
    attached.ended();
    session.finish();
    assert_eq!(
        fs::read_to_string(session.directory.path().join("resized")).unwrap(),
        "100x30"
    );
    assert_eq!(crate::count(&attached.display, b"fixture-ready"), 1);
}

/// An interactive attachment at the fixtures' 91x37 session size, ready and
/// with the fixture's `fixture-ready` received.
fn ready_attachment(session: &Session, optional: &[&str]) -> Attached {
    let mut attached = Attached::open(session, geometry(91, 37), optional, geometry(91, 37));
    attached.confirm_ready();
    attached.until_displayed(b"fixture-ready", 1);
    attached
}

/// CTL-05 (Contract, and Fault through a raw peer injecting stale
/// authority): Input, Resize and Ready under an earlier control period are
/// `stale_period` from the controller, and under a superseded restoration of
/// its current period `stale_restoration`; from an attachment that does not
/// control they are `not_controller`. A resize before readiness replaces the
/// restoration within its period. A Ready under late authority never
/// re-enables input: Input stays `not_ready` until the controller confirms
/// its current restoration. No rejected byte or size reaches the command.
#[test]
fn stale_and_superseded_authority_is_rejected_and_never_re_enables_input() {
    let mut session = Session::start("lines");
    let mut first = ready_attachment(&session, &[]);
    let first_authority = first.authority();
    first.typed(b"a1\n");
    crate::wait_line(&session, "a1 91x37");

    let mut second = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    first.revoked(RevocationReason::Takeover);
    assert_eq!((second.period, second.restoration), (2, 2));
    let superseded = second.authority();
    assert_eq!(
        second.resize(100, 30),
        Outcome::Ok(Success::ResizeAccepted {})
    );
    second.restored(3, geometry(100, 30));
    rejected(second.ready_as(superseded), "stale_restoration");
    rejected(second.input_as(superseded, b"stale\n"), "stale_restoration");
    rejected(second.resize_as(superseded, 120, 40), "stale_restoration");
    rejected(second.input(b"early\n"), "not_ready");
    rejected(
        first.input_as(first_authority, b"revoked\n"),
        "not_controller",
    );
    rejected(first.ready_as(first_authority), "not_controller");
    rejected(first.resize_as(first_authority, 120, 40), "not_controller");
    second.confirm_ready();
    second.typed(b"b1\n");
    crate::wait_line(&session, "b1 100x30");

    first.acquire(geometry(91, 37));
    second.revoked(RevocationReason::Takeover);
    assert_eq!((first.period, first.restoration), (3, 4));
    rejected(first.ready_as(first_authority), "stale_period");
    rejected(first.input_as(first_authority, b"late\n"), "stale_period");
    rejected(first.resize_as(first_authority, 120, 40), "stale_period");
    rejected(first.input(b"late\n"), "not_ready");
    first.confirm_ready();
    first.typed(b"a2\nend\n");
    first.ended();
    assert_closed(&mut second.stream);
    session.finish();
    assert_eq!(
        fs::read_to_string(session.directory.path().join("lines")).unwrap(),
        "a1 91x37\nb1 100x30\na2 91x37\n"
    );
}

/// COM-05 (Contract, geometry and results): a ready controller's Resize is
/// accepted before it applies. The session's actual geometry then follows
/// as Geometry under the current period, and the command's terminal has
/// it. A host beyond the supported session size gets the clamped geometry,
/// and a host change that leaves the session geometry as it was reports
/// none.
#[test]
fn a_ready_controllers_resize_reports_the_actual_geometry_after_its_acceptance() {
    let mut session = Session::start("lines");
    let mut attached = ready_attachment(&session, &[]);
    let geometry_report = |cols, rows| {
        ServerMessage::Geometry(wire::ActualGeometry {
            period: counter(1),
            geometry: geometry(cols, rows),
        })
    };
    for (cols, rows, actual) in [(100, 30, (100, 30)), (600, 300, (500, 200))] {
        assert_eq!(
            attached.resize(cols, rows),
            Outcome::Ok(Success::ResizeAccepted {})
        );
        assert_eq!(
            attached.next_after_display(),
            geometry_report(actual.0, actual.1)
        );
    }
    assert_eq!(
        attached.resize(700, 300),
        Outcome::Ok(Success::ResizeAccepted {})
    );
    attached.typed(b"x\n");
    crate::wait_line(&session, "x 500x200");
    assert_eq!(
        attached.resize(100, 30),
        Outcome::Ok(Success::ResizeAccepted {})
    );
    assert_eq!(attached.next_after_display(), geometry_report(100, 30));
    attached.typed(b"y\nend\n");
    attached.ended();
    session.finish();
    assert_eq!(
        fs::read_to_string(session.directory.path().join("lines")).unwrap(),
        "x 500x200\ny 100x30\n"
    );
}

/// COM-04 (Contract): binary Input and Display carry exact bytes. The
/// literal `00 ff 41`, NUL and invalid UTF-8 reach the application
/// unchanged, in Input split across requests with an ID each, and the raw
/// application's unchanged echo returns as Display, whatever its chunk
/// boundaries, followed by the release.
#[test]
fn binary_input_and_display_pass_unchanged() {
    let mut session = Session::start("raw");
    let mut attached = ready_attachment(&session, &[]);
    let mut payload: Vec<u8> = (0..65535).map(|value| (value % 256) as u8).collect();
    payload[..8].copy_from_slice(&[0x00, 0xff, 0x41, 0x00, 0xc3, 0x28, 0xfe, 0x80]);
    let start = attached.display.len();
    for chunk in [&payload[..3], &payload[3..40000], &payload[40000..]] {
        attached.typed(chunk);
    }
    attached.ended();
    session.finish();
    let echoed = &attached.display[start..];
    assert_eq!(&echoed[..payload.len()], payload);
    assert_eq!(&echoed[payload.len()..], crate::release_at(37, 83));
}

/// COM-02 (Contract, binary Input headers): an Input whose binary header
/// breaks the rules is connection-fatal with exactly the documented code,
/// as the first frame after hello and on an attachment alike. A body
/// shorter than the 24-byte header is `invalid_record`, whatever it holds; a
/// zero request ID is `invalid_request_id`; a zero period or restoration ID
/// is `invalid_record`; and a request ID that does not increase is
/// `invalid_request_id`. A controller lost this way departed unexpectedly:
/// as the final attachment, its loss ends the session, and no byte of the
/// refused Input reaches the command.
#[test]
fn malformed_binary_input_is_connection_fatal() {
    let mut session = Session::start("lines");
    let first_frames = [
        (vec![1; 23], "invalid_record"),
        (
            [input_header(0, 1, 1), b"x".to_vec()].concat(),
            "invalid_request_id",
        ),
        (
            [input_header(1, 0, 1), b"x".to_vec()].concat(),
            "invalid_record",
        ),
        (
            [input_header(1, 1, 0), b"x".to_vec()].concat(),
            "invalid_record",
        ),
    ];
    for (body, code) in first_frames {
        let mut stream = peer(&session);
        stream.write_all(&raw_frame(0x31, &body)).unwrap();
        super::assert_fatal(&mut stream, code);
    }
    let mut attached = ready_attachment(&session, &[]);
    attached.typed(b"a1\n");
    crate::wait_line(&session, "a1 91x37");
    let (period, restoration) = attached.authority();
    let repeated = input_header(attached.request, period.get(), restoration.get());
    let body = [repeated, b"never\n".to_vec()].concat();
    attached.stream.write_all(&raw_frame(0x31, &body)).unwrap();
    match attached.next_after_display() {
        ServerMessage::FatalError(wire::FatalError(error)) => {
            assert_eq!(error.code.as_str(), "invalid_request_id");
        }
        other => panic!("expected Fatal error invalid_request_id, got {other:?}"),
    }
    assert_closed(&mut attached.stream);
    session.finish();
    assert_eq!(
        fs::read_to_string(session.directory.path().join("lines")).unwrap(),
        "a1 91x37\n"
    );
}

/// INP-03 (Contract): bracketed-paste bytes, binary ones included, reach the
/// application exactly as sent. A paste the application still has open
/// when the controller's input admission ends, here through another
/// attachment's takeover, is closed with exactly its end marker before the
/// next controller's input.
#[test]
fn bracketed_paste_is_exact_and_an_open_paste_is_closed_when_admission_ends() {
    let mut session = Session::start("paste-record");
    let mut first = ready_attachment(&session, &[]);
    let whole = b"\x1b[200~\x00\xffA\x1b[A\tline\r\n\x1b[201~".to_vec();
    first.typed(&whole);
    let open = b"\x1b[200~still-open".to_vec();
    first.typed(&open);
    let path = session.directory.path().join("pasted");
    let written = [whole.as_slice(), &open].concat();
    crate::wait_contents(&path, |contents| contents == written);

    let mut second = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    first.revoked(RevocationReason::Takeover);
    second.confirm_ready();
    second.typed(b"after-END");
    second.ended();
    assert_closed(&mut first.stream);
    session.finish();
    let expected = [written.as_slice(), b"\x1b[201~after-END"].concat();
    assert_eq!(fs::read(&path).unwrap(), expected);
}

/// TRM-02 (Contract, session queries): the daemon is the only responder to
/// a session's terminal queries. The application's query never reaches the
/// attachment's Display, the application receives exactly one reply, the
/// daemon's, and the attachment sends none.
#[test]
fn session_queries_are_answered_by_the_daemon_alone() {
    let mut session = Session::start("reply-record");
    let mut attached = ready_attachment(&session, &[]);
    attached.typed(b"x");
    let path = session.directory.path().join("recorded");
    crate::wait_contents(&path, |contents| {
        contents.len() > 1 && contents.ends_with(b"c")
    });
    attached.typed(b"END");
    attached.ended();
    session.finish();
    let recorded = fs::read(&path).unwrap();
    let reply = recorded
        .strip_prefix(b"x".as_slice())
        .and_then(|rest| rest.strip_suffix(b"END".as_slice()))
        .unwrap_or_else(|| panic!("{:?}", String::from_utf8_lossy(&recorded)));
    assert!(
        reply.starts_with(b"\x1b[?") && reply.ends_with(b"c") && crate::count(reply, b"\x1b") == 1,
        "{:?}",
        String::from_utf8_lossy(reply)
    );
    assert_eq!(crate::count(&attached.display, b"\x1b[c"), 0);
}

/// TRM-02 (Contract, titles) and CTL-03 (Contract, title case for an
/// attachment): an attachment that selected `title-v1` receives titles as
/// inert text in Title changed, as passive connections do, and its Display
/// only ever carries a title as an `OSC 2` whose control characters are
/// percent-encoded, never raw. Text that merely looks like a title
/// sequence stays text and changes no title.
#[test]
fn titles_reach_an_attachment_as_inert_text() {
    let session = Session::start("script");
    let mut attached = ready_attachment(&session, &["title-v1"]);
    let title = "café — \u{9b}[31m%41";
    crate::script(&session, &format!("title {title}"));
    assert_eq!(
        attached.next_after_display(),
        ServerMessage::TitleChanged(wire::TitleChanged {
            title: Some(title.into())
        })
    );
    let encoded = "\x1b]2;café — %C2%9B[31m%41\x1b\\";
    assert_eq!(crate::count(&attached.display, encoded.as_bytes()), 1);
    assert_eq!(crate::count(&attached.display, "\u{9b}".as_bytes()), 0);
    crate::script(&session, "text ]2;looks-like-a-title");
    crate::script(&session, "title after");
    assert_eq!(
        attached.next_after_display(),
        ServerMessage::TitleChanged(wire::TitleChanged {
            title: Some("after".into())
        })
    );
    assert_eq!(
        crate::count(&attached.display, b"]2;looks-like-a-title\r"),
        1
    );
    crate::issue(&session, "exit");
    attached.ended();
    let mut session = session;
    session.finish();
}

/// A Focus success.
fn focus_ok(changed: bool, focused: bool, accepted_bytes: FocusBytes) -> Outcome {
    Outcome::Ok(Success::Focus {
        changed,
        focused,
        accepted_bytes,
    })
}

/// FOC-01 (Contract): Focus needs `focus-v1`, checked first, then the ready
/// controller's current authority, and it remembers the desired application
/// focus of the control period. A change reaches the application as exactly
/// `ESC [ I` or `ESC [ O` while it has focus reporting on, accepting those
/// three bytes; a repeated state, or any state while reporting is off,
/// forwards nothing, so there are no duplicates. Newly enabled reporting
/// receives the current state once, and nothing while it is unknown. Focus
/// before readiness or under an earlier period is rejected and never
/// replayed: a new control period starts unknown, and its new readiness
/// synchronizes the latest intent under the current authority.
#[test]
fn focus_reaches_the_application_once_per_change_and_resynchronizes_on_new_readiness() {
    let mut session = Session::start("focus-record");
    let path = session.directory.path().join("focused");
    let mut recorded = Vec::new();
    let mut expect = |bytes: &[u8]| {
        recorded.extend_from_slice(bytes);
        let expected = recorded.clone();
        crate::wait_contents(&path, |contents| contents == expected);
    };
    let mut first = ready_attachment(&session, &["focus-v1"]);
    assert_eq!(first.focus(true), focus_ok(true, true, FocusBytes::None));
    assert_eq!(first.focus(true), focus_ok(false, true, FocusBytes::None));
    first.typed(b"ON");
    first.until_displayed(b"focus-on", 1);
    expect(b"ON\x1b[I");
    assert_eq!(first.focus(true), focus_ok(false, true, FocusBytes::None));
    assert_eq!(
        first.focus(false),
        focus_ok(true, false, FocusBytes::Sequence)
    );
    expect(b"\x1b[O");
    assert_eq!(first.focus(false), focus_ok(false, false, FocusBytes::None));
    first.typed(b"OFF");
    first.until_displayed(b"focus-off", 1);
    assert_eq!(first.focus(true), focus_ok(true, true, FocusBytes::None));
    first.typed(b"ON");
    first.until_displayed(b"focus-on", 2);
    expect(b"OFFON\x1b[I");
    let stale = first.authority();

    let mut second = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    first.revoked(RevocationReason::Takeover);
    rejected(second.focus(true), "unsupported_capability");
    second.confirm_ready();
    second.typed(b"OFF");
    second.until_displayed(b"focus-off", 1);
    second.typed(b"ON");
    second.until_displayed(b"focus-on", 1);
    expect(b"OFFON");

    first.acquire(geometry(91, 37));
    second.revoked(RevocationReason::Takeover);
    rejected(first.focus_as(stale, true), "stale_period");
    rejected(first.focus(false), "not_ready");
    first.confirm_ready();
    assert_eq!(
        first.focus(true),
        focus_ok(true, true, FocusBytes::Sequence)
    );
    expect(b"\x1b[I");
    first.typed(b"END");
    first.ended();
    assert_closed(&mut second.stream);
    session.finish();
    assert_eq!(
        fs::read(&path).unwrap(),
        [recorded.as_slice(), b"END"].concat()
    );
}

/// FOC-01 and INP-03 (Contract, Focus after Input): a focus report reaches
/// the application in request order, after the Input accepted before it,
/// however the requests were split across reads: in one write with that
/// Input, or after its response. A focus-out sent with the last Input of a
/// paste split across several requests follows the paste's end marker and
/// never lands inside the application's paste.
#[test]
fn focus_reports_follow_the_input_accepted_before_them() {
    let mut session = Session::start("focus-record");
    let path = session.directory.path().join("focused");
    let mut recorded = Vec::new();
    let mut expect = |bytes: &[u8]| {
        recorded.extend_from_slice(bytes);
        let expected = recorded.clone();
        crate::wait_contents(&path, |contents| contents == expected);
    };
    let mut attached = ready_attachment(&session, &["focus-v1"]);
    assert_eq!(attached.focus(true), focus_ok(true, true, FocusBytes::None));
    attached.typed(b"ON");
    attached.until_displayed(b"focus-on", 1);
    expect(b"ON\x1b[I");
    attached.type_then_focus(b"abc", false);
    expect(b"abc\x1b[O");
    attached.typed(b"def");
    assert_eq!(
        attached.focus(true),
        focus_ok(true, true, FocusBytes::Sequence)
    );
    expect(b"def\x1b[I");
    attached.typed(b"\x1b[200~first ");
    expect(b"\x1b[200~first ");
    attached.type_then_focus(b"second\x1b[201~", false);
    expect(b"second\x1b[201~\x1b[O");
    attached.typed(b"END");
    attached.ended();
    session.finish();
    assert_eq!(
        fs::read(&path).unwrap(),
        [recorded.as_slice(), b"END"].concat()
    );
}

/// COM-01 and COM-03 (Contract, interactive first roles): an interactive
/// Open selects the requested families this daemon supports, `focus-v1`
/// included, and is rejected without effects, ending its stream, for an
/// unsupported revision, an unknown required family or a geometry beyond
/// the supported terminal size, one dimension at a time. The session keeps
/// its one attachment.
#[test]
fn interactive_open_selects_supported_families_and_rejects_without_effects() {
    let mut session = Session::start("exit");
    let optional = ["focus-v1", "title-v1", "status-v1", "future-v9"];
    let mut attached = ready_attachment(&session, &optional);
    assert_eq!(attached.selected, ["focus-v1", "title-v1"]);
    let mut later_revision = open(1, &[], &[], Mode::Interactive(geometry(80, 24)));
    later_revision.revision = 2;
    let refused = [
        (later_revision, "unsupported_revision"),
        (
            open(1, &["future-v9"], &[], Mode::Interactive(geometry(80, 24))),
            "unsupported_capability",
        ),
        (
            open(1, &[], &[], Mode::Interactive(geometry(65536, 24))),
            "invalid_geometry",
        ),
        (
            open(1, &[], &[], Mode::Interactive(geometry(80, 65536))),
            "invalid_geometry",
        ),
    ];
    for (role, code) in refused {
        let mut refused = peer(&session);
        request(&mut refused, &role);
        assert_rejected(&mut refused, 1, code);
        assert_closed(&mut refused);
    }
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );
    attached.typed(b"go\n");
    attached.ended();
    session.finish();
}

/// COM-02 and COM-03 (Contract, interactive requests): each request on an
/// interactive attachment gets its documented result or rejection, without
/// effects when rejected and with the stream going on. A later Open or Kill
/// never repurposes the attachment (`invalid_state`); Set viewing and
/// Acknowledge stay `invalid_state` until #91; empty Input is
/// `invalid_argument` and a geometry beyond the supported size
/// `invalid_geometry`; Focus without `focus-v1` is `unsupported_capability`.
/// Ready again is an explicit no-op, as is Acquire by the controller, which
/// keeps its period and restoration. Leave releases the presentation, then
/// reports the detached attachment, and the stream ends; as the final
/// deliberate departure it leaves the session running, detached.
#[test]
fn requests_on_an_interactive_attachment_get_their_documented_results() {
    let mut session = Session::start("exit");
    let mut attached = ready_attachment(&session, &[]);
    let (period, restoration_id) = attached.authority();
    let request_id = attached.next();
    request(
        &mut attached.stream,
        &open(request_id.get(), &[], &[], Mode::Monitor),
    );
    rejected(attached.response(), "invalid_state");
    let request_id = attached.next();
    request(
        &mut attached.stream,
        &Kill {
            request_id,
            revision: 1,
            required_capabilities: super::families(&[]),
        },
    );
    rejected(attached.response(), "invalid_state");
    let viewing = attached.ask(
        attached.authority(),
        |request_id, period, restoration_id| {
            SetViewing {
                request_id,
                period,
                restoration_id,
                viewing: true,
            }
            .to_frame()
        },
    );
    rejected(viewing, "invalid_state");
    let acknowledged = attached.ask(
        attached.authority(),
        |request_id, period, restoration_id| {
            Acknowledge {
                request_id,
                period,
                restoration_id,
            }
            .to_frame()
        },
    );
    rejected(acknowledged, "invalid_state");
    rejected(attached.input(b""), "invalid_argument");
    rejected(attached.resize(65536, 30), "invalid_geometry");
    rejected(attached.resize(100, 65536), "invalid_geometry");
    rejected(attached.focus(true), "unsupported_capability");
    let again = Success::Ready {
        changed: false,
        period,
        restoration_id,
    };
    assert_eq!(attached.ready(), Outcome::Ok(again));
    let request_id = attached.next();
    request(&mut attached.stream, &Acquire { request_id });
    let unchanged = Success::Acquired {
        changed: false,
        period,
        restoration_id,
    };
    assert_eq!(attached.response(), Outcome::Ok(unchanged));
    let mark = attached.display.len();
    attached.leave();
    assert!(
        attached.display[mark..].ends_with(&crate::release_at(3, 14)),
        "{:?}",
        String::from_utf8_lossy(&attached.display[mark..])
    );
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );
    let mut controlling = session.attach();
    session.wait_marker(&mut controlling);
    crate::send(&mut controlling, ClientMessage::Input(b"go\n".to_vec()));
    let (_, end) = crate::until_terminal(&mut controlling);
    assert_eq!(end, PrivateMessage::Exit { code: 42 });
    session.finish();
    assert_eq!(
        fs::read_dir(session.directory.path().join("sessions"))
            .unwrap()
            .count(),
        0
    );
}

/// COM-03 (Contract, committed termination): a terminating session rejects
/// new interactive work with `session_terminating`: an interactive Open,
/// which ends its stream, and Input, Resize, Ready, Focus and Acquire on its
/// attachments. Leave still works: the controller's presentation is
/// released before the departure is reported, and neither attachment is
/// handed control.
#[test]
fn committed_termination_rejects_new_interactive_work_and_leave_still_works() {
    let mut session = Session::start("ignore-hup");
    let mut other = ready_attachment(&session, &[]);
    let mut controller =
        Attached::open(&session, geometry(91, 37), &["focus-v1"], geometry(91, 37));
    other.revoked(RevocationReason::Takeover);
    controller.confirm_ready();
    let mut killer = session.connect();
    killer
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    crate::send(&mut killer, ClientMessage::Kill);
    let deadline = Instant::now() + Duration::from_secs(20);
    while crate::advertised_state(&session).0 != Lifecycle::Terminating {
        assert!(Instant::now() < deadline, "the kill never committed");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut refused = peer(&session);
    request(
        &mut refused,
        &open(1, &[], &[], Mode::Interactive(geometry(91, 37))),
    );
    assert_rejected(&mut refused, 1, "session_terminating");
    assert_closed(&mut refused);
    rejected(controller.input(b"late"), "session_terminating");
    rejected(controller.resize(100, 30), "session_terminating");
    rejected(controller.ready(), "session_terminating");
    rejected(controller.focus(true), "session_terminating");
    let request_id = other.next();
    request(&mut other.stream, &Acquire { request_id });
    rejected(other.response(), "session_terminating");
    other.leave();
    let mark = controller.display.len();
    controller.leave();
    assert!(
        controller.display[mark..].ends_with(&crate::release_at(3, 14)),
        "{:?}",
        String::from_utf8_lossy(&controller.display[mark..])
    );
    assert!(matches!(
        crate::receive(&mut killer),
        PrivateMessage::Terminated { .. }
    ));
    session.finish();
}

/// COM-01 (Contract, ordinary attach parity) and COM-03 (Contract, Acquire
/// and Leave): takeover, hand-back and the leave policy match the private
/// protocol, across both protocols. An attach or Acquire takes control at
/// once and revokes the previous controller after its release; a departing
/// controller, by Leave or by loss, hands control back to the most recently
/// controlling remaining attachment (`hand_back`); a final deliberate Leave
/// keeps the session running, detached, and a final unexpected departure
/// ends it.
#[test]
fn takeover_hand_back_and_leave_policy_match_the_private_protocol() {
    let mut session = Session::start("exit");
    let mut first = ready_attachment(&session, &[]);
    let mut private = crate::attach_at(&session, TerminalSize { cols: 91, rows: 37 });
    first.revoked(RevocationReason::Takeover);
    let (restoration, _) = crate::restoration(&mut private);
    assert_eq!(restoration, 2, "the private protocol names restoration 2");
    crate::send(
        &mut private,
        ClientMessage::Ready {
            period: restoration,
        },
    );

    first.acquire(geometry(91, 37));
    assert_eq!((first.period, first.restoration), (3, 3));
    let (released, revoked) = crate::until_terminal(&mut private);
    assert_eq!(revoked, PrivateMessage::Revoked);
    assert!(
        released.ends_with(&crate::release_at(3, 14)),
        "{released:?}"
    );
    first.confirm_ready();

    let mut second = ready_attachment(&session, &[]);
    first.revoked(RevocationReason::Takeover);
    assert_eq!((second.period, second.restoration), (4, 4));
    second.leave();
    first.granted(GrantReason::HandBack, geometry(91, 37));
    assert_eq!((first.period, first.restoration), (5, 5));
    first.confirm_ready();
    let third = ready_attachment(&session, &[]);
    first.revoked(RevocationReason::Takeover);
    drop(third);
    first.granted(GrantReason::HandBack, geometry(91, 37));
    assert_eq!((first.period, first.restoration), (7, 7));
    first.confirm_ready();
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 2, true)
    );
    crate::send(&mut private, ClientMessage::Detach);
    assert_eq!(
        crate::until_terminal(&mut private).1,
        PrivateMessage::Detached
    );
    first.leave();
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );

    let last = ready_attachment(&session, &[]);
    assert_eq!((last.period, last.restoration), (8, 8));
    drop(last);
    session.finish();
}

/// CTL-05 (Contract, restoring) and COM-05 (Contract, restoration chunks):
/// Ready needs the complete current restoration, not merely its beginning.
/// A restoration far larger than the attachment has read is produced as
/// the attachment drains it, in Display chunks of its authority; Ready sent
/// before its Restoration complete is `not_ready` and changes nothing, so
/// Input stays `not_ready`, while Resize is already accepted. Once the
/// restoration is complete, Ready succeeds and Input is admitted.
#[test]
fn ready_needs_the_complete_current_restoration() {
    let mut session = Session::start("script");
    crate::script(&session, "colors 4000");
    let mut stream = peer(&session);
    request(
        &mut stream,
        &open(1, &[], &[], Mode::Interactive(geometry(91, 37))),
    );
    let mut attached = Attached::opened_begun(stream, geometry(91, 37));
    rejected(attached.ready(), "not_ready");
    rejected(attached.input(b"early"), "not_ready");
    assert_eq!(
        attached.resize(91, 37),
        Outcome::Ok(Success::ResizeAccepted {})
    );
    attached.completed();
    assert!(
        attached.display.len() > 1_048_576,
        "{}",
        attached.display.len()
    );
    attached.confirm_ready();
    attached.typed(b"x");
    crate::issue(&session, "exit");
    attached.ended();
    session.finish();
}
