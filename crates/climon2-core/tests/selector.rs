//! Plan 1.2: selector shape (usage error) and matching against live sessions.
use climon2_core::{SessionId, cli::UsageError, selector::*};
use std::ffi::OsStr;

const A: &str = "550e8400-e29b-41d4-a716-446655440000";
const B: &str = "550e8400-e29b-41d4-a716-446655440001";
const C: &str = "660e8400-e29b-41d4-a716-446655440000";

fn ids() -> Vec<SessionId> {
    [A, B, C].map(|id| id.parse().unwrap()).to_vec()
}

fn selector(text: &str) -> Selector {
    Selector::parse(OsStr::new(text)).unwrap()
}

#[test]
fn selectors_normalise_hyphens_and_case_to_8_through_32_hex_digits() {
    for (text, normal) in [
        ("550e8400", "550e8400"),
        ("550E8400", "550e8400"),
        ("550e-8400", "550e8400"),
        ("-550e8400-", "550e8400"),
        ("5-5-0-e-8-4-0-0", "550e8400"),
        (A, "550e8400e29b41d4a716446655440000"),
        (
            "550E8400-E29B-41D4-A716-446655440000",
            "550e8400e29b41d4a716446655440000",
        ),
        ("abcdefABCDEF0123", "abcdefabcdef0123"),
    ] {
        assert_eq!(selector(text).as_str(), normal, "{text}");
    }
}

#[test]
fn malformed_or_omitted_selectors_are_usage_errors() {
    for bad in [
        "",
        "-",
        "--------",
        "550e840",
        "550e-840",
        "550e8400e29b41d4a7164466554400001",
        "550e840g",
        "550e8400 ",
        " 550e8400",
        "550e8400_",
        "550e8400\u{0661}",
        "５50e8400",
        "{550e8400-e29b-41d4-a716-446655440000}",
    ] {
        assert_eq!(Selector::parse(OsStr::new(bad)), Err(UsageError), "{bad:?}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(
            Selector::parse(OsStr::from_bytes(b"550e8400\xff")),
            Err(UsageError)
        );
    }
}

#[test]
fn a_unique_prefix_or_full_id_selects_exactly_that_candidate() {
    let candidates = ids();
    for (text, expected) in [
        ("660e8400", C),
        ("660E8400E29B", C),
        (A, A),
        ("550e8400e29b41d4a716446655440001", B),
        ("550e8400-e29b-41d4-a716-44665544000-1", B),
    ] {
        let found = selector(text).select(&candidates, |id| id).unwrap();
        assert_eq!(found.as_str(), expected, "{text}");
    }
    assert!(selector("550e8400").matches(&candidates[0]));
    assert!(!selector("550e8400").matches(&candidates[2]));
    assert!(!selector("e29b41d4").matches(&candidates[0]));
}

#[test]
fn no_match_and_ambiguity_fail_and_ambiguity_lists_every_full_id() {
    let candidates = ids();
    let error = selector("770e8400")
        .select(&candidates, |id| id)
        .unwrap_err();
    assert_eq!(
        error,
        SelectionError::NoMatch {
            selector: selector("770e8400")
        }
    );
    assert_eq!(error.to_string(), "no live session matches 770e8400");
    let empty: [SessionId; 0] = [];
    assert!(matches!(
        selector(A).select(&empty, |id| id),
        Err(SelectionError::NoMatch { .. })
    ));

    let error = selector("550E8400")
        .select(&candidates, |id| id)
        .unwrap_err();
    assert_eq!(
        error,
        SelectionError::Ambiguous {
            selector: selector("550e8400"),
            matches: vec![A.parse().unwrap(), B.parse().unwrap()],
        }
    );
    assert_eq!(
        error.to_string(),
        format!("session selector 550e8400 is ambiguous; it matches {A}, {B}")
    );
    let pairs: Vec<_> = candidates.iter().map(|id| (id.clone(), "x")).collect();
    let found = selector("550e8400e29b41d4a716").select(&pairs, |pair| &pair.0);
    assert!(matches!(found, Err(SelectionError::Ambiguous { matches, .. }) if matches.len() == 2));
}
