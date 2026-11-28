use monitor_selector_prototype::config::Settings;

#[test]
fn jsonc_startup_defaults_and_boolean_settings() {
    assert!(Settings::parse(b"{}").unwrap().bell);
    assert!(Settings::parse(b"{\"monitor\":{}}").unwrap().bell);
    let config = Settings::parse(
        br#"{
        // Isolated prototype snapshot.
        "monitor": {"bell": false, "show_key_hints": false,},
        "unrelated": {"arbitrary": true,},
    }"#,
    )
    .unwrap();
    assert_eq!(
        config,
        Settings {
            bell: false,
            show_key_hints: false
        }
    );
}

#[test]
fn wrong_types_null_duplicate_known_fields_and_malformed_jsonc_are_errors() {
    for text in [
        "[]",
        "null",
        "{\"monitor\": []}",
        "{\"monitor\": null}",
        "{\"monitor\":{\"bell\":null}}",
        "{\"monitor\":{\"bell\":\"false\"}}",
        "{\"monitor\":{\"bell\":true,\"bell\":false}}",
        "{\"monitor\":{},\"monitor\":{}}",
        "{\"monitor\":{\"show_key_hints\":1}}",
        "{\"monitor\":{\"show_key_hints\":true,\"show_key_hints\":false}}",
        "{\"monitor\":{\"bell\":tru/*split token*/e}}",
    ] {
        assert!(Settings::parse(text.as_bytes()).is_err(), "{text}");
    }
}

#[test]
fn invalid_config_fails_before_entering_the_terminal() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.jsonc");
    std::fs::write(&path, r#"{"monitor":{"bell":null}}"#).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_monitor-selector-prototype"))
        .args(["--config", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("invalid type")
    );
    assert!(output.stdout.is_empty());
}
