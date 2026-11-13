use climon2_vt::{Fed, Geometry, Vt, VtConfig};
use std::time::{Duration, Instant};

fn vt() -> Vt {
    Vt::new(
        Geometry::for_host(80, 24).unwrap(),
        VtConfig {
            max_history_rows: 100,
            default_title: "climon2 exec bash".into(),
        },
    )
}

fn feed(vt: &mut Vt, bytes: &[u8]) -> Fed {
    vt.feed(bytes, Instant::now())
}

fn live(bytes: &[u8]) -> Vec<u8> {
    feed(&mut vt(), bytes).live
}

#[track_caller]
fn assert_forwarded(bytes: &[u8]) {
    assert_eq!(
        feed(&mut vt(), bytes),
        Fed {
            live: bytes.to_vec(),
            replies: Vec::new(),
            dropped: 0,
            titles: Vec::new(),
            focus_enabled: false,
        },
        "{:?}",
        String::from_utf8_lossy(bytes)
    );
}

#[track_caller]
fn assert_stripped(bytes: &[u8]) {
    assert_eq!(
        feed(&mut vt(), bytes),
        Fed::default(),
        "{:?}",
        String::from_utf8_lossy(bytes)
    );
}

#[test]
fn text_controls_and_ordinary_sequences_are_forwarded_unchanged() {
    for bytes in [
        &b"hello\r\n\x07\x08\t"[..],
        b"\x1b[1;31;48;2;1;2;3mred\x1b[0m",
        b"\x1b[5;10H\x1b[2J\x1b[3J\x1b[K",
        b"\x1b[?1049h\x1b[?25l\x1b[?2004h\x1b[?1000;1006h",
        b"\x1b7\x1b8\x1b(0\x1b=\x1b>\x1bc",
        b"\x1b[s\x1b[u\x1b[2 q\x1b[!p",
        "é😀".as_bytes(),
    ] {
        assert_forwarded(bytes);
    }
}

#[test]
fn c0_controls_inside_escape_sequences_keep_their_original_order() {
    let bytes = b"\x1b\x1c\x1d\x1e\x1f 0";
    assert_forwarded(bytes);
}

#[test]
fn c0_controls_survive_a_stripped_query_in_their_original_order() {
    let fed = feed(&mut vt(), b"\x1b[6\x1cn");
    assert_eq!(fed.live, b"\x1c");
    assert_eq!(fed.replies, b"\x1b[1;1R");
}

#[test]
fn a_cpr_shaped_sequence_in_output_is_forwarded_and_answers_nothing() {
    assert_forwarded(b"\x1b[12;5R");
}

#[test]
fn supported_queries_are_answered_by_the_engine_and_stripped() {
    for (query, reply) in [
        (&b"\x1b[c"[..], &b"\x1b[?6c"[..]),
        (b"\x1b[0c", b"\x1b[?6c"),
        (b"\x1b[>c", b"\x1b[>0;2600;1c"),
        (b"\x1b[>0c", b"\x1b[>0;2600;1c"),
        (b"\x1bZ", b"\x1b[?6c"),
        (b"\x1b[5n", b"\x1b[0n"),
        (b"\x1b[6n", b"\x1b[1;1R"),
        (b"\x1b[4$p", b"\x1b[4;2$y"),
        (b"\x1b[?25$p", b"\x1b[?25;1$y"),
        (b"\x1b[18t", b"\x1b[8;24;80t"),
    ] {
        let fed = feed(&mut vt(), query);
        assert_eq!(
            (fed.live, fed.replies),
            (Vec::new(), reply.to_vec()),
            "{query:?}"
        );
    }
}

#[test]
fn a_query_split_across_reads_is_answered_once() {
    let mut vt = vt();
    let now = Instant::now();
    let first = vt.feed(b"x\x1b[", now);
    let second = vt.feed(b"6", now);
    let third = vt.feed(b"n", now);
    assert_eq!((first.live, first.replies), (b"x".to_vec(), Vec::new()));
    assert_eq!(second, Fed::default());
    assert_eq!(
        (third.live, third.replies),
        (Vec::new(), b"\x1b[1;2R".to_vec())
    );
}

