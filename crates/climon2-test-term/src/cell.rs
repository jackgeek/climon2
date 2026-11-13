//! What a user sees in one cell of the emulated host, and its cursor, in terms a
//! test can specify from the SGR, DECSCUSR and DECTCEM definitions alone.

use crate::terminal::Modes;
use alacritty_terminal::{
    Term,
    event::EventListener,
    term::cell::{self, Flags},
    vte::ansi::{self, NamedColor},
};

/// One display cell of the emulated host's visible screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// The character written to the cell followed by its combining characters:
    /// a space for a blank cell and empty for the [`Width::Spacer`] of a wide
    /// character. Concealed (`hidden`) text is still reported.
    pub text: String,
    pub width: Width,
    pub foreground: Color,
    pub background: Color,
    /// SGR 58; [`Color::Default`] (SGR 59) draws underlines in the foreground.
    pub underline_color: Color,
    pub rendition: Rendition,
}

impl Default for Cell {
    /// A blank cell in the host's default colours and rendition.
    fn default() -> Self {
        Self {
            text: " ".to_owned(),
            width: Width::Single,
            foreground: Color::Default,
            background: Color::Default,
            underline_color: Color::Default,
            rendition: Rendition::default(),
        }
    }
}

impl Cell {
    pub(crate) fn of(cell: &cell::Cell) -> Self {
        Self {
            text: text(cell),
            width: Width::of(cell.flags),
            foreground: Color::of(cell.fg),
            background: Color::of(cell.bg),
            underline_color: cell.underline_color().map_or(Color::Default, Color::of),
            rendition: Rendition::of(cell.flags),
        }
    }
}

fn text(cell: &cell::Cell) -> String {
    if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
        return String::new();
    }
    let combining = cell.zerowidth().unwrap_or_default();
    std::iter::once(cell.c)
        .chain(combining.iter().copied())
        .collect()
}

/// How many columns the character in a cell occupies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Width {
    /// A one-column character or blank. The blank a host leaves at the end of
    /// a line when a wide character wraps to the next one is also `Single`.
    #[default]
    Single,
    /// The first column of a two-column character.
    Wide,
    /// The second column of a two-column character, drawn with its colours.
    Spacer,
}

impl Width {
    fn of(flags: Flags) -> Self {
        if flags.contains(Flags::WIDE_CHAR) {
            Self::Wide
        } else if flags.contains(Flags::WIDE_CHAR_SPACER) {
            Self::Spacer
        } else {
            Self::Single
        }
    }
}

/// A cell colour as the application selected it with SGR.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Color {
    /// The host's default colour: SGR 39, 49 or 59, or never set.
    #[default]
    Default,
    /// One of the 16 ANSI colours. Hosts may brighten these for bold text.
    Ansi(AnsiColor),
    /// A 256-colour palette entry: SGR 38;5;n, 48;5;n or 58;5;n.
    Palette(u8),
    /// A direct colour: SGR 38;2;r;g;b, 48;2;r;g;b or 58:2::r:g:b.
    Rgb(u8, u8, u8),
}

impl Color {
    fn of(color: ansi::Color) -> Self {
        match color {
            ansi::Color::Named(name) => Self::named(name),
            ansi::Color::Indexed(index) => Self::Palette(index),
            ansi::Color::Spec(rgb) => Self::Rgb(rgb.r, rgb.g, rgb.b),
        }
    }

    fn named(name: NamedColor) -> Self {
        Self::Ansi(match name {
            NamedColor::Foreground | NamedColor::Background => return Self::Default,
            NamedColor::Black => AnsiColor::Black,
            NamedColor::Red => AnsiColor::Red,
            NamedColor::Green => AnsiColor::Green,
            NamedColor::Yellow => AnsiColor::Yellow,
            NamedColor::Blue => AnsiColor::Blue,
            NamedColor::Magenta => AnsiColor::Magenta,
            NamedColor::Cyan => AnsiColor::Cyan,
            NamedColor::White => AnsiColor::White,
            NamedColor::BrightBlack => AnsiColor::BrightBlack,
            NamedColor::BrightRed => AnsiColor::BrightRed,
            NamedColor::BrightGreen => AnsiColor::BrightGreen,
            NamedColor::BrightYellow => AnsiColor::BrightYellow,
            NamedColor::BrightBlue => AnsiColor::BrightBlue,
            NamedColor::BrightMagenta => AnsiColor::BrightMagenta,
            NamedColor::BrightCyan => AnsiColor::BrightCyan,
            NamedColor::BrightWhite => AnsiColor::BrightWhite,
            renderer => panic!("SGR never puts the renderer colour {renderer:?} in a cell"),
        })
    }
}

