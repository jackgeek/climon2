//! #133 / ADR 0011: the home owner record's format, how an origin reads in a
//! diagnostic, and the refusals' messages.
use climon2_core::{
    metadata::SessionOrigin,
    owner::{Refusal, forced_warning, forgive, from_json, to_json},
};
use std::io;

fn origin(machine: Option<&str>, pid_namespace: Option<u64>) -> SessionOrigin {
    SessionOrigin {
        machine_id: machine.map(|machine| machine.parse().unwrap()),
        pid_namespace,
    }
}

/// A record is a JSON object naming a machine, a PID namespace or both, and
/// unknown members are ignored. One naming neither, or anything malformed, is
/// refused.
#[test]
fn issue133_an_owner_record_names_a_machine_or_a_pid_namespace() {
    for (record, owner) in [
        (
            r#"{"machine_id":"aaaa","pid_namespace":1}"#,
            origin(Some("aaaa"), Some(1)),
        ),
        (r#"{"machine_id":"aaaa"}"#, origin(Some("aaaa"), None)),
        (r#"{"pid_namespace":1}"#, origin(None, Some(1))),
    ] {
        assert_eq!(from_json(record.as_bytes()).unwrap(), owner);
        assert_eq!(to_json(&owner).unwrap(), record.as_bytes());
    }
    let later = r#"{"machine_id":"aaaa","pid_namespace":1,"later":true}"#;
    assert_eq!(
        from_json(later.as_bytes()).unwrap(),
        origin(Some("aaaa"), Some(1))
    );
    for record in [
        "",
        "null",
        "[]",
        "{}",
        r#"{"machine_id":null,"pid_namespace":null}"#,
        r#"{"machine_id":"a b"}"#,
        r#"{"pid_namespace":-1}"#,
    ] {
        let error = from_json(record.as_bytes()).unwrap_err();
        assert_eq!(error.0, "invalid home owner record", "{record:?}");
    }
}

/// An origin names what it records; a refusal names the owner, and a foreign
/// home's refusal says how to work across machines and how to move the home.
#[test]
fn issue133_refusals_name_the_owner_and_the_way_forward() {
    for (owner, text) in [
        (
            origin(Some("aaaa"), Some(1)),
            "machine aaaa, PID namespace 1",
        ),
        (origin(Some("aaaa"), None), "machine aaaa"),
        (origin(None, Some(1)), "PID namespace 1"),
        (origin(None, None), "an unknown machine"),
    ] {
        assert_eq!(owner.to_string(), text);
    }
    let owner = origin(Some("aaaa"), Some(1));
    assert_eq!(
        Refusal::Foreign(owner.clone()).to_string(),
        "this climon2 home is in use by another machine or container (machine aaaa, PID \
         namespace 1). climon2 does not work across machines yet (`c2 relay` is planned). To \
         move this home here, run `c2 home release`, or `c2 home release --force` if the host \
         that owns it is gone. To use it from here anyway, unclaimed, give the command `--force`."
    );
    let error = io::Error::other("no origin");
    assert_eq!(
        Refusal::Unchecked { owner, error }.to_string(),
        "this climon2 home belongs to machine aaaa, PID namespace 1, and this host's machine \
         and PID namespace could not be checked: no origin"
    );
    let path = std::path::PathBuf::from("/home/me/.climon2/owner");
    let error = io::Error::other("bad record");
    assert_eq!(
        Refusal::Unreadable { path, error }.to_string(),
        "this climon2 home's owner record /home/me/.climon2/owner cannot be read: bad record. \
         To let the next command claim this home afresh, run `c2 home release` (with `--force` \
         if the home still holds sessions) or delete /home/me/.climon2/owner. To use it anyway, \
         unclaimed, give the command `--force`."
    );
}

/// #133 / ADR 0011: `--force` skips a foreign or unreadable record, handing it
/// back for the warning; without it, and for a host that cannot read its own
/// origin, the refusal stands.
#[test]
fn issue133_force_skips_only_a_foreign_or_unreadable_record() {
    let owner = || origin(Some("aaaa"), Some(1));
    let foreign = || Err(Refusal::Foreign(owner()));
    let unreadable = || {
        Err(Refusal::Unreadable {
            path: "/home/me/.climon2/owner".into(),
            error: io::Error::other("bad"),
        })
    };
    let unchecked = || {
        Err(Refusal::Unchecked {
            owner: owner(),
            error: io::Error::other("no origin"),
        })
    };
    for force in [false, true] {
        assert!(matches!(forgive(Ok(()), force), Ok(None)));
        assert!(matches!(
            forgive(unchecked(), force),
            Err(Refusal::Unchecked { .. })
        ));
    }
    assert!(matches!(
        forgive(foreign(), false),
        Err(Refusal::Foreign(_))
    ));
    assert!(matches!(
        forgive(unreadable(), false),
        Err(Refusal::Unreadable { .. })
    ));
    let Ok(Some(skipped)) = forgive(foreign(), true) else {
        panic!("--force did not skip a foreign owner");
    };
    assert_eq!(
        forced_warning(&skipped),
        format!("climon2: warning: --force uses this climon2 home without claiming it: {skipped}")
    );
    assert!(matches!(
        forgive(unreadable(), true),
        Ok(Some(Refusal::Unreadable { .. }))
    ));
}
