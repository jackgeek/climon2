//! THROWAWAY #43: observable state and explicit public-seam/host limitations.

use alacritty_embedding_probe::{restore::restore, view};
use alacritty_terminal::event::{Event, EventListener, VoidListener};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use core::num::NonZeroU16;
use std::cell::RefCell;
use std::rc::Rc;
use vte::ansi::Processor;
use vte::ansi::{CharsetIndex, Handler};

fn engine(input: &[u8]) -> Term<VoidListener> {
    let mut term = Term::new(
        Config {
            kitty_keyboard: false,
            ..Config::default()
        },
        &TermSize::new(20, 4),
        VoidListener,
    );
    feed(&mut term, input);
    term
}

fn feed<T: EventListener>(term: &mut Term<T>, input: &[u8]) {
    let mut parser: Processor = Processor::new();
    parser.advance(term, input);
}

#[test]
fn private_scroll_tabs_and_charset_shift_cannot_be_inferred_from_public_grid() {
    for prefix in [b"".as_slice(), b"N\x1b[?1049h\x1b[H".as_slice()] {
        for (label, left, right, continuation, expected_left, expected_right) in [
            (
                "scroll region",
                b"\x1b[2;3r\x1b[3;1H".as_slice(),
                b"\x1b[3;1H".as_slice(),
                b"\nX".as_slice(),
                (2, 0, 'X'),
                (3, 0, 'X'),
            ),
            (
                "tabs",
                b"\x1b[3g\x1b[5G\x1bH\x1b[H".as_slice(),
                b"\x1b[3g\x1b[H".as_slice(),
                b"\tX".as_slice(),
                (0, 4, 'X'),
                (0, 19, 'X'),
            ),
            (
                "charset shift",
                b"\x1b)0\x0e".as_slice(),
                b"\x1b)0\x0f".as_slice(),
                b"q".as_slice(),
                (0, 0, '\u{2500}'),
                (0, 0, 'q'),
            ),
        ] {
            let mut a = engine(&[prefix, left].concat());
            let mut b = engine(&[prefix, right].concat());
            assert_eq!(view(&a), view(&b), "{label}");
            assert_eq!(a.grid().cursor, b.grid().cursor, "{label}");
            assert_eq!(a.grid().saved_cursor, b.grid().saved_cursor, "{label}");
            feed(&mut a, continuation);
            feed(&mut b, continuation);
            for (term, (line, column, expected)) in [(&a, expected_left), (&b, expected_right)] {
                assert_eq!(view(term).rows[line][column].c, expected, "{label}");
            }
            assert_ne!(
                view(&a),
                view(&b),
                "{label}: private state affects behaviour"
            );
        }
    }
}

#[test]
fn last_column_tabstop_is_hidden_until_a_resize_exposes_it() {
    for prefix in [b"".as_slice(), b"N\x1b[?1049h\x1b[H".as_slice()] {
        let mut set = engine(&[prefix, b"\x1b[3g\x1b[20G\x1bH\x1b[H"].concat());
        let mut unset = engine(&[prefix, b"\x1b[3g\x1b[H"].concat());
        assert_eq!(view(&set), view(&unset));
        set.move_forward_tabs(1);
        unset.move_forward_tabs(1);
        assert_eq!(
            set.grid().cursor,
            unset.grid().cursor,
            "forward-tab always clamps to last column"
        );
        for term in [&mut set, &mut unset] {
            term.resize(TermSize::new(21, 4));
            feed(term, b"\r\tX");
        }
        assert_eq!(view(&set).rows[0][19].c, 'X');
        assert_eq!(view(&unset).rows[0][20].c, 'X');
    }
}

#[test]
fn reversible_resize_probe_reads_every_tabstop_and_preserves_later_behaviour() {
    use alacritty_embedding_probe::restore::probe_tabs;
    for prefix in [b"".as_slice(), b"N\x1b[?1049h\x1b[H".as_slice()] {
        let setup = [
            prefix,
            b"\x1b[3g\x1bH\x1b[5G\x1bH\x1b[20G\x1bH",
            b"\x1b[2;3r\x1b[?6h\x1b[31;1m\x1b7q",
        ]
        .concat();
        let mut term = engine(&setup);
        let mut control = engine(&setup);
        let before = view(&term);
        let cursor = term.grid().cursor.clone();
        let saved = term.grid().saved_cursor.clone();
        assert_eq!(probe_tabs(&mut term), [0, 4, 19]);
        assert_eq!(view(&term), before);
        assert_eq!(term.grid().cursor, cursor);
        assert_eq!(term.grid().saved_cursor, saved);
        for candidate in [&mut term, &mut control] {
            candidate.resize(TermSize::new(21, 4));
            feed(candidate, b"\x1b8\r\tX\x1b[?1049lq\x1b[?1049hq\x1b8X");
        }
        assert_eq!(view(&term), view(&control));
    }
}

#[test]
fn live_repaint_must_not_expand_the_accepted_restore_saved_cursor_exception() {
    use alacritty_embedding_probe::restore::repaint;
    let mut term = engine(b"top\r\nabcdefghijklmnopqrs\x1b[?1049hALT");
    let mut control = engine(b"top\r\nabcdefghijklmnopqrs\x1b[?1049hALT");
    for candidate in [&mut term, &mut control] {
        candidate.resize(TermSize::new(6, 4));
    }
    let _ = repaint(&mut term);
    for candidate in [&mut term, &mut control] {
        feed(candidate, b"\x1b[?1049l\x1b8Z");
    }
    assert_eq!(view(&term), view(&control));
}

