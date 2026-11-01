//! THROWAWAY reduced-profile rendering; no hidden-grid swaps or OSC output.

use crate::{Observed, Transition, View, view};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Term, TermMode};
use std::io::Write;
use vte::ansi::Color;

pub fn restore<T>(term: &Term<T>) -> Vec<u8> {
    Writer::default().frame(&view(term))
}

/// The caller supplies only transitions from the current parser drain.
pub fn updates<T>(observed: &Observed<'_, T>) -> Vec<u8> {
    let mut writer = Writer::default();
    for transition in &observed.transitions {
        let (Transition::NormalExposed(normal) | Transition::NormalBeforeAlternate(normal)) =
            transition;
        writer.write_frame(normal);
    }
    writer.write_frame(&view(observed.term));
    writer.out
}

const LAYOUT: Flags = Flags::WRAPLINE
    .union(Flags::WIDE_CHAR)
    .union(Flags::WIDE_CHAR_SPACER)
    .union(Flags::LEADING_WIDE_CHAR_SPACER);

#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: Color,
    bg: Color,
    flags: Flags,
}

impl Style {
    fn of(cell: &Cell) -> Self {
        Self {
            fg: cell.fg,
            bg: cell.bg,
            flags: cell.flags.difference(LAYOUT),
        }
    }
}

#[derive(Default)]
struct Writer {
    out: Vec<u8>,
    style: Option<Style>,
}

impl Writer {
    fn frame(mut self, frame: &View) -> Vec<u8> {
        self.write_frame(frame);
        self.out
    }

    fn write_frame(&mut self, frame: &View) {
        if frame.mode.contains(TermMode::ALT_SCREEN) {
            self.out.extend(b"\x1b[?1049h");
        } else {
            self.out.extend(b"\x1b[?1049l");
        }
        // Switch first so positioning resets cannot overwrite the saved normal cursor.
        self.out.extend(b"\x1b[?6l\x1b[r\x1b[4l\x1b[?7h\x1b[0m");
        self.style = Some(Style::of(&Cell::default()));
        self.out.extend(b"\x1b[H\x1b[2J");
        let mut continued = true;
        for row in &frame.rows {
            if !continued {
                self.out.extend(b"\r\n");
            }
            continued = self.row(row);
        }
        self.cursor(frame.saved_cursor, &frame.saved_pen);
        self.out.extend(b"\x1b7");
        self.cursor(frame.cursor, &frame.cursor_pen);
        for (mode, enabled, disabled) in [
            (
                TermMode::SHOW_CURSOR,
                b"\x1b[?25h".as_slice(),
                b"\x1b[?25l".as_slice(),
            ),
            (TermMode::APP_CURSOR, b"\x1b[?1h", b"\x1b[?1l"),
            (TermMode::BRACKETED_PASTE, b"\x1b[?2004h", b"\x1b[?2004l"),
        ] {
            self.out.extend(if frame.mode.contains(mode) {
                enabled
            } else {
                disabled
            });
        }
    }

    fn cursor(&mut self, point: (i32, usize), pen: &Cell) {
        write!(self.out, "\x1b[{};{}H", point.0 + 1, point.1 + 1).unwrap();
        self.pen(Style::of(pen));
    }

    fn row(&mut self, row: &[Cell]) -> bool {
        let wrapped = row
            .last()
            .is_some_and(|cell| cell.flags.contains(Flags::WRAPLINE));
        let end = if wrapped {
            row.len()
        } else {
            row.iter()
                .rposition(|cell| {
                    cell.c != ' '
                        || cell.zerowidth().is_some()
                        || Style::of(cell) != Style::of(&Cell::default())
                })
                .map_or(0, |column| column + 1)
        };
        for cell in &row[..end] {
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            self.pen(Style::of(cell));
            let mut buffer = [0; 4];
            self.out.extend(cell.c.encode_utf8(&mut buffer).as_bytes());
            for character in cell.zerowidth().unwrap_or_default() {
                self.out
                    .extend(character.encode_utf8(&mut buffer).as_bytes());
            }
        }
        wrapped
    }

    fn pen(&mut self, style: Style) {
        if self.style == Some(style) {
            return;
        }
        self.style = Some(style);
        self.out.extend(b"\x1b[0");
        for (flag, code) in [
            (Flags::BOLD, 1),
            (Flags::DIM, 2),
            (Flags::ITALIC, 3),
            (Flags::UNDERLINE, 4),
            (Flags::INVERSE, 7),
            (Flags::HIDDEN, 8),
            (Flags::STRIKEOUT, 9),
        ] {
            if style.flags.contains(flag) {
                write!(self.out, ";{code}").unwrap();
            }
        }
        self.color(style.fg, 30);
        self.color(style.bg, 40);
        self.out.push(b'm');
    }

    fn color(&mut self, color: Color, base: u8) {
        match color {
            Color::Named(named) if (named as usize) < 8 => {
                write!(self.out, ";{}", base + named as u8)
            }
            Color::Named(named) if (named as usize) < 16 => {
                write!(self.out, ";{}", base + 52 + named as u8)
            }
            Color::Named(_) => Ok(()),
            Color::Indexed(index) => write!(self.out, ";{};5;{index}", base + 8),
            Color::Spec(rgb) => write!(self.out, ";{};2;{};{};{}", base + 8, rgb.r, rgb.g, rgb.b),
        }
        .unwrap();
    }
}
