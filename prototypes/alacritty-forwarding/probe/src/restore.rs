//! Copied unchanged in behaviour from prototype/alacritty-restore-osc8-43 @
//! f113bb644bac3a1ddd5eeb416c919dba55474db8 (engine lane, #43) to measure
//! restoration cost here; its tests and findings live on that branch.
//!
//! THROWAWAY #43: restore/repaint bytes derived from unchanged-engine state.
//!
//! The inactive normal grid has no public accessor. `Term::swap_alt` is public,
//! so restore swaps to normal, serialises it, swaps back and reinstates the
//! cloned alternate grid that the second swap resets. The normal grid's saved
//! cursor is overwritten by that second swap; see RESTORE-OSC8.txt.

use alacritty_terminal::grid::{Cursor, Dimensions, Grid};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags, Hyperlink};
use alacritty_terminal::term::{Term, TermMode};
use std::io::Write;
use vte::ansi::Color;

pub fn restore<T>(term: &mut Term<T>) -> Vec<u8> {
    let mut writer = Writer::default();
    writer.out.extend(b"\x1b[H\x1b[2J");
    if term.mode().contains(TermMode::ALT_SCREEN) {
        let alternate = term.grid().clone();
        term.swap_alt();
        writer.history_and_screen(term.grid());
        writer.cursor(&term.grid().cursor);
        term.swap_alt();
        *term.grid_mut() = alternate;
        writer.out.extend(b"\x1b[?1049h");
        writer.screen(term.grid());
    } else {
        writer.history_and_screen(term.grid());
    }
    writer.cursors(term.grid());
    writer.modes(*term.mode());
    writer.out
}

/// Live derived output: the active screen only, as a renderer would repaint.
pub fn repaint<T>(term: &Term<T>) -> Vec<u8> {
    let mut writer = Writer::default();
    writer.screen(term.grid());
    writer.cursors(term.grid());
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

impl Default for Style {
    fn default() -> Self {
        Self::of(&Cell::default())
    }
}

#[derive(Default)]
struct Writer {
    out: Vec<u8>,
    style: Style,
    link: Option<Hyperlink>,
}

impl Writer {
    fn history_and_screen(&mut self, grid: &Grid<Cell>) {
        let mut continued = true;
        for line in -(grid.history_size() as i32)..grid.screen_lines() as i32 {
            if !continued {
                self.pen(Style::default(), None);
                self.out.extend(b"\r\n");
            }
            continued = self.row(grid, line);
        }
    }

    fn screen(&mut self, grid: &Grid<Cell>) {
        for line in 0..grid.screen_lines() as i32 {
            self.pen(Style::default(), None);
            write!(self.out, "\x1b[{}H\x1b[2K", line + 1).unwrap();
            self.row(grid, line);
        }
    }

    /// Writes one row and reports whether it soft-wraps into the next.
    fn row(&mut self, grid: &Grid<Cell>, line: i32) -> bool {
        let row = &grid[Line(line)];
        let wrapped = row[Column(grid.last_column().0)]
            .flags
            .contains(Flags::WRAPLINE);
        let blank = Cell::default();
        let end = match wrapped {
            true => grid.columns(),
            false => (0..grid.columns())
                .rposition(|column| row[Column(column)] != blank)
                .map_or(0, |last| last + 1),
        };
        for column in 0..end {
            self.cell(&row[Column(column)]);
        }
        wrapped
    }

    fn cell(&mut self, cell: &Cell) {
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            return;
        }
        self.pen(Style::of(cell), cell.hyperlink());
        let mut buffer = [0; 4];
        self.out.extend(cell.c.encode_utf8(&mut buffer).as_bytes());
        for c in cell.zerowidth().unwrap_or_default() {
            self.out.extend(c.encode_utf8(&mut buffer).as_bytes());
        }
    }

    fn cursors(&mut self, grid: &Grid<Cell>) {
        self.cursor(&grid.saved_cursor);
        self.out.extend(b"\x1b7");
        self.cursor(&grid.cursor);
    }

    fn cursor(&mut self, cursor: &Cursor<Cell>) {
        let point = cursor.point;
        write!(
            self.out,
            "\x1b[{};{}H",
            point.line.0 + 1,
            point.column.0 + 1
        )
        .unwrap();
        self.pen(Style::of(&cursor.template), cursor.template.hyperlink());
    }

    fn pen(&mut self, style: Style, link: Option<Hyperlink>) {
        if style != self.style {
            self.style = style;
            self.sgr(style);
        }
        if link != self.link {
            match &link {
                Some(link) => write!(self.out, "\x1b]8;id={};{}\x1b\\", link.id(), link.uri()),
                None => write!(self.out, "\x1b]8;;\x1b\\"),
            }
            .unwrap();
            self.link = link;
        }
    }

    fn sgr(&mut self, style: Style) {
        self.out.extend(b"\x1b[0");
        for (flag, code) in SGR_FLAGS {
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

    fn modes(&mut self, mode: TermMode) {
        for (flag, set, unset) in MODES {
            let bytes = if mode.contains(flag) { set } else { unset };
            self.out.extend(bytes.as_bytes());
        }
    }
}

const SGR_FLAGS: [(Flags, &str); 11] = [
    (Flags::BOLD, "1"),
    (Flags::DIM, "2"),
    (Flags::ITALIC, "3"),
    (Flags::UNDERLINE, "4"),
    (Flags::DOUBLE_UNDERLINE, "4:2"),
    (Flags::UNDERCURL, "4:3"),
    (Flags::DOTTED_UNDERLINE, "4:4"),
    (Flags::DASHED_UNDERLINE, "4:5"),
    (Flags::INVERSE, "7"),
    (Flags::HIDDEN, "8"),
    (Flags::STRIKEOUT, "9"),
];

/// Host modes a fresh terminal must be driven to. ORIGIN is omitted: it is
/// relative to the scroll region, which the engine does not expose.
const MODES: [(TermMode, &str, &str); 15] = [
    (TermMode::SHOW_CURSOR, "\x1b[?25h", "\x1b[?25l"),
    (TermMode::APP_CURSOR, "\x1b[?1h", "\x1b[?1l"),
    (TermMode::APP_KEYPAD, "\x1b=", "\x1b>"),
    (TermMode::MOUSE_REPORT_CLICK, "\x1b[?1000h", ""),
    (TermMode::MOUSE_DRAG, "\x1b[?1002h", ""),
    (TermMode::MOUSE_MOTION, "\x1b[?1003h", ""),
    (TermMode::FOCUS_IN_OUT, "\x1b[?1004h", "\x1b[?1004l"),
    (TermMode::UTF8_MOUSE, "\x1b[?1005h", ""),
    (TermMode::SGR_MOUSE, "\x1b[?1006h", ""),
    (TermMode::ALTERNATE_SCROLL, "\x1b[?1007h", "\x1b[?1007l"),
    (TermMode::URGENCY_HINTS, "\x1b[?1042h", "\x1b[?1042l"),
    (TermMode::BRACKETED_PASTE, "\x1b[?2004h", "\x1b[?2004l"),
    (TermMode::INSERT, "\x1b[4h", "\x1b[4l"),
    (TermMode::LINE_FEED_NEW_LINE, "\x1b[20h", "\x1b[20l"),
    (TermMode::LINE_WRAP, "\x1b[?7h", "\x1b[?7l"),
];
