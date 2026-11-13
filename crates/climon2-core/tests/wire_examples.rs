//! DOC-01 (Contract case for the daemon wire pages): every published example
//! decodes to the values its page documents and re-encodes to the same
//! bytes. The examples are read from the pages themselves, so the published
//! text and the codec cannot drift apart. The same examples seed the member
//! strictness sweeps (COM-02).
use climon2_core::wire::{
    Acknowledge, Acquire, ActualGeometry, Available, CommandFinal, CommandResult, ControlGranted,
    ControlRecord, ControlRevoked, Display, Error, FatalCode, FatalError, Focus, FocusBytes, Frame,
    GrantReason, InitialMetadata, Input, Kill, Leave, Mode, Notification, NotificationSource, Open,
    OperationalCode, Outcome, Progress, ProgressChanged, ProgressState, Ready, Rejection, Request,
    Requested, Resize, Response, RestorationBegin, RestorationComplete, RevocationReason,
    ServerMessage, SetViewing, Status, StatusChanged, StatusState, Success, TitleChanged,
    read_frame, write_frame,
};
use climon2_core::{
    hello::{
        Advertised, Capability, HelloExtensions, Lifecycle, SessionState, advertise,
        advertise_revision1,
    },
    protocol::{self, HelloReply},
    wire::{Advertisement, Family, FrameLimits},
};
use serde_json::{Map, Value, json};
use std::{collections::BTreeSet, num::NonZeroU32};
use support::{counter, families, geometry, limit, page_dumps, page_examples, pages};

mod support;

/// The single JSON example on one daemon wire page.
fn example(page: &str) -> String {
    let mut blocks = page_examples(page);
    assert_eq!(blocks.len(), 1, "{page} publishes one JSON example");
    blocks.remove(0)
}

fn request(code: u8, page: &str) -> (Request, Frame) {
    let frame = Frame::new(code, example(page).into_bytes());
    (Request::from_frame(&frame).unwrap(), frame)
}

/// DOC-01: Open `0x30`, Leave `0x37` and Kill `0x38`.
#[test]
fn published_request_examples_round_trip_byte_identically() {
    let (open, frame) = request(0x30, "request-open.md");
    let expected = Open {
        request_id: counter(1),
        revision: 1,
        capabilities: Requested::new(
            families(&["status-v1"]),
            families(&["title-v1", "notification-v1", "progress-v1", "focus-v1"]),
        )
        .unwrap(),
        mode: Mode::Monitor,
    };
    assert_eq!(open, Request::Open(expected.clone()));
    assert_eq!(expected.to_frame(), frame);

    let (leave, frame) = request(0x37, "request-leave.md");
    let expected = Leave {
        request_id: counter(8),
    };
    assert_eq!(leave, Request::Leave(expected.clone()));
    assert_eq!(expected.to_frame(), frame);

    let (kill, frame) = request(0x38, "request-kill.md");
    let expected = Kill {
        request_id: counter(1),
        revision: 1,
        required_capabilities: families(&[]),
    };
    assert_eq!(kill, Request::Kill(expected.clone()));
    assert_eq!(expected.to_frame(), frame);
}

/// DOC-01: Resize `0x32`, Acquire `0x33`, Ready `0x34`, Set viewing `0x35`
/// and Acknowledge `0x36`.
#[test]
fn published_control_request_examples_round_trip_byte_identically() {
    let (resize, frame) = request(0x32, "request-resize.md");
    let expected = Resize {
        request_id: counter(3),
        period: counter(1),
        restoration_id: counter(1),
        geometry: geometry(100, 30),
    };
    assert_eq!(resize, Request::Resize(expected));
    assert_eq!(expected.to_frame(), frame);

    let (acquire, frame) = request(0x33, "request-acquire.md");
    let expected = Acquire {
        request_id: counter(4),
    };
    assert_eq!(acquire, Request::Acquire(expected));
    assert_eq!(expected.to_frame(), frame);

    let (ready, frame) = request(0x34, "request-ready.md");
    let expected = Ready {
        request_id: counter(5),
        period: counter(1),
        restoration_id: counter(1),
    };
    assert_eq!(ready, Request::Ready(expected));
    assert_eq!(expected.to_frame(), frame);

    let (viewing, frame) = request(0x35, "request-viewing.md");
    let expected = SetViewing {
        request_id: counter(6),
        period: counter(1),
        restoration_id: counter(1),
        viewing: true,
    };
    assert_eq!(viewing, Request::SetViewing(expected));
    assert_eq!(expected.to_frame(), frame);

    let (acknowledge, frame) = request(0x36, "request-acknowledge.md");
    let expected = Acknowledge {
        request_id: counter(7),
        period: counter(1),
        restoration_id: counter(1),
    };
    assert_eq!(acknowledge, Request::Acknowledge(expected));
    assert_eq!(expected.to_frame(), frame);
}

