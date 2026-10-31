//! THROWAWAY #43: restore derived from unchanged Alacritty, checked against an
//! independent oracle. Expected screens are hand-written; vt100 (a separate
//! screen model) only replays the derived bytes as a fresh attached terminal.

use alacritty_embedding_probe::restore::restore;
use alacritty_embedding_probe::view;
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use core::num::NonZeroU16;
use vte::ansi::Processor;

fn engine(columns: usize, lines: usize, history: usize, bytes: &[u8]) -> Term<VoidListener> {
    let config = Config {
        scrolling_history: history,
        ..Config::default()
    };
    let mut term = Term::new(config, &TermSize::new(columns, lines), VoidListener);
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, bytes);
    term
}

/// Derives restore bytes and replays them into a fresh oracle terminal.
fn attach(term: &mut Term<VoidListener>) -> vt100::Parser {
    use alacritty_terminal::grid::Dimensions;
    let restored = restore(term);
    let rows = NonZeroU16::new(term.screen_lines() as u16).unwrap();
    let columns = NonZeroU16::new(term.columns() as u16).unwrap();
    let mut host = vt100::Parser::new(rows, columns, 100);
    host.process(&restored);
    host
}

fn screen(host: &vt100::Parser) -> Vec<String> {
    let columns = host.screen().size().1.get();
    host.screen().rows(0, columns).collect()
}

/// Screen rows with the oracle scrolled to the top of its native history.
fn history_and_screen(host: &mut vt100::Parser) -> Vec<String> {
    let lines = host.screen().size().0.get() as usize;
    host.screen_mut().set_scrollback(usize::MAX);
    let history = host.screen().scrollback();
    let mut rows = screen(host);
    host.screen_mut().set_scrollback(0);
    rows.truncate(history.min(lines));
    rows.extend(screen(host));
    rows
}

/// Columns and lines applied while the application is on alternate screen.
type Geometry = (usize, usize);

const NORMAL: &[u8] = b"h0\r\nh1\r\nh2\r\n\x1b[1mB\x1b[0mn1\r\nn2";
const ALTERNATE: &[u8] = b"\x1b[?1049h\x1b[2;3HALT";

#[test]
fn alternate_restore_reproduces_normal_and_history_then_alternate_then_normal_on_exit() {
    for (limit, expected_normal) in [
        (0, vec!["h2", "Bn1", "n2X"]),
        (2, vec!["h0", "h1", "h2", "Bn1", "n2X"]),
    ] {
        let mut term = engine(10, 3, limit, &[NORMAL, ALTERNATE].concat());
        let mut host = attach(&mut term);

        assert!(host.screen().alternate_screen(), "limit {limit}");
        assert_eq!(screen(&host), ["", "  ALT", ""], "limit {limit}");
        assert_eq!(host.screen().cursor_position(), (1, 5));

        host.process(b"\x1b[?1049lX");
        assert!(!host.screen().alternate_screen());
        assert_eq!(screen(&host), ["h2", "Bn1", "n2X"], "limit {limit}");
        assert_eq!(host.screen().cursor_position(), (2, 3));
        assert!(host.screen().cell(1, 0).unwrap().bold());
        assert!(!host.screen().cell(1, 1).unwrap().bold());
        assert!(!host.screen().cell(2, 2).unwrap().bold());
        assert_eq!(
            history_and_screen(&mut host),
            expected_normal,
            "limit {limit}"
        );
    }
}

#[test]
fn alternate_saved_cursor_and_pen_survive_restore() {
    let mut term = engine(
        10,
        3,
        2,
        &[
            NORMAL,
            b"\x1b[?1049h\x1b[3;4H\x1b[31m\x1b7\x1b[0m\x1b[1;1HA",
        ]
        .concat(),
    );
    let mut host = attach(&mut term);
    host.process(b"\x1b8Z");
    assert_eq!(screen(&host), ["A", "", "   Z"]);
    assert_eq!(
        host.screen().cell(2, 3).unwrap().fgcolor(),
        vt100::Color::Idx(1)
    );
    host.process(b"\x1b[?1049lX");
    assert_eq!(screen(&host), ["h2", "Bn1", "n2X"]);
    assert_eq!(host.screen().cursor_position(), (2, 3));
}