/// The 16 ANSI colours: SGR 30-37 and 40-47 select the first eight, and SGR
/// 90-97 and 100-107 the bright eight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnsiColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}

/// The SGR rendition of a cell. The emulated host ignores SGR 5 and 6 (blinking
/// text), so neither the host nor this oracle can show them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rendition {
    /// SGR 1.
    pub bold: bool,
    /// SGR 2.
    pub dim: bool,
    /// SGR 3.
    pub italic: bool,
    /// SGR 4, 4:1 to 4:5.
    pub underline: Option<Underline>,
    /// SGR 7.
    pub inverse: bool,
    /// SGR 8.
    pub hidden: bool,
    /// SGR 9.
    pub strikeout: bool,
}

impl Rendition {
    fn of(flags: Flags) -> Self {
        Self {
            bold: flags.contains(Flags::BOLD),
            dim: flags.contains(Flags::DIM),
            italic: flags.contains(Flags::ITALIC),
            underline: Underline::of(flags),
            inverse: flags.contains(Flags::INVERSE),
            hidden: flags.contains(Flags::HIDDEN),
            strikeout: flags.contains(Flags::STRIKEOUT),
        }
    }
}

/// The underline style SGR 4 selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Underline {
    /// SGR 4 or 4:1.
    Single,
    /// SGR 4:2.
    Double,
    /// SGR 4:3.
    Curly,
    /// SGR 4:4.
    Dotted,
    /// SGR 4:5.
    Dashed,
}

const UNDERLINES: [(Flags, Underline); 5] = [
    (Flags::UNDERLINE, Underline::Single),
    (Flags::DOUBLE_UNDERLINE, Underline::Double),
    (Flags::UNDERCURL, Underline::Curly),
    (Flags::DOTTED_UNDERLINE, Underline::Dotted),
    (Flags::DASHED_UNDERLINE, Underline::Dashed),
];

impl Underline {
    fn of(flags: Flags) -> Option<Self> {
        UNDERLINES
            .iter()
            .find(|(flag, _)| flags.contains(*flag))
            .map(|(_, underline)| *underline)
    }
}

/// The emulated host's cursor, read in one snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    /// Visible-screen row, 0-based.
    pub row: usize,
    /// Column, 0-based.
    pub column: usize,
    /// DECTCEM: `CSI ? 25 h` shows the cursor and `CSI ? 25 l` hides it.
    pub visible: bool,
    /// DECSCUSR (`CSI Ps SP q`); `Ps` 0 restores the host default, a steady block.
    pub shape: CursorShape,
    /// Odd DECSCUSR values blink and even ones are steady; `CSI ? 12 h` and
    /// `CSI ? 12 l` start and stop blinking without changing the shape.
    pub blinking: bool,
}

impl Cursor {
    pub(crate) fn of<T: EventListener>(term: &Term<T>) -> Self {
        let point = term.grid().cursor.point;
        let style = term.cursor_style();
        Self {
            row: point.line.0 as usize,
            column: point.column.0,
            visible: Modes::of(*term.mode()).cursor_visible,
            shape: CursorShape::of(style.shape),
            blinking: style.blinking,
        }
    }
}

/// The cursor shape DECSCUSR selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    /// `Ps` 1 or 2.
    Block,
    /// `Ps` 3 or 4.
    Underline,
    /// `Ps` 5 or 6.
    Bar,
}

impl CursorShape {
    fn of(shape: ansi::CursorShape) -> Self {
        match shape {
            ansi::CursorShape::Block => Self::Block,
            ansi::CursorShape::Underline => Self::Underline,
            ansi::CursorShape::Beam => Self::Bar,
            renderer => panic!("DECSCUSR never selects the renderer shape {renderer:?}"),
        }
    }
}
