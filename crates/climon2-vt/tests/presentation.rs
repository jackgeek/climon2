mod support;

use alacritty_terminal::term::TermMode;
use climon2_vt::{Geometry, Mode, Presentation};
use support::*;

const BASELINE: &[u8] = b"\x1b[r\x1b[?6l\x1b[4l\x1b[20l\x1b[?7h\
\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1004l\x1b[?2004l\
\x1b[?1l\x1b>\x1b[?25h\x1b[0 q\x1b[0m";

fn release(alternate: bool) -> Vec<u8> {
    let screen: &[u8] = if alternate { b"\x1b[?1049l" } else { b"" };
    [b"\x1b[?2026l", screen, BASELINE].concat()
}

#[test]
fn a_fresh_presentation_is_direct_with_nothing_open() {
    let mut presentation = Presentation::new(80, 24);
    assert_eq!(presentation.mode(), Mode::Direct);
    assert_eq!(presentation.end_sync(), b"");
    assert_eq!(presentation.release(), release(false));
}

#[test]
fn an_empty_restoration_releases_at_the_first_cell() {
    let mut vt = vt(20, 6, 0);
    let mut presentation = Presentation::new(20, 6);
    restore(&mut vt, &mut presentation);
    assert_eq!(
        presentation.release(),
        [release(false), b"\x1b[1;1H".to_vec()].concat()
    );
}

#[test]
fn committed_live_output_tracks_the_alternate_screen_and_synchronized_updates() {
    let mut vt = vt(80, 24, 0);
    let mut presentation = Presentation::new(80, 24);
    commit(&mut vt, &mut presentation, b"text\x1b[?1049");
    commit(&mut vt, &mut presentation, b"h\x1b[?25;2026h");
    assert_eq!(presentation.end_sync(), b"\x1b[?2026l");
    assert_eq!(presentation.end_sync(), b"");
    commit(
        &mut vt,
        &mut presentation,
        b"\x1b[?2026h\x1b[?2026l\x1b[?1049l\x1b[?1049h",
    );
    assert_eq!(presentation.end_sync(), b"");
    assert_eq!(
        presentation.release(),
        [release(true), b"\x1b[1;5H".to_vec()].concat()
    );
    assert_eq!(
        presentation.release(),
        [release(false), b"\x1b[1;5H".to_vec()].concat()
    );
}

#[test]
fn leaving_the_alternate_screen_in_live_output_needs_no_release_of_it() {
    let mut vt = vt(80, 24, 0);
    let mut presentation = Presentation::new(80, 24);
    commit(&mut vt, &mut presentation, b"\x1b[?1049h\x1b[?2026h");
    commit(&mut vt, &mut presentation, b"\x1b[?1049l");
    assert_eq!(
        presentation.release(),
        [release(false), b"\x1b[1;1H".to_vec()].concat()
    );
    assert_eq!(presentation.end_sync(), b"");
}

#[test]
fn direct_release_preserves_the_normal_cursor_without_restoring_application_modes() {
    for source in [
        &b"\x1b[3;7H"[..],
        b"\x1b[2;5r\x1b[?6h\x1b[2;7H\x1b[31m",
        b"\x1b[4;9H\x1b[?1049h\x1b[2;3H",
        b"\x1b[4;9H\x1b[?1049h\x1b[2;3H\x1b[?1049l\x1b[5;11H",
    ] {
        let mut vt = vt(20, 6, 0);
        let mut presentation = Presentation::new(20, 6);
        commit(&mut vt, &mut presentation, source);
        let mut reference = source.to_vec();
        reference.extend_from_slice(b"\x1b[?1049l");
        let expected = cursor(&host(20, 6, &reference));
        let mut bytes = source.to_vec();
        bytes.extend(presentation.release());
        let released = host(20, 6, &bytes);
        assert_eq!(cursor(&released), expected, "{source:?}");
        assert!(
            !released
                .mode()
                .intersects(TermMode::ORIGIN | TermMode::ALT_SCREEN)
        );
        let mut xterm = vt100::Parser::new(6.try_into().unwrap(), 20.try_into().unwrap(), 0);
        xterm.process(&bytes);
        assert_eq!(
            xterm.screen().cursor_position(),
            (expected.0 as u16, expected.1 as u16),
            "{source:?}"
        );
    }
}

#[test]
fn release_uses_the_committed_restoration_cursor_not_later_session_output() {
    let mut vt = vt(20, 6, 0);
    feed(&mut vt, b"\x1b[3;7H");
    let mut first = Presentation::new(20, 6);
    let (mut bytes, mut restoration) = vt.grant(&mut first);
    feed(&mut vt, b"\x1b[5;11H");
    let chunk = vt.produce(&mut restoration, &mut first, 4096);
    assert!(chunk.done);
    bytes.extend(chunk.bytes);
    let mut second = Presentation::new(20, 6);
    let mut second_bytes = restore(&mut vt, &mut second);
    bytes.extend(first.release());
    second_bytes.extend(second.release());
    assert_eq!(cursor(&host(20, 6, &bytes)), (2, 6, false));
    assert_eq!(cursor(&host(20, 6, &second_bytes)), (4, 10, false));
}

#[test]
fn derived_release_keeps_the_last_painted_normal_cursor() {
    let mut vt = vt(20, 6, 0);
    let mut presentation = Presentation::new(30, 8);
    let mut bytes = restore(&mut vt, &mut presentation);
    feed(&mut vt, b"\x1b[4;9H");
    bytes.extend(vt.derive(&mut presentation));
    feed(&mut vt, b"\x1b[5;11H\x1b[?1049h\x1b[2;3H");
    bytes.extend(vt.derive(&mut presentation));
    bytes.extend(presentation.release());
    assert_eq!(cursor(&host(30, 8, &bytes)), (3, 8, false));
    vt.resize(Geometry::for_host(10, 3).unwrap());
    let mut resized = Presentation::new(10, 3);
    let mut bytes = restore(&mut vt, &mut resized);
    bytes.extend(resized.release());
    assert_eq!(cursor(&host(10, 3, &bytes)), (2, 9, false));
}

#[test]
fn derived_history_pushes_leave_the_normal_cursor_at_the_host_bottom() {
    let mut vt = vt(20, 6, 10);
    let mut presentation = Presentation::new(30, 8);
    let mut bytes = restore(&mut vt, &mut presentation);
    feed(&mut vt, b"\x1b[4;9H\x1b[?1049h");
    bytes.extend(vt.derive(&mut presentation));
    feed(&mut vt, b"\x1b[?1049l\x1b[6;1H\nhistory\n\x1b[?1049h");
    bytes.extend(vt.derive(&mut presentation));
    let released = presentation.release();
    assert_eq!(released, [release(true), b"\x1b[8;1H".to_vec()].concat());
    bytes.extend(released);
    assert_eq!(cursor(&host(30, 8, &bytes)), (7, 0, false));
}

#[test]
fn direct_resize_updates_the_cursor_used_for_release() {
    let mut vt = vt(20, 6, 0);
    let mut presentation = Presentation::new(20, 6);
    restore(&mut vt, &mut presentation);
    commit(&mut vt, &mut presentation, b"\x1b[5;11H");
    vt.resize(Geometry::for_host(10, 3).unwrap());
    assert!(vt.present(&mut presentation, 10, 3).is_empty());
    let bytes = presentation.release();
    assert_eq!(cursor(&host(10, 3, &bytes)), (2, 9, false));
}
