mod support;

use climon2_vt::{Geometry, Mode, Presentation, Vt};
use support::*;

const ERASE: &str = "\x1b[0m\x1b[2K";

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// A Derived controller (host 12x4 over a 10x2 session) whose restoration
/// has been committed; returns the host bytes so far.
fn derived(vt: &mut Vt) -> (Presentation, Vec<u8>) {
    let mut presentation = Presentation::new(12, 4);
    let bytes = restore(vt, &mut presentation);
    assert_eq!(presentation.mode(), Mode::Derived);
    (presentation, bytes)
}

fn painted(line: usize, row: &str) -> String {
    format!("\x1b[{line};1H{ERASE}\x1b[0m{row}\x1b[0m")
}

#[test]
fn nothing_changed_derives_nothing() {
    let mut vt = vt(10, 2, 10);
    feed(&mut vt, b"a\r\nb");
    let (mut presentation, _) = derived(&mut vt);
    assert_eq!(vt.derive(&mut presentation), b"");
}

#[test]
fn changed_rows_and_the_cursor_are_repainted() {
    let mut vt = vt(10, 2, 10);
    feed(&mut vt, b"a\r\nb");
    let (mut presentation, _) = derived(&mut vt);
    feed(&mut vt, b"c");
    assert_eq!(
        text(&vt.derive(&mut presentation)),
        [painted(2, "bc"), "\x1b[2;3H".into()].concat()
    );
    feed(&mut vt, b"\x1b[1;1H");
    assert_eq!(text(&vt.derive(&mut presentation)), "\x1b[1;1H");
}

#[test]
fn pushed_rows_reach_native_scrollback_then_the_screen_is_repainted() {
    let mut vt = vt(10, 2, 10);
    feed(&mut vt, b"a\r\nb");
    let (mut presentation, mut host_bytes) = derived(&mut vt);
    feed(&mut vt, b"\r\nc\r\nd");
    let update = text(&vt.derive(&mut presentation));
    let expected = [
        painted(1, "a"),
        painted(2, "b"),
        "\x1b[4;1H\n\n".into(),
        painted(1, "c"),
        painted(2, "d"),
        "\x1b[3;1H\x1b[0m\x1b[J\x1b[2;2H".into(),
    ]
    .concat();
    assert_eq!(update, expected);
    host_bytes.extend(update.bytes());
    let host = host(12, 4, &host_bytes);
    assert_eq!(&history(&host)[history(&host).len() - 2..], ["a", "b"]);
    assert_eq!(screen(&host), ["c", "d", "", ""]);
}

#[test]
fn a_pushed_run_longer_than_retention_starts_with_the_notice() {
    let mut vt = vt(10, 2, 2);
    let (mut presentation, mut host_bytes) = derived(&mut vt);
    feed(&mut vt, &lines(1..=5));
    let update = text(&vt.derive(&mut presentation));
    assert!(
        update.starts_with(&format!("\x1b[1;1H{ERASE}[climon2: ol{}", painted(2, "3"))),
        "{update:?}"
    );
    host_bytes.extend(update.bytes());
    let host = host(12, 4, &host_bytes);
    let history = history(&host);
    assert_eq!(&history[history.len() - 3..], ["[climon2: ol", "3", "4"]);
    assert_eq!(screen(&host), ["5", "", "", ""]);
}

#[test]
fn zero_retention_pushes_nothing_but_repaints() {
    let mut vt = vt(10, 2, 0);
    let (mut presentation, _) = derived(&mut vt);
    feed(&mut vt, &lines(1..=5));
    let update = text(&vt.derive(&mut presentation));
    assert_eq!(
        update,
        [
            painted(1, "5"),
            painted(2, ""),
            "\x1b[3;1H\x1b[0m\x1b[J\x1b[2;1H".into()
        ]
        .concat()
    );
}

#[test]
fn screen_switches_follow_the_engine_and_pushes_use_the_normal_screen() {
    let mut vt = vt(10, 2, 10);
    let (mut presentation, _) = derived(&mut vt);
    feed(&mut vt, b"\x1b[?1049hA");
    let update = text(&vt.derive(&mut presentation));
    assert!(
        update.starts_with(&format!("\x1b[?1049h{}", painted(1, "A"))),
        "{update:?}"
    );
    assert!(text(&presentation.release()).contains("\x1b[?1049l"));
    feed(&mut vt, b"\x1b[?1049h");
    let mut presentation = Presentation::new(12, 4);
    restore(&mut vt, &mut presentation);
    feed(&mut vt, b"\x1b[?1049l1\r\n2\r\n3\x1b[?1049hB");
    let update = text(&vt.derive(&mut presentation));
    assert!(
        update.starts_with(&format!(
            "\x1b[?1049l{}\x1b[4;1H\n\x1b[?1049h",
            painted(1, "1")
        )),
        "{update:?}"
    );
    feed(&mut vt, b"\x1b[?1049l");
    let update = text(&vt.derive(&mut presentation));
    assert!(
        update.starts_with(&format!("\x1b[?1049l{}", painted(1, "2"))),
        "{update:?}"
    );
}

#[test]
fn modes_style_title_and_bells_are_sent_when_they_change() {
    let mut vt = vt(10, 2, 10);
    let (mut presentation, _) = derived(&mut vt);
    feed(
        &mut vt,
        b"\x1b[?1h\x1b[?25l\x1b[4 q\x1b]2;t\x07\x07\x07\x1b[?7l\x1b[4h\x1b[?6l\x1b[20h",
    );
    assert_eq!(
        text(&vt.derive(&mut presentation)),
        "\x1b[?25l\x1b[?1h\x1b[4 q\x1b]2;t\x1b\\\x07"
    );
    assert_eq!(vt.derive(&mut presentation), b"");
    feed(&mut vt, b"\x1b[?25h\x1b]2;t\x07");
    assert_eq!(text(&vt.derive(&mut presentation)), "\x1b[?25h");
}

