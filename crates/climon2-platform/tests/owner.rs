//! #133 / ADR 0011: one climon2 home, one host. The first check claims a home
//! for its host; every later one passes only on that host. Origins are
//! injected, so each case runs on every platform.
use climon2_core::{
    metadata::SessionOrigin,
    owner::{OWNER_FILE, Refusal},
};
use climon2_platform::owner::{check, check_as};
use std::{fs, io, path::Path, sync::Barrier};

fn origin(machine: Option<&str>, pid_namespace: Option<u64>) -> SessionOrigin {
    SessionOrigin {
        machine_id: machine.map(|machine| machine.parse().unwrap()),
        pid_namespace,
    }
}

fn here() -> SessionOrigin {
    origin(Some("aaaa"), Some(1))
}

fn record(home: &Path) -> String {
    fs::read_to_string(home.join(OWNER_FILE)).unwrap()
}

/// Every name in the home: a claim leaves only its record.
fn names(home: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(home)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn unreadable_origin() -> io::Result<SessionOrigin> {
    Err(io::Error::from_raw_os_error(2))
}

/// The first check claims the home for its host, and that host passes every
/// later check. A host without a stable machine ID in the same PID namespace
/// is the same host.
#[test]
fn issue133_the_first_check_claims_the_home_and_its_host_passes() {
    let home = tempfile::tempdir().unwrap();
    check_as(home.path(), Ok(here())).unwrap();
    assert_eq!(
        record(home.path()),
        r#"{"machine_id":"aaaa","pid_namespace":1}"#
    );
    assert_eq!(names(home.path()), [OWNER_FILE]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(home.path().join(OWNER_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    check_as(home.path(), Ok(here())).unwrap();
    check_as(home.path(), Ok(origin(None, Some(1)))).unwrap();
    assert_eq!(
        record(home.path()),
        r#"{"machine_id":"aaaa","pid_namespace":1}"#
    );
}

/// Another machine, or another PID namespace of this one, is refused with the
/// owner it found, and the record stays as it was.
#[test]
fn issue133_another_machine_or_pid_namespace_is_refused() {
    let home = tempfile::tempdir().unwrap();
    check_as(home.path(), Ok(here())).unwrap();
    for other in [
        origin(Some("bbbb"), Some(1)),
        origin(Some("aaaa"), Some(2)),
        origin(None, Some(2)),
        origin(Some("aaaa"), None),
    ] {
        let Err(Refusal::Foreign(owner)) = check_as(home.path(), Ok(other.clone())) else {
            panic!("{other:?} passed");
        };
        assert_eq!(owner, here(), "{other:?}");
    }
    assert_eq!(
        record(home.path()),
        r#"{"machine_id":"aaaa","pid_namespace":1}"#
    );
}

/// A host that cannot read its own origin carries on without claiming a home
/// that has no owner, and is refused by one that has.
#[test]
fn issue133_an_unreadable_origin_claims_nothing_and_cannot_pass_an_owner() {
    let home = tempfile::tempdir().unwrap();
    check_as(home.path(), unreadable_origin()).unwrap();
    assert!(names(home.path()).is_empty());
    check_as(home.path(), Ok(here())).unwrap();
    let Err(Refusal::Unchecked { owner, error }) = check_as(home.path(), unreadable_origin())
    else {
        panic!("an unreadable origin passed");
    };
    assert_eq!(owner, here());
    assert_eq!(error.raw_os_error(), Some(2));
}

/// Hosts that claim a new home at once agree on one owner: the one whose
/// record is in place. Every other is refused, and no temporary file is left.
#[test]
fn issue133_a_concurrent_first_claim_has_one_owner() {
    const HOSTS: u64 = 16;
    let home = tempfile::tempdir().unwrap();
    let start = Barrier::new(HOSTS as usize);
    let results: Vec<_> = std::thread::scope(|scope| {
        let claims: Vec<_> = (0..HOSTS)
            .map(|namespace| {
                let (home, start) = (home.path(), &start);
                scope.spawn(move || {
                    start.wait();
                    check_as(home, Ok(origin(Some("aaaa"), Some(namespace))))
                })
            })
            .collect();
        claims
            .into_iter()
            .map(|claim| claim.join().unwrap())
            .collect()
    });
    let owner: SessionOrigin = serde_json::from_str(&record(home.path())).unwrap();
    let winner = owner.pid_namespace.unwrap();
    for (namespace, result) in (0..HOSTS).zip(results) {
        match result {
            Ok(()) => assert_eq!(namespace, winner),
            Err(Refusal::Foreign(found)) => assert_eq!(found, owner),
            Err(other) => panic!("{other:?}"),
        }
    }
    assert_eq!(names(home.path()), [OWNER_FILE]);
}

/// A record that is not a valid owner, or not a plain file, is refused and
/// left as it is: it is never replaced.
#[test]
fn issue133_a_bad_owner_record_is_refused_and_left_alone() {
    for contents in [
        "",
        "not json",
        "[]",
        "{}",
        r#"{"machine_id":"","pid_namespace":1}"#,
        r#"{"machine_id":"aaaa","pid_namespace":-1}"#,
        r#"{"pid_namespace":"1"}"#,
    ] {
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join(OWNER_FILE), contents).unwrap();
        let Err(Refusal::Unreadable(error)) = check_as(home.path(), Ok(here())) else {
            panic!("{contents:?} passed");
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{contents:?}");
        assert_eq!(record(home.path()), contents);
    }
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(home.path().join(OWNER_FILE)).unwrap();
    assert!(matches!(
        check_as(home.path(), Ok(here())),
        Err(Refusal::Unreadable(_))
    ));
    assert!(home.path().join(OWNER_FILE).is_dir());
}

/// A link at the record's name is never followed, even to a valid record of
/// this host's, and never replaced.
#[test]
fn issue133_a_linked_owner_record_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let target = elsewhere.path().join("owner");
    fs::write(&target, r#"{"machine_id":"aaaa","pid_namespace":1}"#).unwrap();
    let link = home.path().join(OWNER_FILE);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&target, &link).unwrap();
    assert!(matches!(
        check_as(home.path(), Ok(here())),
        Err(Refusal::Unreadable(_))
    ));
    assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
}

/// A home where no record can be written is not claimed: the check carries
/// on, as it does without an origin.
#[test]
fn issue133_a_home_that_cannot_be_claimed_is_used_unclaimed() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing");
    check_as(&missing, Ok(here())).unwrap();
    assert!(!missing.exists());
}

/// The check that every command makes reads this host's own origin, and so
/// claims a new home for this host.
#[test]
fn issue133_check_claims_a_home_for_this_host() {
    let home = tempfile::tempdir().unwrap();
    check(home.path()).unwrap();
    let owner: SessionOrigin = serde_json::from_str(&record(home.path())).unwrap();
    assert_eq!(owner, climon2_platform::origin::current().unwrap());
    check(home.path()).unwrap();
}
