//! Plan 1.11/1.15: optional hello extensions stay lenient and separate from the
//! frozen mandatory reply; the daemon's reply leads with exactly the frozen
//! tokens and keeps their state beside its revision-1 advertisement.
use climon2_core::{
    hello::*,
    protocol::*,
    wire::{Advertisement, Family, FrameLimit, FrameLimits},
};
use serde_json::{Value, json};

const ID: &str = "550e8400-e29b-41d4-a716-446655440000";

fn reply() -> HelloReply {
    HelloReply {
        session_id: ID.parse().unwrap(),
        climon2_version: "1.2.3".parse().unwrap(),
        protocol: 1,
    }
}

fn with(extra: Value) -> HelloExtensions {
    let mut body = json!({"session_id": ID, "climon2_version": "1.2.3", "protocol": 1});
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let bytes = serde_json::to_vec(&body).unwrap();
    let frame = Frame::new(1, bytes.clone()).unwrap();
    assert_eq!(
        ServerMessage::from_frame(frame).unwrap(),
        ServerMessage::Hello(reply()),
        "optional data never changes the mandatory reply"
    );
    assert_eq!(
        HelloExtensions::from_body(&bytes),
        HelloExtensions::from_value(&body)
    );
    HelloExtensions::from_value(&body)
}

fn state(lifecycle: Lifecycle, attachments: u32, controlled: bool) -> Option<SessionState> {
    SessionState::new(lifecycle, attachments, controlled)
}

/// Revision 1 as the smallest advertisement: `daemon-wire-v1` alone.
fn core_only() -> Advertisement {
    Advertisement::new([], frame_limits(1_048_576, 1_048_576))
}

/// COM-01 (codec case): the daemon's reply leads its capabilities with
/// exactly the frozen tokens, whose operations it supports, and carries a
/// state beside the mandatory reply, which decodes unchanged; revision 1
/// follows them.
#[test]
fn the_daemon_reply_leads_with_the_frozen_tokens_and_a_state_beside_the_mandatory_reply() {
    let snapshot = state(Lifecycle::Attached, 2, true).unwrap();
    let frame = advertise_revision1(&reply(), snapshot, &core_only()).unwrap();
    assert_eq!(frame.type_code(), 0x01);
    assert_eq!(
        std::str::from_utf8(frame.body()).unwrap(),
        format!(
            r#"{{"session_id":"{ID}","climon2_version":"1.2.3","protocol":1,"capabilities":["attach","kill","state-v1","client-startup-v1","daemon-wire-v1"],"state":{{"lifecycle":"attached","attachments":2,"controlled":true}},"daemon_wire_v1":{{"max_request_frame_bytes":1048576,"max_server_frame_bytes":1048576}}}}"#
        )
    );
    let extensions = HelloExtensions::from_body(frame.body());
    let capabilities = extensions.capabilities.valid().unwrap();
    assert!(capabilities.supports_attach());
    assert!(capabilities.supports_kill());
    assert!(capabilities.supports_state());
    assert_eq!(extensions.state, Advertised::Valid(snapshot));
    assert_eq!(
        ServerMessage::from_frame(frame).unwrap(),
        ServerMessage::Hello(reply())
    );
    assert_eq!(
        Capability::ADVERTISED.map(Capability::token),
        ["attach", "kill", "state-v1", "client-startup-v1"]
    );
}

#[test]
fn absent_extensions_grant_nothing_and_are_not_malformed() {
    let extensions = with(json!({}));
    assert_eq!(extensions.capabilities, Advertised::Absent);
    assert_eq!(extensions.state, Advertised::Absent);
    assert_eq!(extensions.capabilities.valid(), None);
    for body in [b"not json".as_slice(), b"[]", b"null", b""] {
        assert_eq!(
            HelloExtensions::from_body(body),
            HelloExtensions {
                capabilities: Advertised::Absent,
                state: Advertised::Absent,
                daemon_wire_v1: Advertised::Absent,
            }
        );
    }
}