/// DOC-01: Focus `0x39` and its accepted and rejected Responses.
#[test]
fn published_focus_examples_round_trip_byte_identically() {
    let [focus, accepted, rejected]: [String; 3] =
        page_examples("request-focus.md").try_into().unwrap();

    let frame = Frame::new(0x39, focus.into_bytes());
    let expected = Focus {
        request_id: counter(8),
        period: counter(2),
        restoration_id: counter(3),
        focused: false,
    };
    assert_eq!(Request::from_frame(&frame), Ok(Request::Focus(expected)));
    assert_eq!(expected.to_frame(), frame);

    let frame = Frame::new(0x40, accepted.into_bytes());
    let expected = Response {
        request_id: counter(8),
        outcome: Outcome::Ok(Success::Focus {
            changed: true,
            focused: false,
            accepted_bytes: FocusBytes::None,
        }),
    };
    assert_eq!(
        ServerMessage::from_frame(&frame),
        Ok(ServerMessage::Response(expected.clone()))
    );
    assert_eq!(expected.to_frame(), frame);

    let frame = Frame::new(0x40, rejected.into_bytes());
    let expected = Response {
        request_id: counter(8),
        outcome: Outcome::Rejected(Rejection {
            error: Error {
                code: OperationalCode::UnsupportedCapability.into(),
                message: "focus-v1 was not selected".into(),
            },
            known_final: None,
        }),
    };
    assert_eq!(
        ServerMessage::from_frame(&frame),
        Ok(ServerMessage::Response(expected.clone()))
    );
    assert_eq!(expected.to_frame(), frame);
}

fn server(code: u8, page: &str) -> (ServerMessage, Frame) {
    let frame = Frame::new(code, example(page).into_bytes());
    (ServerMessage::from_frame(&frame).unwrap(), frame)
}

fn status(state: StatusState, changed_at: &str, episode: Option<u64>) -> Status {
    Status::new(state, changed_at.parse().unwrap(), episode.map(counter)).unwrap()
}

/// DOC-01: Response `0x40` and Fatal error `0x4d`.
#[test]
fn published_response_and_fatal_error_examples_round_trip_byte_identically() {
    let (response, frame) = server(0x40, "response.md");
    let expected = Response {
        request_id: counter(7),
        outcome: Outcome::Ok(Success::Acknowledgement { changed: false }),
    };
    assert_eq!(response, ServerMessage::Response(expected.clone()));
    assert_eq!(expected.to_frame(), frame);

    let (fatal, frame) = server(0x4d, "event-fatal-error.md");
    let expected = FatalError(Error {
        code: FatalCode::InvalidRequestId.into(),
        message: "Request ID did not increase".into(),
    });
    assert_eq!(fatal, ServerMessage::FatalError(expected.clone()));
    assert_eq!(expected.to_frame(), frame);
}

