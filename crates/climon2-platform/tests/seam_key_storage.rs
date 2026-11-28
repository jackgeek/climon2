//! The key storage seam's contract suite (ONE-03): every platform's key
//! storage passes these by itself, with no link code and no second machine.
//! Key files are owner-only, writes are atomic and a damaged file is an
//! error.
#![cfg(unix)]

use climon2_platform::key_storage::KeyStore;
use std::{
    fs, io,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    thread,
};

const IDENTITY: &str = "relay-identity.json";

/// A links folder inside a fresh climon2 home, with the process creating
/// files as permissively as a umask allows: every suite in this file must
/// hold whatever the umask.
fn links() -> (tempfile::TempDir, KeyStore) {
    // SAFETY: umask only swaps the process's file mode mask.
    unsafe { libc::umask(0) };
    let home = tempfile::tempdir().expect("a climon2 home");
    let store = KeyStore::open(&home.path().join("links")).expect("the links folder opens");
    (home, store)
}

fn folder(home: &tempfile::TempDir) -> std::path::PathBuf {
    home.path().join("links")
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path)
        .expect("the path exists")
        .permissions()
        .mode()
        & 0o7777
}

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("the folder lists")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .into_string()
                .expect("a name")
        })
        .collect();
    names.sort();
    names
}

fn damage(result: io::Result<Option<Vec<u8>>>) -> String {
    let error = result.expect_err("a damaged key file is an error, never contents or absence");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
    error.to_string()
}

#[test]
fn opening_creates_an_owner_only_folder() {
    let (home, _store) = links();

    assert!(folder(&home).is_dir());
    assert_eq!(mode(&folder(&home)), 0o700);
}

#[test]
fn opening_refuses_a_folder_others_can_reach_and_leaves_it_alone() {
    let home = tempfile::tempdir().expect("a climon2 home");
    let links = home.path().join("links");
    fs::create_dir(&links).expect("a folder made by hand");
    fs::set_permissions(&links, fs::Permissions::from_mode(0o750)).expect("group access");

    let error = KeyStore::open(&links).expect_err("the folder is refused");

    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(mode(&links), 0o750);
}

#[test]
fn stored_contents_load_back_from_an_owner_only_file() {
    let (home, store) = links();

    store
        .store(IDENTITY, b"secret key")
        .expect("the key stores");

    assert_eq!(
        store.load(IDENTITY).expect("the key loads"),
        Some(b"secret key".to_vec())
    );
    assert_eq!(mode(&folder(&home).join(IDENTITY)), 0o600);
    assert_eq!(names(&folder(&home)), [IDENTITY]);
}

#[test]
fn a_reopened_folder_still_holds_what_was_stored() {
    let (home, store) = links();
    store
        .store(IDENTITY, b"secret key")
        .expect("the key stores");

    let reopened = KeyStore::open(&folder(&home)).expect("the folder reopens");

    assert_eq!(
        reopened.load(IDENTITY).expect("the key loads"),
        Some(b"secret key".to_vec())
    );
}

#[test]
fn a_file_never_stored_is_absent_not_an_error() {
    let (_home, store) = links();
    store
        .store("relays.json", b"other")
        .expect("another file stores");

    assert_eq!(store.load(IDENTITY).expect("absence is not damage"), None);
}

#[test]
fn storing_again_replaces_the_contents_and_leaves_nothing_else_behind() {
    let (home, store) = links();
    store
        .store(IDENTITY, b"first, and longer")
        .expect("the first stores");

    store.store(IDENTITY, b"second").expect("the second stores");

    assert_eq!(
        store.load(IDENTITY).expect("the key loads"),
        Some(b"second".to_vec())
    );
    assert_eq!(names(&folder(&home)), [IDENTITY]);
}

#[test]
fn storing_replaces_a_file_others_could_read_with_an_owner_only_one() {
    let (home, store) = links();
    let path = folder(&home).join(IDENTITY);
    fs::write(&path, b"exposed").expect("a file made by hand");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("world readable");

    store
        .store(IDENTITY, b"secret key")
        .expect("the key stores");

    assert_eq!(mode(&path), 0o600);
    assert_eq!(
        store.load(IDENTITY).expect("the key loads"),
        Some(b"secret key".to_vec())
    );
}

#[test]
fn storing_replaces_a_link_and_never_writes_through_it() {
    let (home, store) = links();
    let elsewhere = home.path().join("elsewhere");
    fs::write(&elsewhere, b"untouched").expect("a file outside the folder");
    symlink(&elsewhere, folder(&home).join(IDENTITY)).expect("a link at the key's name");

    store
        .store(IDENTITY, b"secret key")
        .expect("the key stores");

    assert_eq!(
        fs::read(&elsewhere).expect("the outside file reads"),
        b"untouched"
    );
    assert!(!folder(&home).join(IDENTITY).is_symlink());
    assert_eq!(
        store.load(IDENTITY).expect("the key loads"),
        Some(b"secret key".to_vec())
    );
}