#[test]
fn each_operation_needs_its_own_tokens_and_unknown_tokens_are_ignored() {
    for (tokens, attach, kill, state) in [
        (json!([]), false, false, false),
        (json!(["attach"]), false, false, false),
        (json!(["kill"]), false, false, false),
        (json!(["client-startup-v1"]), false, false, false),
        (json!(["attach", "client-startup-v1"]), true, false, false),
        (json!(["client-startup-v1", "kill"]), false, true, false),
        (json!(["state-v1"]), false, false, true),
        (
            json!([
                "nesting-v1",
                "future",
                "ATTACH",
                "attach ",
                "attach",
                "client-startup-v1"
            ]),
            true,
            false,
            false,
        ),
        (
            json!(["kill", "kill", "state-v1", "attach", "client-startup-v1"]),
            true,
            true,
            true,
        ),
    ] {
        let extensions = with(json!({ "capabilities": tokens }));
        let capabilities = extensions.capabilities.valid().unwrap();
        assert_eq!(
            (
                capabilities.supports_attach(),
                capabilities.supports_kill(),
                capabilities.supports_state()
            ),
            (attach, kill, state),
            "{tokens}"
        );
    }
    let only = with(json!({"capabilities": ["state-v1", "other"]}));
    let capabilities = only.capabilities.valid().unwrap();
    assert!(capabilities.contains(Capability::StateV1));
    for absent in [
        Capability::Attach,
        Capability::Kill,
        Capability::ClientStartupV1,
    ] {
        assert!(!capabilities.contains(absent), "{absent:?}");
    }
}

#[test]
fn a_malformed_advertisement_grants_nothing_without_touching_the_rest() {
    for malformed in [
        json!("attach"),
        json!({"attach": true}),
        json!(["attach", 1]),
        json!(["attach", null]),
        json!([["attach"]]),
        json!(null),
        json!(true),
    ] {
        let extensions = with(json!({
            "capabilities": malformed,
            "state": {"lifecycle": "detached", "attachments": 0, "controlled": false},
        }));
        assert_eq!(
            extensions.capabilities,
            Advertised::Malformed,
            "{malformed}"
        );
        assert!(extensions.capabilities.is_malformed());
        assert_eq!(extensions.capabilities.valid(), None);
        assert_eq!(
            extensions.state,
            Advertised::Valid(state(Lifecycle::Detached, 0, false).unwrap())
        );
    }
    assert!(!Advertised::<Capabilities>::Absent.is_malformed());
    assert!(
        !with(json!({"capabilities": []}))
            .capabilities
            .is_malformed()
    );
}

#[test]
fn state_lifecycles_have_their_frozen_names() {
    for (lifecycle, name) in [
        (Lifecycle::Starting, "starting"),
        (Lifecycle::Attached, "attached"),
        (Lifecycle::Detached, "detached"),
        (Lifecycle::Terminating, "terminating"),
    ] {
        assert_eq!(lifecycle.name(), name);
        let attachments = u32::from(lifecycle == Lifecycle::Attached);
        let controlled = lifecycle == Lifecycle::Attached;
        let snapshot = state(lifecycle, attachments, controlled).unwrap();
        assert_eq!(
            (
                snapshot.lifecycle(),
                snapshot.attachments(),
                snapshot.controlled()
            ),
            (lifecycle, attachments, controlled)
        );
        let parsed = with(json!({"state": {
            "lifecycle": name, "attachments": attachments, "controlled": controlled, "future": 1
        }}));
        assert_eq!(parsed.state, Advertised::Valid(snapshot), "{name}");
        let frame = advertise_revision1(&reply(), snapshot, &core_only()).unwrap();
        assert_eq!(
            HelloExtensions::from_body(frame.body()).state,
            Advertised::Valid(snapshot)
        );
    }
}

#[test]
fn only_consistent_point_in_time_states_exist() {
    use Lifecycle::*;
    for (lifecycle, attachments, controlled, consistent) in [
        (Starting, 0, false, true),
        (Starting, 0, true, false),
        (Starting, 1, false, false),
        (Starting, 1, true, false),
        (Attached, 0, false, false),
        (Attached, 0, true, false),
        (Attached, 1, false, false),
        (Attached, 1, true, true),
        (Attached, 2, true, true),
        (Attached, u32::MAX, true, true),
        (Detached, 0, false, true),
        (Detached, 0, true, false),
        (Detached, 1, false, false),
        (Detached, 2, true, false),
        (Terminating, 0, false, true),
        (Terminating, 0, true, false),
        (Terminating, 1, false, true),
        (Terminating, 1, true, true),
        (Terminating, 3, true, true),
    ] {
        assert_eq!(
            state(lifecycle, attachments, controlled).is_some(),
            consistent,
            "{lifecycle:?} {attachments} {controlled}"
        );
    }
}

