//! One cell as the monitor paints it: what it holds, in which colours and
//! rendition.

use super::lookup;
use alacritty_terminal::{
    term::cell::{Cell as EngineCell, Flags},
    vte::ansi::Color,
};

/// One cell of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell<'a> {
    pub content: Content<'a>,
    pub foreground: Colour,
    pub background: Colour,
    /// The colour underlines are drawn in; [`Colour::Default`] draws them in
    /// the foreground colour.
    pub underline: Colour,
    pub rendition: Rendition,
}

impl<'a> Cell<'a> {
    pub(crate) fn of(cell: &'a EngineCell) -> Self {
        Self {
            content: Content::of(cell),
            foreground: Colour::of(cell.fg),
            background: Colour::of(cell.bg),
            underline: cell.underline_color().map_or(Colour::Default, Colour::of),
            rendition: Rendition::of(cell.flags),
        }
    }
}

/// What a cell holds, by the columns its character covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content<'a> {
    /// A character one column wide; a blank cell holds a space.
    Narrow(Glyph<'a>),
    /// A character two columns wide, covering this cell and the next, whose
    /// content is a [`Content::Spacer`].
    Wide(Glyph<'a>),
    /// The second column of a wide character, with nothing of its own to
    /// draw.
    Spacer,
}

impl<'a> Content<'a> {
    fn of(cell: &'a EngineCell) -> Self {
        let glyph = Glyph::of(cell);
        if cell.flags.contains(Flags::WIDE_CHAR) {
            Self::Wide(glyph)
        } else if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            Self::Spacer
        } else {
            Self::Narrow(glyph)
        }
    }
}

/// A character and the combining marks printed after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph<'a> {
    pub character: char,
    /// In the order they were printed.
    pub combining: &'a [char],
}

impl<'a> Glyph<'a> {
    fn of(cell: &'a EngineCell) -> Self {
        Self {
            // The engine marks where a tab began with `\t` in a blank cell.
            character: if cell.c == '\t' { ' ' } else { cell.c },
            combining: cell.zerowidth().unwrap_or_default(),
        }
    }
}

/// A colour as the application selected it with SGR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colour {
    /// The default colour: never selected, or reset by SGR 0, 39, 49 or 59.
    Default,
    /// One of the sixteen ANSI colours.
    Ansi(Ansi),
    /// An entry of the 256-colour palette (`;5;n`).
    Indexed(u8),
    /// A direct colour: red, green and blue (`;2;r;g;b`).
    Rgb(u8, u8, u8),
}

impl Colour {
    fn of(colour: Color) -> Self {
        match colour {
            Color::Named(name) => ANSI
                .get(name as usize)
                .map_or(Self::Default, |ansi| Self::Ansi(*ansi)),
            Color::Indexed(index) => Self::Indexed(index),
            Color::Spec(rgb) => Self::Rgb(rgb.r, rgb.g, rgb.b),
        }
    }
}

/// The sixteen ANSI colours: SGR 30-37 and 40-47 select the first eight,
/// SGR 90-97 and 100-107 their bright variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ansi {
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

/// The ANSI colours, in the order the engine numbers them; it numbers the
/// default colours after them.
const ANSI: [Ansi; 16] = [
    Ansi::Black,
    Ansi::Red,
    Ansi::Green,
    Ansi::Yellow,
    Ansi::Blue,
    Ansi::Magenta,
    Ansi::Cyan,
    Ansi::White,
    Ansi::BrightBlack,
    Ansi::BrightRed,
    Ansi::BrightGreen,
    Ansi::BrightYellow,
    Ansi::BrightBlue,
    Ansi::BrightMagenta,
    Ansi::BrightCyan,
    Ansi::BrightWhite,
];

/// How SGR draws a cell's character. The engine ignores SGR 5 and 6
/// (blinking text), so no cell blinks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rendition {
    /// SGR 1.
    pub bold: bool,
    /// SGR 2: faint.
    pub dim: bool,
    /// SGR 3.
    pub italic: bool,
    /// SGR 4, in one style at a time.
    pub underline: Option<Underline>,
    /// SGR 7: foreground and background swapped.
    pub inverse: bool,
    /// SGR 8: concealed.
    pub hidden: bool,
    /// SGR 9: crossed out.
    pub strikeout: bool,
}

impl Rendition {
    fn of(flags: Flags) -> Self {
        Self {
            bold: flags.contains(Flags::BOLD),
            dim: flags.contains(Flags::DIM),
            italic: flags.contains(Flags::ITALIC),
            underline: lookup(&UNDERLINES, |flag| flags.contains(*flag)),
            inverse: flags.contains(Flags::INVERSE),
            hidden: flags.contains(Flags::HIDDEN),
            strikeout: flags.contains(Flags::STRIKEOUT),
        }
    }
}

/// The underline styles SGR 4 selects.
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
