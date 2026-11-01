mod support;

use alacritty_terminal::grid::Dimensions;
use climon2_vt::{Geometry, Presentation};
use support::*;

const PROLOGUE: &str = "\x1b[?2026l\x1b[r\x1b[?6l\x1b[4l\x1b[20l\x1b[?7h\x1b[0m";
const NOTICE: &str = "\x1b[0m[climon2: older session scrollback was discarded]\r\n";
const NOTICE_TEXT: &str = "[climon2: older session scrollback was discarded]";
const MODES: &str = "\x1b[?1l\x1b>\x1b[?2004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1005l\x1b[?1004l\x1b[?25h\x1b[?1007h\x1b[0 q";
const DIRECT_MODES: &str = "\x1b[?6l\x1b[4l\x1b[20l\x1b[?7h";

fn row(text: &str) -> String {
    format!("\x1b[0m{text}\x1b[0m\r\n")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// History rows as a fresh host terminal keeps them after the prefix.
fn restored_history(cols: usize, rows: usize, prefix: &str) -> Vec<String> {
    history(&host(cols, rows, prefix.as_bytes()))
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect()
}

#[test]
fn a_normal_only_restoration_is_lead_history_scroll_off_screen_then_suffix() {
    let mut vt = vt(10, 3, 10);
    feed(&mut vt, b"a\r\nb\r\nc\r\nd\r\ne");
    let expected = [
        PROLOGUE,
        "\x1b[3;1H\n\n\n",
        &row("a"),
        &row("b"),
        "\x1b[3;1H\n\n",
        "\x1b[1;1H\x1b[0mc\x1b[0m\x1b[2;1H\x1b[0md\x1b[0m\x1b[3;1H\x1b[0me\x1b[0m",
        "\x1b[1;1H\x1b[0m\x1b7\x1b[3;2H\x1b[0m",
        MODES,
        DIRECT_MODES,
        "\x1b]2;climon2 exec app\x1b\\",
    ]
    .concat();
    assert_eq!(text(&restored(&mut vt)), expected);
}

#[test]
fn phase_a_is_returned_by_the_grant_before_any_production() {
    let mut vt = vt(10, 3, 10);
    feed(&mut vt, b"a\r\nb\r\nc\r\nd");
    let mut presentation = Presentation::new(10, 7);
    let (lead, mut restoration) = vt.grant(&mut presentation);
    assert_eq!(text(&lead), format!("{PROLOGUE}\x1b[7;1H\n\n\n\n\n\n\n"));
    let first = vt.produce(&mut restoration, &mut presentation, 1);
    assert_eq!((text(&first.bytes), first.done), (row("a"), false));
    let rest = vt.produce(&mut restoration, &mut presentation, 1);
    assert!(text(&rest.bytes).starts_with("\x1b[7;1H\n\n\n\n\n\n\x1b[1;1H"));
    assert!(rest.done);
    let after = vt.produce(&mut restoration, &mut presentation, 1);
    assert_eq!((after.bytes, after.done), (Vec::new(), true));
}

#[test]
fn production_stops_at_the_budget_after_whole_rows() {
    let mut vt = vt(10, 2, 10);
    feed(&mut vt, b"aa\r\nbb\r\ncc\r\ndd\r\nee");
    let mut presentation = Presentation::new(10, 2);
    let (_, mut restoration) = vt.grant(&mut presentation);
    let budget = row("aa").len();
    let chunk = vt.produce(&mut restoration, &mut presentation, budget);
    assert_eq!(text(&chunk.bytes), row("aa"));
    let chunk = vt.produce(&mut restoration, &mut presentation, budget + 1);
    assert_eq!(text(&chunk.bytes), [row("bb"), row("cc")].concat());
}

#[test]
fn without_history_there_is_no_scroll_off_before_the_screen() {
    let mut vt = vt(10, 3, 10);
    feed(&mut vt, b"a");
    let bytes = text(&restored(&mut vt));
    assert!(bytes.starts_with(&format!("{PROLOGUE}\x1b[3;1H\n\n\n\x1b[1;1H\x1b[0ma")));
}

#[test]
fn a_replacement_restoration_leaves_this_attachments_alternate_screen_first() {
    let mut vt = vt(10, 3, 10);
    let mut presentation = Presentation::new(10, 3);
    presentation.commit_live(b"\x1b[?1049h\x1b[?2026h");
    let (lead, _) = vt.grant(&mut presentation);
    assert!(text(&lead).starts_with(&format!("\x1b[?1049l{PROLOGUE}")));
    assert_eq!(presentation.end_sync(), b"");
    assert!(!text(&presentation.release()).contains("1049"));
}

#[test]
fn a_replacement_grant_at_a_new_host_size_keeps_what_the_host_was_put_into() {
    let mut vt = vt(10, 3, 10);
    let mut presentation = Presentation::new(10, 3);
    presentation.commit_live(b"\x1b[?1049h");
    presentation.resize_host(12, 5);
    let (lead, _) = vt.grant(&mut presentation);
    let lead = text(&lead);
    assert!(lead.starts_with(&format!("\x1b[?1049l{PROLOGUE}\x1b[5;1H\n\n\n\n\n")));
    assert_eq!(presentation.mode(), climon2_vt::Mode::Derived);
}

#[test]
fn retained_history_is_the_newest_max_rows_rows() {
    let mut vt = vt(60, 2, 3);
    feed(&mut vt, &lines(1..=6));
    let prefix = text(&restored(&mut vt));
    assert!(prefix.contains(&[NOTICE, &row("3"), &row("4"), &row("5")].concat()));
    assert_eq!(
        restored_history(60, 2, &prefix),
        [NOTICE_TEXT, "3", "4", "5"]
    );
}

#[test]
fn exactly_full_history_has_no_notice_and_identical_rows_still_evict() {
    let mut full = vt(10, 2, 3);
    feed(&mut full, &lines(1..=4));
    let prefix = text(&restored(&mut full));
    assert_eq!(restored_history(10, 2, &prefix), ["1", "2", "3"]);
    let mut identical = vt(60, 2, 3);
    feed(&mut identical, &b"y\r\n".repeat(5));
    let prefix = text(&restored(&mut identical));
    assert_eq!(
        restored_history(60, 2, &prefix),
        [NOTICE_TEXT, "y", "y", "y"]
    );
}

#[test]
fn clearing_history_clears_the_eviction_flag() {
    for clear in [&b"\x1b[3J"[..], b"\x1bc"] {
        let mut vt = vt(10, 2, 3);
        feed(&mut vt, &lines(1..=6));
        feed(&mut vt, clear);
        feed(&mut vt, &lines(7..=9));
        let prefix = text(&restored(&mut vt));
        let history = restored_history(10, 2, &prefix);
        assert!(
            history.ends_with(&["7".into(), "8".into()]),
            "{clear:?} {history:?}"
        );
        assert!(!prefix.contains(NOTICE), "{clear:?}");
    }
}

#[test]
fn zero_retention_sends_no_history_and_no_notice() {
    let mut vt = vt(10, 2, 0);
    feed(&mut vt, &lines(1..=6));
    let prefix = text(&restored(&mut vt));
    assert!(
        prefix.starts_with(&format!(
            "{PROLOGUE}\x1b[2;1H\n\n\x1b[1;1H\x1b[0m6\x1b[0m\x1b[1;1H"
        )),
        "{prefix:?}"
    );
}

#[test]
fn rows_pushed_out_of_retention_during_production_become_a_gap_notice() {
    let mut vt = vt(10, 2, 3);
    feed(&mut vt, &lines(1..=4));
    let mut presentation = Presentation::new(10, 2);
    let (_, mut restoration) = vt.grant(&mut presentation);
    let first = vt.produce(&mut restoration, &mut presentation, 1);
    assert_eq!(text(&first.bytes), row("1"));
    feed(&mut vt, &lines(5..=6));
    let rest = text(
        &vt.produce(&mut restoration, &mut presentation, 1 << 20)
            .bytes,
    );
    assert!(
        rest.starts_with(&[NOTICE, &row("3"), "\x1b[2;1H\n\x1b[1;1H"].concat()),
        "{rest:?}"
    );
}

#[test]
fn the_lead_notice_is_not_repeated_at_an_immediate_gap() {
    let mut vt = vt(10, 2, 3);
    feed(&mut vt, &lines(1..=5));
    let mut presentation = Presentation::new(10, 2);
    let (lead, mut restoration) = vt.grant(&mut presentation);
    assert!(text(&lead).ends_with(NOTICE));
    feed(&mut vt, &lines(6..=7));
    let rest = text(
        &vt.produce(&mut restoration, &mut presentation, 1 << 20)
            .bytes,
    );
    assert!(
        rest.starts_with(&[row("4"), "\x1b[2;1H\n\x1b[1;1H".into()].concat()),
        "{rest:?}"
    );
}

#[test]
fn when_every_snapshot_row_left_retention_one_notice_ends_phase_b() {
    let mut vt = vt(10, 2, 3);
    feed(&mut vt, &lines(1..=3));
    let mut presentation = Presentation::new(10, 2);
    let (_, mut restoration) = vt.grant(&mut presentation);
    feed(&mut vt, &lines(4..=20));
    let rest = text(
        &vt.produce(&mut restoration, &mut presentation, 1 << 20)
            .bytes,
    );
    assert!(
        rest.starts_with(&[NOTICE, "\x1b[2;1H\n\x1b[1;1H\x1b[0m3"].concat()),
        "{rest:?}"
    );
}

#[test]
fn clearing_history_during_production_ends_phase_b_without_a_notice() {
    for clear in [&b"\x1b[3J"[..], b"\x1bc"] {
        let mut vt = vt(10, 2, 3);
        feed(&mut vt, &lines(1..=3));
        let mut presentation = Presentation::new(10, 2);
        let (_, mut restoration) = vt.grant(&mut presentation);
        vt.produce(&mut restoration, &mut presentation, 1);
        feed(&mut vt, clear);
        let rest = text(
            &vt.produce(&mut restoration, &mut presentation, 1 << 20)
                .bytes,
        );
        assert!(
            rest.starts_with("\x1b[2;1H\n\x1b[1;1H\x1b[0m3"),
            "{clear:?} {rest:?}"
        );
    }
}

#[test]
fn clearing_history_on_the_alternate_screen_keeps_normal_history() {
    let mut vt = vt(10, 2, 3);
    feed(&mut vt, &lines(1..=3));
    let mut presentation = Presentation::new(10, 2);
    let (_, mut restoration) = vt.grant(&mut presentation);
    feed(&mut vt, b"\x1b[?1049h\x1b[3J");
    let rest = text(
        &vt.produce(&mut restoration, &mut presentation, 1 << 20)
            .bytes,
    );
    assert!(
        rest.starts_with(&[row("1"), row("2"), "\x1b[2;1H\n".into()].concat()),
        "{rest:?}"
    );
}

#[test]
fn large_reads_are_stepped_so_pushes_are_counted_past_capacity() {
    let mut vt = vt(500, 2, 5);
    let headroom = 2 * Geometry::for_host(500, 2).unwrap().push_bound() as u32;
    let last = headroom + 100;
    feed(&mut vt, &lines(1..=last));
    let mut presentation = Presentation::new(500, 2);
    let (lead, mut restoration) = vt.grant(&mut presentation);
    assert!(text(&lead).ends_with(NOTICE));
    feed(&mut vt, &lines(last + 1..=last + 2));
    let rest = text(
        &vt.produce(&mut restoration, &mut presentation, 1 << 20)
            .bytes,
    );
    let expected: Vec<String> = (last - 3..last)
        .map(|number| row(&number.to_string()))
        .collect();
    assert!(rest.starts_with(&expected.concat()), "{rest:?}");
}

#[test]
fn push_bound_covers_the_worst_step_at_the_geometry_limits() {
    assert_eq!(Geometry::for_host(2, 1).unwrap().push_bound(), 65536);
    assert_eq!(Geometry::for_host(80, 24).unwrap().push_bound(), 1640);
    assert_eq!(Geometry::for_host(500, 200).unwrap().push_bound(), 264);
    for (cols, rows) in [(2usize, 1usize), (500, 200)] {
        let bound = Geometry::for_host(cols as u16, rows as u16)
            .unwrap()
            .push_bound();
        for worst in [
            b"\n".repeat(64),
            "x".repeat(64).into_bytes(),
            "漢".repeat(21).into_bytes(),
            b"\x1b[65535S".to_vec(),
            b"\x1b[2J".to_vec(),
            "漢\x1b[65535b".as_bytes().to_vec(),
        ] {
            let mut full = format!("\x1b[{rows};1H").into_bytes();
            full.extend("X".repeat(cols * rows).bytes());
            let before = host(cols, rows, &full).grid().history_size();
            let after = host(cols, rows, &[full, worst.clone()].concat())
                .grid()
                .history_size();
            assert!(
                after - before <= bound,
                "{cols}x{rows} {worst:?}: {} > {bound}",
                after - before
            );
        }
    }
}

#[test]
fn a_resize_that_may_drop_full_history_sets_the_eviction_flag() {
    let mut full = vt(10, 4, 3);
    feed(&mut full, &lines(1..=6));
    full.resize(Geometry::for_host(10, 2).unwrap());
    assert!(text(&restored(&mut full)).contains(NOTICE));
    let mut roomy = vt(10, 4, 10);
    feed(&mut roomy, &lines(1..=6));
    roomy.resize(Geometry::for_host(10, 2).unwrap());
    let prefix = text(&restored(&mut roomy));
    assert_eq!(restored_history(10, 2, &prefix), ["1", "2", "3", "4", "5"]);
}

#[test]
fn shrinking_while_on_the_alternate_screen_sets_the_eviction_flag() {
    for (cols, rows, notice) in [(9, 4, true), (10, 3, true), (11, 5, false), (10, 5, false)] {
        let mut vt = vt(10, 4, 10);
        feed(&mut vt, &lines(1..=6));
        feed(&mut vt, b"\x1b[?1049h");
        vt.resize(Geometry::for_host(cols, rows).unwrap());
        let prefix = text(&restored(&mut vt));
        assert_eq!(prefix.contains(NOTICE), notice, "{cols}x{rows}");
    }
}

#[test]
fn a_resize_retires_history_production_and_an_equal_size_does_not() {
    let mut vt = vt(10, 2, 10);
    feed(&mut vt, &lines(1..=4));
    let mut presentation = Presentation::new(10, 2);
    let (_, mut restoration) = vt.grant(&mut presentation);
    vt.produce(&mut restoration, &mut presentation, 1);
    vt.resize(Geometry::for_host(10, 2).unwrap());
    assert_eq!(
        text(&vt.produce(&mut restoration, &mut presentation, 1).bytes),
        row("2")
    );
    vt.resize(Geometry::for_host(12, 2).unwrap());
    let rest = text(&vt.produce(&mut restoration, &mut presentation, 1).bytes);
    assert!(rest.starts_with("\x1b[2;1H\n\x1b[1;1H\x1b[0m4"), "{rest:?}");
    assert_eq!(vt.geometry(), Geometry::for_host(12, 2).unwrap());
}

#[test]
fn restoration_roundtrips_into_a_fresh_terminal() {
    let mut vt = vt(20, 4, 100);
    let source = "one\r\n\x1b[1;31mtwo\x1b[0m\r\nthree is a long wrapped line here\r\nfour\r\nfive\r\nsix 漢字\r\nseven\x1b[2;3H";
    feed(&mut vt, source.as_bytes());
    let reference = host(20, 4, source.as_bytes());
    let restored = host(20, 4, &restored(&mut vt));
    let history = history(&restored);
    assert_eq!(
        history[history.len() - 4..],
        ["one", "two", "three is a long wrap", "ped line here"]
    );
    assert_eq!(screen(&restored), screen(&reference));
    assert_eq!(cursor(&restored), cursor(&reference));
    assert_eq!(cell(&restored, -3, 0).fg, cell(&reference, -3, 0).fg);
    assert_eq!(
        cell(&restored, -2, 19).flags,
        cell(&reference, -2, 19).flags
    );
}

#[test]
fn modes_cursor_style_and_title_are_restored() {
    let mut vt = vt(10, 3, 10);
    feed(
        &mut vt,
        b"\x1b]2;my\x7ftitle\x07\x1b[?25l\x1b[?1h\x1b=\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b[?1007l\x1b[3 q\x1b[?7l\x1b[4h\x1b[20h\x1b[?6h\x1b[2;3H",
    );
    let prefix = text(&restored(&mut vt));
    let modes = "\x1b[?25l\x1b[?1000l\x1b[?1003l\x1b[?1005l\x1b[?1007l\x1b[?1h\x1b=\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b[3 q\x1b[?7l\x1b[?6h\x1b[4h\x1b[20h\x1b[2;3H\x1b[0m\x1b]2;my%7Ftitle\x1b\\";
    assert!(prefix.ends_with(modes), "{prefix:?}");
}

#[test]
fn cursor_style_codes_follow_shape_and_blinking() {
    for (style, code) in [
        ("\x1b[1 q", "1"),
        ("\x1b[2 q", "0"),
        ("\x1b[4 q", "4"),
        ("\x1b[5 q", "5"),
        ("\x1b[6 q", "6"),
    ] {
        let mut vt = vt(10, 3, 10);
        feed(&mut vt, style.as_bytes());
        let prefix = text(&restored(&mut vt));
        assert!(
            prefix.contains(&format!("\x1b[?1007h\x1b[{code} q")),
            "{style:?} {prefix:?}"
        );
    }
}

#[test]
fn derived_restoration_keeps_the_prologue_modes_and_skips_pending_wrap() {
    let mut modal = vt(10, 3, 10);
    feed(&mut modal, b"\x1b[4h\x1b[?6h\x1b[3;3H");
    let prefix = text(&restore(&mut modal, &mut Presentation::new(11, 3)));
    assert!(
        prefix.ends_with(&format!(
            "\x1b[3;3H\x1b[0m{MODES}\x1b]2;climon2 exec app\x1b\\"
        )),
        "{prefix:?}"
    );
    let mut wrapped = vt(10, 3, 10);
    feed(&mut wrapped, b"0123456789");
    let prefix = text(&restore(&mut wrapped, &mut Presentation::new(10, 4)));
    assert!(prefix.contains("\x1b[1;10H\x1b[0m\x1b[?1l"), "{prefix:?}");
}

#[test]
fn synchronized_output_is_never_buffered_by_the_engine() {
    let mut vt = vt(10, 2, 10);
    let start = std::time::Instant::now();
    vt.feed(b"\x1b[?2026hvisible", start);
    assert!(vt.tick(start + std::time::Duration::from_millis(150)));
    assert_eq!(screen(&host(10, 2, &restored(&mut vt))), ["visible", ""]);
}

#[test]
fn set_options_title_events_never_reach_live_output() {
    let mut vt = vt(10, 2, 2);
    feed(&mut vt, b"\x1b]2;kept\x07");
    let output = lines(1..=40);
    assert_eq!(feed(&mut vt, &output), output);
    assert!(text(&restored(&mut vt)).ends_with("\x1b]2;kept\x1b\\"));
}

#[test]
fn consecutive_gaps_emit_one_notice() {
    let mut vt = vt(10, 2, 3);
    feed(&mut vt, &lines(1..=4));
    let mut presentation = Presentation::new(10, 2);
    let (_, mut restoration) = vt.grant(&mut presentation);
    vt.produce(&mut restoration, &mut presentation, 1);
    feed(&mut vt, &lines(5..=6));
    let gap = vt.produce(&mut restoration, &mut presentation, 1);
    assert_eq!(text(&gap.bytes), NOTICE);
    feed(&mut vt, &lines(7..=7));
    let rest = text(
        &vt.produce(&mut restoration, &mut presentation, 1 << 20)
            .bytes,
    );
    assert!(rest.starts_with("\x1b[2;1H\n\x1b[1;1H\x1b[0m4"), "{rest:?}");
}

#[test]
fn a_lead_notice_alone_is_scrolled_off_without_an_extra_line() {
    let mut vt = vt(10, 2, 3);
    feed(&mut vt, &lines(1..=5));
    let mut presentation = Presentation::new(10, 2);
    let (lead, mut restoration) = vt.grant(&mut presentation);
    assert!(text(&lead).ends_with(NOTICE));
    feed(&mut vt, b"\x1b[3J");
    let rest = text(
        &vt.produce(&mut restoration, &mut presentation, 1 << 20)
            .bytes,
    );
    assert!(rest.starts_with("\x1b[2;1H\n\x1b[1;1H\x1b[0m5"), "{rest:?}");
}

#[test]
fn a_soft_wrapped_last_history_row_is_ended_before_the_scroll_off() {
    let mut hard = vt(4, 2, 10);
    feed(&mut hard, b"abcdefgh\r\nx\r\ny");
    let prefix = text(&restored(&mut hard));
    assert!(
        prefix.contains("\x1b[0mabcd\x1b[0m\x1b[0mefgh\x1b[0m\r\n\x1b[2;1H\n\x1b[1;1H"),
        "{prefix:?}"
    );
    let mut soft = vt(4, 2, 10);
    feed(&mut soft, b"abcdefghijkl");
    let prefix = text(&restored(&mut soft));
    assert!(
        prefix.contains("\x1b[0mabcd\x1b[0m\r\n\x1b[2;1H\n\x1b[1;1H"),
        "{prefix:?}"
    );
}

#[test]
fn origin_mode_reestablishes_a_pending_wrap_after_the_modes() {
    let mut vt = vt(10, 3, 10);
    feed(&mut vt, b"\x1b[?6h0123456789");
    let prefix = text(&restored(&mut vt));
    assert!(
        prefix.ends_with("\x1b[?7h\x1b[?6h\x1b[1;10H\x1b[0m9\x1b[0m\x1b]2;climon2 exec app\x1b\\"),
        "{prefix:?}"
    );
}