#[test]
fn malformed_or_inconsistent_state_is_reported_and_never_trusted() {
    for malformed in [
        json!(null),
        json!("attached"),
        json!(["attached", 1, true]),
        json!({"lifecycle": "Attached", "attachments": 1, "controlled": true}),
        json!({"lifecycle": "running", "attachments": 1, "controlled": true}),
        json!({"lifecycle": "attached", "attachments": -1, "controlled": true}),
        json!({"lifecycle": "attached", "attachments": 1.5, "controlled": true}),
        json!({"lifecycle": "attached", "attachments": "1", "controlled": true}),
        json!({"lifecycle": "attached", "attachments": 4_294_967_296_u64, "controlled": true}),
        json!({"lifecycle": "attached", "attachments": 1, "controlled": "true"}),
        json!({"lifecycle": "attached", "attachments": 1}),
        json!({"lifecycle": "attached", "controlled": true}),
        json!({"attachments": 1, "controlled": true}),
        json!({"lifecycle": "starting", "attachments": 1, "controlled": false}),
        json!({"lifecycle": "attached", "attachments": 0, "controlled": true}),
    ] {
        let extensions = with(json!({"capabilities": ["state-v1"], "state": malformed}));
        assert_eq!(extensions.state, Advertised::Malformed, "{malformed}");
        assert!(extensions.capabilities.valid().unwrap().supports_state());
    }
}

fn frame_limits(request: u32, server: u32) -> FrameLimits {
    FrameLimits {
        max_request_frame_bytes: FrameLimit::new(request).unwrap(),
        max_server_frame_bytes: FrameLimit::new(server).unwrap(),
    }
}

/// COM-01 (codec case): revision 1 is available only when hello advertises
/// both `daemon-wire-v1` and valid `daemon_wire_v1` limits. Anything missing
/// or malformed is unavailable; the frozen reply still parses and the other
/// optional fields are untouched.
#[test]
fn a_revision1_advertisement_needs_its_token_and_valid_frame_limits() {
    let limits = json!({"max_request_frame_bytes": 1, "max_server_frame_bytes": 4_294_967_295_u32});
    let token = json!(["daemon-wire-v1"]);
    let valid = with(json!({"capabilities": token, "daemon_wire_v1": limits}));
    assert_eq!(
        valid.daemon_wire_v1,
        Advertised::Valid(Advertisement::new([], frame_limits(1, u32::MAX)))
    );
    assert_eq!(
        valid.capabilities,
        Advertised::Valid(Capabilities::default())
    );
    let lenient = with(json!({"capabilities": token, "daemon_wire_v1": {
        "max_request_frame_bytes": 65_536, "max_server_frame_bytes": 1_048_576, "future": [1]
    }}));
    assert_eq!(
        lenient.daemon_wire_v1.valid().unwrap().limits(),
        frame_limits(65_536, 1_048_576)
    );

    assert_eq!(with(json!({})).daemon_wire_v1, Advertised::Absent);
    assert_eq!(
        with(json!({"capabilities": ["attach", "state-v1"]})).daemon_wire_v1,
        Advertised::Absent
    );
    for (capabilities, wire) in [
        (token.clone(), None),
        (json!(["attach"]), Some(limits.clone())),
        (json!([]), Some(limits.clone())),
        (json!(["daemon-wire-v1", 1]), Some(limits.clone())),
        (json!("daemon-wire-v1"), Some(limits.clone())),
        (json!(["DAEMON-WIRE-V1"]), Some(limits.clone())),
        (
            token.clone(),
            Some(json!({"max_request_frame_bytes": 0, "max_server_frame_bytes": 1})),
        ),
        (
            token.clone(),
            Some(json!({"max_request_frame_bytes": 1, "max_server_frame_bytes": 0})),
        ),
        (
            token.clone(),
            Some(
                json!({"max_request_frame_bytes": 4_294_967_296_u64, "max_server_frame_bytes": 1}),
            ),
        ),
        (
            token.clone(),
            Some(json!({"max_request_frame_bytes": "1", "max_server_frame_bytes": 1})),
        ),
        (
            token.clone(),
            Some(json!({"max_request_frame_bytes": 1.5, "max_server_frame_bytes": 1})),
        ),
        (
            token.clone(),
            Some(json!({"max_request_frame_bytes": -1, "max_server_frame_bytes": 1})),
        ),
        (token.clone(), Some(json!({"max_request_frame_bytes": 1}))),
        (token.clone(), Some(json!({"max_server_frame_bytes": 1}))),
        (token.clone(), Some(json!([1, 1]))),
        (token.clone(), Some(json!(null))),
        (token.clone(), Some(json!(true))),
    ] {
        let mut extra = json!({"capabilities": capabilities, "state": {
            "lifecycle": "detached", "attachments": 0, "controlled": false
        }});
        if let Some(wire) = wire {
            extra["daemon_wire_v1"] = wire;
        }
        let extensions = with(extra.clone());
        assert_eq!(extensions.daemon_wire_v1, Advertised::Malformed, "{extra}");
        assert!(extensions.daemon_wire_v1.is_malformed());
        assert_eq!(
            extensions.state,
            Advertised::Valid(state(Lifecycle::Detached, 0, false).unwrap()),
            "{extra}"
        );
    }
}

