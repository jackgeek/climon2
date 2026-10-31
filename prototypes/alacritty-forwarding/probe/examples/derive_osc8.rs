//! THROWAWAY #43: print engine-derived restore bytes for a hyperlink URI.
//! Usage: cargo run --example derive_osc8 -- <uri>  (the URI is fed as UTF-8)

use alacritty_embedding_probe::restore::restore;
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use std::io::Write;
use vte::ansi::Processor;

fn main() {
    let uri = std::env::args().nth(1).expect("uri argument");
    let mut term = Term::new(Config::default(), &TermSize::new(20, 3), VoidListener);
    let mut parser: Processor = Processor::new();
    let app = format!("SAFE\r\n\x1b]8;;{uri}\x1b\\L\x1b]8;;\x1b\\");
    parser.advance(&mut term, app.as_bytes());
    std::io::stdout().write_all(&restore(&mut term)).unwrap();
}