#[test]
fn clearing_history_repaints_without_pushing() {
    let mut vt = vt(10, 2, 10);
    feed(&mut vt, b"a\r\nb");
    let (mut presentation, _) = derived(&mut vt);
    feed(&mut vt, b"\r\nc\x1b[3J");
    let update = text(&vt.derive(&mut presentation));
    assert_eq!(
        update,
        [
            painted(1, "b"),
            painted(2, "c"),
            "\x1b[3;1H\x1b[0m\x1b[J\x1b[2;2H".into()
        ]
        .concat()
    );
}

#[test]
fn derived_output_keeps_the_session_rectangle_on_a_larger_host() {
    let mut vt = vt(10, 3, 100);
    let (mut presentation, mut host_bytes) = derived_at(&mut vt, 30, 6);
    let source = b"\x1b[1;31mred line that wraps around\x1b[0m\r\nnext\x1b[2;8Hmid\r\n\r\nend\x1b[1;1H\x1b[2K";
    for chunk in source.chunks(7) {
        feed(&mut vt, chunk);
        host_bytes.extend(vt.derive(&mut presentation));
    }
    let reference = host(10, 3, source);
    let host = host(30, 6, &host_bytes);
    assert_eq!(screen(&host)[..3], screen(&reference)[..]);
    assert_eq!(screen(&host)[3..], ["", "", ""]);
    assert_eq!(
        history(&host)
            .iter()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>(),
        history(&reference).iter().collect::<Vec<_>>()
    );
    assert_eq!(cell(&host, -1, 0).fg, cell(&reference, -1, 0).fg);
    assert_eq!(cursor(&host).0, cursor(&reference).0);
}

fn derived_at(vt: &mut Vt, cols: u16, rows: u16) -> (Presentation, Vec<u8>) {
    let mut presentation = Presentation::new(cols, rows);
    let bytes = restore(vt, &mut presentation);
    (presentation, bytes)
}

#[test]
fn resizes_between_direct_and_derived_switch_presentation() {
    let mut vt = vt(10, 2, 10);
    feed(&mut vt, b"a\x1b[4h");
    let mut presentation = Presentation::new(10, 2);
    restore(&mut vt, &mut presentation);
    assert_eq!(vt.present(&mut presentation, 10, 2), b"");
    let update = text(&vt.present(&mut presentation, 10, 3));
    assert_eq!(presentation.mode(), Mode::Derived);
    assert_eq!(
        update,
        [
            "\x1b[?2026l\x1b[r\x1b[?6l\x1b[4l\x1b[20l\x1b[?7h\x1b[0m",
            &painted(1, "a"),
            &painted(2, ""),
            "\x1b[3;1H\x1b[0m\x1b[J\x1b[1;2H",
        ]
        .concat()
    );
    let update = text(&vt.present(&mut presentation, 12, 3));
    assert_eq!(
        update,
        [
            painted(1, "a"),
            painted(2, ""),
            "\x1b[3;1H\x1b[0m\x1b[J\x1b[1;2H".into()
        ]
        .concat()
    );
    vt.resize(Geometry::for_host(12, 3).unwrap());
    let update = text(&vt.present(&mut presentation, 12, 3));
    assert_eq!(presentation.mode(), Mode::Direct);
    assert_eq!(
        update,
        [
            painted(1, "a"),
            painted(2, ""),
            painted(3, ""),
            "\x1b[1;2H\x1b[?6l\x1b[20l\x1b[?7h\x1b[4h".into()
        ]
        .concat()
    );
}

#[test]
fn pushed_rows_beyond_a_host_screenful_are_sent_in_chunks() {
    let mut vt = vt(10, 2, 10);
    let (mut presentation, mut host_bytes) = derived_at(&mut vt, 12, 3);
    feed(&mut vt, &lines(1..=5));
    let update = text(&vt.derive(&mut presentation));
    let chunks = [
        painted(1, "1"),
        painted(2, "2"),
        painted(3, "3"),
        "\x1b[3;1H\n\n\n".into(),
        painted(1, "4"),
        "\x1b[3;1H\n".into(),
    ]
    .concat();
    assert!(update.starts_with(&chunks), "{update:?}");
    host_bytes.extend(update.bytes());
    let host = host(12, 3, &host_bytes);
    assert_eq!(
        &history(&host)[history(&host).len() - 4..],
        ["1", "2", "3", "4"]
    );
    assert_eq!(screen(&host), ["5", "", ""]);
}

#[test]
fn changes_on_the_alternate_screen_stay_on_it() {
    let mut vt = vt(10, 2, 10);
    feed(&mut vt, b"\x1b[?1049h");
    let (mut presentation, _) = derived(&mut vt);
    feed(&mut vt, b"x");
    assert_eq!(
        text(&vt.derive(&mut presentation)),
        [painted(1, "x"), "\x1b[1;2H".into()].concat()
    );
}

#[test]
fn a_single_retained_row_still_follows_the_notice() {
    let mut vt = vt(10, 2, 1);
    let (mut presentation, _) = derived(&mut vt);
    feed(&mut vt, &lines(1..=4));
    let update = text(&vt.derive(&mut presentation));
    assert!(
        update.starts_with(&format!("\x1b[1;1H{ERASE}[climon2: ol{}", painted(2, "3"))),
        "{update:?}"
    );
}
