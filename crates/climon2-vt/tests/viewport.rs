mod support;

use climon2_vt::Geometry;
use support::*;

/// STA-02 (crate case): the active viewport is its characters only, row by
/// row and every cell of each row, so a sample compares what the screen
/// says, not how it says it. Styles and colours, cursor-only movement, an
/// identical redraw and a switch to or from the alternate screen holding
/// the same characters leave it unchanged; a changed character, a combining
/// mark and a resize change it. Scrollback is not part of it.
#[test]
fn the_viewport_is_the_active_screens_characters_only() {
    let mut vt = vt(5, 2, 10);
    assert_eq!(vt.viewport(), "     \n     ");
    feed(&mut vt, b"ab");
    let written = "ab   \n     ";
    assert_eq!(vt.viewport(), written);
    let unchanged: [&[u8]; 5] = [
        b"\x1b[H\x1b[1;31;44mab\x1b[0m",
        b"\x1b[2;4H",
        b"\x1b[2J\x1b[Hab",
        b"\x1b[?1049h\x1b[Hab",
        b"\x1b[?1049l",
    ];
    for bytes in unchanged {
        feed(&mut vt, bytes);
        assert_eq!(vt.viewport(), written, "{bytes:?}");
    }
    feed(&mut vt, b"\x1b[Hac");
    assert_eq!(vt.viewport(), "ac   \n     ");
    feed(&mut vt, "\x1b[1;3He\u{301}".as_bytes());
    assert_eq!(vt.viewport(), "ace\u{301}  \n     ");
    feed(&mut vt, b"\x1b[2;1H\r\n\r\nzz");
    assert_eq!(
        vt.viewport(),
        "     \nzz   ",
        "scrolled history is not shown"
    );
    vt.resize(Geometry::for_host(3, 1).unwrap());
    assert_eq!(vt.viewport(), "zz ");
}
