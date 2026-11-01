mod support;

use climon2_vt::{Geometry, Mode, Presentation};
use support::*;

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// The suffix after phase A's preservation scroll of a 3-row host.
fn suffix(prefix: &str) -> &str {
    let lead = "\x1b[?2026l\x1b[r\x1b[?6l\x1b[4l\x1b[20l\x1b[?7h\x1b[0m\x1b[3;1H\n\n\n";
    prefix.strip_prefix(lead).unwrap()
}

const ALTERNATE_APP: &[u8] = b"n1\r\nn2\x1b[?1049h\x1b[2;2HA\x1b[3;3H\x1b7\x1b[1;5H\x1b[?1h";

#[test]
fn the_alternate_screen_is_restored_after_the_normal_screen_beneath_it() {
    let mut vt = vt(10, 3, 10);
    feed(&mut vt, ALTERNATE_APP);
    let mut presentation = Presentation::new(10, 3);
    let prefix = text(&restore(&mut vt, &mut presentation));
    let expected = [
        "\x1b[1;1H\x1b[0mn1\x1b[0m\x1b[2;1H\x1b[0mn2\x1b[0m",
        "\x1b[2;3H\x1b[0m\x1b7\x1b[2;3H\x1b[0m",
        "\x1b[?1049h",
        "\x1b[2;1H\x1b[0m A\x1b[0m",
        "\x1b[3;3H\x1b[0m\x1b7\x1b[1;5H\x1b[0m",
        "\x1b>\x1b[?2004l",
    ]
    .concat();
    assert!(suffix(&prefix).starts_with(&expected), "{prefix:?}");
    assert!(prefix.contains("\x1b[?1h"));
    let host = host(10, 3, prefix.as_bytes());
    assert_eq!(screen(&host), ["", " A", ""]);
    assert!(text(&presentation.release()).starts_with("\x1b[?2026l\x1b[?1049l\x1b[r"));
}

#[test]
fn presentation_enters_the_alternate_screen_only_with_the_suffix() {
    let mut vt = vt(10, 3, 10);
    feed(&mut vt, b"h\r\n\r\n\r\n");
    feed(&mut vt, ALTERNATE_APP);
    let mut presentation = Presentation::new(10, 3);
    let (_, mut restoration) = vt.grant(&mut presentation);
    let first = vt.produce(&mut restoration, &mut presentation, 1);
    assert!(!first.done);
    assert!(!text(&presentation.release()).contains("1049"));
    let rest = vt.produce(&mut restoration, &mut presentation, 1 << 20);
    assert!(rest.done);
    assert!(text(&presentation.release()).contains("\x1b[?1049l"));
}

#[test]
fn restoring_leaves_the_alternate_screen_exactly_as_it_was() {
    let mut vt = vt(10, 3, 10);
    feed(&mut vt, ALTERNATE_APP);
    let first = text(&restored(&mut vt));
    let second = text(&restored(&mut vt));
    assert_eq!(first, second);
    feed(&mut vt, b"Z\x1b8S");
    let prefix = text(&restored(&mut vt));
    let host = host(10, 3, prefix.as_bytes());
    assert_eq!(screen(&host), ["    Z", " A", "  S"]);
}

#[test]
fn after_leaving_the_alternate_screen_normal_state_matches_an_unrestored_session() {
    let source = [&b"h1\r\nh2\r\nh3\r\nh4\r\n"[..], ALTERNATE_APP].concat();
    let mut granted = vt(10, 3, 10);
    let mut control = vt(10, 3, 10);
    feed(&mut granted, &source);
    feed(&mut control, &source);
    restored(&mut granted);
    let leave = b"\x1b[?1049lX\x1b8Y";
    feed(&mut granted, leave);
    feed(&mut control, leave);
    let granted = text(&restored(&mut granted));
    assert_eq!(granted, text(&restored(&mut control)));
    let host = host(10, 3, granted.as_bytes());
    assert_eq!(
        history(&host)[history(&host).len() - 3..],
        ["h1", "h2", "h3"]
    );
    assert_eq!(screen(&host), ["h4", "n1", "n2Y"]);
    assert!(granted.contains("\x1b[?1h"));
}

#[test]
fn e1_sets_the_normal_saved_cursor_to_the_current_cursor_after_a_hidden_resize() {
    let after = |grant: bool, leave: &[u8]| {
        let mut vt = vt(10, 4, 10);
        feed(&mut vt, b"\x1b[3;1Habcdefgh\x1b[?1049halt");
        vt.resize(Geometry::for_host(4, 4).unwrap());
        if grant {
            restored(&mut vt);
        }
        feed(&mut vt, leave);
        screen(&host(4, 4, &restored(&mut vt)))
    };
    let granted = after(true, b"\x1b[?1049l\x1b8@");
    assert_eq!(granted, after(false, b"\x1b[?1049l@"));
    assert_ne!(granted, after(false, b"\x1b[?1049l\x1b8@"));
}

#[test]
fn modes_set_on_the_alternate_screen_survive_restoration() {
    let mut vt = vt(10, 3, 10);
    feed(&mut vt, b"\x1b[?1049h\x1b[?2004h\x1b=");
    restored(&mut vt);
    assert!(vt.bracketed_paste());
    feed(&mut vt, b"\x1b[?1049l");
    let prefix = text(&restored(&mut vt));
    assert!(prefix.contains("\x1b=\x1b[?2004h"), "{prefix:?}");
    assert!(!prefix.contains("1049h"));
}

#[test]
fn presentation_mode_follows_the_host_size_at_each_grant() {
    let mut vt = vt(10, 3, 10);
    for (cols, rows, mode) in [
        (10, 3, Mode::Direct),
        (11, 3, Mode::Derived),
        (10, 4, Mode::Derived),
        (9, 2, Mode::Direct),
    ] {
        let mut presentation = Presentation::new(cols, rows);
        vt.grant(&mut presentation);
        assert_eq!(presentation.mode(), mode, "{cols}x{rows}");
    }
}
