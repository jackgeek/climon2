//! ADR 0005 / #26: append-only compatibility fixtures and pure metadata boundary.
use climon2_core::metadata::SessionMetadata;
use std::path::Path;

const FILE: &str = "550e8400-e29b-41d4-a716-446655440000.pty";
const V1: &[u8] = include_bytes!("goldens/metadata-v1.json");

#[test]
fn adr0005_every_checked_in_golden_is_readable_by_the_current_reader() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|part| part.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let fields: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let filename = format!("{}.pty", fields["session_id"].as_str().unwrap());
        SessionMetadata::from_json(Path::new(&filename), &bytes).unwrap();
    }
}

#[test]
fn adr0005_metadata_paths_are_absolute_cross_os_display_strings_not_native_paths() {
    use climon2_core::metadata::MetadataPath;
    for path in [
        "/",
        "/a/b",
        r"C:\Users\example",
        "C:/",
        "z:/example",
        r"\\server\share\path",
    ] {
        assert_eq!(path.parse::<MetadataPath>().unwrap().as_str(), path);
    }
    for bad in [
        "",
        "relative",
        r"C:relative",
        r"\relative",
        "1:/path",
        r"\\server",
        r"\\server\",
        "\0/absolute",
        "/absolute\0",
    ] {
        assert!(bad.parse::<MetadataPath>().is_err(), "{bad:?}");
    }
}

#[test]
fn adr0005_metadata_requires_a_json_object_not_a_positional_array() {
    let record: serde_json::Value = serde_json::from_slice(V1).unwrap();
    let positional = serde_json::json!([
        record["format_version"],
        record["session_id"],
        record["socket_path"],
        record["daemon_pid"],
        record["daemon_start_time"],
        record["command"],
        record["args"],
        record["cwd"],
        record["depth"],
        record["created_at"],
        record["climon2_version"]
    ]);
    assert!(
        SessionMetadata::from_json(Path::new(FILE), &serde_json::to_vec(&positional).unwrap())
            .is_err()
    );
}

#[test]
fn adr0005_frozen_fields_reject_missing_wrong_types_invalid_values_and_filename_mismatch() {
    let original: serde_json::Value = serde_json::from_slice(V1).unwrap();
    for field in original.as_object().unwrap().keys() {
        let mut missing = original.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            SessionMetadata::from_json(Path::new(FILE), &serde_json::to_vec(&missing).unwrap())
                .is_err(),
            "{field}"
        );
        let mut mistyped = original.clone();
        mistyped[field] = serde_json::Value::Bool(true);
        assert!(
            SessionMetadata::from_json(Path::new(FILE), &serde_json::to_vec(&mistyped).unwrap())
                .is_err(),
            "{field}"
        );
    }
    for (field, invalid) in [
        ("format_version", serde_json::json!(0)),
        ("format_version", serde_json::json!(4_294_967_296_u64)),
        ("depth", serde_json::json!(0)),
        ("depth", serde_json::json!(-1)),
        ("daemon_pid", serde_json::json!(4_294_967_296_u64)),
        ("daemon_start_time", serde_json::json!(-1)),
        ("socket_path", serde_json::json!("relative.sock")),
        ("cwd", serde_json::json!("")),
        (
            "session_id",
            serde_json::json!("550E8400-E29B-41D4-A716-446655440000"),
        ),
        ("climon2_version", serde_json::json!("../0.1.0")),
        ("created_at", serde_json::json!("2026-10-28T07:21:02Z")),
        ("args", serde_json::json!(["valid", 3])),
    ] {
        let mut invalid_record = original.clone();
        invalid_record[field] = invalid;
        assert!(
            SessionMetadata::from_json(
                Path::new(FILE),
                &serde_json::to_vec(&invalid_record).unwrap()
            )
            .is_err(),
            "{field}"
        );
    }
    for filename in [
        "550e8400-e29b-41d4-a716-446655440001.pty",
        "550e8400-e29b-41d4-a716-446655440000.json",
        "not-a-guid.pty",
        "550e8400-e29b-41d4-a716-446655440000.pty.new",
    ] {
        assert!(SessionMetadata::from_json(Path::new(filename), V1).is_err());
    }
    assert!(SessionMetadata::from_json(Path::new(FILE), b"{").is_err());
}

#[test]
fn adr0005_display_is_lossy_only_and_never_used_as_runtime_argv() {
    use climon2_core::metadata::DisplayCommand;
    use std::{ffi::OsString, path::PathBuf};
    let command = OsString::from("command with spaces");
    let args = vec![OsString::from("a\nb"), OsString::from("日本語")];
    let cwd = std::env::current_dir().unwrap().join("some directory");
    let display = DisplayCommand::from_runtime(&command, &args, &cwd).unwrap();
    assert_eq!(display.command, "command with spaces");
    assert_eq!(display.args, ["a\nb", "日本語"]);
    assert_eq!(PathBuf::from(display.cwd.as_str()), cwd);
    assert_eq!(args[1], OsString::from("日本語"));
    assert!(DisplayCommand::from_runtime(&command, &args, Path::new("relative")).is_err());
}

#[test]
fn adr0005_append_only_goldens_include_future999_and_unknown_fields() {
    for (bytes, version, depth) in [
        (V1, 1, 1),
        (
            include_bytes!("goldens/metadata-v999.json").as_slice(),
            999,
            13,
        ),
    ] {
        let metadata = SessionMetadata::from_json(Path::new(FILE), bytes).unwrap();
        assert_eq!(metadata.format_version.get(), version);
        assert_eq!(metadata.depth.get(), depth);
        let written = metadata.to_json().unwrap();
        let object: serde_json::Value = serde_json::from_slice(&written).unwrap();
        let fields = object.as_object().unwrap();
        assert_eq!(fields.len(), 11);
        assert!(fields["daemon_start_time"].is_u64());
        assert_eq!(fields["args"].as_array().unwrap().len(), 2);
        assert_eq!(
            SessionMetadata::from_json(Path::new(FILE), &written).unwrap(),
            metadata
        );
    }
}
