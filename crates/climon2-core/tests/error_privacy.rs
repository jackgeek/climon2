//! #25/#34 diagnostics must not leak supplied command or environment contents.
use climon2_core::{
    metadata::SessionMetadata,
    protocol::{Frame, HelloRequest},
};
use std::path::Path;

#[test]
fn malformed_metadata_and_control_errors_do_not_echo_external_values() {
    let secret = "PRIVATE_ARG_OR_ENV_2df4c8";
    let mut record: serde_json::Value =
        serde_json::from_slice(include_bytes!("goldens/metadata-v1.json")).unwrap();
    record["args"] = serde_json::json!(secret);
    let error = SessionMetadata::from_json(
        Path::new("550e8400-e29b-41d4-a716-446655440000.pty"),
        &serde_json::to_vec(&record).unwrap(),
    )
    .unwrap_err();
    assert!(!error.to_string().contains(secret), "{error}");
    let body = serde_json::json!({ "protocol": secret });
    let error =
        HelloRequest::from_frame(&Frame::new(0x01, serde_json::to_vec(&body).unwrap()).unwrap())
            .unwrap_err();
    assert!(!error.to_string().contains(secret), "{error}");
}