#[test]
fn normal_reflowed_while_hidden_is_restored_at_the_new_width_with_wrap_continuity() {
    let mut term = engine(10, 3, 1, b"top\r\n0123456789AB\r\nxy\x1b[?1049hALT");
    term.resize(TermSize::new(6, 3));
    let mut host = attach(&mut term);
    assert_eq!(screen(&host), ["", "", "  ALT"]);
    host.process(b"\x1b[?1049lX");
    assert_eq!(
        history_and_screen(&mut host),
        ["top", "012345", "6789AB", "xyX"]
    );
    assert!(host.screen().row_wrapped(0));
    assert_eq!(host.screen().cursor_position(), (2, 3));
}

#[test]
fn alternate_modes_colours_and_pen_are_restored() {
    let mut term = engine(
        10,
        3,
        0,
        b"n\x1b[?1049h\x1b[?2004h\x1b[?1h\x1b[?25l\x1b[?1000h\x1b[?1006h\x1b[H\x1b[31;44mR",
    );
    let mut host = attach(&mut term);
    let restored = host.screen();
    assert!(restored.bracketed_paste());
    assert!(restored.application_cursor());
    assert!(restored.hide_cursor());
    assert_eq!(
        restored.mouse_protocol_mode(),
        vt100::MouseProtocolMode::PressRelease
    );
    assert_eq!(
        restored.mouse_protocol_encoding(),
        vt100::MouseProtocolEncoding::Sgr
    );
    assert_eq!(restored.cell(0, 0).unwrap().fgcolor(), vt100::Color::Idx(1));
    assert_eq!(restored.cell(0, 0).unwrap().bgcolor(), vt100::Color::Idx(4));
    host.process(b"x");
    assert_eq!(screen(&host), ["Rx", "", ""]);
    assert_eq!(
        host.screen().cell(0, 1).unwrap().bgcolor(),
        vt100::Color::Idx(4)
    );
}

#[test]
fn normal_only_restore_reproduces_history_screen_and_cursor() {
    let mut term = engine(10, 3, 2, NORMAL);
    let mut host = attach(&mut term);
    assert!(!host.screen().alternate_screen());
    host.process(b"X");
    assert_eq!(
        history_and_screen(&mut host),
        ["h0", "h1", "h2", "Bn1", "n2X"]
    );
}

/// Restoration reads the inactive normal grid through the public swap seam.
/// The live engine must behave exactly as if no restore had happened.
#[test]
fn restore_does_not_perturb_the_source_of_truth() {
    let continuations: [&[u8]; 4] = [
        b"\x1b[?1049lX",
        b"\x1b8Z\x1b[?1049lX",
        b"\x1b[?1049l\x1b8Z",
        b"\x1b[5;5HW\x1b[?1049l\x1b[?1049hY",
    ];
    let setups: [(&[u8], Option<Geometry>); 3] = [
        (&[NORMAL, ALTERNATE].concat(), None),
        (
            b"top\r\n0123456789AB\r\nxy\x1b7\x1b[H\x1b[?1049hALT",
            Some((6, 3)),
        ),
        (b"top\r\n0123456789AB\r\nxy\x1b[?1049hALT", Some((6, 2))),
    ];
    for (setup, resize) in setups {
        for continuation in continuations {
            let mut control = engine(10, 3, 1, setup);
            let mut restored = engine(10, 3, 1, setup);
            if let Some((columns, lines)) = resize {
                control.resize(TermSize::new(columns, lines));
                restored.resize(TermSize::new(columns, lines));
            }
            let _ = restore(&mut restored);
            let mut parser: Processor = Processor::new();
            parser.advance(&mut control, continuation);
            let mut parser: Processor = Processor::new();
            parser.advance(&mut restored, continuation);
            assert_eq!(
                view(&restored),
                view(&control),
                "setup {:?} resize {resize:?} continuation {:?}",
                String::from_utf8_lossy(setup),
                String::from_utf8_lossy(continuation)
            );
        }
    }
}

