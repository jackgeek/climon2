#![cfg(target_os = "macos")]

use climon2_platform::{peer, runtime};
use std::{
    fs,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        net::UnixStream,
    },
};

fn directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(".rt")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap()
}

#[test]
fn issue18_private_directory_refuses_symlink_file_and_permissive_mode_without_repair() {
    let root = directory();
    let private = root.path().join("private");
    runtime::private_directory(&private).unwrap();
    assert_eq!(
        fs::metadata(&private).unwrap().permissions().mode() & 0o777,
        0o700
    );
    runtime::private_directory(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(runtime::private_directory(&private).is_err());
    assert_eq!(
        fs::metadata(&private).unwrap().permissions().mode() & 0o777,
        0o750
    );
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&private, &link).unwrap();
    assert!(runtime::private_directory(&link).is_err());
    let file = root.path().join("file");
    fs::write(&file, b"").unwrap();
    assert!(runtime::private_directory(&file).is_err());
}

#[test]
fn issue18_home_is_created_then_canonicalized_and_socket_is_private_and_authenticated() {
    let root = directory();
    let home = runtime::canonical_home(&root.path().join("home")).unwrap();
    assert!(home.is_absolute());
    let path = root.path().join("socket");
    let listener = runtime::bind(&path).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let client = UnixStream::connect(&path).unwrap();
    let server = peer::accept(&listener).unwrap();
    peer::authenticate(&server).unwrap();
    assert_eq!(peer::uid(&server).unwrap(), peer::effective_uid());
    assert!(peer::authenticate_uid(&server, peer::effective_uid() + 1).is_err());
    drop(client);
    assert!(runtime::bind(&root.path().join("x".repeat(104))).is_err());
}

#[test]
fn issue18_private_directory_refuses_existing_root_owned_directory_without_repair() {
    let path = std::path::Path::new("/private");
    let before = fs::symlink_metadata(path).unwrap();
    assert!(before.is_dir());
    assert_eq!(before.uid(), 0);
    assert_ne!(before.uid(), peer::effective_uid());

    let error = runtime::private_directory(path).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("wrong owner"));

    let after = fs::symlink_metadata(path).unwrap();
    assert_eq!(
        (
            after.dev(),
            after.ino(),
            after.uid(),
            after.gid(),
            after.mode()
        ),
        (
            before.dev(),
            before.ino(),
            before.uid(),
            before.gid(),
            before.mode()
        )
    );
}