/// COM-01 (codec case): advertised families are the known case-sensitive
/// revision-1 tokens; unknown, repeated and frozen tokens are not families,
/// and support is never inferred from the release.
#[test]
fn advertised_families_are_the_known_revision1_tokens() {
    let extensions = with(json!({
        "capabilities": ["attach", "status-v1", "future-v9", "STATUS-V1", "daemon-wire-v1",
            "title-v1", "status-v1", "kill", "client-startup-v1"],
        "daemon_wire_v1": {"max_request_frame_bytes": 1_048_576, "max_server_frame_bytes": 1_048_576},
    }));
    let advertisement = extensions.daemon_wire_v1.valid().unwrap();
    assert_eq!(
        advertisement.families().collect::<Vec<_>>(),
        [Family::DaemonWireV1, Family::TitleV1, Family::StatusV1]
    );
    for family in [Family::DaemonWireV1, Family::TitleV1, Family::StatusV1] {
        assert!(advertisement.supports(family), "{family:?}");
    }
    for family in [
        Family::MonitorV1,
        Family::NotificationV1,
        Family::ProgressV1,
        Family::FocusV1,
    ] {
        assert!(!advertisement.supports(family), "{family:?}");
    }
    assert!(extensions.capabilities.valid().unwrap().supports_attach());
    assert!(extensions.capabilities.valid().unwrap().supports_kill());
    for (family, token) in [
        (Family::DaemonWireV1, "daemon-wire-v1"),
        (Family::MonitorV1, "monitor-v1"),
        (Family::TitleV1, "title-v1"),
        (Family::StatusV1, "status-v1"),
        (Family::NotificationV1, "notification-v1"),
        (Family::ProgressV1, "progress-v1"),
        (Family::FocusV1, "focus-v1"),
    ] {
        assert_eq!(family.token(), token);
        assert_eq!(Family::from_token(token), Some(family));
    }
    assert_eq!(Family::from_token("attach"), None);
    assert_eq!(Family::from_token("Monitor-v1"), None);
}

/// COM-01 (codec case): revision 1 joins the frozen tokens and state in the
/// same reply without changing them, and decodes back to the same support.
#[test]
fn revision1_support_is_advertised_beside_the_frozen_tokens_and_state() {
    let snapshot = state(Lifecycle::Attached, 1, true).unwrap();
    let advertisement = Advertisement::new(
        [Family::StatusV1, Family::MonitorV1],
        frame_limits(65_536, 1_048_576),
    );
    let frame = advertise_revision1(&reply(), snapshot, &advertisement).unwrap();
    assert_eq!(
        std::str::from_utf8(frame.body()).unwrap(),
        format!(
            r#"{{"session_id":"{ID}","climon2_version":"1.2.3","protocol":1,"capabilities":["attach","kill","state-v1","client-startup-v1","daemon-wire-v1","monitor-v1","status-v1"],"state":{{"lifecycle":"attached","attachments":1,"controlled":true}},"daemon_wire_v1":{{"max_request_frame_bytes":65536,"max_server_frame_bytes":1048576}}}}"#
        )
    );
    let extensions = HelloExtensions::from_body(frame.body());
    assert_eq!(extensions.daemon_wire_v1, Advertised::Valid(advertisement));
    assert_eq!(extensions.state, Advertised::Valid(snapshot));
    assert_eq!(
        ServerMessage::from_frame(frame).unwrap(),
        ServerMessage::Hello(reply())
    );
}
