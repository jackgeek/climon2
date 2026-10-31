use std::cell::RefCell;
use std::rc::Rc;

use alacritty_embedding_probe::timing::{FirstBeginTimeout, flush_if_due};
use alacritty_embedding_probe::{Observed, view};
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use vte::ansi::Processor;

#[derive(Clone, Default)]
struct Events(Rc<RefCell<Vec<String>>>);

impl EventListener for Events {
    fn send_event(&self, event: Event) {
        self.0.borrow_mut().push(format!("{event:?}"));
    }
}

// Differential checks detect adapter regressions, not upstream correctness.
const FIXTURES: &[(&str, &[u8])] = &[
    ("unicode", "Aé界e\u{301}".as_bytes()),
    ("cursor", b"one\r\ntwo\x1b[A\x1b[2C!\x1b[2;3H?\x1b[1Gx\x1b[2dY"),
    ("relative_cursor", b"\x1b[2;5H\x1b[1D\x1b[1B\x1b[1E+\x1b[1F-"),
    ("attributes", b"\x1b[1;3;4;7;9;38;2;12;34;56;48;5;123mX\x1b[0mY"),
    ("editing", b"abcdef\r\x1b[2C\x1b[2@Q\x1b[P\x1b[X\x1b[K"),
    ("line_editing", b"a\r\nb\r\nc\x1b[2;1H\x1b[LX\x1b[M"),
    ("erase", b"abc\r\ndef\x1b[1K\x1b[1J\x1b[2J\x1b[3J"),
    ("scrolling", b"a\r\nb\r\nc\r\nd\x1b[S\x1b[T\x1b[2;3r\x1b[M\x1bM"),
    ("tabs", b"a\tb\x08!\x1b[3g\x1b[5G\x1bH\r\tX\x1b[Z\x1b[IX"),
    ("saved_cursor", b"\x1b[2;3H\x1b7\x1b[HX\x1b8Y"),
    ("charset", b"\x1b(0lqk\x1b(B\x1b)0\x0ex\x0fZ"),
    ("modes", b"\x1b[4hABC\rX\x1b[4l\x1b[20h\nY\x1b[20l"),
    ("private_modes", b"\x1b[?1;6;7;25;1000;1004;1006;2004h\x1b[?1;6;7;25;1000;1004;1006;2004l"),
    ("keypad", b"\x1b=\x1b>"),
    ("cursor_style", b"\x1b[5 q\x1b[?12h\x1b[?12l"),
    ("identity_status_modes", b"\x1b[c\x1b[>c\x1b[5n\x1b[6n\x1b[4$p\x1b[?25$p"),
    ("palette", b"\x1b]4;1;#123456\x07\x1b]4;1;?\x07\x1b]104;1\x07\x1b]10;?\x1b\\"),
    ("titles_bell", b"\x1b]2;first\x07\x1b[22;0t\x1b]2;second\x1b\\\x1b[23;0t\x07"),
    ("size_queries", b"\x1b[14t\x1b[18t"),
    ("excluded_metadata", b"\x1b]8;id=x;https://example.invalid\x1b\\link\x1b]8;;\x1b\\\x1b]52;c;dGVzdA==\x07\x1b]52;c;?\x07"),
    ("extended_keyboard", b"\x1b[?u\x1b[>3u\x1b[=1;2u\x1b[?u\x1b[<u\x1b[>4;2m\x1b[?4m"),
    ("decaln_reset", b"\x1b#8\x1bc"),
    ("screens", b"normal\x1b[?1049hALT\x1b[?1049lQ\x1b[?1049hALT2"),
];

#[test]
fn forwarding_preserves_upstream_cells_modes_cursor_and_events_across_feed_and_sync_boundaries() {
    for (name, fixture) in FIXTURES {
        for chunk_size in [1, 2, 7, 65536] {
            for drain in ["plain", "explicit_end", "timeout"] {
                let expected_events = Events::default();
                let observed_events = Events::default();
                let mut expected = Term::new(
                    Config::default(),
                    &TermSize::new(10, 3),
                    expected_events.clone(),
                );
                let mut actual = Term::new(
                    Config::default(),
                    &TermSize::new(10, 3),
                    observed_events.clone(),
                );
                let mut direct = Processor::<FirstBeginTimeout>::new();
                let mut wrapped = Processor::<FirstBeginTimeout>::new();
                let mut observer = Observed::new(&mut actual);
                if drain != "plain" {
                    direct.advance(&mut expected, b"\x1b[?2026h");
                    wrapped.advance(&mut observer, b"\x1b[?2026h");
                }
                for bytes in fixture.chunks(chunk_size) {
                    direct.advance(&mut expected, bytes);
                    wrapped.advance(&mut observer, bytes);
                    assert_eq!(
                        view(observer.term),
                        view(&expected),
                        "{name}/{chunk_size}/{drain}"
                    );
                    assert_eq!(
                        *observed_events.0.borrow(),
                        *expected_events.0.borrow(),
                        "{name}"
                    );
                }
                match drain {
                    "explicit_end" => {
                        direct.advance(&mut expected, b"\x1b[?2026l");
                        wrapped.advance(&mut observer, b"\x1b[?2026l");
                    }
                    "timeout" => {
                        let expected_deadline = direct.sync_timeout().deadline().unwrap();
                        let actual_deadline = wrapped.sync_timeout().deadline().unwrap();
                        assert!(flush_if_due(&mut direct, &mut expected, expected_deadline));
                        assert!(flush_if_due(&mut wrapped, &mut observer, actual_deadline));
                    }
                    _ => {}
                }
                assert_eq!(
                    view(observer.term),
                    view(&expected),
                    "{name}/{chunk_size}/{drain}"
                );
                assert_eq!(
                    *observed_events.0.borrow(),
                    *expected_events.0.borrow(),
                    "{name}"
                );
                // Expose the saved normal state through actual application input as well.
                direct.advance(&mut expected, b"\x1b[?1049l");
                wrapped.advance(&mut observer, b"\x1b[?1049l");
                assert_eq!(view(observer.term), view(&expected), "{name}/saved-normal");
                assert_eq!(
                    *observed_events.0.borrow(),
                    *expected_events.0.borrow(),
                    "{name}"
                );
            }
        }
    }
}

#[test]
fn daemon_query_events_and_live_effects_have_independent_expected_values() {
    let events = Events::default();
    let mut term = Term::new(Config::default(), &TermSize::new(10, 3), events.clone());
    let mut parser: Processor = Processor::new();
    parser.advance(
        &mut Observed::new(&mut term),
        b"\x1b[2;4H\x1b[5n\x1b[6n\x1b]2;fixture-title\x07\x07",
    );
    assert_eq!(
        *events.0.borrow(),
        [
            "PtyWrite(\u{1b}[0n)",
            "PtyWrite(\u{1b}[2;4R)",
            "Title(fixture-title)",
            "Bell",
        ]
    );
}