/// DOC-01: Initial metadata `0x41`, Title changed `0x42`, Status changed
/// `0x43`, Notification `0x44`, Progress changed `0x45` and Command final `0x4c`.
#[test]
fn published_core_metadata_examples_round_trip_byte_identically() {
    let (metadata, frame) = server(0x41, "event-initial-metadata.md");
    let expected = InitialMetadata::new(
        Available::Available(None),
        Available::Available(status(
            StatusState::NeedsAttention,
            "2026-11-06T12:00:00.000Z",
            Some(1),
        )),
        Available::Unavailable,
        None,
    )
    .unwrap();
    assert_eq!(metadata, ServerMessage::InitialMetadata(expected.clone()));
    assert_eq!(expected.to_frame(), frame);

    let (title, frame) = server(0x42, "event-title-changed.md");
    let expected = TitleChanged {
        title: Some("Build — café".into()),
    };
    assert_eq!(title, ServerMessage::TitleChanged(expected.clone()));
    assert_eq!(expected.to_frame(), frame);

    let (changed, frame) = server(0x43, "event-status-changed.md");
    let expected = StatusChanged::new(status(
        StatusState::Acknowledged,
        "2026-11-06T12:00:01.000Z",
        Some(1),
    ))
    .unwrap();
    assert_eq!(changed, ServerMessage::StatusChanged(expected.clone()));
    assert_eq!(expected.to_frame(), frame);

    let (notification, frame) = server(0x44, "event-notification.md");
    let expected = Notification {
        source: NotificationSource::Osc777,
        source_id: None,
        title: Some(String::new()),
        body: Some("Build ready; inspect results".into()),
        episode_id: Some(counter(2)),
    };
    assert_eq!(notification, ServerMessage::Notification(expected.clone()));
    assert_eq!(expected.to_frame(), frame);

    let (progress, frame) = server(0x45, "event-progress-changed.md");
    let expected = ProgressChanged {
        progress: Progress::new(ProgressState::Error, None).unwrap(),
    };
    assert_eq!(progress, ServerMessage::ProgressChanged(expected));
    assert_eq!(expected.to_frame(), frame);

    let (command_final, frame) = server(0x4c, "event-command-final.md");
    let expected = CommandFinal::new(
        CommandResult::Signal {
            number: NonZeroU32::new(15).unwrap(),
            name: Some("SIGTERM".into()),
        },
        Some(status(
            StatusState::Failed,
            "2026-11-06T12:00:02.000Z",
            None,
        )),
    )
    .unwrap();
    assert_eq!(command_final, ServerMessage::CommandFinal(expected.clone()));
    assert_eq!(expected.to_frame(), frame);
}

/// DOC-01: Control granted `0x46`, Control revoked `0x47`, Restoration begin
/// `0x48`, Restoration complete `0x4a` and Geometry `0x4b`.
#[test]
fn published_control_and_restoration_examples_round_trip_byte_identically() {
    let (granted, frame) = server(0x46, "event-control-granted.md");
    let expected = ControlGranted {
        period: counter(2),
        restoration_id: counter(3),
        geometry: geometry(80, 24),
        reason: GrantReason::Successor,
    };
    assert_eq!(granted, ServerMessage::ControlGranted(expected));
    assert_eq!(expected.to_frame(), frame);

    let (revoked, frame) = server(0x47, "event-control-revoked.md");
    let expected = ControlRevoked {
        period: counter(1),
        restoration_id: counter(1),
        reason: RevocationReason::Takeover,
    };
    assert_eq!(revoked, ServerMessage::ControlRevoked(expected));
    assert_eq!(expected.to_frame(), frame);

    let (begin, frame) = server(0x48, "event-restoration-begin.md");
    let expected = RestorationBegin {
        period: counter(2),
        restoration_id: counter(3),
        geometry: geometry(80, 24),
    };
    assert_eq!(begin, ServerMessage::RestorationBegin(expected));
    assert_eq!(expected.to_frame(), frame);

    let (complete, frame) = server(0x4a, "event-restoration-complete.md");
    let expected = RestorationComplete {
        period: counter(2),
        restoration_id: counter(3),
    };
    assert_eq!(complete, ServerMessage::RestorationComplete(expected));
    assert_eq!(expected.to_frame(), frame);

    let (actual, frame) = server(0x4b, "event-geometry.md");
    let expected = ActualGeometry {
        period: counter(2),
        geometry: geometry(100, 30),
    };
    assert_eq!(actual, ServerMessage::Geometry(expected));
    assert_eq!(expected.to_frame(), frame);
}

