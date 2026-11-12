//! Daemon wire revision 1 JSON server messages (session daemon to client):
//! Response, Fatal error, the core metadata events and the control and
//! restoration events. Each test is the codec case of the matrix scenarios
//! it cites; the daemon and library cases come with their own tickets.
use climon2_core::wire::{
    Available, Cleanup, CommandFinal, CommandResult, ControlGranted, ControlRecord, ControlRevoked,
    Counter, Error, ErrorCode, Failure, Families, FatalCode, FocusBytes, Frame, GrantReason, Guid,
    InitialMetadata, KnownResult, OperationalCode, Outcome, PartialCleanup, PartialTermination,
    Percent, Progress, ProgressState, Rejection, Response, RevocationReason, ServerMessage, Status,
    StatusChanged, StatusState, Success, Utc,
};
use std::num::NonZeroU32;
use support::{counter, geometry};

mod support;

const RESPONSE: u8 = 0x40;
const INITIAL_METADATA: u8 = 0x41;
const TITLE_CHANGED: u8 = 0x42;
const STATUS_CHANGED: u8 = 0x43;
const NOTIFICATION: u8 = 0x44;
const PROGRESS_CHANGED: u8 = 0x45;
const CONTROL_GRANTED: u8 = 0x46;
const CONTROL_REVOKED: u8 = 0x47;
const COMMAND_FINAL: u8 = 0x4c;
const FATAL_ERROR: u8 = 0x4d;

fn server(code: u8, body: impl AsRef<[u8]>) -> Result<ServerMessage, FatalCode> {
    ServerMessage::from_frame(&Frame::new(code, body.as_ref().to_vec()))
}

/// Decodes `body`, checks it is the expected message and re-encodes it to the
/// same bytes.
fn round_trip(code: u8, body: &str) -> ServerMessage {
    let message = server(code, body).unwrap_or_else(|code| panic!("{body}: {code:?}"));
    assert_eq!(
        encoded(&message),
        Frame::new(code, body.as_bytes().to_vec())
    );
    message
}

fn encoded(message: &ServerMessage) -> Frame {
    match message {
        ServerMessage::Response(response) => response.to_frame(),
        ServerMessage::InitialMetadata(metadata) => metadata.to_frame(),
        ServerMessage::TitleChanged(title) => title.to_frame(),
        ServerMessage::StatusChanged(status) => status.to_frame(),
        ServerMessage::Notification(notification) => notification.to_frame(),
        ServerMessage::ProgressChanged(progress) => progress.to_frame(),
        ServerMessage::ControlGranted(grant) => grant.to_frame(),
        ServerMessage::ControlRevoked(revocation) => revocation.to_frame(),
        ServerMessage::RestorationBegin(begin) => begin.to_frame(),
        ServerMessage::Display(display) => display.to_frame(),
        ServerMessage::RestorationComplete(complete) => complete.to_frame(),
        ServerMessage::Geometry(geometry) => geometry.to_frame(),
        ServerMessage::CommandFinal(result) => result.to_frame(),
        ServerMessage::FatalError(error) => error.to_frame(),
    }
}

fn utc(text: &str) -> Utc {
    text.parse().unwrap()
}

const AT: &str = "2026-11-06T12:00:00.000Z";

