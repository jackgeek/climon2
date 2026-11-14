//! The bytes that paint a Ratatui frame on the host (#108): every cell with
//! its exact SGR, so a session's ANSI colours reach the host as the same
//! SGR 30-37, 90-97 (and their backgrounds) the application chose, never
//! rewritten as palette entries; then the cursor, or none. Nothing is erased
//! first and the host is never queried.
use ratatui::{
    buffer::{Buffer, Cell, CellWidth},
    style::{Color, Modifier},
};
use std::fmt::Write;
use unicode_width::UnicodeWidthStr;

/// The host cursor a frame shows: where, and its DECSCUSR style.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HostCursor {
    pub(crate) column: u16,
    pub(crate) row: u16,
    /// DECSCUSR's `Ps`: 0 for the host's own shape.
    pub(crate) style: u8,
}

/// One frame as the host shows it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Frame {
    pub(crate) buffer: Buffer,
    pub(crate) cursor: Option<HostCursor>,
    /// The host keypad is in application mode (DECKPAM), as the session
    /// taking input asked: the host sends its keypad keys as that
    /// application expects them.
    pub(crate) keypad: bool,
}

/// The cursor is hidden while cells are written, every cell's rendition is
/// set in full where it changes, and the frame ends with SGR 0 and, if it
/// shows one, the cursor placed, styled and shown. A cell's width is the
/// one it was given (a session cell's is the engine's), else its measured
/// one; wherever the host's own measure of its symbol may differ, the next
/// cell is placed with CUP.
pub(crate) fn paint(frame: &Frame) -> Vec<u8> {
    let mut out = String::from("\x1b[?25l");
    let mut next = None;
    let mut style = None;
    for (x, y, cell) in cells(&frame.buffer) {
        if next != Some((x, y)) {
            let _ = write!(out, "\x1b[{};{}H", y + 1, x + 1);
        }
        let sgr = sgr(cell);
        if style.as_ref() != Some(&sgr) {
            out.push_str(&sgr);
            style = Some(sgr);
        }
        out.push_str(cell.symbol());
        let width = cell.cell_width();
        next = (cell.symbol().width() == usize::from(width)).then_some((x + width, y));
    }
    out.push_str("\x1b[0m");
    if let Some(cursor) = frame.cursor {
        let _ = write!(
            out,
            "\x1b[{};{}H\x1b[{} q\x1b[?25h",
            cursor.row + 1,
            cursor.column + 1,
            cursor.style
        );
    }
    out.into_bytes()
}

/// Every cell of `buffer` with its position, except those a wide character
/// before them covers: writing one would overwrite that character's second
/// half.
fn cells(buffer: &Buffer) -> impl Iterator<Item = (u16, u16, &Cell)> {
    let mut covered = 0;
    buffer
        .content()
        .iter()
        .enumerate()
        .filter_map(move |(index, cell)| {
            if covered > 0 {
                covered -= 1;
                return None;
            }
            covered = cell.cell_width().saturating_sub(1);
            let (x, y) = buffer.pos_of(index);
            Some((x, y, cell))
        })
}

/// SGR parameters of each rendition a [`Modifier`] carries.
const MODIFIERS: [(Modifier, &str); 7] = [
    (Modifier::BOLD, "1"),
    (Modifier::DIM, "2"),
    (Modifier::ITALIC, "3"),
    (Modifier::UNDERLINED, "4"),
    (Modifier::REVERSED, "7"),
    (Modifier::HIDDEN, "8"),
    (Modifier::CROSSED_OUT, "9"),
];

/// The 16 ANSI colours in SGR order.
pub(crate) const ANSI: [Color; 16] = [
    Color::Black,
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Gray,
    Color::DarkGray,
    Color::LightRed,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
    Color::LightMagenta,
    Color::LightCyan,
    Color::White,
];

/// The whole rendition of `cell`, from SGR 0. SGR 0 also resets the
/// underline colour, so SGR 58 is written only where it shows: on an
/// underlined cell with a colour of its own.
fn sgr(cell: &Cell) -> String {
    let modifiers = MODIFIERS
        .iter()
        .filter(|(modifier, _)| cell.modifier.contains(*modifier))
        .map(|(_, code)| (*code).to_owned());
    let colours = [colour(cell.fg, 30), colour(cell.bg, 40)];
    let underlined = cell.modifier.contains(Modifier::UNDERLINED);
    let codes: Vec<_> = std::iter::once("0".to_owned())
        .chain(modifiers)
        .chain(colours)
        .chain(underline(cell.underline_color).filter(|_| underlined))
        .collect();
    format!("\x1b[{}m", codes.join(";"))
}

/// A foreground (`base` 30) or background (40) colour: the default, one of
/// the 16 ANSI colours as their own SGR, a palette entry or a direct colour.
fn colour(colour: Color, base: u8) -> String {
    match (ansi(colour), colour) {
        // SGR 30-37 for the first eight, 90-97 for the bright eight.
        (Some(index), _) => (base + index + 52 * u8::from(index >= 8)).to_string(),
        (None, Color::Indexed(index)) => format!("{};5;{index}", base + 8),
        (None, Color::Rgb(red, green, blue)) => format!("{};2;{red};{green};{blue}", base + 8),
        (None, _) => (base + 9).to_string(),
    }
}

/// SGR 58's underline colour, in the colon form of ITU T.416: a host that
/// does not know SGR 58 then skips the one parameter whole, instead of
/// reading its 5 or 2 as blink or dim. It has no 16-colour form of its own,
/// and the default colour needs none.
fn underline(colour: Color) -> Option<String> {
    match (ansi(colour), colour) {
        (Some(index), _) | (None, Color::Indexed(index)) => Some(format!("58:5:{index}")),
        (None, Color::Rgb(red, green, blue)) => Some(format!("58:2::{red}:{green}:{blue}")),
        (None, _) => None,
    }
}

