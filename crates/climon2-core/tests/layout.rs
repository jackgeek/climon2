//! #17/#18/#28 and ADRs 0001/0002: pure native-path install/home contracts.
use climon2_core::layout::*;
use std::{ffi::OsStr, path::Path};

#[test]
fn issue17_home_defaults_only_when_unset_and_never_accepts_empty_or_relative() {
    let user = std::env::current_dir().unwrap().join("user");
    let default = HomeLayout::resolve(None, &user).unwrap();
    assert_eq!(default.root(), user.join(".climon2"));
    let explicit = user.join("custom home");
    let home = HomeLayout::resolve(Some(explicit.as_os_str()), Path::new("ignored")).unwrap();
    assert_eq!(home.root(), explicit);
    assert_eq!(home.general_logs(), explicit.join("logs/general"));
    for bad in ["", ".", "relative", "~/.climon2"] {
        assert!(HomeLayout::resolve(Some(OsStr::new(bad)), &user).is_err());
    }
    assert!(HomeLayout::resolve(None, Path::new("relative")).is_err());
    let id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    assert_eq!(
        home.metadata(&id),
        explicit.join("sessions/550e8400-e29b-41d4-a716-446655440000.pty")
    );
    assert_eq!(
        home.session_logs(&id),
        explicit.join("logs/550e8400-e29b-41d4-a716-446655440000")
    );
}

#[test]
fn adr0001_version_matched_client_has_exact_frozen_invocation() {
    let root = std::env::current_dir().unwrap().join("install");
    let layout = ExecutableLayout::from_cli_path(&root.join("2.0.0/climon2-cli")).unwrap();
    assert_eq!(layout.root(), root);
    assert_eq!(layout.version().as_str(), "2.0.0");
    let metadata = root.join("home/sessions/550e8400-e29b-41d4-a716-446655440000.pty");
    let invocation = layout
        .client_invocation(&"1.0.0".parse().unwrap(), &metadata)
        .unwrap();
    assert_eq!(invocation.program, root.join("1.0.0/climon2-client"));
    assert_eq!(
        invocation.args,
        [
            OsStr::new("attach"),
            OsStr::new("--metadata"),
            metadata.as_os_str()
        ]
    );
    assert!(
        layout
            .client_invocation(&"1.0.0".parse().unwrap(), Path::new("relative.pty"))
            .is_err()
    );
    for bad in [
        "../climon2-cli",
        "not-semver/climon2-cli",
        "1.0.0/other",
        "1.0.0/../climon2-cli",
    ] {
        assert!(
            ExecutableLayout::from_cli_path(&root.join(bad)).is_err(),
            "{bad}"
        );
    }
    let filesystem_root = root.ancestors().last().unwrap();
    assert!(
        ExecutableLayout::from_cli_path(&filesystem_root.join("climon2-cli")).is_err(),
        "#18: a CLI directly under the filesystem root has no version folder"
    );
    for content in ["1.2.3", "1.2.3\n", "1.2.3\r\n"] {
        assert_eq!(parse_current(content).unwrap().as_str(), "1.2.3");
    }
    for bad in ["", "1.2.3\n\n", "1.2.3\n2.0.0", " ../1.2.3", "/1.2.3"] {
        assert!(parse_current(bad).is_err(), "{bad}");
    }
    let windows = ExecutableLayout::from_cli_path(&root.join("1.2.3/climon2-cli.exe")).unwrap();
    assert_eq!(
        windows.binary(windows.version(), VersionBinary::Cli),
        root.join("1.2.3/climon2-cli.exe")
    );
    assert_eq!(
        windows.binary(windows.version(), VersionBinary::Daemon),
        root.join("1.2.3/climon2-daemon.exe")
    );
}

#[test]
fn issue18_runtime_hash_is_sha256_of_exact_canonical_home_and_socket_is_bounded() {
    assert_eq!(home_hash8(b"/home/example/.climon2"), "64c3a98c");
    let root = std::env::current_dir().unwrap();
    let runtime = RuntimeLayout::new(&root, 501, &root.join("home")).unwrap();
    let id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    let socket = runtime.socket_path(&id, 10_000).unwrap();
    assert_eq!(
        socket.file_name().unwrap(),
        "550e8400-e29b-41d4-a716-446655440000.sock"
    );
    assert!(
        runtime
            .directory()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("climon2-501-")
    );
    let length = socket.as_os_str().as_encoded_bytes().len();
    assert_eq!(runtime.socket_path(&id, length).unwrap(), socket);
    assert!(runtime.socket_path(&id, length - 1).is_err());
    assert!(RuntimeLayout::new(Path::new("relative"), 501, &root).is_err());
}