/// FINDING: the second `swap_alt` copies the normal cursor over the normal
/// saved cursor (term/mod.rs:720 at the pin). They differ only when the cursor
/// row reflowed while hidden; a DECRC after leaving alternate then diverges.
#[test]
fn restore_overwrites_hidden_normal_saved_cursor_after_cursor_row_reflow() {
    let setup = b"top\r\nabcdefghi\x1b[?1049hALT";
    let mut control = engine(10, 3, 1, setup);
    let mut restored = engine(10, 3, 1, setup);
    control.resize(TermSize::new(6, 3));
    restored.resize(TermSize::new(6, 3));
    let _ = restore(&mut restored);
    for term in [&mut control, &mut restored] {
        let mut parser: Processor = Processor::new();
        parser.advance(term, b"\x1b[?1049l\x1b8Z");
    }
    assert_eq!(
        view(&control).text(),
        ["top   ", "abcdef", "ghi  Z", "      "]
    );
    assert_eq!(
        view(&restored).text(),
        ["top   ", "abcdef", "ghiZ  ", "      "]
    );
}

#[test]
fn hidden_normal_cursor_away_from_content_end_is_restored_on_exit() {
    let mut term = engine(10, 3, 0, b"abc\r\ndef\x1b[1;2H\x1b[?1049hALT");
    let mut host = attach(&mut term);
    host.process(b"\x1b[?1049lX");
    assert_eq!(screen(&host), ["aXc", "def", ""]);
    assert_eq!(host.screen().cursor_position(), (0, 2));
}

#[test]
fn bright_indexed_and_rgb_colours_in_history_are_restored() {
    let mut term = engine(
        10,
        2,
        2,
        b"\x1b[91mb\x1b[38;5;123mi\x1b[48;2;1;2;3mr\x1b[0m\r\nx\r\ny\x1b[?1049h",
    );
    let mut host = attach(&mut term);
    host.process(b"\x1b[?1049l");
    host.screen_mut().set_scrollback(1);
    let cell = |column| host.screen().cell(0, column).unwrap();
    assert_eq!(cell(0).contents(), "b");
    assert_eq!(cell(0).fgcolor(), vt100::Color::Idx(9));
    assert_eq!(cell(1).fgcolor(), vt100::Color::Idx(123));
    assert_eq!(cell(2).fgcolor(), vt100::Color::Idx(123));
    assert_eq!(cell(2).bgcolor(), vt100::Color::Rgb(1, 2, 3));
}

#[test]
fn wide_and_combining_characters_keep_their_columns() {
    let mut term = engine(10, 2, 0, "a\u{3042}e\u{301}z\x1b[?1049h".as_bytes());
    let mut host = attach(&mut term);
    host.process(b"\x1b[?1049lX");
    let cell = |column| host.screen().cell(0, column).unwrap().contents().to_owned();
    assert_eq!(cell(1), "\u{3042}");
    assert_eq!(cell(3), "e\u{301}");
    assert_eq!(cell(4), "z");
    assert_eq!(cell(5), "X");
}

/// Hosts with background colour erase paint scrolled-in rows with the active pen.
#[test]
fn every_line_break_is_emitted_with_the_default_pen() {
    let mut term = engine(4, 2, 2, b"\x1b[44mab\r\nc\r\nd\x1b[?1049h");
    let restored = restore(&mut term);
    assert_eq!(
        restored.windows(2).filter(|pair| pair == b"\r\n").count(),
        2
    );
    let mut host = vt100::Parser::new(
        NonZeroU16::new(2).unwrap(),
        NonZeroU16::new(4).unwrap(),
        100,
    );
    for (index, _) in restored
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair == b"\r\n")
    {
        host.process(&restored[..index]);
        assert_eq!(host.screen().attributes_formatted(), "\x1b[m");
        host = vt100::Parser::new(
            NonZeroU16::new(2).unwrap(),
            NonZeroU16::new(4).unwrap(),
            100,
        );
    }
}

#[test]
fn reattaching_to_a_host_already_on_the_alternate_screen_replaces_stale_rows() {
    let mut term = engine(6, 2, 0, b"\x1b[?1049hab\r\nc");
    let restored = restore(&mut term);
    let mut host = vt100::Parser::new(
        NonZeroU16::new(2).unwrap(),
        NonZeroU16::new(6).unwrap(),
        100,
    );
    host.process(b"\x1b[?1049hSTALE1\r\nSTALE2");
    host.process(&restored);
    assert_eq!(screen(&host), ["ab", "c"]);
}