/// The atomic write: a reader racing a writer sees one whole stored value
/// or the other, never a mixture, a prefix, an empty file or an absence.
#[test]
fn a_reader_racing_a_writer_only_ever_sees_a_whole_stored_value() {
    let (_home, store) = links();
    let old = vec![b'a'; 300_000];
    let new = vec![b'b'; 700_000];
    store.store(IDENTITY, &old).expect("the first stores");
    let done = AtomicBool::new(false);

    thread::scope(|scope| {
        let reader = scope.spawn(|| {
            let mut reads = 0;
            while !done.load(Ordering::Relaxed) || reads < 10 {
                let seen = store
                    .load(IDENTITY)
                    .expect("no read finds damage")
                    .expect("no read finds the file missing");
                assert!(
                    seen == old || seen == new,
                    "a torn read of {} bytes",
                    seen.len()
                );
                reads += 1;
            }
        });
        for round in 0..200 {
            let contents = if round % 2 == 0 { &new } else { &old };
            store.store(IDENTITY, contents).expect("each write stores");
        }
        done.store(true, Ordering::Relaxed);
        reader.join().expect("the reader saw only whole values");
    });
}

#[test]
fn a_write_that_cannot_be_published_fails_and_leaves_the_folder_as_it_was() {
    let (home, store) = links();
    store
        .store("relays.json", b"kept")
        .expect("another file stores");
    fs::create_dir(folder(&home).join(IDENTITY)).expect("a folder at the key's name");
    fs::write(folder(&home).join(IDENTITY).join("inside"), b"x").expect("a file inside it");

    store
        .store(IDENTITY, b"secret key")
        .expect_err("a folder cannot be replaced by a file");

    assert_eq!(names(&folder(&home)), [IDENTITY, "relays.json"]);
    assert_eq!(
        store.load("relays.json").expect("the other file loads"),
        Some(b"kept".to_vec())
    );
}

#[test]
fn empty_contents_are_refused_and_the_stored_value_stays() {
    let (_home, store) = links();
    store
        .store(IDENTITY, b"secret key")
        .expect("the key stores");

    let error = store
        .store(IDENTITY, b"")
        .expect_err("nothing is not a key");

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(
        store.load(IDENTITY).expect("the key loads"),
        Some(b"secret key".to_vec())
    );
}

#[test]
fn an_empty_file_is_damaged() {
    let (home, store) = links();
    let path = folder(&home).join(IDENTITY);
    fs::write(&path, b"").expect("a truncated file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("owner only");

    let message = damage(store.load(IDENTITY));

    assert!(message.contains("empty"), "{message}");
    assert!(message.contains(IDENTITY), "{message}");
}

#[test]
fn a_file_others_can_reach_is_damaged() {
    for loose in [0o640, 0o604, 0o620, 0o602, 0o610, 0o601] {
        let (home, store) = links();
        let path = folder(&home).join(IDENTITY);
        fs::write(&path, b"secret key").expect("a file made by hand");
        fs::set_permissions(&path, fs::Permissions::from_mode(loose)).expect("loose permissions");

        let message = damage(store.load(IDENTITY));

        assert!(
            message.contains("group or other permissions"),
            "{loose:o}: {message}"
        );
    }
}

#[test]
fn a_link_where_a_key_file_belongs_is_damaged() {
    let (home, store) = links();
    let elsewhere = home.path().join("elsewhere");
    fs::write(&elsewhere, b"secret key").expect("a file outside the folder");
    fs::set_permissions(&elsewhere, fs::Permissions::from_mode(0o600)).expect("owner only");
    symlink(&elsewhere, folder(&home).join(IDENTITY)).expect("a link at the key's name");

    damage(store.load(IDENTITY));
}

#[test]
fn a_dangling_link_where_a_key_file_belongs_is_damaged_not_absent() {
    let (home, store) = links();
    symlink(home.path().join("nowhere"), folder(&home).join(IDENTITY)).expect("a dangling link");

    damage(store.load(IDENTITY));
}

#[test]
fn a_folder_where_a_key_file_belongs_is_damaged() {
    let (home, store) = links();
    fs::create_dir(folder(&home).join(IDENTITY)).expect("a folder at the key's name");
    fs::set_permissions(
        folder(&home).join(IDENTITY),
        fs::Permissions::from_mode(0o700),
    )
    .expect("owner only");

    let message = damage(store.load(IDENTITY));

    assert!(message.contains("not a regular file"), "{message}");
}

#[test]
fn a_file_that_cannot_be_read_is_damaged() {
    // SAFETY: geteuid only reads the process's identity.
    if unsafe { libc::geteuid() } == 0 {
        // Root reads any file, so the permission cannot make this one unreadable.
        return;
    }
    let (home, store) = links();
    let path = folder(&home).join(IDENTITY);
    fs::write(&path, b"secret key").expect("a file made by hand");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("no access");

    damage(store.load(IDENTITY));
}

#[test]
fn only_a_plain_file_name_names_a_key_file() {
    let (home, store) = links();
    fs::write(home.path().join("outside"), b"outside").expect("a file beside the folder");

    for name in [
        "",
        ".",
        "..",
        "../outside",
        "sub/key",
        "/etc/hostname",
        "key/",
        "./key",
    ] {
        let stored = store.store(name, b"secret key").expect_err(name);
        let loaded = store.load(name).expect_err(name);

        assert_eq!(stored.kind(), io::ErrorKind::InvalidInput, "store {name:?}");
        assert_eq!(loaded.kind(), io::ErrorKind::InvalidInput, "load {name:?}");
    }
    assert_eq!(
        fs::read(home.path().join("outside")).expect("it reads"),
        b"outside"
    );
    assert_eq!(names(&folder(&home)), [] as [&str; 0]);
}