#[test]
fn host_dependent_requests_are_stripped_and_unanswered() {
    for request in [
        &b"\x1b[=c"[..],
        b"\x1b[?c",
        b"\x1b[1c",
        b"\x1b[?6n",
        b"\x1b[7n",
        b"\x1b[x",
        b"\x1b[1x",
        b"\x1b[>q",
        b"\x1b[>0q",
        b"\x1b[14t",
        b"\x1b[22;0t",
        b"\x1b[?1;1;0S",
        b"\x1b]11;?\x07",
        b"\x1b]4;1;?\x1b\\",
        b"\x1bP$qm\x1b\\",
        b"\x1bP+q544e\x1b\\",
        b"\x1b_Gq\x1b\\",
        b"\x1b^pm\x1b\\",
        b"\x1bXsos\x1b\\",
        b"\x1b[?4m",
    ] {
        assert_stripped(request);
    }
}

#[test]
fn keyboard_protocol_negotiation_is_stripped() {
    for request in [
        &b"\x1b[?u"[..],
        b"\x1b[>1u",
        b"\x1b[<u",
        b"\x1b[=1;1u",
        b"\x1b[>4;2m",
    ] {
        assert_stripped(request);
    }
}

#[test]
fn clipboard_operations_are_stripped() {
    assert_stripped(b"\x1b]52;c;aGVsbG8=\x07");
    assert_stripped(b"\x1b]52;c;?\x1b\\");
}

#[test]
fn hyperlinks_become_plain_text() {
    assert_eq!(
        live(b"see \x1b]8;id=1;https://example.com\x1b\\link\x1b]8;;\x07 now"),
        b"see link now"
    );
}

#[test]
fn every_other_osc_is_stripped() {
    for osc in [
        &b"\x1b]7;file://host/tmp\x07"[..],
        b"\x1b]4;1;rgb:ff/00/00\x1b\\",
        b"\x1b]104\x07",
        b"\x1b]133;A\x07",
        b"\x1b]1;icon\x07",
    ] {
        assert_stripped(osc);
    }
}

#[test]
fn malformed_and_ignored_sequences_are_stripped() {
    let many = format!("\x1b[{}m", vec!["1"; 40].join(";"));
    for bytes in [&b"\x1b[1?h"[..], b"\x1b[1;2!\"#m", many.as_bytes()] {
        assert_stripped(bytes);
    }
}

#[test]
fn unsupported_alternate_screen_modes_are_removed() {
    assert_stripped(b"\x1b[?1047h");
    assert_stripped(b"\x1b[?47l");
    assert_eq!(live(b"\x1b[?47;25;1047l"), b"\x1b[?25l");
    assert_eq!(live(b"\x1b[?1047;1049h"), b"\x1b[?1049h");
    assert_forwarded(b"\x1b[?0025h");
    assert_forwarded(b"\x1b[47;4h");
}

#[test]
fn oversized_tokens_are_dropped_and_counted() {
    let mut huge = b"\x1b]2;".to_vec();
    huge.resize(5000, b'a');
    huge.extend_from_slice(b"\x07ok");
    assert_eq!(
        feed(&mut vt(), &huge),
        Fed {
            live: b"ok".to_vec(),
            replies: Vec::new(),
            dropped: 1,
            titles: Vec::new(),
            focus_enabled: false,
        }
    );
}

#[test]
fn title_changes_are_reencoded_and_unchanged_titles_are_not_repeated() {
    let mut vt = vt();
    assert_eq!(feed(&mut vt, b"\x1b]0;hi\x07a").live, b"\x1b]2;hi\x1b\\a");
    assert_eq!(feed(&mut vt, b"\x1b]2;hi\x1b\\").live, b"");
    assert_eq!(feed(&mut vt, b"\x1b]2;\x07").live, b"\x1b]2;\x1b\\");
}

#[test]
fn control_characters_in_titles_are_percent_encoded() {
    assert_eq!(
        live("\x1b]2;a\x7fb\u{9b}c\u{85}d€ é\x07".as_bytes()),
        "\x1b]2;a%7Fb%C2%9Bc%C2%85d€ é\x1b\\".as_bytes()
    );
}

#[test]
fn title_stack_and_reset_return_to_earlier_and_default_titles() {
    let mut vt = vt();
    feed(&mut vt, b"\x1b]2;one\x07");
    assert_eq!(
        feed(&mut vt, b"\x1b[22t\x1b]2;two\x07").live,
        b"\x1b]2;two\x1b\\"
    );
    assert_eq!(feed(&mut vt, b"\x1b[23t").live, b"\x1b]2;one\x1b\\");
    assert_eq!(
        feed(&mut vt, b"\x1bc").live,
        b"\x1bc\x1b]2;climon2 exec bash\x1b\\"
    );
    assert_eq!(feed(&mut vt, b"\x1bc").live, b"\x1bc");
    feed(&mut vt, b"\x1b[22t\x1b]2;three\x07");
    assert_eq!(
        feed(&mut vt, b"\x1b[23t").live,
        b"\x1b]2;climon2 exec bash\x1b\\"
    );
}

