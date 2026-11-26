//! #133 / ADR 0011: the home owner record's format, how an origin reads in a
//! diagnostic, and the refusals' messages.
use climon2_core::{
    metadata::SessionOrigin,
    owner::{Refusal, from_json, to_json},
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
         namespace 1). To work across machines, use `c2 relay`. To move this home here, run \
         `c2 home release` on the owning host."
    );
    let error = io::Error::other("no origin");
    assert_eq!(
        Refusal::Unchecked { owner, error }.to_string(),
        "this climon2 home belongs to machine aaaa, PID namespace 1, and this host's machine \
         and PID namespace could not be checked: no origin"
    );
    assert_eq!(
        Refusal::Unreadable(io::Error::other("bad record")).to_string(),
        "this climon2 home's owner record cannot be read: bad record"
    );
}
