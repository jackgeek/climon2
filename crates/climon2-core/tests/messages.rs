//! #20, #27, #28 and ADR 0001: public typed protocol and state boundaries.
use climon2_core::protocol::*;

#[test]
fn issue20_control_json_is_an_object_with_required_typed_fields_and_direction() {
    for (code, valid) in [
        (0x01, r#"{"protocol":1,"future":true}"#),
        (0x10, r#"{"cols":80,"rows":24,"future":true}"#),
        (0x12, r#"{"cols":80,"rows":24,"future":true}"#),
        (0x7f, r#"{"message":"reason","future":true}"#),
    ] {
        assert!(
            ClientMessage::from_frame(Frame::new(code, valid.as_bytes().to_vec()).unwrap()).is_ok()
        );
        for invalid in [b"[".as_slice(), b"null", b"[]", b"{}", b"\xff", b"{}{}"] {
            assert!(
                ClientMessage::from_frame(Frame::new(code, invalid.to_vec()).unwrap()).is_err(),
                "{code}: {invalid:?}"
            );
        }
    }
    for (code, valid) in [
        (0x20, r#"{"future":true}"#),
        (0x22, r#"{"code":4294967295,"future":true}"#),
        (0x7f, r#"{"message":"reason","future":true}"#),
    ] {
        assert!(
            ServerMessage::from_frame(Frame::new(code, valid.as_bytes().to_vec()).unwrap()).is_ok()
        );
        for invalid in [b"[".as_slice(), b"null", b"[]", b"\xff", b"{}{}"] {
            assert!(
                ServerMessage::from_frame(Frame::new(code, invalid.to_vec()).unwrap()).is_err(),
                "{code}: {invalid:?}"
            );
        }
    }
    for invalid in [
        r#"{"protocol":"1"}"#,
        r#"{"protocol":-1}"#,
        r#"{"protocol":4294967296}"#,
    ] {
        assert!(
            ClientMessage::from_frame(Frame::new(1, invalid.as_bytes().to_vec()).unwrap()).is_err()
        );
    }
    for code in [0x20, 0x21, 0x22] {
        assert!(ClientMessage::from_frame(Frame::new(code, b"{}".to_vec()).unwrap()).is_err());
    }
    for code in [0x10, 0x11, 0x12] {
        assert!(ServerMessage::from_frame(Frame::new(code, b"{}".to_vec()).unwrap()).is_err());
    }
}

#[test]
fn adr0001_hello_ignores_future_fields_for_probe_and_version_matched_client() {
    let frame = Frame::new(1, br#"{"session_id":"550e8400-e29b-41d4-a716-446655440000","climon2_version":"1.2.3","protocol":1,"future":{"attached":true}}"#.to_vec()).unwrap();
    let ServerMessage::Hello(reply) = ServerMessage::from_frame(frame).unwrap() else {
        panic!("expected hello");
    };
    let id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    assert!(reply.is_live_session(&id));
    reply
        .validate_client(&id, &"1.2.3".parse().unwrap())
        .unwrap();
    assert!(
        reply
            .validate_client(&id, &"1.2.4".parse().unwrap())
            .is_err()
    );
    let future = HelloReply {
        protocol: 999,
        ..reply
    };
    assert!(future.is_live_session(&id));
    assert!(
        future
            .validate_client(&id, &"1.2.3".parse().unwrap())
            .is_err()
    );
}

#[test]
fn issue20_protocol1_exact_wire_codes_and_json_have_no_feature2_reservations() {
    let pairs = [
        (
            ClientMessage::Hello { protocol: 1 },
            0x01,
            br#"{"protocol":1}"#.as_slice(),
        ),
        (
            ClientMessage::Attach(TerminalSize { cols: 80, rows: 24 }),
            0x10,
            br#"{"cols":80,"rows":24}"#.as_slice(),
        ),
        (ClientMessage::Input(vec![0, 255]), 0x11, &[0, 255]),
        (
            ClientMessage::Resize(TerminalSize {
                cols: 120,
                rows: 40,
            }),
            0x12,
            br#"{"cols":120,"rows":40}"#.as_slice(),
        ),
        (
            ClientMessage::Error {
                message: "bad".into(),
            },
            0x7f,
            br#"{"message":"bad"}"#.as_slice(),
        ),
    ];
    for (message, code, body) in pairs {
        let frame = message.to_frame().unwrap();
        assert_eq!((frame.type_code(), frame.body()), (code, body));
        assert_eq!(ClientMessage::from_frame(frame).unwrap(), message);
    }
}

#[test]
fn issue20_probe_is_not_attached_and_protocol_errors_are_terminal() {
    let mut state = ConnectionState::default();
    state
        .receive(&ClientMessage::Hello { protocol: 1 })
        .unwrap();
    assert_eq!(state, ConnectionState::Probing);
    state
        .receive(&ClientMessage::Attach(TerminalSize { cols: 80, rows: 24 }))
        .unwrap();
    state.receive(&ClientMessage::Input(vec![b'x'])).unwrap();
    state
        .receive(&ClientMessage::Resize(TerminalSize {
            cols: 120,
            rows: 40,
        }))
        .unwrap();
    let error = state
        .receive(&ClientMessage::Attach(TerminalSize { cols: 80, rows: 24 }))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "session already has a controlling client"
    );
    assert_eq!(state, ConnectionState::Closed);

    for message in [
        ClientMessage::Input(vec![]),
        ClientMessage::Resize(TerminalSize { cols: 1, rows: 1 }),
        ClientMessage::Hello { protocol: 1 },
    ] {
        let mut state = ConnectionState::Probing;
        assert!(state.receive(&message).is_err());
        assert_eq!(state, ConnectionState::Closed);
    }

    let mut state = ConnectionState::default();
    assert!(
        state
            .receive(&ClientMessage::Hello { protocol: 2 })
            .is_err()
    );
    assert_eq!(state, ConnectionState::Closed);
}

#[test]
fn issue20_errors_close_from_every_state_and_prehello_input_is_rejected() {
    for initial in [
        ConnectionState::AwaitingHello,
        ConnectionState::Probing,
        ConnectionState::Attached,
        ConnectionState::Closed,
    ] {
        let mut state = initial;
        state
            .receive(&ClientMessage::Error {
                message: "stop".into(),
            })
            .unwrap();
        assert_eq!(state, ConnectionState::Closed);
    }
    for message in [
        ClientMessage::Input(vec![1]),
        ClientMessage::Resize(TerminalSize { cols: 1, rows: 1 }),
        ClientMessage::Attach(TerminalSize { cols: 1, rows: 1 }),
    ] {
        let mut state = ConnectionState::default();
        assert!(state.receive(&message).is_err());
        assert_eq!(state, ConnectionState::Closed);
    }
    let reply = HelloReply {
        session_id: "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
        climon2_version: "1.0.0".parse().unwrap(),
        protocol: 1,
    };
    let other = "550e8400-e29b-41d4-a716-446655440001".parse().unwrap();
    assert!(!reply.is_live_session(&other));
    assert!(
        reply
            .validate_client(&other, &reply.climon2_version)
            .is_err()
    );
    let message = ServerMessage::Hello(reply);
    assert_eq!(
        ServerMessage::from_frame(message.to_frame().unwrap()).unwrap(),
        message
    );
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

#[test]
fn issue20_server_protocol1_exact_wire_codes_and_json() {
    let server = [
        (ServerMessage::Attached, 0x20, b"{}".as_slice()),
        (ServerMessage::Output(vec![0, 255]), 0x21, &[0, 255]),
        (
            ServerMessage::Exit { code: 143 },
            0x22,
            br#"{"code":143}"#.as_slice(),
        ),
        (
            ServerMessage::Error {
                message: "bad".into(),
            },
            0x7f,
            br#"{"message":"bad"}"#.as_slice(),
        ),
    ];
    for (message, code, body) in server {
        let frame = message.to_frame().unwrap();
        assert_eq!((frame.type_code(), frame.body()), (code, body));
        assert_eq!(ServerMessage::from_frame(frame).unwrap(), message);
    }
}
