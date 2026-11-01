//! Plan 1.12 / ADR 0006: the schema-1 `CLIMON2_CLIENT_STARTUP` envelope.
use climon2_core::{
    keys::{ClientSettings, ControlKey},
    startup_envelope::*,
};

const ATTACH: &str =
    r#"{"schema":1,"operation":"attach","settings":{"escape_prefix":29,"take_control_key":20}}"#;
const KILL: &str = r#"{"schema":1,"operation":"kill"}"#;

fn key(byte: u8) -> Option<ControlKey> {
    ControlKey::from_byte(byte)
}

fn settings(escape: Option<ControlKey>, take: Option<ControlKey>) -> ClientSettings {
    ClientSettings::new(escape, take).unwrap()
}

fn parse(text: &str) -> Result<StartupEnvelope, EnvelopeError> {
    StartupEnvelope::parse(text.as_bytes())
}

fn rejected(text: &str) -> String {
    parse(text).unwrap_err().to_string()
}

#[test]
fn the_variable_name_and_documented_envelopes_are_exact() {
    assert_eq!(ENVIRONMENT_VARIABLE, "CLIMON2_CLIENT_STARTUP");
    assert_eq!(MAX_ENVELOPE_LENGTH, 4096);
    let attach = StartupEnvelope::Attach(ClientSettings::default());
    assert_eq!(attach.to_json(), ATTACH);
    assert_eq!(parse(ATTACH).unwrap(), attach);
    assert_eq!(StartupEnvelope::Kill.to_json(), KILL);
    assert_eq!(parse(KILL).unwrap(), StartupEnvelope::Kill);
}

