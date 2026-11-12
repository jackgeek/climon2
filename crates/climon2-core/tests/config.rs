//! Plan 1.3: `<home>/config.jsonc` parsing, validation scope and one-shot loading.
use climon2_core::{
    config::*,
    keys::{ClientSettings, ControlKey},
    layout::HomeLayout,
};
use std::{fs, path::PathBuf};

const SAMPLE: &str = r#"{
  "client": {
    "escape_prefix": "Ctrl+]",     // null disables escape interception
    "take_control_key": "Ctrl+T"   // null disables keyboard takeover
  },
  "scrollback": {
    "max_rows": 10000
  },
}
"#;

fn key(byte: u8) -> Option<ControlKey> {
    ControlKey::from_byte(byte)
}

fn exec(text: &str) -> Result<ExecConfig, ConfigError> {
    ExecConfig::parse(text.as_bytes())
}

fn attach(text: &str) -> Result<AttachConfig, ConfigError> {
    AttachConfig::parse(text.as_bytes())
}

fn bindings(settings: ClientSettings) -> (Option<ControlKey>, Option<ControlKey>) {
    (settings.escape_prefix(), settings.take_control_key())
}

fn rows(text: &str) -> u32 {
    exec(text).unwrap().scrollback.max_rows.get()
}

fn malformed(result: Result<impl std::fmt::Debug, ConfigError>) -> String {
    match result.unwrap_err() {
        ConfigError::Malformed(detail) => detail,
        other => panic!("expected malformed, got {other:?}"),
    }
}

fn invalid(result: Result<impl std::fmt::Debug, ConfigError>) -> String {
    match result.unwrap_err() {
        ConfigError::Invalid(detail) => detail,
        other => panic!("expected invalid, got {other:?}"),
    }
}

#[test]
fn documented_sample_with_comments_and_trailing_comma_is_the_default() {
    let config = exec(SAMPLE).unwrap();
    assert_eq!(bindings(config.client), (key(0x1D), key(0x14)));
    assert_eq!(config.scrollback.max_rows.get(), 10_000);
    assert_eq!(attach(SAMPLE).unwrap().client, config.client);
}

