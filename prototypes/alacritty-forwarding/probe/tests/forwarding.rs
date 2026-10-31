use alacritty_embedding_probe::Observed;
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use vte::ansi::{Color, NamedColor, Processor};

#[test]
fn ordinary_output_keeps_line_breaks_styles_and_cursor_motion() {
    let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
    let mut parser: Processor = Processor::new();
    parser.advance(
        &mut Observed::new(&mut term),
        b"first\r\n\x1b[31;1msecond\x1b[0m\x1b[1D!",
    );
    let row = |line| {
        (0..10)
            .map(|column| term.grid()[Line(line)][Column(column)].c)
            .collect::<String>()
    };
    assert_eq!(row(0), "first     ");
    assert_eq!(row(1), "secon!    ");
    let red = &term.grid()[Line(1)][Column(0)];
    assert_eq!(red.fg, Color::Named(NamedColor::Red));
    assert!(red.flags.contains(Flags::BOLD));
    let default = &term.grid()[Line(1)][Column(5)];
    assert_eq!(default.fg, Color::Named(NamedColor::Foreground));
    assert!(!default.flags.contains(Flags::BOLD));
    assert_eq!(term.grid().cursor.point.line, Line(1));
    assert_eq!(term.grid().cursor.point.column, Column(6));
}
