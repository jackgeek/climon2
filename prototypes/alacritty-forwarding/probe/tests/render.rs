//! Independent hand-written expectations; vt100 only consumes derived bytes.

use alacritty_embedding_probe::render::{restore, updates};
use alacritty_embedding_probe::timing::{FirstBeginTimeout, flush_if_due};
use alacritty_embedding_probe::{Observed, view};
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use std::num::NonZeroU16;
use vte::ansi::Processor;

fn host() -> vt100::Parser {
    vt100::Parser::new(
        NonZeroU16::new(3).unwrap(),
        NonZeroU16::new(10).unwrap(),
        100,
    )
}

fn rows(host: &vt100::Parser) -> Vec<String> {
    host.screen()
        .rows(0, host.screen().size().1.get())
        .collect()
}

#[test]
fn restore_normal_text_history_styles_unicode_and_cursor_without_replaying_effects() {
    let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
    let mut parser: Processor = Processor::new();
    parser.advance(
        &mut term,
        "h0\r\nh1\r\n\x1b[31;1mR\x1b[0m\r\n界e\u{301}\r\nend\x1b]2;old-title\x07\x07\x1b]52;c;dGVzdA==\x07".as_bytes(),
    );
    let before = view(&term);
    let bytes = restore(&term);
    assert_eq!(view(&term), before);
    assert!(!bytes.contains(&7), "Historical bells must not replay");
    assert!(
        !bytes.windows(2).any(|bytes| bytes == b"\x1b]"),
        "No OSC side effects"
    );
    let mut host = host();
    host.process(&bytes);
    assert_eq!(rows(&host), ["R", "界e\u{301}", "end"]);
    assert_eq!(host.screen().cursor_position(), (2, 3));
    assert!(host.screen().cell(0, 0).unwrap().bold());
    assert_eq!(
        host.screen().cell(0, 0).unwrap().fgcolor(),
        vt100::Color::Idx(1)
    );
    host.screen_mut().set_scrollback(2);
    assert_eq!(rows(&host), ["h0", "h1", "R"]);
}

#[test]
fn alternate_attach_defers_normal_until_actual_exposure_and_preserves_brief_output() {
    let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
    let mut parser: Processor = Processor::new();
    parser.advance(
        &mut term,
        b"h0\r\nh1\r\nn0\r\nn1\r\nprompt>\x1b[?1049h\x1b[HALT",
    );
    let mut host = host();
    host.process(&restore(&term));
    assert!(host.screen().alternate_screen());
    assert_eq!(rows(&host), ["ALT", "", ""]);
    host.process(b"\x1b[?1049l");
    assert_eq!(rows(&host), ["", "", ""], "No hidden normal preload");
    host.process(b"\x1b[?1049h");
    let mut observed = Observed::new(&mut term);
    parser.advance(&mut observed, b"\x1b[?1049lQ\x1b[?1049h\x1b[HALT2");
    host.process(&updates(&observed));
    assert!(host.screen().alternate_screen());
    assert_eq!(rows(&host), ["ALT2", "", ""]);
    host.process(b"\x1b[?1049l");
    assert_eq!(rows(&host), ["n0", "n1", "prompt>Q"]);
    assert_eq!(host.screen().cursor_position(), (2, 8));
    host.screen_mut().set_scrollback(2);
    assert_eq!(rows(&host), ["h0", "h1", "n0"]);
}

#[test]
fn hostile_excluded_hyperlink_metadata_never_enters_derived_stream() {
    let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
    let mut parser: Processor = Processor::new();
    parser.advance(
        &mut term,
        "\x1b]8;id=evil;https://example.invalid/\u{009c}\u{009b}2J\x1b\\link\x1b]8;;\x1b\\"
            .as_bytes(),
    );
    let bytes = restore(&term);
    assert!(!bytes.windows(2).any(|bytes| bytes == b"\x1b]"));
    let mut host = host();
    host.process(&bytes);
    assert_eq!(rows(&host), ["link", "", ""]);
}

#[test]
fn synchronized_drains_preserve_normal_output_before_alternate_reentry() {
    for explicit_end in [true, false] {
        for chunk_size in [1, 7, 65536] {
            let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
            let mut parser = Processor::<FirstBeginTimeout>::new();
            parser.advance(&mut term, b"saved\x1b[?1049h\x1b[HALT");
            let mut host = host();
            host.process(&restore(&term));
            let mut observed = Observed::new(&mut term);
            parser.advance(&mut observed, b"\x1b[?2026h");
            for bytes in b"\x1b[?1049lQ\x1b[?1049h\x1b[HNEXT".chunks(chunk_size) {
                parser.advance(&mut observed, bytes);
            }
            assert!(observed.transitions.is_empty());
            assert_eq!(rows(&host), ["ALT", "", ""]);
            if explicit_end {
                parser.advance(&mut observed, b"\x1b[?2026l");
            } else {
                let deadline = parser.sync_timeout().deadline().unwrap();
                assert!(flush_if_due(&mut parser, &mut observed, deadline));
            }
            host.process(&updates(&observed));
            assert_eq!(rows(&host), ["NEXT", "", ""]);
            host.process(b"\x1b[?1049l");
            assert_eq!(rows(&host), ["savedQ", "", ""]);
            assert_eq!(host.screen().cursor_position(), (0, 6));
        }
    }
}

#[test]
fn reflow_restore_retains_soft_wrap_and_repaint_clears_stale_cells() {
    let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, b"abcdefghijklmnopqrstu");
    term.resize(TermSize::new(6, 3));
    let mut host = vt100::Parser::new(
        NonZeroU16::new(3).unwrap(),
        NonZeroU16::new(6).unwrap(),
        100,
    );
    host.process(&restore(&term));
    assert_eq!(rows(&host), ["ghijkl", "mnopqr", "stu"]);
    assert!(host.screen().row_wrapped(0));
    host.screen_mut().set_scrollback(1);
    assert_eq!(rows(&host), ["abcdef", "ghijkl", "mnopqr"]);
    host.screen_mut().set_scrollback(0);
    parser.advance(&mut term, b"\x1b[H\x1b[2JX\x1b[?1;2004h\x1b[?25l");
    host.process(&restore(&term));
    assert_eq!(rows(&host), ["X", "", ""]);
    assert!(host.screen().application_cursor());
    assert!(host.screen().bracketed_paste());
    assert!(host.screen().hide_cursor());
}

#[test]
fn attachment_preserves_current_pen_and_saved_cursor_pen_on_each_screen() {
    for alternate in [false, true] {
        let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
        let mut parser: Processor = Processor::new();
        if alternate {
            parser.advance(&mut term, b"\x1b[?1049h");
        }
        parser.advance(&mut term, b"\x1b[3;4H\x1b[31;1m\x1b7\x1b[1;2H\x1b[0;34m");
        let before = view(&term);
        let mut host = host();
        host.process(&restore(&term));
        assert_eq!(view(&term), before);
        host.process(b"X");
        assert_eq!(rows(&host), [" X", "", ""]);
        let current = host.screen().cell(0, 1).unwrap();
        assert_eq!(current.fgcolor(), vt100::Color::Idx(4));
        assert!(!current.bold());
        host.process(b"\x1b8Z");
        assert_eq!(rows(&host), [" X", "", "   Z"]);
        let saved = host.screen().cell(2, 3).unwrap();
        assert_eq!(saved.fgcolor(), vt100::Color::Idx(1));
        assert!(saved.bold());
        assert_eq!(host.screen().cursor_position(), (2, 4));
    }
}
