use climon2_core::SessionId;
use climon2_platform::{peer, runtime};
use std::{fs, os::unix::fs::PermissionsExt};

/// #18: each canonical home gets a private production /tmp runtime and authenticated socket.
#[test]
fn production_runtime_isolates_homes_and_authenticates_real_sockets() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let first_home = runtime::canonical_home(first.path()).unwrap();
    let second_home = runtime::canonical_home(second.path()).unwrap();
    let first_runtime = runtime::RuntimeDirectory::prepare(&first_home).unwrap();
    let second_runtime = runtime::RuntimeDirectory::prepare(&second_home).unwrap();
    assert_ne!(first_runtime.path(), second_runtime.path());
    assert!(
        first_runtime
            .path()
            .to_str()
            .unwrap()
            .starts_with(&format!("/tmp/climon2-{}-", peer::effective_uid()))
    );
    assert_eq!(
        fs::symlink_metadata(first_runtime.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let id: SessionId = "12345678-1234-4234-8234-123456789abc".parse().unwrap();
    let path = first_runtime.socket_path(&id).unwrap();
    assert_eq!(
        path.file_name().unwrap(),
        "12345678-1234-4234-8234-123456789abc.sock"
    );
    assert!(path.as_os_str().as_encoded_bytes().len() <= 103);
    let listener = runtime::bind(&path).unwrap();
    let client = runtime::SocketStream::connect(&path).unwrap();
    let accepted = peer::accept(&listener).unwrap();
    assert_eq!(peer::uid(&accepted).unwrap(), peer::effective_uid());
    peer::authenticate(&client).unwrap();
    drop((accepted, client, listener));
    fs::remove_file(path).unwrap();
    assert_eq!(first_runtime.path().read_dir().unwrap().count(), 0);
    assert!(
        second_runtime.path().exists(),
        "runtime directories are never removed"
    );
}