#[test]
fn missing_objects_and_fields_mean_defaults_and_null_disables_a_binding() {
    let defaults = ExecConfig::default();
    assert_eq!(bindings(defaults.client), (key(0x1D), key(0x14)));
    assert_eq!(defaults.scrollback.max_rows, MaxRows::default());
    assert_eq!(MaxRows::default().get(), 10_000);
    assert_eq!(AttachConfig::default().client, ClientSettings::default());
    for empty in [
        "{}",
        " {\"client\":{},\"scrollback\":{}} ",
        "{\"client\":{}}",
    ] {
        assert_eq!(exec(empty).unwrap(), defaults, "{empty}");
        assert_eq!(attach(empty).unwrap(), AttachConfig::default(), "{empty}");
    }
    for (text, expected) in [
        (r#"{"client":{"escape_prefix":null}}"#, (None, key(0x14))),
        (r#"{"client":{"take_control_key":null}}"#, (key(0x1D), None)),
        (
            r#"{"client":{"escape_prefix":null,"take_control_key":null}}"#,
            (None, None),
        ),
        (
            r#"{"client":{"escape_prefix":"Ctrl+[","take_control_key":"Ctrl+Space"}}"#,
            (key(0x1B), key(0x00)),
        ),
        (
            r#"{"client":{"escape_prefix":"Ctrl+\\"}}"#,
            (key(0x1C), key(0x14)),
        ),
    ] {
        assert_eq!(bindings(exec(text).unwrap().client), expected, "{text}");
        assert_eq!(bindings(attach(text).unwrap().client), expected, "{text}");
    }
}

#[test]
fn max_rows_accepts_only_integers_from_zero_to_one_hundred_thousand() {
    assert_eq!(rows(r#"{"scrollback":{"max_rows":0}}"#), 0);
    assert_eq!(rows(r#"{"scrollback":{"max_rows":1}}"#), 1);
    assert_eq!(rows(r#"{"scrollback":{"max_rows":100000}}"#), 100_000);
    assert_eq!(MaxRows::new(100_000).map(MaxRows::get), Some(100_000));
    assert_eq!(MaxRows::new(0).map(MaxRows::get), Some(0));
    assert_eq!(MaxRows::new(100_001), None);
    let detail = invalid(exec(r#"{"scrollback":{"max_rows":100001}}"#));
    assert!(
        detail.contains("max_rows must be an integer from 0 to 100000"),
        "{detail}"
    );
    for bad in [
        "-1",
        "1.5",
        "1e4",
        "\"10\"",
        "null",
        "true",
        "4294967296",
        "{}",
        "[]",
    ] {
        invalid(exec(&format!(r#"{{"scrollback":{{"max_rows":{bad}}}}}"#)));
    }
}

#[test]
fn comments_are_whitespace_between_tokens_and_never_inside_strings() {
    let text = "/*lead*/{/*a*/\"client\"/*b*/:/*c*/{//x\n\"escape_prefix\"//y\n:\"Ctrl+B\"/**/\n,\"take_control_key\":/***/\"Ctrl+C\"/* multi\n line */}}//end";
    assert_eq!(bindings(exec(text).unwrap().client), (key(0x02), key(0x03)));
    for preserved in [
        r#"{"x":"a // b /* c */ d","client":{"escape_prefix":"Ctrl+B"}}"#,
        r#"{"x":"\"// still a string","client":{"escape_prefix":"Ctrl+B"}}"#,
        r#"{"x":"\\"//comment after an escaped backslash
,"client":{"escape_prefix":"Ctrl+B"}}"#,
        r#"{"x":"/*","client":{"escape_prefix":"Ctrl+B"},"y":"*/"}"#,
    ] {
        assert_eq!(
            bindings(exec(preserved).unwrap().client).0,
            key(0x02),
            "{preserved}"
        );
    }
    let detail = invalid(exec(r#"{"client":{"escape_prefix":"Ctrl+// x /* y */"}}"#));
    assert!(
        detail.contains(r#"invalid key string "Ctrl+// x /* y */""#),
        "{detail}"
    );
}

#[test]
fn a_comment_never_joins_the_tokens_on_either_side() {
    for split in [
        r#"{"scrollback":{"max_rows":1/* c */0000}}"#,
        r#"{"scrollback":{"max_rows":1//c
0000}}"#,
        r#"{"x":tr/* c */ue}"#,
        r#"{"x":tr// c
ue}"#,
        "{\"x\":1}/*",
        "{}/*/",
        "{\"x\":1} /* unterminated *",
        "",
        "  // only a comment",
        "{\"client\":",
        "{\"client\":{}",
        "{\"x\":\"unterminated}",
        "{\"x\":1}}",
        "{\"x\":1} /",
        "{\"x\":/1}",
        "{,,}",
        "{\"x\":[1,,]}",
        "{\"x\":{\"y\":1,,}}",
        "{\"x\":[1 2]}",
    ] {
        malformed(exec(split));
        malformed(attach(split));
    }
    malformed(ExecConfig::parse(b"{\"x\":\"\xff\"}"));
}

#[test]
fn a_trailing_comma_is_one_followed_only_by_whitespace_or_comments_then_a_closer() {
    for text in [
        r#"{"client":{"escape_prefix":"Ctrl+B",},}"#,
        "{\"x\":[1,2,/*c*/],\"client\":{\"escape_prefix\":\"Ctrl+B\"\n,\n// last\n}\n,\n}",
        "{\"x\":[1,2,\t\r\n],\"client\":{\"escape_prefix\":\"Ctrl+B\",/**///\n}}",
        r#"{"x":"a,}","client":{"escape_prefix":"Ctrl+B",}}"#,
        r#"{"x":["a","b"],"y":{"a":"b"},"client":{"escape_prefix":"Ctrl+B"}}"#,
    ] {
        assert_eq!(bindings(exec(text).unwrap().client).0, key(0x02), "{text}");
    }
    let detail = invalid(exec(r#"{"client":{"escape_prefix":"Ctrl+,]",},}"#));
    assert!(detail.contains(r#""Ctrl+,]""#), "{detail}");
}

#[test]
fn known_objects_reject_duplicates_wrong_types_bad_keys_and_conflicts() {
    for (text, expected) in [
        (r#"{"client":{},"client":{}}"#, "duplicate field `client`"),
        (
            r#"{"client":{"escape_prefix":"Ctrl+A","escape_prefix":"Ctrl+B"}}"#,
            "duplicate field `escape_prefix`",
        ),
        (
            r#"{"client":{"take_control_key":null,"take_control_key":null}}"#,
            "duplicate field `take_control_key`",
        ),
        (r#"{"client":[]}"#, "expected a JSON object"),
        (
            r#"{"client":["Ctrl+B","Ctrl+C"]}"#,
            "expected a JSON object",
        ),
        (r#"{"client":null}"#, "expected a JSON object"),
        (r#"{"client":"Ctrl+B"}"#, "expected a JSON object"),
        (r#"{"client":{"escape_prefix":29}}"#, "invalid type"),
        (r#"{"client":{"escape_prefix":true}}"#, "invalid type"),
        (
            r#"{"client":{"take_control_key":"Ctrl+a"}}"#,
            r#"invalid key string "Ctrl+a""#,
        ),
        (
            r#"{"client":{"escape_prefix":"Alt+X"}}"#,
            r#"invalid key string "Alt+X""#,
        ),
        (
            r#"{"client":{"take_control_key":"Ctrl+["}}"#,
            "take_control_key may not be Ctrl+[",
        ),
        (
            r#"{"client":{"escape_prefix":"Ctrl+T"}}"#,
            "escape_prefix and take_control_key are the same key",
        ),
        (
            r#"{"client":{"escape_prefix":"Ctrl+A","take_control_key":"Ctrl+A"}}"#,
            "escape_prefix and take_control_key are the same key",
        ),
        (r#"[]"#, "expected a JSON object"),
        (r#"[{},{}]"#, "expected a JSON object"),
        (r#"null"#, "expected a JSON object"),
        (r#"7"#, "expected a JSON object"),
    ] {
        let detail = invalid(exec(text));
        assert!(detail.contains(expected), "{text}: {detail}");
        let detail = invalid(attach(text));
        assert!(detail.contains(expected), "{text}: {detail}");
    }
    for text in [
        r#"{"scrollback":{},"scrollback":{}}"#,
        r#"{"scrollback":{"max_rows":1,"max_rows":1}}"#,
        r#"{"scrollback":[1]}"#,
        r#"{"scrollback":null}"#,
    ] {
        invalid(exec(text));
    }
}

#[test]
fn attach_ignores_scrollback_and_every_operation_ignores_unknown_fields() {
    for text in [
        r#"{"scrollback":{"max_rows":100001}}"#,
        r#"{"scrollback":{"max_rows":"many"},"scrollback":{}}"#,
        r#"{"scrollback":null}"#,
        r#"{"scrollback":[1]}"#,
    ] {
        assert_eq!(attach(text).unwrap(), AttachConfig::default(), "{text}");
        invalid(exec(text));
    }
    let text = r#"{"future":{"x":[1,{"y":null}]},"future":1,"client":{"other":"Ctrl+?","other":2},"scrollback":{"max_bytes":"huge","max_rows":5}}"#;
    assert_eq!(rows(text), 5);
    assert_eq!(attach(text).unwrap(), AttachConfig::default());
}

#[test]
fn errors_name_their_class_for_the_one_line_diagnostic() {
    assert_eq!(
        ConfigError::Malformed("x".into()).to_string(),
        "config.jsonc is malformed: x"
    );
    assert_eq!(
        ConfigError::Invalid("y".into()).to_string(),
        "config.jsonc is invalid: y"
    );
    assert_eq!(
        ConfigError::Unreadable("z".into()).to_string(),
        "cannot read config.jsonc: z"
    );
    let detail = malformed(exec("{\"x\":tr/* c */ue}"));
    assert!(detail.contains("line 1"), "{detail}");
    assert!(malformed(exec("{} /*")).contains("unterminated comment"));
}

fn home(name: &str) -> HomeLayout {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("config-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    HomeLayout::resolve(Some(root.as_os_str()), &root).unwrap()
}

#[test]
fn source_is_read_once_from_the_home_and_missing_means_defaults() {
    let home = home("load");
    assert_eq!(home.config(), home.root().join("config.jsonc"));
    let missing = ConfigSource::read(&home).unwrap();
    assert_eq!(missing.exec().unwrap(), ExecConfig::default());
    assert_eq!(missing.attach().unwrap(), AttachConfig::default());

    fs::write(
        home.config(),
        r#"{"client":{"escape_prefix":"Ctrl+B"},"scrollback":{"max_rows":7}}"#,
    )
    .unwrap();
    let source = ConfigSource::read(&home).unwrap();
    fs::remove_file(home.config()).unwrap();
    let config = source.exec().unwrap();
    assert_eq!(bindings(config.client), (key(0x02), key(0x14)));
    assert_eq!(config.scrollback.max_rows.get(), 7);
    assert_eq!(source.attach().unwrap().client, config.client);

    fs::write(home.config(), r#"{"scrollback":{"max_rows":-1}}"#).unwrap();
    let source = ConfigSource::read(&home).unwrap();
    invalid(source.exec());
    assert_eq!(source.attach().unwrap(), AttachConfig::default());
    fs::write(home.config(), "{").unwrap();
    malformed(ConfigSource::read(&home).unwrap().attach());
    fs::remove_dir_all(home.root()).unwrap();
}

#[test]
fn an_existing_unreadable_file_is_an_error_not_defaults() {
    let home = home("unreadable");
    fs::create_dir(home.config()).unwrap();
    let error = ConfigSource::read(&home).unwrap_err();
    assert!(matches!(error, ConfigError::Unreadable(_)), "{error:?}");
    fs::remove_dir_all(home.root()).unwrap();
}
