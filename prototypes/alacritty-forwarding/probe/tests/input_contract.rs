use std::cell::RefCell;
use std::rc::Rc;

use alacritty_embedding_probe::Observed;
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use vte::ansi::Processor;

#[derive(Clone, Default)]
struct Replies(Rc<RefCell<Vec<String>>>);

impl EventListener for Replies {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(bytes) = event {
            self.0.borrow_mut().push(bytes);
        }
    }
}

#[test]
fn cursor_position_reply_collides_with_a_legacy_modified_function_key() {
    let replies = Replies::default();
    let mut term = Term::new(Config::default(), &TermSize::new(80, 24), replies.clone());
    let mut parser: Processor = Processor::new();
    parser.advance(&mut Observed::new(&mut term), b"\x1b[1;5H\x1b[6n");
    // macOS xterm-256color terminfo kf27 (Ctrl+F3), captured alongside these probes.
    let legacy_control_f3 = "\x1b[1;5R";
    assert_eq!(*replies.0.borrow(), [legacy_control_f3]);
}

#[test]
fn public_legacy_configuration_ignores_kitty_state_and_queries_without_engine_patches() {
    for enabled in [false, true] {
        let replies = Replies::default();
        let config = Config {
            kitty_keyboard: enabled,
            ..Config::default()
        };
        let mut term = Term::new(config, &TermSize::new(80, 24), replies.clone());
        let mut parser: Processor = Processor::new();
        parser.advance(
            &mut Observed::new(&mut term),
            b"\x1b[?u\x1b[>1u\x1b[?u\x1b[=2;2u\x1b[?u\x1b[=1;3u\x1b[?u",
        );
        if enabled {
            assert!(!replies.0.borrow().is_empty());
            assert!(term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL));
        } else {
            assert!(replies.0.borrow().is_empty());
            assert!(!term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL));
        }
        parser.advance(
            &mut Observed::new(&mut term),
            b"\x1b[?1049h\x1b[>3u\x1b[?u\x1b[<u\x1b[?1049l\x1b[<u\x1bc\x1b[?u",
        );
        if !enabled {
            assert!(replies.0.borrow().is_empty());
            assert!(!term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL));
        }
    }
}

#[test]
fn legacy_input_modes_are_observable_for_host_alignment_without_a_key_encoder() {
    let mut term = Term::new(
        Config::default(),
        &TermSize::new(80, 24),
        Replies::default(),
    );
    let mut parser: Processor = Processor::new();
    let mut observed = Observed::new(&mut term);
    let modes = TermMode::APP_CURSOR
        | TermMode::APP_KEYPAD
        | TermMode::BRACKETED_PASTE
        | TermMode::MOUSE_REPORT_CLICK
        | TermMode::SGR_MOUSE
        | TermMode::FOCUS_IN_OUT;
    assert!(!observed.term.mode().intersects(modes));
    parser.advance(&mut observed, b"\x1b[?1;2004;1000;1006;1004h\x1b=");
    assert!(observed.term.mode().contains(modes));
    parser.advance(&mut observed, b"\x1b[?1;2004;1000;1006;1004l\x1b>");
    assert!(!observed.term.mode().intersects(modes));
}