fn status_json(state: &str, episode: &str) -> String {
    format!(r#"{{"state":"{state}","changed_at":"{AT}","episode_id":{episode}}}"#)
}

fn status(state: StatusState, episode: Option<u64>) -> Status {
    Status::new(state, utc(AT), episode.map(counter)).unwrap()
}

fn response(outcome: &str, result: &str, error: &str) -> String {
    format!(r#"{{"request_id":"7","outcome":"{outcome}","result":{result},"error":{error}}}"#)
}

const ERROR: &str = r#"{"code":"invalid_state","message":"not now"}"#;

fn exit(code: u32) -> CommandResult {
    CommandResult::Exit { code }
}

fn sigterm() -> CommandResult {
    CommandResult::Signal {
        number: NonZeroU32::new(15).unwrap(),
        name: Some("SIGTERM".into()),
    }
}

/// COM-02 (codec case): the client decodes only server types; a request type
/// is `wrong_direction` and a type outside both ranges is `unknown_message_type`.
#[test]
fn server_messages_are_identified_by_type_and_direction() {
    let title = br#"{"title":null}"#;
    assert!(server(TITLE_CHANGED, title).is_ok());
    for code in 0x30..=0x39 {
        assert_eq!(
            server(code, title),
            Err(FatalCode::WrongDirection),
            "{code:#x}"
        );
    }
    for code in [0x00, 0x01, 0x27, 0x2f, 0x3a, 0x3f, 0x4e, 0x7f, 0xff] {
        assert_eq!(
            server(code, title),
            Err(FatalCode::UnknownMessageType),
            "{code:#x}"
        );
    }
    assert_eq!(
        server(TITLE_CHANGED, b"{\"title\":\"\xff\"}"),
        Err(FatalCode::InvalidJson)
    );
    assert_eq!(
        server(TITLE_CHANGED, r#"[null]"#),
        Err(FatalCode::InvalidRecord)
    );
}

/// COM-03 (codec case): every documented success result decodes to its
/// operation's members and re-encodes byte-identically.
#[test]
fn every_documented_success_result_round_trips() {
    let guid = "550e8400-e29b-41d4-a716-446655440000";
    let complete = r#"{"kind":"exit","code":0}"#;
    for (result, expected) in [
        (
            format!(
                r#"{{"kind":"opened","connection_id":"{guid}","selected_capabilities":["status-v1","title-v1"]}}"#
            ),
            Success::Opened {
                connection_id: guid.parse().unwrap(),
                selected_capabilities: Families::new(vec!["status-v1".into(), "title-v1".into()])
                    .unwrap(),
            },
        ),
        (
            r#"{"kind":"input_accepted","accepted_bytes":1}"#.into(),
            Success::InputAccepted {
                accepted_bytes: NonZeroU32::MIN,
            },
        ),
        (
            r#"{"kind":"input_accepted","accepted_bytes":4294967295}"#.into(),
            Success::InputAccepted {
                accepted_bytes: NonZeroU32::MAX,
            },
        ),
        (
            r#"{"kind":"resize_accepted"}"#.into(),
            Success::ResizeAccepted {},
        ),
        (
            r#"{"kind":"acquired","changed":true,"period":"2","restoration_id":"3"}"#.into(),
            Success::Acquired {
                changed: true,
                period: counter(2),
                restoration_id: counter(3),
            },
        ),
        (
            r#"{"kind":"ready","changed":false,"period":"2","restoration_id":"3"}"#.into(),
            Success::Ready {
                changed: false,
                period: counter(2),
                restoration_id: counter(3),
            },
        ),
        (
            r#"{"kind":"viewing","changed":true,"viewing":false}"#.into(),
            Success::Viewing {
                changed: true,
                viewing: false,
            },
        ),
        (
            r#"{"kind":"acknowledgement","changed":true}"#.into(),
            Success::Acknowledgement { changed: true },
        ),
        (
            r#"{"kind":"left","attachment_detached":false}"#.into(),
            Success::Left {
                attachment_detached: false,
            },
        ),
        (
            format!(r#"{{"kind":"termination","command_result":{complete},"cleanup":"complete"}}"#),
            Success::Termination {
                command_result: exit(0),
                cleanup: Cleanup::Complete,
            },
        ),
        (
            r#"{"kind":"focus","changed":true,"focused":false,"accepted_bytes":0}"#.into(),
            Success::Focus {
                changed: true,
                focused: false,
                accepted_bytes: FocusBytes::None,
            },
        ),
        (
            r#"{"kind":"focus","changed":true,"focused":true,"accepted_bytes":3}"#.into(),
            Success::Focus {
                changed: true,
                focused: true,
                accepted_bytes: FocusBytes::Sequence,
            },
        ),
    ] {
        let body = response("ok", &result, "null");
        assert_eq!(
            round_trip(RESPONSE, &body),
            ServerMessage::Response(Response {
                request_id: counter(7),
                outcome: Outcome::Ok(expected),
            }),
            "{body}"
        );
    }
}

/// COM-02 (codec case): result members keep their documented types and bounds.
#[test]
fn success_results_reject_undocumented_shapes() {
    for result in [
        r#"{"kind":"input_accepted","accepted_bytes":0}"#,
        r#"{"kind":"input_accepted","accepted_bytes":4294967296}"#,
        r#"{"kind":"resize_accepted","changed":true}"#,
        r#"{"kind":"acquired","changed":true,"period":"0","restoration_id":"3"}"#,
        r#"{"kind":"acquired","changed":true,"period":"2"}"#,
        r#"{"kind":"viewing","changed":"true","viewing":false}"#,
        r#"{"kind":"acknowledgement","changed":true,"changed":true}"#,
        r#"{"kind":"left","attachment_detached":false,"kind":"left"}"#,
        r#"{"kind":"focus","changed":true,"focused":false,"accepted_bytes":1}"#,
        r#"{"kind":"focus","changed":true,"focused":false,"accepted_bytes":2}"#,
        r#"{"kind":"focus","changed":true,"focused":false,"accepted_bytes":4}"#,
        r#"{"kind":"Acknowledgement","changed":true}"#,
        r#"{"kind":"handshake"}"#,
        r#"{"changed":true}"#,
        r#"["acknowledgement",true]"#,
        r#""acknowledgement""#,
    ] {
        assert_eq!(
            server(RESPONSE, response("ok", result, "null")),
            Err(FatalCode::InvalidRecord),
            "{result}"
        );
    }
}

/// COM-02 (codec case): connection IDs are lowercase canonical hyphenated GUIDs.
#[test]
fn connection_ids_are_lowercase_canonical_guids() {
    let opened = |id: &str| {
        response(
            "ok",
            &format!(r#"{{"kind":"opened","connection_id":"{id}","selected_capabilities":[]}}"#),
            "null",
        )
    };
    for id in [
        "550e8400-e29b-41d4-a716-446655440000",
        "00000000-0000-0000-0000-000000000000",
        "ffffffff-ffff-ffff-ffff-ffffffffffff",
    ] {
        round_trip(RESPONSE, &opened(id));
        assert_eq!(id.parse::<Guid>().unwrap().as_str(), id);
    }
    for id in [
        "550E8400-E29B-41D4-A716-446655440000",
        "{550e8400-e29b-41d4-a716-446655440000}",
        "550e8400e29b41d4a716446655440000",
        "urn:uuid:550e8400-e29b-41d4-a716-446655440000",
        "550e8400-e29b-41d4-a716-44665544000",
        "550e8400-e29b-41d4-a716-4466554400000",
        "550e8400-e29b-41d4-a716-44665544000g",
        "",
    ] {
        assert_eq!(
            server(RESPONSE, opened(id)),
            Err(FatalCode::InvalidRecord),
            "{id}"
        );
        assert!(id.parse::<Guid>().is_err(), "{id}");
    }
}

/// COM-03 / COM-06 (codec case): each outcome pairs with an error and the
/// results it may carry. `ok` carries a success result and no error; every
/// other outcome carries an error. A rejection may carry only a known final
/// result. A failed or indeterminate outcome may expose what is known of the
/// operation, such as Focus's result or a Kill's partial termination, but a
/// confirmed termination is never a failure. Any other pairing is
/// `invalid_record`. The codec cannot know which operation a request ID
/// names, so matching a result to its request is left to the connection.
#[test]
fn responses_pair_each_outcome_with_its_result_and_error() {
    let focus = r#"{"kind":"focus","changed":true,"focused":false,"accepted_bytes":0}"#;
    let confirmed =
        r#"{"kind":"termination","command_result":{"kind":"exit","code":0},"cleanup":"complete"}"#;
    let partial = r#"{"kind":"termination","command_result":null,"cleanup":"incomplete"}"#;
    let known_final = r#"{"kind":"known_final","command_result":{"kind":"exit","code":3}}"#;
    let error = Error {
        code: OperationalCode::InvalidState.into(),
        message: "not now".into(),
    };
    let focused = Success::Focus {
        changed: true,
        focused: false,
        accepted_bytes: FocusBytes::None,
    };
    let termination = Success::Termination {
        command_result: exit(0),
        cleanup: Cleanup::Complete,
    };
    let incomplete = KnownResult::Termination(PartialTermination {
        command_result: None,
        cleanup: PartialCleanup::Incomplete,
    });
    let failure = |known: Option<KnownResult>| Failure::new(error.clone(), known).unwrap();
    let rejection = |known_final: Option<CommandResult>| Rejection {
        error: error.clone(),
        known_final,
    };
    let accepted = [
        ("ok", focus, "null", Outcome::Ok(focused.clone())),
        ("ok", confirmed, "null", Outcome::Ok(termination.clone())),
        (
            "rejected",
            "null",
            ERROR,
            Outcome::Rejected(rejection(None)),
        ),
        (
            "rejected",
            known_final,
            ERROR,
            Outcome::Rejected(rejection(Some(exit(3)))),
        ),
        ("failed", "null", ERROR, Outcome::Failed(failure(None))),
        (
            "failed",
            focus,
            ERROR,
            Outcome::Failed(failure(Some(KnownResult::Operation(focused.clone())))),
        ),
        (
            "failed",
            partial,
            ERROR,
            Outcome::Failed(failure(Some(incomplete.clone()))),
        ),
        (
            "indeterminate",
            "null",
            ERROR,
            Outcome::Indeterminate(failure(None)),
        ),
        (
            "indeterminate",
            focus,
            ERROR,
            Outcome::Indeterminate(failure(Some(KnownResult::Operation(focused)))),
        ),
        (
            "indeterminate",
            partial,
            ERROR,
            Outcome::Indeterminate(failure(Some(incomplete))),
        ),
    ];
    for outcome in ["ok", "rejected", "failed", "indeterminate"] {
        for result in ["null", focus, confirmed, partial, known_final] {
            for error in ["null", ERROR] {
                let body = response(outcome, result, error);
                let expected = accepted.iter().find(|(name, known, fault, _)| {
                    (*name, *known, *fault) == (outcome, result, error)
                });
                match expected {
                    Some((.., expected)) => {
                        let ServerMessage::Response(decoded) = round_trip(RESPONSE, &body) else {
                            panic!("{body} is a response");
                        };
                        assert_eq!(&decoded.outcome, expected, "{body}");
                    }
                    None => assert_eq!(
                        server(RESPONSE, &body),
                        Err(FatalCode::InvalidRecord),
                        "{body}"
                    ),
                }
            }
        }
    }
    assert_eq!(
        Failure::new(error.clone(), Some(KnownResult::Operation(termination))),
        None,
        "the daemon cannot encode a confirmed termination as a failure"
    );
    for outcome in ["failed", "indeterminate"] {
        for (command_result, command) in [
            ("null", None),
            (r#"{"kind":"exit","code":0}"#, Some(exit(0))),
        ] {
            for (cleanup, partial) in [
                ("incomplete", Some(PartialCleanup::Incomplete)),
                ("unknown", Some(PartialCleanup::Unknown)),
                ("complete", None),
            ] {
                let body = response(
                    outcome,
                    &format!(
                        r#"{{"kind":"termination","command_result":{command_result},"cleanup":"{cleanup}"}}"#
                    ),
                    r#"{"code":"cleanup_failed","message":""}"#,
                );
                let Some(cleanup) = partial else {
                    assert_eq!(
                        server(RESPONSE, &body),
                        Err(FatalCode::InvalidRecord),
                        "{body}"
                    );
                    continue;
                };
                let ServerMessage::Response(decoded) = round_trip(RESPONSE, &body) else {
                    panic!("{body} is a response");
                };
                let (Outcome::Failed(failure) | Outcome::Indeterminate(failure)) = decoded.outcome
                else {
                    panic!("{body} is neither failed nor indeterminate");
                };
                assert_eq!(
                    failure.known(),
                    Some(&KnownResult::Termination(PartialTermination {
                        command_result: command.clone(),
                        cleanup,
                    })),
                    "{body}"
                );
                assert_eq!(
                    failure.error().code.operational(),
                    Some(OperationalCode::CleanupFailed)
                );
            }
        }
    }
    for (result, valid) in [
        (
            r#"{"kind":"termination","command_result":{"kind":"exit","code":0},"cleanup":"complete"}"#,
            true,
        ),
        (
            r#"{"kind":"termination","command_result":null,"cleanup":"complete"}"#,
            false,
        ),
        (
            r#"{"kind":"termination","command_result":{"kind":"exit","code":0},"cleanup":"incomplete"}"#,
            false,
        ),
    ] {
        let body = response("ok", result, "null");
        assert_eq!(server(RESPONSE, &body).is_ok(), valid, "{body}");
    }
    let left = r#"{"kind":"left","attachment_detached":true}"#;
    for body in [
        response("OK", left, "null"),
        response("unknown", "null", ERROR),
        format!(r#"{{"request_id":"7","outcome":"ok","result":{left}}}"#),
        format!(r#"{{"request_id":"7","outcome":"rejected","error":{ERROR}}}"#),
        format!(r#"{{"request_id":"7","outcome":"ok","result":{left},"error":null,"note":null}}"#),
    ] {
        assert_eq!(
            server(RESPONSE, &body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
    assert_eq!(
        server(
            RESPONSE,
            response("ok", left, "null").replace(r#""7""#, r#""07""#)
        ),
        Err(FatalCode::InvalidRequestId)
    );
}

/// COM-03 (codec case): error codes are nonempty. Each documented operational
/// code is recognised, and an unknown code stays an explicit non-success.
#[test]
fn error_codes_are_nonempty_and_unknown_codes_stay_non_successes() {
    for (text, code) in [
        ("unsupported_revision", OperationalCode::UnsupportedRevision),
        (
            "unsupported_capability",
            OperationalCode::UnsupportedCapability,
        ),
        ("invalid_state", OperationalCode::InvalidState),
        ("invalid_argument", OperationalCode::InvalidArgument),
        ("invalid_geometry", OperationalCode::InvalidGeometry),
        ("not_attached", OperationalCode::NotAttached),
        ("not_controller", OperationalCode::NotController),
        ("stale_period", OperationalCode::StalePeriod),
        ("stale_restoration", OperationalCode::StaleRestoration),
        ("not_ready", OperationalCode::NotReady),
        ("not_viewing", OperationalCode::NotViewing),
        ("session_terminating", OperationalCode::SessionTerminating),
        ("resource_limit", OperationalCode::ResourceLimit),
        ("io_error", OperationalCode::IoError),
        ("cleanup_failed", OperationalCode::CleanupFailed),
        ("internal_error", OperationalCode::InternalError),
    ] {
        let body = response(
            "rejected",
            "null",
            &format!(r#"{{"code":"{text}","message":"m"}}"#),
        );
        let ServerMessage::Response(decoded) = round_trip(RESPONSE, &body) else {
            panic!("{body} is a response");
        };
        let Outcome::Rejected(failure) = decoded.outcome else {
            panic!("{body} is rejected");
        };
        assert_eq!(failure.error.code.operational(), Some(code), "{text}");
        assert_eq!(failure.error.code.as_str(), text);
        assert_eq!(ErrorCode::from(code).as_str(), text);
        assert_eq!(code.as_str(), text);
    }
    let body = response(
        "rejected",
        "null",
        r#"{"code":"future_reason","message":""}"#,
    );
    let ServerMessage::Response(decoded) = round_trip(RESPONSE, &body) else {
        panic!("{body} is a response");
    };
    let Outcome::Rejected(failure) = decoded.outcome else {
        panic!("an unknown code is still a non-success");
    };
    assert_eq!(failure.error.code.as_str(), "future_reason");
    assert_eq!(failure.error.code.operational(), None);
    assert_eq!(failure.error.code.fatal(), None);
    for error in [
        r#"{"code":"","message":"m"}"#,
        r#"{"code":null,"message":"m"}"#,
        r#"{"code":7,"message":"m"}"#,
        r#"{"code":"invalid_state"}"#,
        r#"{"code":"invalid_state","message":null}"#,
        r#"{"code":"invalid_state","message":"m","retry":false}"#,
        r#"["invalid_state","m"]"#,
    ] {
        assert_eq!(
            server(RESPONSE, response("rejected", "null", error)),
            Err(FatalCode::InvalidRecord),
            "{error}"
        );
    }
    assert!("".parse::<ErrorCode>().is_err());
}

/// COM-02 (codec case): Fatal error carries each connection-fatal code and
/// keeps unknown codes as explicit failures.
#[test]
fn fatal_errors_carry_each_connection_fatal_code() {
    for (text, code) in [
        ("invalid_frame_length", FatalCode::InvalidFrameLength),
        ("frame_too_large", FatalCode::FrameTooLarge),
        ("truncated_frame", FatalCode::TruncatedFrame),
        ("invalid_json", FatalCode::InvalidJson),
        ("invalid_record", FatalCode::InvalidRecord),
        ("invalid_request_id", FatalCode::InvalidRequestId),
        ("unknown_message_type", FatalCode::UnknownMessageType),
        ("wrong_direction", FatalCode::WrongDirection),
        ("internal_error", FatalCode::InternalError),
    ] {
        let body = format!(r#"{{"code":"{text}","message":"diagnostic"}}"#);
        let ServerMessage::FatalError(fatal) = round_trip(FATAL_ERROR, &body) else {
            panic!("{body} is a fatal error");
        };
        assert_eq!(fatal.0.code.fatal(), Some(code), "{text}");
        assert_eq!(code.as_str(), text);
        assert_eq!(ErrorCode::from(code).as_str(), text);
    }
    let ServerMessage::FatalError(fatal) =
        round_trip(FATAL_ERROR, r#"{"code":"future_fault","message":""}"#)
    else {
        panic!("an unknown fatal code is still a fatal error");
    };
    assert_eq!(fatal.0.code.fatal(), None);
    for body in [
        r#"{"code":"","message":""}"#,
        r#"{"code":"invalid_json"}"#,
        r#"{"code":"invalid_json","message":"","request_id":"1"}"#,
    ] {
        assert_eq!(
            server(FATAL_ERROR, body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
}

/// COM-02 / STA-07 (codec case): status change times are exact UTC
/// milliseconds on real calendar dates with seconds 00 through 59.
#[test]
fn utc_times_are_exact_calendar_milliseconds() {
    let status_changed = |at: &str| {
        format!(r#"{{"status":{{"state":"idle","changed_at":"{at}","episode_id":null}}}}"#)
    };
    for at in [
        "2026-11-06T12:00:00.000Z",
        "2026-12-31T23:59:59.999Z",
        "2024-02-29T00:00:00.000Z",
        "2026-02-28T00:00:00.000Z",
        "0000-01-01T00:00:00.000Z",
        "9999-12-31T23:59:59.999Z",
    ] {
        let ServerMessage::StatusChanged(changed) = round_trip(STATUS_CHANGED, &status_changed(at))
        else {
            panic!("{at} is a status change");
        };
        assert_eq!(changed.status().changed_at().as_str(), at);
    }
    for at in [
        "2026-11-06T12:00:60.000Z",
        "2016-12-31T23:59:60.000Z",
        "2026-11-06T12:60:00.000Z",
        "2026-11-06T24:00:00.000Z",
        "2026-02-29T00:00:00.000Z",
        "2026-04-31T00:00:00.000Z",
        "2026-13-01T00:00:00.000Z",
        "2026-00-01T00:00:00.000Z",
        "2026-01-00T00:00:00.000Z",
        "2026-11-06t12:00:00.000Z",
        "2026-11-06T12:00:00.000z",
        "2026-11-06 12:00:00.000Z",
        "2026-11-06T12:00:00Z",
        "2026-11-06T12:00:00.00Z",
        "2026-11-06T12:00:00.0000Z",
        "2026-11-06T12:00:00.000+00:00",
        "+2026-11-06T12:00:00.000Z",
        "-2026-11-06T12:00:00.000Z",
        "+026-11-06T12:00:00.000Z",
        "026-11-06T12:00:00.000Z",
        "10000-01-01T00:00:00.000Z",
        "2026-11-06T12:00:00.000Z ",
        "",
    ] {
        assert_eq!(
            server(STATUS_CHANGED, status_changed(at)),
            Err(FatalCode::InvalidRecord),
            "{at}"
        );
        assert!(at.parse::<Utc>().is_err(), "{at}");
    }
}

/// COM-02 / COM-06 / STA-07 (codec case): only the two attention states
/// carry a nonnull episode; Status changed never carries the final transition.
#[test]
fn status_episodes_belong_only_to_attention_states() {
    for (state, episode, expected) in [
        ("idle", "null", status(StatusState::Idle, None)),
        ("running", "null", status(StatusState::Running, None)),
        (
            "needs_attention",
            r#""1""#,
            status(StatusState::NeedsAttention, Some(1)),
        ),
        (
            "acknowledged",
            r#""18446744073709551615""#,
            status(StatusState::Acknowledged, Some(u64::MAX)),
        ),
    ] {
        let body = format!(r#"{{"status":{}}}"#, status_json(state, episode));
        let ServerMessage::StatusChanged(changed) = round_trip(STATUS_CHANGED, &body) else {
            panic!("{body} is a status change");
        };
        assert_eq!(changed.status(), &expected);
        assert_eq!(
            changed.status().episode().map(Counter::get),
            episode.trim_matches('"').parse().ok()
        );
        assert_eq!(
            StatusChanged::new(expected.clone()).unwrap().status(),
            &expected
        );
    }
    for (state, episode) in [
        ("idle", r#""1""#),
        ("running", r#""1""#),
        ("needs_attention", "null"),
        ("acknowledged", "null"),
        ("needs_attention", r#""0""#),
        ("needs_attention", "1"),
        ("completed", "null"),
        ("failed", "null"),
        ("Idle", "null"),
        ("working", "null"),
    ] {
        let body = format!(r#"{{"status":{}}}"#, status_json(state, episode));
        assert_eq!(
            server(STATUS_CHANGED, &body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
    for body in [
        format!(r#"{{"status":{{"state":"idle","changed_at":"{AT}"}}}}"#),
        format!(
            r#"{{"status":{{"state":"idle","changed_at":"{AT}","episode_id":null,"source":null}}}}"#
        ),
        format!(r#"{{"status":["idle","{AT}",null]}}"#),
        r#"{"status":null}"#.into(),
    ] {
        assert_eq!(
            server(STATUS_CHANGED, &body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
    assert_eq!(
        Status::new(StatusState::Idle, utc(AT), Some(counter(1))),
        None
    );
    assert_eq!(
        Status::new(StatusState::NeedsAttention, utc(AT), None),
        None
    );
    for terminal in [StatusState::Completed, StatusState::Failed] {
        assert_eq!(StatusChanged::new(status(terminal, None)), None);
    }
}

/// COM-02 (codec case): progress percent is an integer 0 through 100 that
/// normal requires, none and indeterminate forbid, and error and paused allow.
#[test]
fn progress_percent_matches_its_state() {
    for (state, percent, expected_state, expected_percent) in [
        ("none", "null", ProgressState::None, None),
        ("normal", "0", ProgressState::Normal, Some(0)),
        ("normal", "100", ProgressState::Normal, Some(100)),
        ("indeterminate", "null", ProgressState::Indeterminate, None),
        ("error", "null", ProgressState::Error, None),
        ("error", "42", ProgressState::Error, Some(42)),
        ("paused", "null", ProgressState::Paused, None),
        ("paused", "7", ProgressState::Paused, Some(7)),
    ] {
        let body = format!(r#"{{"progress":{{"state":"{state}","percent":{percent}}}}}"#);
        let ServerMessage::ProgressChanged(changed) = round_trip(PROGRESS_CHANGED, &body) else {
            panic!("{body} is a progress change");
        };
        assert_eq!(changed.progress.state(), expected_state, "{body}");
        assert_eq!(
            changed.progress.percent().map(|percent| percent.get()),
            expected_percent,
            "{body}"
        );
    }
    for (state, percent) in [
        ("normal", "null"),
        ("normal", "101"),
        ("error", "101"),
        ("none", "0"),
        ("indeterminate", "50"),
        ("paused", "-1"),
        ("paused", "50.0"),
        ("paused", r#""50""#),
        ("Normal", "50"),
        ("warning", "50"),
    ] {
        let body = format!(r#"{{"progress":{{"state":"{state}","percent":{percent}}}}}"#);
        assert_eq!(
            server(PROGRESS_CHANGED, &body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
    for body in [
        r#"{"progress":{"state":"none"}}"#,
        r#"{"progress":{"state":"none","percent":null,"value":null}}"#,
        r#"{"progress":["none",null]}"#,
    ] {
        assert_eq!(
            server(PROGRESS_CHANGED, body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
    let percent = Percent::new(100).unwrap();
    assert_eq!(Percent::new(101), None);
    assert!(Progress::new(ProgressState::Normal, Some(percent)).is_some());
    assert_eq!(Progress::new(ProgressState::Normal, None), None);
    assert_eq!(Progress::new(ProgressState::None, Some(percent)), None);
}

/// COM-06 (codec case): a raw u32 normal exit is distinct from a source-host
/// signal; zero exit completes and any other exit or signal fails.
#[test]
fn command_results_distinguish_normal_exits_from_signals() {
    let completed = ("completed", StatusState::Completed);
    let failed = ("failed", StatusState::Failed);
    for (result, expected, (name, state)) in [
        (r#"{"kind":"exit","code":0}"#, exit(0), completed),
        (r#"{"kind":"exit","code":143}"#, exit(143), failed),
        (
            r#"{"kind":"exit","code":4294967295}"#,
            exit(u32::MAX),
            failed,
        ),
        (
            r#"{"kind":"signal","number":15,"name":"SIGTERM"}"#,
            sigterm(),
            failed,
        ),
        (
            r#"{"kind":"signal","number":1,"name":null}"#,
            CommandResult::Signal {
                number: NonZeroU32::MIN,
                name: None,
            },
            failed,
        ),
    ] {
        let body = format!(
            r#"{{"command_result":{result},"status":{}}}"#,
            status_json(name, "null")
        );
        let ServerMessage::CommandFinal(command_final) = round_trip(COMMAND_FINAL, &body) else {
            panic!("{body} is a command final");
        };
        assert_eq!(command_final.command_result(), &expected);
        assert_eq!(command_final.status(), Some(&status(state, None)));
    }
    assert_ne!(exit(143), sigterm());
    assert_eq!(exit(0).final_state(), StatusState::Completed);
    assert_eq!(exit(143).final_state(), StatusState::Failed);
    assert_eq!(sigterm().final_state(), StatusState::Failed);
    for result in [
        r#"{"kind":"exit","code":-1}"#,
        r#"{"kind":"exit","code":4294967296}"#,
        r#"{"kind":"exit"}"#,
        r#"{"kind":"exit","code":0,"name":null}"#,
        r#"{"kind":"signal","number":0,"name":null}"#,
        r#"{"kind":"signal","number":15}"#,
        r#"{"kind":"signal","number":15,"name":15}"#,
        r#"{"kind":"Exit","code":0}"#,
        r#"{"code":0}"#,
        r#"["exit",0]"#,
    ] {
        let body = format!(r#"{{"command_result":{result},"status":null}}"#);
        assert_eq!(
            server(COMMAND_FINAL, &body),
            Err(FatalCode::InvalidRecord),
            "{result}"
        );
    }
}

/// COM-06 (codec case): Command final carries terminal status only when
/// status is selected, and that status agrees with the command result.
#[test]
fn command_final_status_is_terminal_and_agrees_with_the_result() {
    let exit_zero = r#"{"kind":"exit","code":0}"#;
    let exit_one = r#"{"kind":"exit","code":1}"#;
    let signal = r#"{"kind":"signal","number":15,"name":"SIGTERM"}"#;
    for (result, status) in [
        (exit_zero, "null".to_owned()),
        (signal, "null".to_owned()),
        (exit_zero, status_json("completed", "null")),
        (exit_one, status_json("failed", "null")),
        (signal, status_json("failed", "null")),
    ] {
        round_trip(
            COMMAND_FINAL,
            &format!(r#"{{"command_result":{result},"status":{status}}}"#),
        );
    }
    for (result, status) in [
        (exit_zero, status_json("failed", "null")),
        (exit_one, status_json("completed", "null")),
        (signal, status_json("completed", "null")),
        (exit_zero, status_json("running", "null")),
        (exit_one, status_json("idle", "null")),
        (exit_one, status_json("needs_attention", r#""1""#)),
    ] {
        let body = format!(r#"{{"command_result":{result},"status":{status}}}"#);
        assert_eq!(
            server(COMMAND_FINAL, &body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
    assert_eq!(
        server(
            COMMAND_FINAL,
            format!(r#"{{"command_result":{exit_zero}}}"#)
        ),
        Err(FatalCode::InvalidRecord)
    );
    assert!(CommandFinal::new(exit(0), None).is_some());
    assert!(CommandFinal::new(exit(0), Some(status(StatusState::Completed, None))).is_some());
    assert_eq!(
        CommandFinal::new(exit(0), Some(status(StatusState::Failed, None))),
        None
    );
}

fn metadata(title: &str, status: &str, progress: &str, result: &str) -> String {
    format!(
        r#"{{"title":{title},"status":{status},"progress":{progress},"command_result":{result}}}"#
    )
}

const UNAVAILABLE: &str = r#"{"available":false,"value":null}"#;

/// CTL-03 (codec case): Initial metadata reports each family's availability
/// explicitly; unset, empty and unavailable titles stay distinct.
#[test]
fn initial_metadata_reports_availability_explicitly() {
    let idle = format!(
        r#"{{"available":true,"value":{}}}"#,
        status_json("idle", "null")
    );
    let none = r#"{"available":true,"value":{"state":"none","percent":null}}"#;
    for (title, expected) in [
        (UNAVAILABLE.to_owned(), Available::Unavailable),
        (
            r#"{"available":true,"value":null}"#.to_owned(),
            Available::Available(None),
        ),
        (
            r#"{"available":true,"value":""}"#.to_owned(),
            Available::Available(Some(String::new())),
        ),
        (
            r#"{"available":true,"value":"vim — notes"}"#.to_owned(),
            Available::Available(Some("vim — notes".into())),
        ),
    ] {
        let body = metadata(&title, &idle, none, "null");
        let ServerMessage::InitialMetadata(decoded) = round_trip(INITIAL_METADATA, &body) else {
            panic!("{body} is initial metadata");
        };
        assert_eq!(decoded.title(), &expected, "{body}");
        assert_eq!(
            decoded.status(),
            &Available::Available(status(StatusState::Idle, None))
        );
        assert_eq!(
            decoded.progress(),
            &Available::Available(Progress::new(ProgressState::None, None).unwrap())
        );
        assert_eq!(decoded.command_result(), None);
    }
    let ServerMessage::InitialMetadata(unavailable) = round_trip(
        INITIAL_METADATA,
        &metadata(UNAVAILABLE, UNAVAILABLE, UNAVAILABLE, "null"),
    ) else {
        panic!("every family may be unavailable");
    };
    assert_eq!(unavailable.status(), &Available::Unavailable);
    assert_eq!(unavailable.progress(), &Available::Unavailable);
    let rejected = [
        metadata(r#"{"available":false,"value":"x"}"#, &idle, none, "null"),
        metadata(r#"{"available":false,"value":""}"#, &idle, none, "null"),
        metadata(
            UNAVAILABLE,
            r#"{"available":true,"value":null}"#,
            none,
            "null",
        ),
        metadata(
            UNAVAILABLE,
            &format!(
                r#"{{"available":false,"value":{}}}"#,
                status_json("idle", "null")
            ),
            none,
            "null",
        ),
        metadata(
            UNAVAILABLE,
            UNAVAILABLE,
            r#"{"available":true,"value":null}"#,
            "null",
        ),
        metadata(
            UNAVAILABLE,
            UNAVAILABLE,
            r#"{"available":false,"value":{"state":"none","percent":null}}"#,
            "null",
        ),
        metadata(
            r#"{"available":"true","value":null}"#,
            UNAVAILABLE,
            UNAVAILABLE,
            "null",
        ),
        metadata(r#"{"available":true}"#, UNAVAILABLE, UNAVAILABLE, "null"),
        metadata(r#"{"value":null}"#, UNAVAILABLE, UNAVAILABLE, "null"),
        metadata(
            r#"{"available":true,"value":null,"stale":false}"#,
            UNAVAILABLE,
            UNAVAILABLE,
            "null",
        ),
        metadata(r#"[true,null]"#, UNAVAILABLE, UNAVAILABLE, "null"),
        metadata("null", UNAVAILABLE, UNAVAILABLE, "null"),
        format!(r#"{{"title":{UNAVAILABLE},"status":{UNAVAILABLE},"progress":{UNAVAILABLE}}}"#),
    ];
    for body in rejected {
        assert_eq!(
            server(INITIAL_METADATA, &body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
}

/// COM-06 (codec case): when Initial metadata carries a terminal status and a
/// command result, they agree.
#[test]
fn initial_metadata_terminal_status_agrees_with_a_known_result() {
    let available = |state: &str| {
        format!(
            r#"{{"available":true,"value":{}}}"#,
            status_json(state, "null")
        )
    };
    let exit_zero = r#"{"kind":"exit","code":0}"#;
    let exit_one = r#"{"kind":"exit","code":1}"#;
    let signal = r#"{"kind":"signal","number":9,"name":"SIGKILL"}"#;
    for (status, result) in [
        (available("completed"), exit_zero),
        (available("failed"), exit_one),
        (available("failed"), signal),
        (available("completed"), "null"),
        (available("running"), exit_zero),
        (UNAVAILABLE.to_owned(), exit_one),
    ] {
        round_trip(
            INITIAL_METADATA,
            &metadata(UNAVAILABLE, &status, UNAVAILABLE, result),
        );
    }
    for (status, result) in [
        (available("completed"), exit_one),
        (available("completed"), signal),
        (available("failed"), exit_zero),
    ] {
        let body = metadata(UNAVAILABLE, &status, UNAVAILABLE, result);
        assert_eq!(
            server(INITIAL_METADATA, &body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
    let completed = Available::Available(status(StatusState::Completed, None));
    assert!(
        InitialMetadata::new(
            Available::Unavailable,
            completed.clone(),
            Available::Unavailable,
            Some(exit(0))
        )
        .is_some()
    );
    assert_eq!(
        InitialMetadata::new(
            Available::Unavailable,
            completed,
            Available::Unavailable,
            Some(sigterm())
        ),
        None
    );
}

/// CTL-03 (codec case): a title change distinguishes unset (null) from empty.
#[test]
fn title_changes_distinguish_unset_from_empty() {
    for (body, expected) in [
        (r#"{"title":null}"#, None),
        (r#"{"title":""}"#, Some("")),
        (
            r#"{"title":"\u001b]0;inert\u0007"}"#,
            Some("\u{1b}]0;inert\u{7}"),
        ),
    ] {
        let ServerMessage::TitleChanged(changed) = server(TITLE_CHANGED, body).unwrap() else {
            panic!("{body} is a title change");
        };
        assert_eq!(changed.title.as_deref(), expected, "{body}");
    }
    round_trip(TITLE_CHANGED, r#"{"title":"Build — café"}"#);
    for body in [
        r#"{}"#,
        r#"{"title":7}"#,
        r#"{"title":null,"title":null}"#,
        r#"{"title":"a","icon":null}"#,
    ] {
        assert_eq!(
            server(TITLE_CHANGED, body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
}

/// COM-02 (codec case): notifications are normalized inert records from
/// OSC 9, 99 or 777 with nullable source ID, title, body and episode.
#[test]
fn notifications_are_normalized_records_from_known_sources() {
    for source in ["osc9", "osc99", "osc777"] {
        round_trip(
            NOTIFICATION,
            &format!(
                r#"{{"source":"{source}","source_id":"build","title":"Done","body":null,"episode_id":null}}"#
            ),
        );
    }
    for body in [
        r#"{"source":"osc8","source_id":null,"title":"t","body":null,"episode_id":null}"#,
        r#"{"source":"OSC9","source_id":null,"title":"t","body":null,"episode_id":null}"#,
        r#"{"source":"osc9","source_id":null,"title":"t","body":null}"#,
        r#"{"source":"osc9","title":"t","body":null,"episode_id":null}"#,
        r#"{"source":"osc9","source_id":null,"title":"t","body":null,"episode_id":"0"}"#,
        r#"{"source":"osc9","source_id":null,"title":"t","body":null,"episode_id":2}"#,
        r#"{"source":"osc9","source_id":null,"title":1,"body":null,"episode_id":null}"#,
        r#"{"source":"osc9","source_id":null,"title":"t","body":null,"episode_id":null,"urgency":1}"#,
    ] {
        assert_eq!(
            server(NOTIFICATION, body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
}

/// COM-02 (codec case): a grant's reason is `attach`, `acquire`, `hand_back`
/// or `successor`, and a revocation's `takeover` or `session_ending`, spelled
/// exactly. Each decodes and re-encodes; any other name, including the other
/// event's, or any other type is `invalid_record`.
#[test]
fn grant_and_revocation_reasons_are_their_documented_names() {
    let grant = |reason: &str| {
        format!(
            r#"{{"period":"2","restoration_id":"3","geometry":{{"cols":80,"rows":24}},"reason":{reason}}}"#
        )
    };
    let revocation =
        |reason: &str| format!(r#"{{"period":"1","restoration_id":"1","reason":{reason}}}"#);
    for (name, reason) in [
        ("attach", GrantReason::Attach),
        ("acquire", GrantReason::Acquire),
        ("hand_back", GrantReason::HandBack),
        ("successor", GrantReason::Successor),
    ] {
        assert_eq!(
            round_trip(CONTROL_GRANTED, &grant(&format!(r#""{name}""#))),
            ServerMessage::ControlGranted(ControlGranted {
                period: counter(2),
                restoration_id: counter(3),
                geometry: geometry(80, 24),
                reason,
            })
        );
    }
    for (name, reason) in [
        ("takeover", RevocationReason::Takeover),
        ("session_ending", RevocationReason::SessionEnding),
    ] {
        assert_eq!(
            round_trip(CONTROL_REVOKED, &revocation(&format!(r#""{name}""#))),
            ServerMessage::ControlRevoked(ControlRevoked {
                period: counter(1),
                restoration_id: counter(1),
                reason,
            })
        );
    }
    for reason in [
        r#""Attach""#,
        r#""hand-back""#,
        r#""handback""#,
        r#""takeover""#,
        r#""""#,
        "null",
        "1",
    ] {
        let body = grant(reason);
        assert_eq!(
            server(CONTROL_GRANTED, &body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
    for reason in [
        r#""Takeover""#,
        r#""session-ending""#,
        r#""successor""#,
        r#""""#,
        "null",
        "1",
    ] {
        let body = revocation(reason);
        assert_eq!(
            server(CONTROL_REVOKED, &body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
}
