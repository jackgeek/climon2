//! Daemon wire revision 1 requests (client to daemon): Open, Leave and Kill.
//! Validation failures are connection-fatal codes; an identifiable request
//! with a well-formed but unsupported value is left to the daemon.
use climon2_core::wire::{
    ControlRecord, Counter, Families, FatalCode, Frame, Geometry, Mode, Open, Request, Requested,
};
use std::num::NonZeroU32;
use support::{counter, families};

mod support;

const OPEN: u8 = 0x30;
const LEAVE: u8 = 0x37;
const KILL: u8 = 0x38;

fn request(code: u8, body: impl AsRef<[u8]>) -> Result<Request, FatalCode> {
    Request::from_frame(&Frame::new(code, body.as_ref().to_vec()))
}

fn open_body(capabilities: &str, mode: &str) -> String {
    format!(r#"{{"request_id":"1","revision":1,{capabilities},{mode}}}"#)
}

const NO_CAPABILITIES: &str = r#""required_capabilities":[],"optional_capabilities":[]"#;
const MONITOR: &str = r#""mode":"monitor","geometry":null"#;

/// COM-02 (codec case): request IDs are canonical decimal strings from 1
/// through 18446744073709551615. A malformed ID is `invalid_request_id`; a
/// missing one is a missing member.
#[test]
fn request_ids_are_canonical_decimal_counters() {
    for (id, value) in [("1", 1), ("8", 8), ("18446744073709551615", u64::MAX)] {
        let Ok(Request::Leave(leave)) = request(LEAVE, format!(r#"{{"request_id":"{id}"}}"#))
        else {
            panic!("{id} is a valid request ID");
        };
        assert_eq!(leave.request_id.get(), value);
        assert_eq!(leave.request_id.to_string(), id);
        assert_eq!(id.parse::<Counter>(), Ok(leave.request_id));
    }
    for id in [
        "0",
        "01",
        "+1",
        "-1",
        " 1",
        "1 ",
        "",
        "18446744073709551616",
        "1.0",
        "1e3",
        "0x1",
    ] {
        assert_eq!(
            request(LEAVE, format!(r#"{{"request_id":"{id}"}}"#)),
            Err(FatalCode::InvalidRequestId),
            "{id}"
        );
        assert!(id.parse::<Counter>().is_err(), "{id}");
    }
    for id in ["1", "null", "true", "{}", "[]"] {
        assert_eq!(
            request(LEAVE, format!(r#"{{"request_id":{id}}}"#)),
            Err(FatalCode::InvalidRequestId),
            "{id} is not a string"
        );
    }
    assert_eq!(Counter::new(0), None);
    assert_eq!(request(LEAVE, "{}"), Err(FatalCode::InvalidRecord));
}

/// COM-02 (codec case): no unknown, duplicate or undocumented members, and
/// every listed field is required, including nullable fields that are null.
#[test]
fn requests_reject_unknown_duplicate_and_missing_members() {
    for body in [
        r#"{"request_id":"8","extra":null}"#,
        r#"{"request_id":"8","request_id":"8"}"#,
    ] {
        assert_eq!(
            request(LEAVE, body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
    for body in [
        r#"{"request_id":"1","revision":1}"#,
        r#"{"request_id":"1","required_capabilities":[]}"#,
        r#"{"request_id":"1","revision":1,"revision":1,"required_capabilities":[]}"#,
        r#"{"request_id":"1","revision":1,"required_capabilities":[],"optional_capabilities":[]}"#,
    ] {
        assert_eq!(request(KILL, body), Err(FatalCode::InvalidRecord), "{body}");
    }
    let complete = open_body(NO_CAPABILITIES, MONITOR);
    assert!(request(OPEN, &complete).is_ok());
    for body in [
        complete.replace(r#","geometry":null"#, ""),
        complete.replace(r#""optional_capabilities":[],"#, ""),
        complete.replace(r#""mode":"monitor","#, ""),
        complete.replace('}', r#","geometry":null}"#),
    ] {
        assert_eq!(
            request(OPEN, &body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
}

/// COM-02 (codec case): bodies are complete UTF-8 JSON objects. Broken JSON or
/// text is `invalid_json`; well-formed JSON of the wrong shape is
/// `invalid_record`.
#[test]
fn requests_must_be_complete_utf8_json_objects() {
    for body in [
        &b""[..],
        b"{",
        br#"{"request_id":"8"} {}"#,
        br#"{"request_id":"8",}"#,
        b"{\"request_id\":\"\xff\"}",
        br#"{"request_id":"\ud800"}"#,
        b"\xef\xbb\xbf{\"request_id\":\"8\"}",
    ] {
        assert_eq!(
            request(LEAVE, body),
            Err(FatalCode::InvalidJson),
            "{}",
            String::from_utf8_lossy(body)
        );
    }
    for body in [r#"["8"]"#, r#""8""#, "null", "8"] {
        assert_eq!(
            request(LEAVE, body),
            Err(FatalCode::InvalidRecord),
            "{body}"
        );
    }
    assert!(request(LEAVE, " {\n\"request_id\" : \"8\" } ").is_ok());
}

/// COM-02 (codec case): unsigned fields are plain decimal integer tokens: no
/// strings, signs, fractions or exponents. An unsupported but well-formed
/// revision is the daemon's to reject.
#[test]
fn unsigned_fields_are_plain_integer_tokens() {
    for (revision, value) in [("0", 0), ("1", 1), ("4294967295", u32::MAX)] {
        let body =
            format!(r#"{{"request_id":"1","revision":{revision},"required_capabilities":[]}}"#);
        let Ok(Request::Kill(kill)) = request(KILL, &body) else {
            panic!("{revision} is an unsigned 32-bit revision");
        };
        assert_eq!(kill.revision, value);
        assert_eq!(kill.to_frame(), Frame::new(KILL, body.into_bytes()));
    }
    for revision in ["4294967296", r#""1""#, "1.0", "1e0", "-1", "-0", "null"] {
        let body =
            format!(r#"{{"request_id":"1","revision":{revision},"required_capabilities":[]}}"#);
        assert_eq!(
            request(KILL, &body),
            Err(FatalCode::InvalidRecord),
            "{revision}"
        );
    }
}

/// COM-01 / COM-02 (codec case): capability arrays hold unique, case-sensitive
/// strings; unknown families are preserved for the daemon to judge, and an
/// Open's required and optional arrays never overlap.
#[test]
fn capability_arrays_are_unique_case_sensitive_and_disjoint() {
    for (tokens, expected) in [
        ("[]", &[][..]),
        (
            r#"["status-v1","STATUS-V1"]"#,
            &["status-v1", "STATUS-V1"][..],
        ),
        (
            r#"["future-v9","status-v1"]"#,
            &["future-v9", "status-v1"][..],
        ),
    ] {
        let body = format!(r#"{{"request_id":"1","revision":1,"required_capabilities":{tokens}}}"#);
        let Ok(Request::Kill(kill)) = request(KILL, &body) else {
            panic!("{tokens} is a valid capability array");
        };
        assert_eq!(kill.required_capabilities.tokens(), expected);
    }
    for tokens in [
        r#"["status-v1","status-v1"]"#,
        r#"["status-v1",1]"#,
        r#"["status-v1",null]"#,
        r#""status-v1""#,
        "null",
    ] {
        let body = format!(r#"{{"request_id":"1","revision":1,"required_capabilities":{tokens}}}"#);
        assert_eq!(
            request(KILL, &body),
            Err(FatalCode::InvalidRecord),
            "{tokens}"
        );
    }
    assert_eq!(Families::new(vec!["a".into(), "a".into()]), None);

    let overlapping =
        r#""required_capabilities":["status-v1","title-v1"],"optional_capabilities":["title-v1"]"#;
    assert_eq!(
        request(OPEN, open_body(overlapping, MONITOR)),
        Err(FatalCode::InvalidRecord)
    );
    assert_eq!(
        Requested::new(families(&["title-v1"]), families(&["title-v1"])),
        None
    );
    let disjoint =
        r#""required_capabilities":["status-v1"],"optional_capabilities":["Status-v1","title-v1"]"#;
    let Ok(Request::Open(open)) = request(OPEN, open_body(disjoint, MONITOR)) else {
        panic!("disjoint arrays are valid");
    };
    assert_eq!(open.capabilities.required(), &families(&["status-v1"]));
    assert_eq!(
        open.capabilities.optional(),
        &families(&["Status-v1", "title-v1"])
    );
}

/// COM-02 (codec case): monitor mode has null geometry and interactive mode has
/// a geometry of positive unsigned 32-bit columns and rows.
#[test]
fn open_geometry_is_null_for_monitor_and_positive_for_interactive() {
    let geometry = |cols: u32, rows: u32| Geometry {
        cols: NonZeroU32::new(cols).unwrap(),
        rows: NonZeroU32::new(rows).unwrap(),
    };
    for (body, size) in [
        (r#"{"cols":80,"rows":24}"#, geometry(80, 24)),
        (r#"{"cols":1,"rows":1}"#, geometry(1, 1)),
        (
            r#"{"cols":4294967295,"rows":4294967295}"#,
            geometry(u32::MAX, u32::MAX),
        ),
    ] {
        let text = open_body(
            NO_CAPABILITIES,
            &format!(r#""mode":"interactive","geometry":{body}"#),
        );
        let Ok(Request::Open(open)) = request(OPEN, &text) else {
            panic!("{body} is an interactive geometry");
        };
        assert_eq!(open.mode, Mode::Interactive(size));
        assert_eq!(open.to_frame(), Frame::new(OPEN, text.into_bytes()));
    }
    for mode in [
        r#""mode":"monitor","geometry":{"cols":80,"rows":24}"#,
        r#""mode":"interactive","geometry":null"#,
        r#""mode":"Monitor","geometry":null"#,
        r#""mode":"observer","geometry":null"#,
        r#""mode":null,"geometry":null"#,
        r#""mode":"interactive","geometry":{"cols":0,"rows":24}"#,
        r#""mode":"interactive","geometry":{"cols":80,"rows":0}"#,
        r#""mode":"interactive","geometry":{"cols":4294967296,"rows":24}"#,
        r#""mode":"interactive","geometry":{"cols":80}"#,
        r#""mode":"interactive","geometry":{"cols":80,"rows":24,"depth":1}"#,
        r#""mode":"interactive","geometry":{"cols":80,"cols":80,"rows":24}"#,
        r#""mode":"interactive","geometry":[80,24]"#,
        r#""mode":"interactive","geometry":"80x24""#,
    ] {
        assert_eq!(
            request(OPEN, open_body(NO_CAPABILITIES, mode)),
            Err(FatalCode::InvalidRecord),
            "{mode}"
        );
    }
}

/// COM-02 (codec case): an interactive Open encodes its documented members in
/// order.
#[test]
fn an_interactive_open_encodes_its_documented_members_in_order() {
    let open = Open {
        request_id: counter(u64::MAX),
        revision: 1,
        capabilities: Requested::new(families(&[]), families(&["title-v1"])).unwrap(),
        mode: Mode::Interactive(Geometry {
            cols: NonZeroU32::new(80).unwrap(),
            rows: NonZeroU32::new(24).unwrap(),
        }),
    };
    assert_eq!(
        open.to_frame(),
        Frame::new(
            OPEN,
            br#"{"request_id":"18446744073709551615","revision":1,"required_capabilities":[],"optional_capabilities":["title-v1"],"mode":"interactive","geometry":{"cols":80,"rows":24}}"#.to_vec()
        )
    );
}

/// COM-02 (codec case): the daemon decodes only request types; a server type is
/// `wrong_direction` and a type outside both ranges is `unknown_message_type`.
#[test]
fn requests_are_identified_by_type_and_direction() {
    let leave = br#"{"request_id":"8"}"#;
    assert!(request(LEAVE, leave).is_ok());
    for code in [0x40, 0x41, 0x44, 0x49, 0x4c, 0x4d] {
        assert_eq!(
            request(code, leave),
            Err(FatalCode::WrongDirection),
            "{code:#x}"
        );
    }
    for code in [0x00, 0x01, 0x10, 0x2f, 0x3a, 0x3f, 0x4e, 0x7f, 0xff] {
        assert_eq!(
            request(code, leave),
            Err(FatalCode::UnknownMessageType),
            "{code:#x}"
        );
    }
}
