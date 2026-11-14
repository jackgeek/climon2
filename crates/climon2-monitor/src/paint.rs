//! The bytes that paint a Ratatui frame on the host (#108): every cell with
//! its exact SGR, so a session's ANSI colours reach the host as the same
//! SGR 30-37, 90-97 (and their backgrounds) the application chose, never
//! rewritten as palette entries; then the cursor, or none. Nothing is erased
//! first and the host is never queried.
use ratatui::{
    buffer::{Buffer, Cell},
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
}

/// The cursor is hidden while cells are written, every cell's rendition is
/// set in full where it changes, and the frame ends with SGR 0 and, if it
/// shows one, the cursor placed, styled and shown.
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
        next = Some((x + width(cell), y));
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

/// The columns `cell` covers: a wide character two, anything else one.
fn width(cell: &Cell) -> u16 {
    cell.symbol().width().max(1) as u16
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
            covered = width(cell) - 1;
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

/// The whole rendition of `cell`, from SGR 0.
fn sgr(cell: &Cell) -> String {
    let modifiers = MODIFIERS
        .iter()
        .filter(|(modifier, _)| cell.modifier.contains(*modifier))
        .map(|(_, code)| (*code).to_owned());
    let colours = [
        colour(cell.fg, 30),
        colour(cell.bg, 40),
        underline(cell.underline_color),
    ];
    let codes: Vec<_> = std::iter::once("0".to_owned())
        .chain(modifiers)
        .chain(colours)
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

/// SGR 58's underline colour; it has no 16-colour form of its own.
fn underline(colour: Color) -> String {
    match (ansi(colour), colour) {
        (Some(index), _) | (None, Color::Indexed(index)) => format!("58;5;{index}"),
        (None, Color::Rgb(red, green, blue)) => format!("58;2;{red};{green};{blue}"),
        (None, _) => "59".to_owned(),
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
    use ratatui::{layout::Rect, style::Style};

    fn text(bytes: Vec<u8>) -> String {
        String::from_utf8(bytes).unwrap()
    }

    /// TRM-01 (pure supplement): each of the 16 ANSI colours keeps its own
    /// SGR in the foreground and background; palette, direct and default
    /// colours, the underline colour and every rendition are exact.
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
        buffer[(1, 1)].set_style(Style::new().underline_color(Color::Rgb(4, 5, 6)));
        buffer[(2, 1)].set_style(Style::new().underline_color(Color::LightRed));
        let painted = text(paint(&Frame {
            buffer,
            cursor: None,
        }));
        let expected = [
            "\x1b[?25l\x1b[1;1H",
            "\x1b[0;30;40;59m \x1b[0;31;41;59m \x1b[0;32;42;59m \x1b[0;33;43;59m ",
            "\x1b[0;34;44;59m \x1b[0;35;45;59m \x1b[0;36;46;59m \x1b[0;37;47;59m ",
            "\x1b[0;90;100;59m \x1b[0;91;101;59m \x1b[0;92;102;59m \x1b[0;93;103;59m ",
            "\x1b[0;94;104;59m \x1b[0;95;105;59m \x1b[0;96;106;59m \x1b[0;97;107;59m ",
            "\x1b[2;1H\x1b[0;1;2;3;4;7;8;9;38;5;200;48;2;1;2;3;58;5;9ma",
            "\x1b[0;39;49;58;2;4;5;6m \x1b[0;39;49;58;5;9m ",
            "\x1b[0;39;49;59m             \x1b[0m",
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
        }));
        assert_eq!(
            painted,
            "\x1b[?25l\x1b[1;1H\x1b[0;39;49;59m界e\u{301} \x1b[0m\x1b[1;4H\x1b[6 q\x1b[?25h"
        );
    }

    /// A position the previous cell did not end at is reached with CUP.
    #[test]
    fn each_row_starts_with_its_own_cursor_position() {
        let buffer = Buffer::empty(Rect::new(0, 0, 2, 2));
        assert_eq!(
            text(paint(&Frame {
                buffer,
                cursor: None
            })),
            "\x1b[?25l\x1b[1;1H\x1b[0;39;49;59m  \x1b[2;1H  \x1b[0m"
        );
    }
}
