//! Spec #73 Entry and startup (Introduction preference): the monitor
//! preference file that keeps the key introduction's **Don't show again**
//! choice per climon2 home (HLP-01 Contract; derived scenario: the preference
//! file's versioning and malformed-file behaviour). Every expectation is
//! written here from the README's description of the file, never taken from
//! the code under test.
use climon2_core::{layout::HomeLayout, preferences::MonitorPreferences};
use std::path::Path;

fn parsed(text: &str) -> MonitorPreferences {
    MonitorPreferences::parse(text.as_bytes()).unwrap_or_else(|error| panic!("{text}: {error}"))
}

fn malformed(text: &str) -> String {
    match MonitorPreferences::parse(text.as_bytes()) {
        Ok(preferences) => panic!("{text} was read as {preferences:?}"),
        Err(error) => error.to_string(),
    }
}

/// HLP-01 (Contract): each climon2 home keeps its own preference file, beside
/// its `config.jsonc`.
#[test]
fn hlp01_each_home_keeps_its_own_preference_file() {
    let root = std::env::current_dir().unwrap();
    for home in [root.join("one home"), root.join("another")] {
        let layout = HomeLayout::resolve(Some(home.as_os_str()), Path::new("/")).unwrap();
        assert_eq!(
            layout.monitor_preferences(),
            home.join("monitor-preferences.json")
        );
    }
}

/// HLP-01 (Contract): a saved choice is version 1 of the schema, one line of
/// compact JSON.
#[test]
fn hlp01_a_saved_choice_is_one_line_of_version_1_json() {
    assert_eq!(
        MonitorPreferences::new(true).to_json(),
        b"{\"version\":1,\"suppress_key_introduction\":true}\n"
    );
    assert_eq!(
        MonitorPreferences::new(false).to_json(),
        b"{\"version\":1,\"suppress_key_introduction\":false}\n"
    );
}

/// HLP-01 (Contract): a version 1 file holds the choice, whatever its
/// whitespace or field order.
#[test]
fn hlp01_a_version_1_file_holds_the_dont_show_again_choice() {
    assert!(
        parsed(r#"{"version":1,"suppress_key_introduction":true}"#).suppresses_key_introduction()
    );
    assert!(
        !parsed(r#"{"version":1,"suppress_key_introduction":false}"#).suppresses_key_introduction()
    );
    assert!(
        parsed("{\n  \"suppress_key_introduction\": true,\n  \"version\": 1\n}\n")
            .suppresses_key_introduction()
    );
}

/// HLP-01 (Contract, across releases): the schema is additive, so every
/// release reads a later version's file like version 1, ignoring the fields
/// it does not know, up to the largest version number; one past it is
/// malformed.
#[test]
fn hlp01_later_versions_are_read_ignoring_unknown_fields() {
    for text in [
        r#"{"version":2,"suppress_key_introduction":true,"selector":{"columns":[1,2]},"note":null}"#,
        r#"{"future":[{"version":0}],"version":4294967295,"suppress_key_introduction":true}"#,
    ] {
        assert!(parsed(text).suppresses_key_introduction(), "{text}");
    }
    assert_eq!(
        malformed(r#"{"version":4294967296,"suppress_key_introduction":true}"#),
        "monitor-preferences.json is malformed: invalid value: integer `4294967296`, expected a nonzero u32 at line 1 column 21"
    );
}

/// Derived scenario (malformed preference file): anything but such an object
/// is malformed, and says why: broken JSON, another JSON type, a missing,
/// zero, negative, fractional or quoted version, a missing, null or
/// non-boolean choice, and a known field given twice.
#[test]
fn hlp01_malformed_files_are_refused_with_their_reason() {
    for (text, reason) in [
        ("", "EOF while parsing a value at line 1 column 0"),
        (
            r#"{"version":1,"suppress_key_introduction":true"#,
            "EOF while parsing an object at line 1 column 45",
        ),
        (
            r#"{"version":1,"suppress_key_introduction":true} x"#,
            "trailing characters at line 1 column 48",
        ),
        (
            "[true]",
            "invalid type: sequence, expected a JSON object at line 1 column 0",
        ),
        (
            "null",
            "invalid type: null, expected a JSON object at line 1 column 4",
        ),
        (
            r#"{"suppress_key_introduction":true}"#,
            "missing field `version` at line 1 column 34",
        ),
        (
            r#"{"version":0,"suppress_key_introduction":true}"#,
            "invalid value: integer `0`, expected a nonzero u32 at line 1 column 12",
        ),
        (
            r#"{"version":-1,"suppress_key_introduction":true}"#,
            "invalid value: integer `-1`, expected a nonzero u32 at line 1 column 13",
        ),
        (
            r#"{"version":1.5,"suppress_key_introduction":true}"#,
            "invalid type: floating point `1.5`, expected a nonzero u32 at line 1 column 14",
        ),
        (
            r#"{"version":"1","suppress_key_introduction":true}"#,
            "invalid type: string \"1\", expected a nonzero u32 at line 1 column 14",
        ),
        (
            r#"{"version":1}"#,
            "missing field `suppress_key_introduction` at line 1 column 13",
        ),
        (
            r#"{"version":1,"suppress_key_introduction":null}"#,
            "invalid type: null, expected a boolean at line 1 column 45",
        ),
        (
            r#"{"version":1,"suppress_key_introduction":"yes"}"#,
            "invalid type: string \"yes\", expected a boolean at line 1 column 46",
        ),
        (
            r#"{"version":1,"version":1,"suppress_key_introduction":true}"#,
            "duplicate field `version` at line 1 column 22",
        ),
    ] {
        assert_eq!(
            malformed(text),
            format!("monitor-preferences.json is malformed: {reason}"),
            "{text}"
        );
    }
}