/// The single hex dump on one daemon wire page, read as one whole frame.
fn dumped(page: &str) -> (Frame, Vec<u8>) {
    let [bytes]: [Vec<u8>; 1] = page_dumps(page).try_into().unwrap();
    let mut reader = bytes.as_slice();
    let frame = read_frame(&mut reader, limit(1_048_576)).unwrap().unwrap();
    assert!(reader.is_empty(), "{page} shows exactly one frame");
    (frame, bytes)
}

fn written(frame: &Frame) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, frame, limit(1_048_576)).unwrap();
    bytes
}

/// DOC-01 / COM-04: Input `0x31` and Display `0x49` publish whole frames as
/// hex dumps that carry the payload `00 ff 41`. Each decodes to its
/// documented counters and exact payload and re-encodes to the same bytes.
#[test]
fn published_binary_examples_round_trip_byte_identically() {
    let (frame, bytes) = dumped("request-input.md");
    let expected = Input {
        request_id: counter(2),
        period: counter(1),
        restoration_id: counter(1),
        payload: vec![0x00, 0xff, 0x41],
    };
    assert_eq!(
        Request::from_frame(&frame),
        Ok(Request::Input(expected.clone()))
    );
    assert_eq!(written(&expected.to_frame()), bytes);

    let (frame, bytes) = dumped("event-display.md");
    let expected = Display {
        period: counter(2),
        restoration_id: counter(3),
        payload: vec![0x00, 0xff, 0x41],
    };
    assert_eq!(
        ServerMessage::from_frame(&frame),
        Ok(ServerMessage::Display(expected.clone()))
    );
    assert_eq!(written(&expected.to_frame()), bytes);
}

fn hello(body: &str) -> (HelloReply, HelloExtensions) {
    let frame = protocol::Frame::new(0x01, body.as_bytes().to_vec()).unwrap();
    let protocol::ServerMessage::Hello(reply) = protocol::ServerMessage::from_frame(frame).unwrap()
    else {
        panic!("{body} is a hello reply");
    };
    (reply, HelloExtensions::from_body(body.as_bytes()))
}

/// DOC-01 / COM-01: the frozen reply, the current advertisement and the
/// revision-1 advertisement each decode to their documented values and
/// re-encode byte-identically.
#[test]
fn published_hello_advertisement_examples_round_trip_byte_identically() {
    let [frozen, current, revision1]: [String; 3] =
        page_examples("hello-reply.md").try_into().unwrap();
    let expected = HelloReply {
        session_id: "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
        climon2_version: "1.2.3".parse().unwrap(),
        protocol: 1,
    };

    let (reply, extensions) = hello(&frozen);
    assert_eq!(reply, expected);
    assert_eq!(
        extensions,
        HelloExtensions {
            capabilities: Advertised::Absent,
            state: Advertised::Absent,
            daemon_wire_v1: Advertised::Absent,
        }
    );
    let encoded = protocol::ServerMessage::Hello(reply).to_frame().unwrap();
    assert_eq!(encoded.body(), frozen.as_bytes());

    let (reply, extensions) = hello(&current);
    let state = SessionState::new(Lifecycle::Attached, 2, true).unwrap();
    assert_eq!(reply, expected);
    let capabilities = extensions.capabilities.valid().unwrap();
    assert!(capabilities.supports_attach() && capabilities.supports_kill());
    assert!(capabilities.supports_state());
    assert_eq!(extensions.state, Advertised::Valid(state));
    assert_eq!(extensions.daemon_wire_v1, Advertised::Absent);
    assert_eq!(advertise(&reply, state).unwrap().body(), current.as_bytes());

    let (reply, extensions) = hello(&revision1);
    let advertisement = Advertisement::new(
        [
            Family::MonitorV1,
            Family::TitleV1,
            Family::StatusV1,
            Family::NotificationV1,
            Family::ProgressV1,
            Family::FocusV1,
        ],
        FrameLimits {
            max_request_frame_bytes: limit(1_048_576),
            max_server_frame_bytes: limit(1_048_576),
        },
    );
    assert_eq!(reply, expected);
    let capabilities = extensions.capabilities.valid().unwrap();
    for capability in Capability::ADVERTISED {
        assert!(capabilities.contains(capability), "{capability:?}");
    }
    assert_eq!(extensions.state, Advertised::Valid(state));
    assert_eq!(
        extensions.daemon_wire_v1,
        Advertised::Valid(advertisement.clone())
    );
    assert_eq!(
        advertise_revision1(&reply, state, &advertisement)
            .unwrap()
            .body(),
        revision1.as_bytes()
    );
}