/// CTL-03 (crate case): each change of the application's own title is
/// reported by value and in order, one per change even within one read:
/// unset (`None`, while the default title shows), empty, or a value. An
/// unchanged title is not reported again, and an oversized title sequence is
/// ignored, never truncated.
#[test]
fn application_title_changes_are_reported_in_order_by_value() {
    let title = |text: &str| Some(text.to_owned());
    let mut vt = vt();
    assert_eq!(vt.application_title(), None);
    let fed = feed(&mut vt, b"\x1b]2;one\x07\x1b]0;two\x07\x1b]2;two\x1b\\x");
    assert_eq!(fed.titles, [title("one"), title("two")]);
    assert_eq!(vt.application_title(), Some("two"));
    assert_eq!(feed(&mut vt, b"\x1b]2;\x07").titles, [title("")]);
    assert_eq!(vt.application_title(), Some(""));
    feed(&mut vt, b"\x1b]2;three\x07");
    assert_eq!(
        feed(&mut vt, b"\x1b[22t\x1b]2;four\x07\x1b[23t").titles,
        [title("four"), title("three")]
    );
    assert_eq!(feed(&mut vt, b"\x1bc").titles, [None]);
    assert_eq!(vt.application_title(), None);
    let mut oversized = b"\x1b]2;".to_vec();
    oversized.resize(5000, b'a');
    oversized.push(0x07);
    assert!(feed(&mut vt, &oversized).titles.is_empty());
    assert_eq!(vt.application_title(), None);
}

#[test]
fn synchronized_update_latches_from_the_first_begin() {
    let mut vt = vt();
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    assert!(!vt.sync_pending(start));
    assert!(!vt.tick(start));
    assert_eq!(vt.feed(b"\x1b[?2026h", start).live, b"\x1b[?2026h");
    assert!(vt.sync_pending(at(149)));
    vt.feed(b"\x1b[?20", at(100));
    vt.feed(b"26h\x1b[?2026;25h", at(100));
    assert!(vt.sync_pending(at(149)));
    assert!(!vt.sync_pending(at(150)));
    assert!(!vt.tick(at(149)));
    assert!(vt.tick(at(150)));
    assert!(!vt.tick(at(151)));
    vt.feed(b"\x1b[?2026h", at(200));
    assert!(vt.sync_pending(at(349)));
    vt.feed(b"\x1b[?2026l", at(210));
    assert!(!vt.sync_pending(at(211)));
    assert!(!vt.tick(at(400)));
}

#[test]
fn an_expired_update_is_rearmed_by_a_new_begin_before_tick() {
    let mut vt = vt();
    let start = Instant::now();
    vt.feed(b"\x1b[?2026h", start);
    vt.feed(b"\x1b[?2026h", start + Duration::from_millis(160));
    assert!(vt.sync_pending(start + Duration::from_millis(309)));
}

/// FOC-01 (crate case): the daemon reports the current application focus
/// once when the application newly enables focus reporting (mode 1004), so
/// each feed says whether a token turned it on and it is still on, however
/// the tokens were split across reads.
#[test]
fn focus_reporting_reports_the_mode_and_each_feed_that_newly_enabled_it() {
    let mut vt = vt();
    assert!(!vt.focus_reporting());
    assert!(!feed(&mut vt, b"plain \x1b[?2004h").focus_enabled);
    assert!(feed(&mut vt, b"a\x1b[?1004hb").focus_enabled);
    assert!(vt.focus_reporting());
    assert!(!feed(&mut vt, b"\x1b[?1004h").focus_enabled);
    assert!(!feed(&mut vt, b"c").focus_enabled);
    assert!(feed(&mut vt, b"\x1b[?1004l\x1b[?1004h").focus_enabled);
    assert!(!feed(&mut vt, b"\x1b[?1004h\x1b[?1004l").focus_enabled);
    assert!(!vt.focus_reporting());
    assert!(!feed(&mut vt, b"\x1b[?1004").focus_enabled);
    assert!(feed(&mut vt, b"h").focus_enabled);
    assert!(vt.focus_reporting());
}

#[test]
fn bracketed_paste_reports_the_application_mode() {
    let mut vt = vt();
    assert!(!vt.bracketed_paste());
    feed(&mut vt, b"\x1b[?2004h");
    assert!(vt.bracketed_paste());
    feed(&mut vt, b"\x1b[?2004l");
    assert!(!vt.bracketed_paste());
}
