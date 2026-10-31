//! THROWAWAY #43: stdin application bytes -> public-seam restore/repaint bytes.

use alacritty_embedding_probe::restore::{repaint, restore};
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::rc::Rc;
use vte::ansi::Processor;

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

fn main() {
    let mode = std::env::args()
        .nth(1)
        .expect("restore, repaint or title-probe");
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input).unwrap();
    let config = Config {
        kitty_keyboard: false,
        ..Config::default()
    };
    let titles = Titles::default();
    let mut term = Term::new(config.clone(), &TermSize::new(20, 4), titles.clone());
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, &input);
    let output = match mode.as_str() {
        "restore" => restore(&mut term),
        "repaint" => repaint(&mut term),
        // UNSAFE evidence only: demonstrates why arbitrary titles are blocked.
        // Never called by restore or repaint.
        "title-probe" => {
            titles.0.borrow_mut().clear();
            term.set_options(config);
            let title = titles
                .0
                .borrow_mut()
                .pop()
                .expect("title event")
                .expect("BLOCKED: ResetTitle requires the attachment's default title");
            format!("\x1b]2;{title}\x1b\\").into_bytes()
        }
        _ => panic!("expected restore, repaint or title-probe"),
    };
    std::io::stdout().write_all(&output).unwrap();
}
