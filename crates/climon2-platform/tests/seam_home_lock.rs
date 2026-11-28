//! The home lock seam's contract suite (#499): one holder at a time, a
//! later taker told what the holder wrote, and a lock that goes when its
//! holder lets go. The operating system lets go for a holder that dies.
#![cfg(unix)]

use climon2_platform::home_lock::{self, Taken};
use std::{
    fs, io,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
};

/// A lock file's path inside a fresh folder, with the process creating
/// files as permissively as a umask allows.
fn folder() -> (tempfile::TempDir, PathBuf) {
    // SAFETY: umask only swaps the process's file mode mask.
    unsafe { libc::umask(0) };
    let folder = tempfile::tempdir().expect("a folder");
    let path = folder.path().join("relay.lock");
    (folder, path)
}

fn held(taken: io::Result<Taken>) -> home_lock::Held {
    match taken.expect("the lock file opens") {
        Taken::Held(held) => held,
        Taken::Busy(note) => panic!("the lock is held, noted {note:?}"),
    }
}

fn busy(taken: io::Result<Taken>) -> String {
    match taken.expect("the lock file opens") {
        Taken::Busy(note) => note,
        Taken::Held(_) => panic!("a second holder took the lock"),
    }
}

#[test]
fn the_first_taker_holds_the_lock_in_an_owner_only_file() {
    let (_folder, path) = folder();
    let _held = held(home_lock::take(&path));
    let mode = fs::symlink_metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o7777, 0o600);
}

#[test]
fn a_second_taker_is_told_what_the_holder_noted() {
    let (_folder, path) = folder();
    let mut first = held(home_lock::take(&path));
    assert_eq!(busy(home_lock::take(&path)), "");
    first.note("pid 41, listening on 127.0.0.1:7222").unwrap();
    assert_eq!(
        busy(home_lock::take(&path)),
        "pid 41, listening on 127.0.0.1:7222"
    );
    // A shorter note replaces a longer one whole.
    first.note("pid 41").unwrap();
    assert_eq!(busy(home_lock::take(&path)), "pid 41");
}

#[test]
fn the_lock_is_free_once_its_holder_lets_go() {
    let (_folder, path) = folder();
    let mut first = held(home_lock::take(&path));
    first.note("the first holder").unwrap();
    drop(first);
    let _second = held(home_lock::take(&path));
    // The last holder's note is gone with it.
    assert_eq!(busy(home_lock::take(&path)), "");
    assert_eq!(fs::read(&path).unwrap(), b"");
}

#[test]
fn a_lock_file_is_never_opened_through_a_link() {
    let (folder, path) = folder();
    let elsewhere = folder.path().join("elsewhere");
    fs::write(&elsewhere, b"kept").unwrap();
    symlink(&elsewhere, &path).unwrap();
    assert!(home_lock::take(&path).is_err());
    assert_eq!(fs::read(&elsewhere).unwrap(), b"kept");
}

#[test]
fn a_lock_in_a_missing_folder_is_an_error() {
    let (folder, _) = folder();
    let error = home_lock::take(&folder.path().join("missing/relay.lock")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

/// A note is one line of text: what a later taker reads back is exactly it.
#[test]
fn a_note_that_is_not_text_reads_as_no_note() {
    let (_folder, path) = folder();
    let _first = held(home_lock::take(&path));
    fs::write(&path, [0xff, 0xfe]).unwrap();
    assert_eq!(busy(home_lock::take(&path)), "");
}