#[test]
fn attach_settings_round_trip_every_binding_including_disabled_ones() {
    for (escape, take, json) in [
        (
            None,
            None,
            r#"{"escape_prefix":null,"take_control_key":null}"#,
        ),
        (
            key(0x00),
            key(0x1F),
            r#"{"escape_prefix":0,"take_control_key":31}"#,
        ),
        (
            key(0x1B),
            None,
            r#"{"escape_prefix":27,"take_control_key":null}"#,
        ),
        (
            None,
            key(0x01),
            r#"{"escape_prefix":null,"take_control_key":1}"#,
        ),
    ] {
        let envelope = StartupEnvelope::Attach(settings(escape, take));
        let text = format!(r#"{{"schema":1,"operation":"attach","settings":{json}}}"#);
        assert_eq!(envelope.to_json(), text);
        assert_eq!(parse(&text).unwrap(), envelope);
    }
}

#[test]
fn unknown_optional_fields_and_whitespace_are_ignored() {
    let text = r#" { "future" : {"x":[1,null]}, "schema" : 1, "settings" : {"take_control_key":null,"escape_prefix":2,"later":"x"}, "operation" : "attach" } "#;
    assert_eq!(
        parse(text).unwrap(),
        StartupEnvelope::Attach(settings(key(0x02), None))
    );
    assert_eq!(
        parse(r#"{"operation":"kill","schema":1,"future":true}"#).unwrap(),
        StartupEnvelope::Kill
    );
}

#[test]
fn the_size_limit_is_exactly_4096_bytes() {
    let fits = format!("{KILL}{}", " ".repeat(MAX_ENVELOPE_LENGTH - KILL.len()));
    assert_eq!(fits.len(), 4096);
    assert_eq!(parse(&fits).unwrap(), StartupEnvelope::Kill);
    let over = format!("{fits} ");
    assert_eq!(
        rejected(&over),
        "invalid client startup envelope: larger than 4096 bytes"
    );
    let padded = format!(
        r#"{{"schema":1,"operation":"kill","pad":"{}"}}"#,
        "x".repeat(4096)
    );
    assert!(rejected(&padded).contains("larger than 4096 bytes"));
}

#[test]
fn malformed_json_and_invalid_utf8_are_errors() {
    for bad in [
        "",
        " ",
        "{",
        "null",
        "[1,\"kill\"]",
        "\"kill\"",
        r#"{"schema":1,"operation":"kill"}x"#,
        r#"{"schema":1,"operation":"kill",}"#,
        r#"{"schema":1,"operation":"kill","x":[1,]}"#,
        r#"{"schema":1,"operation":"kill","x":{"a":1,,}}"#,
        r#"{"schema":1,"operation":"kill"} {}"#,
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
    for bytes in [
        b"{\"schema\":1,\"operation\":\"kill\",\"x\":\"\xff\"}".as_slice(),
        b"{\"schema\":1,\"operation\":\"kill\"}\xff",
        b"\xef\xbb{}",
    ] {
        assert!(StartupEnvelope::parse(bytes).is_err(), "{bytes:?}");
    }
}

#[test]
fn schema_and_operation_are_required_and_exact() {
    for (bad, detail) in [
        (r#"{"operation":"kill"}"#, "missing field `schema`"),
        (r#"{"schema":1}"#, "missing field `operation`"),
        (r#"{"schema":2,"operation":"kill"}"#, "schema must be 1"),
        (r#"{"schema":0,"operation":"kill"}"#, "schema must be 1"),
        (r#"{"schema":"1","operation":"kill"}"#, "invalid type"),
        (r#"{"schema":1.0,"operation":"kill"}"#, "invalid type"),
        (r#"{"schema":-1,"operation":"kill"}"#, "invalid value"),
        (r#"{"schema":null,"operation":"kill"}"#, "invalid type"),
        (r#"{"schema":1,"operation":"Kill"}"#, "unknown variant"),
        (r#"{"schema":1,"operation":"detach"}"#, "unknown variant"),
        (r#"{"schema":1,"operation":" kill"}"#, "unknown variant"),
        (r#"{"schema":1,"operation":1}"#, "expected value"),
        (r#"{"schema":1,"operation":null}"#, "expected value"),
        (
            r#"{"schema":1,"schema":1,"operation":"kill"}"#,
            "duplicate field `schema`",
        ),
        (
            r#"{"schema":1,"operation":"kill","operation":"kill"}"#,
            "duplicate field `operation`",
        ),
    ] {
        let message = rejected(bad);
        assert!(
            message.starts_with("invalid client startup envelope: "),
            "{message}"
        );
        assert!(message.contains(detail), "{bad}: {message}");
    }
}

#[test]
fn kill_never_carries_settings_and_attach_always_does() {
    for bad in [
        r#"{"schema":1,"operation":"kill","settings":{"escape_prefix":29,"take_control_key":20}}"#,
        r#"{"schema":1,"operation":"kill","settings":{}}"#,
        r#"{"schema":1,"operation":"kill","settings":null}"#,
    ] {
        assert!(
            rejected(bad).contains("kill must not carry settings"),
            "{bad}"
        );
    }
    for bad in [
        r#"{"schema":1,"operation":"attach"}"#,
        r#"{"schema":1,"operation":"attach","settings":null}"#,
    ] {
        assert!(rejected(bad).contains("attach requires settings"), "{bad}");
    }
}

#[test]
fn attach_settings_need_both_fields_as_vocabulary_bytes_without_defaults() {
    let envelope =
        |settings: &str| format!(r#"{{"schema":1,"operation":"attach","settings":{settings}}}"#);
    for (bad, detail) in [
        ("{}", "missing escape_prefix"),
        (r#"{"escape_prefix":29}"#, "missing take_control_key"),
        (r#"{"take_control_key":20}"#, "missing escape_prefix"),
        (
            r#"{"escape_prefix":32,"take_control_key":20}"#,
            "escape_prefix is not a control key",
        ),
        (
            r#"{"escape_prefix":29,"take_control_key":127}"#,
            "take_control_key is not a control key",
        ),
        (
            r#"{"escape_prefix":29,"take_control_key":255}"#,
            "take_control_key is not a control key",
        ),
        (
            r#"{"escape_prefix":256,"take_control_key":20}"#,
            "invalid value",
        ),
        (
            r#"{"escape_prefix":-1,"take_control_key":20}"#,
            "invalid value",
        ),
        (
            r#"{"escape_prefix":1.5,"take_control_key":20}"#,
            "invalid type",
        ),
        (
            r#"{"escape_prefix":"Ctrl+]","take_control_key":20}"#,
            "invalid type",
        ),
        (
            r#"{"escape_prefix":true,"take_control_key":20}"#,
            "invalid type",
        ),
        (
            r#"{"escape_prefix":29,"take_control_key":27}"#,
            "take_control_key may not be Ctrl+[",
        ),
        (
            r#"{"escape_prefix":null,"take_control_key":27}"#,
            "take_control_key may not be Ctrl+[",
        ),
        (
            r#"{"escape_prefix":20,"take_control_key":20}"#,
            "escape_prefix and take_control_key are the same key",
        ),
        (
            r#"{"escape_prefix":29,"escape_prefix":29,"take_control_key":20}"#,
            "duplicate field `escape_prefix`",
        ),
        (
            r#"{"escape_prefix":29,"take_control_key":null,"take_control_key":null}"#,
            "duplicate field `take_control_key`",
        ),
        ("[29,20]", "expected a JSON object"),
        (r#""x""#, "expected a JSON object"),
    ] {
        let message = rejected(&envelope(bad));
        assert!(message.contains(detail), "{bad}: {message}");
    }
    let duplicate = r#"{"schema":1,"operation":"attach","settings":{"escape_prefix":1,"take_control_key":2},"settings":{"escape_prefix":1,"take_control_key":2}}"#;
    assert!(rejected(duplicate).contains("duplicate field `settings`"));
}
