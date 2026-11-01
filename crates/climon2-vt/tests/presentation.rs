use climon2_vt::{Mode, Presentation};

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
fn committed_live_output_tracks_the_alternate_screen_and_synchronized_updates() {
    let mut presentation = Presentation::new(80, 24);
    presentation.commit_live(b"text\x1b[?1049");
    presentation.commit_live(b"h\x1b[?25;2026h");
    assert_eq!(presentation.end_sync(), b"\x1b[?2026l");
    assert_eq!(presentation.end_sync(), b"");
    presentation.commit_live(b"\x1b[?2026h\x1b[?2026l\x1b[?1049l\x1b[?1049h");
    assert_eq!(presentation.end_sync(), b"");
    assert_eq!(presentation.release(), release(true));
    assert_eq!(presentation.release(), release(false));
}

#[test]
fn leaving_the_alternate_screen_in_live_output_needs_no_release_of_it() {
    let mut presentation = Presentation::new(80, 24);
    presentation.commit_live(b"\x1b[?1049h\x1b[?2026h");
    presentation.commit_live(b"\x1b[?1049l");
    assert_eq!(presentation.release(), release(false));
    assert_eq!(presentation.end_sync(), b"");
}