#[test]
fn origin_bit_is_public_but_its_unknown_region_controls_cursor_addressing() {
    for prefix in [b"".as_slice(), b"N\x1b[?1049h\x1b[H".as_slice()] {
        let mut term = engine(&[prefix, b"\x1b[2;3r\x1b[?6h"].concat());
        assert!(term.mode().contains(TermMode::ORIGIN));
        feed(&mut term, b"\x1b[1;1HX");
        assert_eq!(view(&term).rows[1][0].c, 'X');
    }
}

#[test]
#[should_panic(expected = "BLOCKED: origin-mode cursor outside scroll region")]
fn origin_cursor_outside_region_is_an_explicit_host_fidelity_blocker() {
    let mut term = engine(b"\x1b7\x1b[2;3r\x1b[?6h\x1b8");
    assert!(term.mode().contains(TermMode::ORIGIN));
    assert_eq!(
        term.grid().cursor.point.line.0,
        0,
        "Alacritty DECRC does not clamp to region"
    );
    feed(&mut term, b"X");
    assert_eq!(view(&term).rows[0][0].c, 'X');
    let _ = restore(&mut term);
}

#[test]
fn reversible_public_probes_read_region_and_shift_without_changing_future_behaviour() {
    use alacritty_embedding_probe::restore::{probe_charset, probe_region};
    for prefix in [b"".as_slice(), b"N\x1b[?1049h\x1b[H".as_slice()] {
        for (shift, index) in [
            (b"\x0f".as_slice(), CharsetIndex::G0),
            (b"\x0e".as_slice(), CharsetIndex::G1),
            (b"\x1bn".as_slice(), CharsetIndex::G2),
            (b"\x1bo".as_slice(), CharsetIndex::G3),
        ] {
            for origin in [b"\x1b[?6h".as_slice(), b"\x1b[?6l".as_slice()] {
                let setup = [
                    prefix,
                    "\x1b[H\u{3042}\x1b[1;20H\u{3042}".as_bytes(),
                    b"\x1b[2;3r\x1b(0\x1b)0\x1b*0\x1b+0\x1b[4h",
                    shift,
                    origin,
                ]
                .concat();
                let mut term = engine(&setup);
                let mut control = engine(&setup);
                term.set_active_charset(index);
                control.set_active_charset(index);
                let before = view(&term);
                let cursor = term.grid().cursor.clone();
                let saved = term.grid().saved_cursor.clone();
                assert_eq!(probe_region(&mut term), (1, 3));
                assert_eq!(probe_charset(&mut term), index);
                assert_eq!(view(&term), before);
                assert_eq!(term.grid().cursor, cursor);
                assert_eq!(term.grid().saved_cursor, saved);
                let continuation = b"q\x1b[3;1H\nX\x1b8q\x1b[?1049lq";
                feed(&mut term, continuation);
                feed(&mut control, continuation);
                assert_eq!(
                    view(&term),
                    view(&control),
                    "probe must not perturb later input"
                );
            }
        }
    }
}

#[derive(Clone, Default)]
struct Titles(Rc<RefCell<Vec<Option<String>>>>);

impl EventListener for Titles {
    fn send_event(&self, event: Event) {
        match event {
            Event::Title(title) => self.0.borrow_mut().push(Some(title)),
            Event::ResetTitle => self.0.borrow_mut().push(None),
            _ => (),
        }
    }
}

#[test]
fn title_can_be_observed_without_a_parallel_model_by_reapplying_the_owned_config() {
    for prefix in [b"".as_slice(), b"N\x1b[?1049h\x1b[H".as_slice()] {
        let events = Titles::default();
        let config = Config {
            kitty_keyboard: false,
            ..Config::default()
        };
        let mut term = Term::new(config.clone(), &TermSize::new(20, 4), events.clone());
        feed(
            &mut term,
            &[prefix, b"\x1b]2;first\x1b\\\x1b[22;0t\x1b]2;second\x1b\\"].concat(),
        );
        events.0.borrow_mut().clear();
        let before = view(&term);
        term.set_options(config.clone());
        assert_eq!(events.0.borrow().as_slice(), &[Some("second".into())]);
        assert_eq!(view(&term), before);
        events.0.borrow_mut().clear();
        feed(&mut term, b"\x1b[23;0t");
        assert_eq!(
            events.0.borrow().as_slice(),
            &[Some("first".into())],
            "title stack untouched"
        );
        // Unlike OSC 8, no title encoding choice was approved. The same
        // parser retains a C1 ST/CSI attack in the title event.
        feed(&mut term, "\x1b]2;a\u{9c}\u{9b}2J\x1b\\".as_bytes());
        events.0.borrow_mut().clear();
        term.set_options(config);
        assert_eq!(
            events.0.borrow().as_slice(),
            &[Some("a\u{9c}\u{9b}2J".into())]
        );
    }
}

#[test]
fn vt100_saved_attributes_diverge_from_the_xterm_per_screen_expectation() {
    let mut term =
        engine(b"\x1b[2;3H\x1b[31;1mN\x1b[?1049h\x1b[3;5H\x1b[34;22m\x1b7\x1b[H\x1b[32mA");
    let mut host = vt100::Parser::new(
        NonZeroU16::new(4).unwrap(),
        NonZeroU16::new(20).unwrap(),
        10,
    );
    host.process(&restore(&mut term));
    host.process(b"\x1b8Z");
    assert_eq!(
        host.screen().cell(2, 4).unwrap().fgcolor(),
        vt100::Color::Idx(4)
    );
    host.process(b"\x1b[?1049lX");
    let x = host.screen().cell(1, 3).unwrap();
    assert_eq!(x.contents(), "X");
    assert_eq!(
        x.fgcolor(),
        vt100::Color::Idx(4),
        "vt100 restores alternate blue, not normal red"
    );
    assert!(!x.bold(), "vt100 loses the normal saved bold attribute");
}
