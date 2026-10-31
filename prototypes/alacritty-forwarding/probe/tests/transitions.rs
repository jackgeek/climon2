use alacritty_embedding_probe::timing::{FirstBeginTimeout, flush_if_due};
use alacritty_embedding_probe::{Observed, Transition, view};
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use vte::ansi::Processor;

const SETUP: &[u8] = b"old0\r\nold1\r\nrow0\r\nrow1\r\nprompt>\x1b[?1049h\x1b[HALT";
const ROUND_TRIP: &[u8] = b"\x1b[?1049lQ\x1b[?1049h\x1b[HALT2";

#[test]
fn brief_normal_output_is_observed_before_same_feed_alternate_reentry() {
    for chunk_size in [1, 2, 7, 65536] {
        let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, SETUP);
        let attached = view(&term);
        assert!(attached.mode.contains(TermMode::ALT_SCREEN));
        assert_eq!(attached.history_rows, 0);
        assert_eq!(attached.text(), ["ALT       ", "          ", "          "]);
        assert_eq!(attached.cursor, (0, 3));

        let mut observed = Observed::new(&mut term);
        for bytes in ROUND_TRIP.chunks(chunk_size) {
            parser.advance(&mut observed, bytes);
        }

        assert_eq!(observed.transitions.len(), 2, "chunk size {chunk_size}");
        let Transition::NormalExposed(restored) = &observed.transitions[0] else {
            panic!("normal state must be exposed before normal output");
        };
        assert_eq!(
            restored.text(),
            [
                "old0      ",
                "old1      ",
                "row0      ",
                "row1      ",
                "prompt>   "
            ]
        );
        assert_eq!(restored.history_rows, 2);
        assert_eq!(restored.cursor, (2, 7));
        let Transition::NormalBeforeAlternate(saved) = &observed.transitions[1] else {
            panic!("updated normal state must be captured before alternate reentry");
        };
        assert_eq!(
            saved.text(),
            [
                "old0      ",
                "old1      ",
                "row0      ",
                "row1      ",
                "prompt>Q  "
            ]
        );
        assert_eq!(saved.cursor, (2, 8));
        assert!(observed.term.mode().contains(TermMode::ALT_SCREEN));
        assert_eq!(view(observed.term).text()[0], "ALT2      ");
    }
}

#[test]
fn synchronized_end_and_timeout_keep_transient_normal_output_in_order() {
    for explicit_end in [true, false] {
        for chunk_size in [1, 2, 7, 65536] {
            let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
            let mut parser = Processor::<FirstBeginTimeout>::new();
            let mut observed = Observed::new(&mut term);
            parser.advance(&mut observed, SETUP);
            observed.transitions.clear();
            parser.advance(&mut observed, b"\x1b[?2026h");
            let before = view(observed.term);
            for bytes in ROUND_TRIP.chunks(chunk_size) {
                parser.advance(&mut observed, bytes);
            }
            assert!(observed.transitions.is_empty());
            assert_eq!(view(observed.term), before);
            if explicit_end {
                for bytes in b"\x1b[?2026l".chunks(chunk_size) {
                    parser.advance(&mut observed, bytes);
                }
            } else {
                let deadline = parser.sync_timeout().deadline().unwrap();
                assert!(flush_if_due(&mut parser, &mut observed, deadline));
            }
            assert_eq!(observed.transitions.len(), 2);
            let Transition::NormalExposed(restored) = &observed.transitions[0] else {
                panic!("normal restore missing during synchronized drain");
            };
            assert_eq!(restored.text()[4], "prompt>   ");
            let Transition::NormalBeforeAlternate(saved) = &observed.transitions[1] else {
                panic!("normal output missing before synchronized alternate reentry");
            };
            assert_eq!(saved.text()[4], "prompt>Q  ");
            assert_eq!(saved.cursor, (2, 8));
            assert_eq!(view(observed.term).text()[0], "ALT2      ");
            assert_eq!(parser.sync_timeout().deadline(), None);
            assert_eq!(parser.sync_bytes_count(), 0);
        }
    }
}

#[test]
fn repeated_screen_modes_do_not_duplicate_transitions_and_reset_exposes_cleared_normal() {
    let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
    let mut parser: Processor = Processor::new();
    let mut observed = Observed::new(&mut term);
    parser.advance(&mut observed, b"saved\x1b[?1049h\x1b[?1049hALT");
    assert_eq!(observed.transitions.len(), 1);
    let Transition::NormalBeforeAlternate(saved) = &observed.transitions[0] else {
        panic!("initial normal content not captured");
    };
    assert_eq!(saved.text()[0], "saved     ");
    parser.advance(&mut observed, b"\x1bc");
    assert_eq!(observed.transitions.len(), 2);
    let Transition::NormalExposed(reset) = &observed.transitions[1] else {
        panic!("reset did not expose normal screen");
    };
    assert!(!reset.mode.contains(TermMode::ALT_SCREEN));
    assert_eq!(reset.text(), ["          ", "          ", "          "]);
    assert_eq!(reset.cursor, (0, 0));
    parser.advance(&mut observed, b"\x1b[?1049l\x1b[?1049l");
    assert_eq!(observed.transitions.len(), 2);
}
