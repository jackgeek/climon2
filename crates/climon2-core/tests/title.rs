//! Plan 1.8.1: title encoding and the default title.
use climon2_core::title::*;
use std::ffi::{OsStr, OsString};

#[test]
fn only_c0_del_and_c1_are_percent_encoded_per_utf8_byte() {
    assert_eq!(encode(""), "");
    assert_eq!(encode("plain ~ 100% /path"), "plain ~ 100% /path");
    assert_eq!(
        encode("日本語 é 🎉\u{a0}\u{2028}\u{fffd}"),
        "日本語 é 🎉\u{a0}\u{2028}\u{fffd}"
    );
    assert_eq!(encode("\0a\x07b\x1b]0;x\x1f\x7f"), "%00a%07b%1B]0;x%1F%7F");
    assert_eq!(encode("\u{80}\u{9b}\u{9f}"), "%C2%80%C2%9B%C2%9F");
    assert_eq!(encode("a\r\nb\tc"), "a%0D%0Ab%09c");
    for code in 0..=0x10FFFF_u32 {
        let Some(c) = char::from_u32(code) else {
            continue;
        };
        let encoded = encode(&c.to_string());
        let control = code < 0x20 || (0x7F..=0x9F).contains(&code);
        if control {
            let expected: String = c.to_string().bytes().map(|b| format!("%{b:02X}")).collect();
            assert_eq!(encoded, expected, "{code:#x}");
        } else {
            assert_eq!(encoded, c.to_string(), "{code:#x}");
        }
    }
}

#[test]
fn the_default_title_joins_the_application_argv_with_single_spaces() {
    assert_eq!(default_title(OsStr::new("bash"), &[]), "climon2 exec bash");
    assert_eq!(
        default_title(OsStr::new("vim"), &[OsString::from("notes.txt")]),
        "climon2 exec vim notes.txt"
    );
    assert_eq!(
        default_title(
            OsStr::new("my tool"),
            &["a b", "", "日本"].map(OsString::from)
        ),
        "climon2 exec my tool a b  日本"
    );
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(
            default_title(
                OsStr::from_bytes(b"x\xff"),
                &[OsStr::from_bytes(b"\xfe!").to_owned()]
            ),
            "climon2 exec x\u{fffd} \u{fffd}!"
        );
    }
}