fn ansi(colour: Color) -> Option<u8> {
    ANSI.iter()
        .position(|ansi| *ansi == colour)
        .map(|index| index as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{buffer::CellDiffOption, layout::Rect, style::Style};
    use std::num::NonZeroU16;

    fn text(bytes: Vec<u8>) -> String {
        String::from_utf8(bytes).unwrap()
    }

    /// TRM-01 (pure supplement): each of the 16 ANSI colours keeps its own
    /// SGR in the foreground and background; palette, direct and default
    /// colours and every rendition are exact. SGR 0 resets the underline
    /// colour, so SGR 58 is written only on an underlined cell with a colour
    /// of its own, in its colon form; non-single underline styles are
    /// painted single until #112.
    #[test]
    fn colours_and_renditions_are_painted_with_their_own_sgr() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 16, 2));
        for (column, colour) in (0..).zip(ANSI) {
            buffer[(column, 0)].set_fg(colour).set_bg(colour);
        }
        buffer[(0, 1)]
            .set_style(
                Style::new()
                    .fg(Color::Indexed(200))
                    .bg(Color::Rgb(1, 2, 3))
                    .underline_color(Color::Indexed(9))
                    .add_modifier(Modifier::all()),
            )
            .set_symbol("a");
        let underlined = Style::new().add_modifier(Modifier::UNDERLINED);
        buffer[(1, 1)].set_style(underlined.underline_color(Color::Rgb(4, 5, 6)));
        buffer[(2, 1)].set_style(Style::new().underline_color(Color::LightRed));
        buffer[(3, 1)].set_style(underlined.underline_color(Color::LightRed));
        buffer[(4, 1)].set_style(underlined);
        let painted = text(paint(&Frame {
            buffer,
            cursor: None,
            keypad: false,
        }));
        let expected = [
            "\x1b[?25l\x1b[1;1H",
            "\x1b[0;30;40m \x1b[0;31;41m \x1b[0;32;42m \x1b[0;33;43m ",
            "\x1b[0;34;44m \x1b[0;35;45m \x1b[0;36;46m \x1b[0;37;47m ",
            "\x1b[0;90;100m \x1b[0;91;101m \x1b[0;92;102m \x1b[0;93;103m ",
            "\x1b[0;94;104m \x1b[0;95;105m \x1b[0;96;106m \x1b[0;97;107m ",
            "\x1b[2;1H\x1b[0;1;2;3;4;7;8;9;38;5;200;48;2;1;2;3;58:5:9ma",
            "\x1b[0;4;39;49;58:2::4:5:6m \x1b[0;39;49m \x1b[0;4;39;49;58:5:9m ",
            "\x1b[0;4;39;49m \x1b[0;39;49m           \x1b[0m",
        ]
        .concat();
        assert_eq!(painted, expected);
    }

    /// TRM-01 (pure supplement): a wide character covers its second column,
    /// which is never written over it; a combining mark stays in its cell;
    /// the cursor is placed one-based with its DECSCUSR style and shown last.
    #[test]
    fn wide_and_combining_text_and_the_cursor_are_painted_in_place() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        buffer[(0, 0)].set_symbol("界");
        buffer[(2, 0)].set_symbol("e\u{301}");
        let painted = text(paint(&Frame {
            buffer,
            cursor: Some(HostCursor {
                column: 3,
                row: 0,
                style: 6,
            }),
            keypad: false,
        }));
        assert_eq!(
            painted,
            "\x1b[?25l\x1b[1;1H\x1b[0;39;49m界e\u{301} \x1b[0m\x1b[1;4H\x1b[6 q\x1b[?25h"
        );
    }

    /// TRM-01 (pure supplement; review of 8582801): a cell's given width,
    /// the engine's for a session cell, decides what it covers, never its
    /// symbol re-measured. An emoji with VS16 that the engine made narrow
    /// covers nothing, and a character it made wide covers its second
    /// column whatever its symbol measures; after either, the next cell is
    /// placed with CUP, since the host may measure the symbol otherwise.
    #[test]
    fn each_cell_covers_the_width_it_was_given() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 1));
        let forced = |width| CellDiffOption::ForcedWidth(NonZeroU16::new(width).unwrap());
        buffer[(0, 0)]
            .set_symbol("\u{2764}\u{fe0f}")
            .set_diff_option(forced(1));
        buffer[(1, 0)].set_symbol("x");
        buffer[(2, 0)].set_symbol("w").set_diff_option(forced(2));
        buffer[(3, 0)].set_symbol("!");
        buffer[(4, 0)].set_symbol("界").set_diff_option(forced(2));
        let painted = text(paint(&Frame {
            buffer,
            cursor: None,
            keypad: false,
        }));
        assert_eq!(
            painted,
            "\x1b[?25l\x1b[1;1H\x1b[0;39;49m\u{2764}\u{fe0f}\x1b[1;2Hxw\x1b[1;5H界\x1b[0m"
        );
    }

    /// A position the previous cell did not end at is reached with CUP.
    #[test]
    fn each_row_starts_with_its_own_cursor_position() {
        let buffer = Buffer::empty(Rect::new(0, 0, 2, 2));
        assert_eq!(
            text(paint(&Frame {
                buffer,
                cursor: None,
                keypad: false,
            })),
            "\x1b[?25l\x1b[1;1H\x1b[0;39;49m  \x1b[2;1H  \x1b[0m"
        );
    }
}
