//! #20, #27 and ADR 0001: the frozen hello request and reply.
use climon2_core::protocol::*;
use support::json_examples;

mod support;

fn frame(body: &str) -> Frame {
    Frame::new(0x01, body.as_bytes().to_vec()).unwrap()
}

/// The hello bodies are JSON objects with typed required fields; unknown
/// fields are ignored.
#[test]
fn issue20_hello_json_is_an_object_with_required_typed_fields() {
    assert_eq!(
        HelloRequest::from_frame(&frame(r#"{"protocol":1,"future":true}"#)).unwrap(),
        HelloRequest { protocol: 1 }
    );
    for invalid in [
        "[",
        "null",
        "[]",
        "{}",
        "{}{}",
        r#"{"protocol":"1"}"#,
        r#"{"protocol":-1}"#,
        r#"{"protocol":4294967296}"#,
    ] {
        assert!(
            HelloRequest::from_frame(&frame(invalid)).is_err(),
            "{invalid}"
        );
    }
    let invalid_utf8 = Frame::new(0x01, b"\xff".to_vec()).unwrap();
    assert!(HelloRequest::from_frame(&invalid_utf8).is_err());
    for invalid in [
        "{}",
        r#"{"session_id":"x","climon2_version":"1.0.0","protocol":1}"#,
        r#"{"session_id":"550e8400-e29b-41d4-a716-446655440000","protocol":1}"#,
    ] {
        assert!(
            HelloReply::from_frame(&frame(invalid)).is_err(),
            "{invalid}"
        );
    }
}

/// ADR 0001: the exact hello bytes in both directions, which round-trip.
#[test]
fn issue20_hello_has_its_exact_wire_code_and_json() {
    let request = HelloRequest { protocol: 1 }.to_frame().unwrap();
    assert_eq!(
        (request.type_code(), request.body()),
        (0x01, br#"{"protocol":1}"#.as_slice())
    );
    let reply = HelloReply {
        session_id: "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
        climon2_version: "1.0.0".parse().unwrap(),
        protocol: 1,
    };
    let encoded = reply.to_frame().unwrap();
    assert_eq!(
        (encoded.type_code(), encoded.body()),
        (
            0x01,
            br#"{"session_id":"550e8400-e29b-41d4-a716-446655440000","climon2_version":"1.0.0","protocol":1}"#
                .as_slice()
        )
    );
    assert_eq!(HelloReply::from_frame(&encoded).unwrap(), reply);
}

/// ADR 0001: a probe identifies the session alone, ignoring future fields,
/// the daemon's release and its protocol.
#[test]
fn adr0001_hello_ignores_future_fields_for_probes() {
    let reply = HelloReply::from_frame(&frame(
        r#"{"session_id":"550e8400-e29b-41d4-a716-446655440000","climon2_version":"1.2.3","protocol":999,"future":{"attached":true}}"#,
    ))
    .unwrap();
    assert_eq!(reply.protocol, 999);
    let id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    assert!(reply.is_live_session(&id));
    let other = "550e8400-e29b-41d4-a716-446655440001".parse().unwrap();
    assert!(!reply.is_live_session(&other));
}

#[test]
fn issue20_size_rules_and_absolute_deadline_do_not_restart_for_partial_io() {
    assert_eq!(
        TerminalSize { cols: 0, rows: 0 }.initial(),
        TerminalSize { cols: 80, rows: 24 }
    );
    assert_eq!(
        TerminalSize {
            cols: 120,
            rows: 40
        }
        .initial(),
        TerminalSize {
            cols: 120,
            rows: 40
        }
    );
    assert!(TerminalSize { cols: 0, rows: 40 }.resize().is_none());
    assert!(TerminalSize { cols: 80, rows: 0 }.resize().is_none());
    assert_eq!(
        TerminalSize { cols: 80, rows: 24 }.resize(),
        Some(TerminalSize { cols: 80, rows: 24 })
    );
    use std::time::Duration;
    assert_eq!(
        hello_time_remaining(Duration::from_millis(750)),
        Some(Duration::from_millis(1250))
    );
    assert_eq!(
        hello_time_remaining(Duration::from_millis(1999)),
        Some(Duration::from_millis(1))
    );
    assert_eq!(hello_time_remaining(Duration::from_secs(2)), None);
    assert_eq!(hello_time_remaining(Duration::from_secs(3)), None);
}

/// COM-01 (Contract after retirement): the frozen hello examples published
/// in the reference still decode.
#[test]
fn protocol_reference_hello_examples_use_the_public_wire_contract() {
    let requests = json_examples(include_str!(
        "../../../docs/for-humans/protocol/daemon-client/hello-request.md"
    ));
    assert!(!requests.is_empty(), "document at least one hello request");
    for body in requests {
        assert_eq!(
            HelloRequest::from_frame(&frame(&body)).unwrap(),
            HelloRequest { protocol: 1 }
        );
    }

    let replies = json_examples(include_str!(
        "../../../docs/for-humans/protocol/daemon-client/hello-reply.md"
    ));
    assert!(!replies.is_empty(), "document at least one hello reply");
    for body in replies {
        let reply = HelloReply::from_frame(&frame(&body)).unwrap();
        assert_eq!(reply.protocol, PROTOCOL_VERSION);
        let extensions = climon2_core::hello::HelloExtensions::from_body(body.as_bytes());
        assert!(!extensions.capabilities.is_malformed());
        assert!(!extensions.state.is_malformed());
    }
}
