use alacritty_embedding_probe::{Observed, Transition, view};
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use vte::ansi::Processor;

#[test]
fn normal_history_reflows_while_hidden_and_is_exposed_at_the_new_width() {
    let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
    let mut parser: Processor = Processor::new();
    let mut observed = Observed::new(&mut term);
    parser.advance(&mut observed, b"abcdefghijklmnopqrstu\x1b[?1049hALT");
    observed.term.resize(TermSize::new(6, 3));
    parser.advance(&mut observed, b"\x1b[?1049l");
    let Transition::NormalExposed(normal) = observed.transitions.last().unwrap() else {
        panic!("resized normal screen not exposed");
    };
    assert_eq!(normal.text(), ["abcdef", "ghijkl", "mnopqr", "stu   "]);
    assert_eq!(normal.history_rows, 1);
    assert_eq!(normal.cursor, (2, 3));
    observed.term.resize(TermSize::new(10, 3));
    let widened = view(observed.term);
    assert_eq!(widened.text(), ["abcdefghij", "klmnopqrst", "u         "]);
    assert_eq!(widened.history_rows, 0);
    assert_eq!(widened.cursor, (2, 1));
}

#[test]
fn zero_and_finite_history_preserve_live_screen_and_evict_oldest_rows() {
    for limit in [0, 2] {
        let config = Config {
            scrolling_history: limit,
            ..Config::default()
        };
        let mut term = Term::new(config, &TermSize::new(10, 3), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(
            &mut Observed::new(&mut term),
            b"zero\r\none\r\ntwo\r\nthree\r\nfour\r\nfive",
        );
        let retained = view(&term);
        assert_eq!(retained.history_rows, limit);
        let expected = if limit == 0 {
            vec!["three     ", "four      ", "five      "]
        } else {
            vec![
                "one       ",
                "two       ",
                "three     ",
                "four      ",
                "five      ",
            ]
        };
        assert_eq!(retained.text(), expected);
    }
}
