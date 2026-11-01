mod support;

use alacritty_terminal::term::cell::Flags;
use climon2_vt::Presentation;
use support::*;

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// The C2 bytes of a one-row screen showing `source`.
fn painted(source: &str) -> String {
    let mut vt = vt(20, 1, 0);
    feed(&mut vt, source.as_bytes());
    let prefix = text(&restored(&mut vt));
    let lead = "\x1b[?2026l\x1b[r\x1b[?6l\x1b[4l\x1b[20l\x1b[?7h\x1b[0m\x1b[1;1H\n\x1b[1;1H";
    let screen = prefix.strip_prefix(lead).unwrap();
    screen[..screen.find("\x1b[1;1H\x1b[0m\x1b7").unwrap()].to_owned()
}

#[test]
fn renditions_and_colours_become_sgr_parameters() {
    let mut cases = vec![
        ("1", "0;1"),
        ("2", "0;2"),
        ("3", "0;3"),
        ("4", "0;4"),
        ("4:2", "0;4:2"),
        ("4:3", "0;4:3"),
        ("4:4", "0;4:4"),
        ("4:5", "0;4:5"),
        ("7", "0;7"),
        ("8", "0;8"),
        ("9", "0;9"),
        ("38;5;200", "0;38;5;200"),
        ("38;5;3", "0;38;5;3"),
        ("38;2;1;2;3", "0;38;2;1;2;3"),
        ("48;5;17", "0;48;5;17"),
        ("48;2;250;0;9", "0;48;2;250;0;9"),
        ("58:5:9", "0;58:5:9"),
        ("58:2::1:2:3", "0;58:2::1:2:3"),
        ("1;3;9;31;42", "0;1;3;9;31;42"),
    ];
    let named: Vec<(String, String)> = (0..8)
        .flat_map(|n| {
            [
                (format!("{}", 30 + n), format!("0;{}", 30 + n)),
                (format!("{}", 90 + n), format!("0;{}", 90 + n)),
                (format!("{}", 40 + n), format!("0;{}", 40 + n)),
                (format!("{}", 100 + n), format!("0;{}", 100 + n)),
            ]
        })
        .collect();
    cases.extend(
        named
            .iter()
            .map(|(input, output)| (input.as_str(), output.as_str())),
    );
    for (input, output) in cases {
        assert_eq!(
            painted(&format!("\x1b[{input}mX")),
            format!("\x1b[0m\x1b[{output}mX\x1b[0m"),
            "{input}"
        );
    }
}

#[test]
fn rendition_changes_only_where_cells_differ() {
    assert_eq!(
        painted("a\x1b[1mbc\x1b[39;49md\x1b[0me\x1b[38;5;1mf"),
        "\x1b[0ma\x1b[0;1mbcd\x1b[0me\x1b[0;38;5;1mf\x1b[0m"
    );
}

#[test]
fn default_colours_and_trailing_blanks_are_not_painted() {
    assert_eq!(painted("\x1b[39;49mab  \x1b[1m "), "\x1b[0mab\x1b[0m");
    assert_eq!(painted("ab  \x1b[4m "), "\x1b[0mab  \x1b[0;4m \x1b[0m");
    assert_eq!(painted("ab\x1b[44m  "), "\x1b[0mab\x1b[0;44m  \x1b[0m");
}

#[test]
fn tabs_wide_characters_and_combining_marks_are_rendered() {
    assert_eq!(painted("a\tb"), "\x1b[0ma       b\x1b[0m");
    assert_eq!(painted("a漢b"), "\x1b[0ma漢b\x1b[0m");
    assert_eq!(
        painted("e\u{301}\u{302}x"),
        "\x1b[0me\u{301}\u{302}x\x1b[0m"
    );
}

#[test]
fn many_marks_on_one_cell_restore_and_serialize() {
    let marks = "\u{301}".repeat(300);
    let source = format!("x{marks}y");
    assert_eq!(painted(&source), format!("\x1b[0mx{marks}y\x1b[0m"));
    let mut vt = vt(20, 1, 0);
    feed(&mut vt, source.as_bytes());
    let host = host(20, 1, &restored(&mut vt));
    assert_eq!(cell(&host, 0, 0).zerowidth().map(<[char]>::len), Some(300));
    assert_eq!(cell(&host, 0, 1).c, 'y');
}

#[test]
fn wide_characters_wrap_in_history_as_on_the_host() {
    let mut vt = vt(10, 2, 10);
    let source = "123456789漢\r\nend\r\n";
    feed(&mut vt, source.as_bytes());
    let restored = host(10, 2, &restored(&mut vt));
    let reference = host(10, 2, source.as_bytes());
    let history = history(&restored);
    assert_eq!(history[history.len() - 2..], ["123456789", "漢"]);
    assert_eq!(screen(&restored), screen(&reference));
    assert!(cell(&restored, -2, 9).flags.contains(Flags::WRAPLINE));
}

#[test]
fn a_pending_wrap_is_recreated_in_direct_presentation() {
    for (source, reprint) in [
        ("0123456789", "\x1b[1;10H\x1b[0m9"),
        (
            "\x1b[31m012345678\x1b[1m9\x1b[0m",
            "\x1b[1;10H\x1b[0m\x1b[0;1;31m9",
        ),
        ("01234567漢", "\x1b[1;9H\x1b[0m漢"),
    ] {
        let mut vt = vt(10, 2, 10);
        feed(&mut vt, source.as_bytes());
        let prefix = text(&restored(&mut vt));
        assert!(
            prefix.contains(&format!("\x1b7{reprint}\x1b[0m\x1b[?1l")),
            "{source:?} {prefix:?}"
        );
        let mut host_bytes = prefix.into_bytes();
        host_bytes.extend_from_slice(b"X");
        let restored = host(10, 2, &host_bytes);
        let reference = host(10, 2, &[source.as_bytes(), b"X"].concat());
        assert_eq!(screen(&restored), screen(&reference), "{source:?}");
        assert_eq!(cursor(&restored), cursor(&reference), "{source:?}");
    }
}

#[test]
fn saved_and_current_cursor_pens_are_restored() {
    let mut vt = vt(10, 3, 10);
    feed(&mut vt, b"\x1b[2;4H\x1b[1;32m\x1b7\x1b[0;7m\x1b[3;5H");
    let prefix = text(&restored(&mut vt));
    assert!(
        prefix.contains("\x1b[2;4H\x1b[0;1;32m\x1b7\x1b[3;5H\x1b[0;7m\x1b[?1l"),
        "{prefix:?}"
    );
    let mut bytes = prefix.into_bytes();
    bytes.extend_from_slice(b"a\x1b8b");
    let host = host(10, 3, &bytes);
    assert_eq!(screen(&host), ["", "   b", "    a"]);
    assert!(cell(&host, 1, 3).flags.contains(Flags::BOLD));
    assert!(cell(&host, 2, 4).flags.contains(Flags::INVERSE));
}

#[test]
fn a_derived_row_is_painted_at_its_session_position() {
    let mut vt = vt(10, 2, 10);
    feed(&mut vt, b"top\r\n\x1b[2;3Hlow");
    let prefix = text(&restore(&mut vt, &mut Presentation::new(30, 5)));
    assert!(
        prefix.starts_with("\x1b[?2026l\x1b[r\x1b[?6l\x1b[4l\x1b[20l\x1b[?7h\x1b[0m\x1b[5;1H\n\n\n\n\n\x1b[1;1H\x1b[0mtop\x1b[0m\x1b[2;1H\x1b[0m  low\x1b[0m"),
        "{prefix:?}"
    );
}
