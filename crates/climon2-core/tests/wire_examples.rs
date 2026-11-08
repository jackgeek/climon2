//! DOC-01 (Contract case for the part-1 daemon wire pages): every published
//! example decodes to the values its page documents and re-encodes to the
//! same bytes. The examples are read from the pages themselves, so the
//! published text and the codec cannot drift apart. The same examples seed
//! the member strictness sweep (COM-02).
use climon2_core::wire::{
    Available, CommandFinal, CommandResult, ControlRecord, Counter, Error, Families, FatalCode,
    FatalError, Frame, InitialMetadata, Kill, Leave, Mode, Notification, NotificationSource, Open,
    Outcome, Progress, ProgressChanged, ProgressState, Request, Requested, Response, ServerMessage,
    Status, StatusChanged, StatusState, Success, TitleChanged,
};
use climon2_core::{
    hello::{
        Advertised, Capabilities, Capability, HelloExtensions, Lifecycle, SessionState, advertise,
        advertise_with,
    },
    protocol::{self, HelloReply},
    wire::{Advertisement, Family, FrameLimit, FrameLimits},
};
use serde_json::{Map, Value};
use std::num::NonZeroU32;
use std::{fs, path::Path};

/// Every JSON example on one daemon wire page, in page order.
fn examples(page: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/protocol/daemon-client")
        .join(page);
    let text = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    text.split("```json\n")
        .skip(1)
        .map(|block| block.split_once("\n```").expect("closed JSON example").0)
        .map(str::to_owned)
        .collect()
}

/// The single JSON example on one daemon wire page.
fn example(page: &str) -> String {
    let mut blocks = examples(page);
    assert_eq!(blocks.len(), 1, "{page} publishes one JSON example");
    blocks.remove(0)
}

fn counter(value: u64) -> Counter {
    Counter::new(value).unwrap()
}

fn families(tokens: &[&str]) -> Families {
    Families::new(tokens.iter().map(|token| token.to_string()).collect()).unwrap()
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
fn published_session_metadata_examples_round_trip_byte_identically() {
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
    let [frozen, current, revision1]: [String; 3] = examples("hello-reply.md").try_into().unwrap();
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
    let limit = FrameLimit::new(1_048_576).unwrap();
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
            max_request_frame_bytes: limit,
            max_server_frame_bytes: limit,
        },
    );
    assert_eq!(reply, expected);
    assert_eq!(
        extensions.capabilities,
        Advertised::Valid(Capabilities::default())
    );
    assert_eq!(extensions.state, Advertised::Absent);
    assert_eq!(
        extensions.daemon_wire_v1,
        Advertised::Valid(advertisement.clone())
    );
    let encoded: &[Capability] = &[];
    assert_eq!(
        advertise_with(&reply, encoded, None, Some(&advertisement))
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

/// Valid part-1 records: every published example, then each documented shape
/// that no page shows.
fn part1_records() -> Vec<(u8, String)> {
    let published = [
        (0x30, "request-open.md"),
        (0x37, "request-leave.md"),
        (0x38, "request-kill.md"),
        (0x40, "response.md"),
        (0x41, "event-initial-metadata.md"),
        (0x42, "event-title-changed.md"),
        (0x43, "event-status-changed.md"),
        (0x44, "event-notification.md"),
        (0x45, "event-progress-changed.md"),
        (0x4c, "event-command-final.md"),
        (0x4d, "event-fatal-error.md"),
    ];
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
    published
        .into_iter()
        .map(|(code, page)| (code, example(page)))
        .chain(unpublished)
        .collect()
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

/// COM-02 (codec case): every member of every part-1 record, at every depth,
/// is required even when null, appears once and is documented. Removing or
/// repeating any member, or adding one to any object, is `invalid_record`.
#[test]
fn every_member_of_every_part1_record_is_required_unique_and_documented() {
    for (code, body) in part1_records() {
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