/// The direction that receives `code` decodes it: request types go to the
/// session daemon, all others to the client.
fn decode(code: u8, body: &str) -> Result<(), FatalCode> {
    let frame = Frame::new(code, body.as_bytes().to_vec());
    match code {
        0x30..=0x39 => Request::from_frame(&frame).map(drop),
        _ => ServerMessage::from_frame(&frame).map(drop),
    }
}

fn ok(result: &str) -> String {
    format!(r#"{{"request_id":"1","outcome":"ok","result":{result},"error":null}}"#)
}

/// Every daemon wire page that publishes JSON examples, with the type of each
/// example in page order. The frozen hello pages have their own tests.
const JSON_PAGES: [(&str, &[u8]); 22] = [
    ("request-open.md", &[0x30]),
    ("request-resize.md", &[0x32]),
    ("request-acquire.md", &[0x33]),
    ("request-ready.md", &[0x34]),
    ("request-viewing.md", &[0x35]),
    ("request-acknowledge.md", &[0x36]),
    ("request-leave.md", &[0x37]),
    ("request-kill.md", &[0x38]),
    ("request-focus.md", &[0x39, 0x40, 0x40]),
    ("response.md", &[0x40]),
    ("event-initial-metadata.md", &[0x41]),
    ("event-title-changed.md", &[0x42]),
    ("event-status-changed.md", &[0x43]),
    ("event-notification.md", &[0x44]),
    ("event-progress-changed.md", &[0x45]),
    ("event-control-granted.md", &[0x46]),
    ("event-control-revoked.md", &[0x47]),
    ("event-restoration-begin.md", &[0x48]),
    ("event-restoration-complete.md", &[0x4a]),
    ("event-geometry.md", &[0x4b]),
    ("event-command-final.md", &[0x4c]),
    ("event-fatal-error.md", &[0x4d]),
];

/// DOC-01: every example on every revision-1 page is exercised: its JSON
/// examples through the sweep table, and its hex dumps, which only Input and
/// Display publish, through the binary round trip. The frozen hello pages
/// have their own tests.
#[test]
fn every_revision1_page_example_is_exercised() {
    let mut dumps = BTreeSet::new();
    for page in pages() {
        if page.starts_with("hello-") {
            continue;
        }
        let listed = JSON_PAGES
            .iter()
            .find(|(listed, _)| *listed == page)
            .map_or(0, |(_, codes)| codes.len());
        assert_eq!(page_examples(&page).len(), listed, "{page}");
        if !page_dumps(&page).is_empty() {
            dumps.insert(page);
        }
    }
    assert_eq!(
        dumps,
        BTreeSet::from(["event-display.md".to_owned(), "request-input.md".to_owned()])
    );
}

/// Valid JSON records: every published example, then each documented shape
/// that no page shows.
fn records() -> Vec<(u8, String)> {
    let published = JSON_PAGES.iter().flat_map(|(page, codes)| {
        let examples = page_examples(page);
        assert_eq!(examples.len(), codes.len(), "{page}");
        codes.iter().copied().zip(examples)
    });
    let exit = r#"{"kind":"exit","code":0}"#;
    let unpublished = [
        (
            0x30,
            r#"{"request_id":"2","revision":1,"required_capabilities":["status-v1"],"optional_capabilities":[],"mode":"interactive","geometry":{"cols":80,"rows":24}}"#.to_owned(),
        ),
        (
            0x40,
            ok(
                r#"{"kind":"opened","connection_id":"550e8400-e29b-41d4-a716-446655440000","selected_capabilities":["status-v1"]}"#,
            ),
        ),
        (0x40, ok(r#"{"kind":"input_accepted","accepted_bytes":3}"#)),
        (0x40, ok(r#"{"kind":"resize_accepted"}"#)),
        (
            0x40,
            ok(r#"{"kind":"acquired","changed":true,"period":"2","restoration_id":"3"}"#),
        ),
        (
            0x40,
            ok(r#"{"kind":"ready","changed":true,"period":"2","restoration_id":"3"}"#),
        ),
        (0x40, ok(r#"{"kind":"viewing","changed":true,"viewing":true}"#)),
        (0x40, ok(r#"{"kind":"left","attachment_detached":true}"#)),
        (
            0x40,
            ok(&format!(
                r#"{{"kind":"termination","command_result":{exit},"cleanup":"complete"}}"#
            )),
        ),
        (
            0x40,
            ok(r#"{"kind":"focus","changed":true,"focused":true,"accepted_bytes":3}"#),
        ),
        (
            0x40,
            format!(
                r#"{{"request_id":"1","outcome":"rejected","result":{{"kind":"known_final","command_result":{exit}}},"error":{{"code":"session_terminating","message":"ended"}}}}"#
            ),
        ),
        (
            0x40,
            r#"{"request_id":"1","outcome":"failed","result":{"kind":"termination","command_result":{"kind":"signal","number":9,"name":null},"cleanup":"incomplete"},"error":{"code":"cleanup_failed","message":"busy"}}"#.to_owned(),
        ),
        (
            0x40,
            r#"{"request_id":"1","outcome":"indeterminate","result":{"kind":"focus","changed":false,"focused":true,"accepted_bytes":3},"error":{"code":"io_error","message":"lost"}}"#.to_owned(),
        ),
        (
            0x41,
            format!(
                r#"{{"title":{{"available":true,"value":"vim"}},"status":{{"available":true,"value":{{"state":"completed","changed_at":"2026-11-06T12:00:00.000Z","episode_id":null}}}},"progress":{{"available":true,"value":{{"state":"normal","percent":50}}}},"command_result":{exit}}}"#
            ),
        ),
        (
            0x4c,
            format!(r#"{{"command_result":{exit},"status":null}}"#),
        ),
    ];
    published.chain(unpublished).collect()
}

/// JSON pointers to `value` and every object nested inside it.
fn objects(value: &Value, pointer: String) -> Vec<String> {
    let children: Vec<(String, &Value)> = match value {
        Value::Object(members) => members
            .iter()
            .map(|(key, member)| (format!("{pointer}/{key}"), member))
            .collect(),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| (format!("{pointer}/{index}"), item))
            .collect(),
        _ => Vec::new(),
    };
    let own = value.is_object().then_some(pointer);
    own.into_iter()
        .chain(
            children
                .into_iter()
                .flat_map(|(pointer, child)| objects(child, pointer)),
        )
        .collect()
}

/// `record` as JSON text after `edit` changes the object at `pointer`.
fn edited(record: &Value, pointer: &str, edit: impl FnOnce(&mut Map<String, Value>)) -> String {
    let mut copy = record.clone();
    edit(copy.pointer_mut(pointer).unwrap().as_object_mut().unwrap());
    serde_json::to_string(&copy).unwrap()
}

/// `record` with the member `key` of the object at `pointer` written twice.
/// JSON values cannot hold a repeated member, so a placeholder key is renamed
/// in the text.
fn repeated(record: &Value, pointer: &str, key: &str) -> String {
    edited(record, pointer, |object| {
        let twin = object[key].clone();
        object.insert("\u{1}".into(), twin);
    })
    .replace(r#""\u0001""#, &format!(r#""{key}""#))
}

/// COM-02 (codec case): every member of every JSON record, at every depth,
/// is required even when null, appears once and is documented. Removing or
/// repeating any member, or adding one to any object, is `invalid_record`.
#[test]
fn every_member_of_every_json_record_is_required_unique_and_documented() {
    for (code, body) in records() {
        assert_eq!(decode(code, &body), Ok(()), "{body}");
        let record: Value = serde_json::from_str(&body).unwrap();
        for pointer in objects(&record, String::new()) {
            let mut variants = vec![edited(&record, &pointer, |object| {
                object.insert("undocumented".into(), Value::Null);
            })];
            let keys = record
                .pointer(&pointer)
                .unwrap()
                .as_object()
                .unwrap()
                .keys();
            for key in keys {
                variants.push(edited(&record, &pointer, |object| {
                    object.remove(key);
                }));
                variants.push(repeated(&record, &pointer, key));
            }
            for variant in variants {
                assert_eq!(
                    decode(code, &variant),
                    Err(FatalCode::InvalidRecord),
                    "{variant}"
                );
            }
        }
    }
}

/// COM-02 (codec case): every member written as a JSON string, at every
/// depth, accepts only a string. A one-member object naming the value, the
/// form serde's derived enums would also take (`{"monitor":null}`), is
/// `invalid_record`. Request IDs are swept with the other request-ID cases.
#[test]
fn string_members_accept_only_json_strings() {
    for (code, body) in records() {
        let record: Value = serde_json::from_str(&body).unwrap();
        for pointer in objects(&record, String::new()) {
            let members = record.pointer(&pointer).unwrap().as_object().unwrap();
            for (key, value) in members {
                let Value::String(text) = value else { continue };
                if pointer.is_empty() && key == "request_id" {
                    continue;
                }
                let variant = edited(&record, &pointer, |object| {
                    object.insert(
                        key.clone(),
                        Value::Object(Map::from_iter([(text.clone(), Value::Null)])),
                    );
                });
                assert_eq!(
                    decode(code, &variant),
                    Err(FatalCode::InvalidRecord),
                    "{variant}"
                );
            }
        }
    }
}

/// Every documented `kind`: the success results, a rejection's known final
/// result, a Kill's partial termination and the command results.
const KINDS: [&str; 13] = [
    "acknowledgement",
    "acquired",
    "exit",
    "focus",
    "input_accepted",
    "known_final",
    "left",
    "opened",
    "ready",
    "resize_accepted",
    "signal",
    "termination",
    "viewing",
];

/// COM-02 (codec case): a result's `kind`, and a command result's, is only
/// its JSON string name. An integer, which serde's derived identifiers also
/// take as a variant index, is `invalid_record`, for every index that a
/// documented kind could have. Every documented kind is swept: the result
/// of a Response and the command result inside one, and the command results
/// of Initial metadata and Command final.
#[test]
fn every_kind_is_only_its_json_string_name() {
    let mut names = BTreeSet::new();
    let mut tagged = BTreeSet::new();
    for (code, body) in records() {
        let record: Value = serde_json::from_str(&body).unwrap();
        for pointer in objects(&record, String::new()) {
            let Some(Value::String(name)) = record.pointer(&pointer).unwrap().get("kind") else {
                continue;
            };
            names.insert(name.clone());
            tagged.insert((code, pointer.clone()));
            for index in 0..KINDS.len() {
                let variant = edited(&record, &pointer, |object| {
                    object.insert("kind".into(), json!(index));
                });
                assert_eq!(
                    decode(code, &variant),
                    Err(FatalCode::InvalidRecord),
                    "{variant}"
                );
            }
        }
    }
    assert_eq!(names, BTreeSet::from(KINDS.map(str::to_owned)));
    assert_eq!(
        tagged,
        BTreeSet::from([
            (0x40, "/result".to_owned()),
            (0x40, "/result/command_result".to_owned()),
            (0x41, "/command_result".to_owned()),
            (0x4c, "/command_result".to_owned()),
        ]),
        "a Response's result and the command result inside one, Initial metadata's and \
         Command final's"
    );
}

/// COM-02 (codec case): every request and the Response carry a `request_id`,
/// so one that is malformed, just outside the counter range at either end or
/// not a string, is `invalid_request_id`, while the largest counter decodes.
/// No other record has one: there it is an undocumented member, whatever its
/// value.
#[test]
fn only_requests_and_responses_carry_request_ids() {
    let mut correlated = BTreeSet::new();
    for (code, body) in records() {
        let record: Value = serde_json::from_str(&body).unwrap();
        let with_id = |id: Value| {
            edited(&record, "", |object| {
                object.insert("request_id".into(), id);
            })
        };
        if record.get("request_id").is_some() {
            correlated.insert(code);
            let largest = with_id(json!("18446744073709551615"));
            assert_eq!(decode(code, &largest), Ok(()), "{largest}");
            for id in [
                json!("0"),
                json!("18446744073709551616"),
                json!(1),
                json!({"1": null}),
                Value::Null,
            ] {
                let variant = with_id(id);
                assert_eq!(
                    decode(code, &variant),
                    Err(FatalCode::InvalidRequestId),
                    "{variant}"
                );
            }
        } else {
            for id in [json!("1"), json!("0")] {
                let variant = with_id(id);
                assert_eq!(
                    decode(code, &variant),
                    Err(FatalCode::InvalidRecord),
                    "{variant}"
                );
            }
        }
    }
    assert_eq!(
        correlated,
        BTreeSet::from([0x30, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x40]),
        "every JSON request and the Response"
    );
}

/// COM-02 (codec case): every control period and restoration ID, wherever a
/// record carries one, is a counter: "1" and the largest decode, while "0",
/// one past the largest, a number and null are `invalid_record`.
#[test]
fn every_period_and_restoration_id_is_a_counter_at_both_bounds() {
    let mut carriers = BTreeSet::new();
    for (code, body) in records() {
        let record: Value = serde_json::from_str(&body).unwrap();
        for pointer in objects(&record, String::new()) {
            for key in ["period", "restoration_id"] {
                if record.pointer(&pointer).unwrap().get(key).is_none() {
                    continue;
                }
                carriers.insert(code);
                let with = |value: Value| {
                    edited(&record, &pointer, |object| {
                        object.insert(key.into(), value);
                    })
                };
                for valid in ["1", "18446744073709551615"] {
                    let variant = with(json!(valid));
                    assert_eq!(decode(code, &variant), Ok(()), "{variant}");
                }
                for invalid in [
                    json!("0"),
                    json!("18446744073709551616"),
                    json!(1),
                    Value::Null,
                ] {
                    let variant = with(invalid);
                    assert_eq!(
                        decode(code, &variant),
                        Err(FatalCode::InvalidRecord),
                        "{variant}"
                    );
                }
            }
        }
    }
    assert_eq!(
        carriers,
        BTreeSet::from([
            0x32, 0x34, 0x35, 0x36, 0x39, 0x40, 0x46, 0x47, 0x48, 0x4a, 0x4b
        ]),
        "Resize, Ready, Set viewing, Acknowledge, Focus, the Acquire and Ready results, \
         Control granted and revoked, Restoration begin and complete, and Geometry"
    );
}

/// COM-02 (codec case): every geometry's columns and rows are positive
/// unsigned 32-bit integer tokens: 1 and 4294967295 decode, while 0 and
/// 4294967296 are `invalid_record`.
#[test]
fn every_geometry_has_positive_u32_columns_and_rows_at_both_bounds() {
    let mut carriers = BTreeSet::new();
    for (code, body) in records() {
        let record: Value = serde_json::from_str(&body).unwrap();
        for pointer in objects(&record, String::new()) {
            if record.pointer(&pointer).unwrap().get("cols").is_none() {
                continue;
            }
            carriers.insert(code);
            for key in ["cols", "rows"] {
                let with = |value: u64| {
                    edited(&record, &pointer, |object| {
                        object.insert(key.into(), json!(value));
                    })
                };
                for valid in [1, 4_294_967_295] {
                    let variant = with(valid);
                    assert_eq!(decode(code, &variant), Ok(()), "{variant}");
                }
                for invalid in [0, 4_294_967_296] {
                    let variant = with(invalid);
                    assert_eq!(
                        decode(code, &variant),
                        Err(FatalCode::InvalidRecord),
                        "{variant}"
                    );
                }
            }
        }
    }
    assert_eq!(
        carriers,
        BTreeSet::from([0x30, 0x32, 0x46, 0x48, 0x4b]),
        "interactive Open, Resize, Control granted, Restoration begin and Geometry"
    );
}
