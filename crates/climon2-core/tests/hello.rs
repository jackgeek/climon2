//! Plan 1.11/1.15: optional hello extensions stay lenient and separate from the
//! frozen mandatory reply; the daemon advertises exactly the frozen tokens.
use climon2_core::{hello::*, protocol::*};
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

#[test]
fn the_daemon_advertises_the_frozen_tokens_and_a_state_beside_the_mandatory_reply() {
    let snapshot = state(Lifecycle::Attached, 2, true).unwrap();
    let frame = advertise(&reply(), snapshot).unwrap();
    assert_eq!(frame.type_code(), 0x01);
    assert_eq!(
        std::str::from_utf8(frame.body()).unwrap(),
        format!(
            r#"{{"session_id":"{ID}","climon2_version":"1.2.3","protocol":1,"capabilities":["attach","kill","state-v1","client-startup-v1"],"state":{{"lifecycle":"attached","attachments":2,"controlled":true}}}}"#
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
        let frame = advertise(&reply(), snapshot).unwrap();
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
