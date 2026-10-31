//! THROWAWAY #43: restore/repaint bytes derived from unchanged-engine state.
//!
//! The inactive normal grid has no public accessor. `Term::swap_alt` is public,
//! so restore swaps to normal, serialises it, swaps back and reinstates the
//! cloned alternate grid that the second swap resets. The normal grid's saved
//! cursor is overwritten by that second swap; see RESTORE-OSC8.txt.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Cursor, Dimensions, Grid};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags, Hyperlink};
use alacritty_terminal::term::{Term, TermMode};
use std::io::Write;
use vte::ansi::{
    CharsetIndex, Color, CursorShape, CursorStyle, Handler, NamedPrivateMode, PrivateMode,
    StandardCharset,
};

pub fn probe_region<T: EventListener>(term: &mut Term<T>) -> (usize, usize) {
    let cursor = term.grid().cursor.clone();
    let origin = term.mode().contains(TermMode::ORIGIN);
    term.set_private_mode(PrivateMode::Named(NamedPrivateMode::Origin));
    let top = term.grid().cursor.point.line.0 as usize;
    term.goto(term.screen_lines() as i32, 0);
    let bottom = term.grid().cursor.point.line.0 as usize + 1;
    if !origin {
        term.unset_private_mode(PrivateMode::Named(NamedPrivateMode::Origin));
    }
    term.grid_mut().cursor = cursor;
    (top, bottom)
}

pub fn probe_charset<T: EventListener>(term: &mut Term<T>) -> CharsetIndex {
    let cursor = term.grid().cursor.clone();
    let point = alacritty_terminal::index::Point::new(Line(0), term.last_column());
    let cell = term.grid()[point].clone();
    let mut selected = None;
    for index in [
        CharsetIndex::G0,
        CharsetIndex::G1,
        CharsetIndex::G2,
        CharsetIndex::G3,
    ] {
        // At the last column width-1 input cannot insert or wrap. A blank
        // scratch cell prevents wide-character cleanup touching its neighbours.
        term.grid_mut().cursor = Cursor::default();
        term.grid_mut().cursor.point = point;
        term.grid_mut().cursor.charsets[index] = StandardCharset::SpecialCharacterAndLineDrawing;
        term.grid_mut()[point] = Cell::default();
        term.input('q');
        if term.grid()[point].c == '\u{2500}' {
            selected = Some(index);
        }
    }
    term.grid_mut()[point] = cell;
    term.grid_mut().cursor = cursor;
    selected.expect("one active charset among G0-G3")
}

pub fn probe_tabs<T: EventListener>(term: &mut Term<T>) -> Vec<usize> {
    let columns = term.columns();
    let lines = term.screen_lines();
    let region = probe_region(term);
    let selection = term.selection.clone();
    let vi_cursor = term.vi_mode_cursor;
    let active = term.grid().clone();
    term.swap_alt();
    let other = term.grid().clone();
    term.swap_alt();
    // A stop in the old last column is otherwise indistinguishable from
    // clamping. Widen temporarily; retained history is snapshotted, not reflowed
    // in a second model. The existing hidden-normal DECSC exception still applies.
    term.resize(alacritty_terminal::term::test::TermSize::new(
        columns + 1,
        lines,
    ));
    let mut stops = Vec::new();
    for column in 0..columns {
        term.grid_mut().cursor.point.column = Column(column + 1);
        term.move_backward_tabs(1);
        if term.grid().cursor.point.column.0 == column {
            stops.push(column);
        }
    }
    term.resize(alacritty_terminal::term::test::TermSize::new(
        columns, lines,
    ));
    term.set_scrolling_region(region.0 + 1, Some(region.1));
    term.swap_alt();
    *term.grid_mut() = other;
    term.swap_alt();
    *term.grid_mut() = active;
    term.selection = selection;
    term.vi_mode_cursor = vi_cursor;
    stops
}

pub fn restore<T: EventListener>(term: &mut Term<T>) -> Vec<u8> {
    let mut writer = Writer::default();
    let region = probe_region(term);
    let charset = probe_charset(term);
    let tabs = probe_tabs(term);
    let origin = term.mode().contains(TermMode::ORIGIN);
    writer.out.extend(b"\x1b[?6l\x1b[r\x1b[H\x1b[2J");
    if term.mode().contains(TermMode::ALT_SCREEN) {
        let alternate = term.grid().clone();
        term.swap_alt();
        writer.history_and_screen(term.grid());
        writer.region(region);
        writer.origin(false);
        writer.cursor(&term.grid().cursor, term.grid(), false, region, charset);
        term.swap_alt();
        *term.grid_mut() = alternate;
        writer.out.extend(b"\x1b[?1049h\x1b[?6l\x1b[r");
        writer.screen(term.grid());
    } else {
        writer.history_and_screen(term.grid());
    }
    writer.region(region);
    writer.tabs(&tabs);
    writer.cursors(term.grid(), origin, region, charset);
    writer.modes(*term.mode());
    writer.cursor_style(term.cursor_style());
    writer.out
}

/// Live derived output: the active screen only, as a renderer would repaint.
pub fn repaint<T: EventListener>(term: &mut Term<T>) -> Vec<u8> {
    let mut writer = Writer::default();
    let region = probe_region(term);
    let charset = probe_charset(term);
    let origin = term.mode().contains(TermMode::ORIGIN);
    writer.out.extend(b"\x1b[?6l\x1b[r");
    writer.screen(term.grid());
    writer.region(region);
    writer.cursors(term.grid(), origin, region, charset);
    writer.cursor_style(term.cursor_style());
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
    underline: Option<Color>,
}

impl Style {
    fn of(cell: &Cell) -> Self {
        Self {
            fg: cell.fg,
            bg: cell.bg,
            flags: cell.flags.difference(LAYOUT),
            underline: cell.underline_color(),
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
        self.out.extend(b"\x1b(B\x0f");
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
        self.out.extend(b"\x1b(B\x0f");
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
        let c = if cell.c == '\t' { ' ' } else { cell.c };
        self.out.extend(c.encode_utf8(&mut buffer).as_bytes());
        for c in cell.zerowidth().unwrap_or_default() {
            self.out.extend(c.encode_utf8(&mut buffer).as_bytes());
        }
    }

    fn region(&mut self, (top, bottom): (usize, usize)) {
        write!(self.out, "\x1b[{};{bottom}r", top + 1).unwrap();
    }

    fn tabs(&mut self, stops: &[usize]) {
        self.origin(false);
        self.out.extend(b"\x1b[3g");
        for column in stops {
            write!(self.out, "\x1b[1;{}H\x1bH", column + 1).unwrap();
        }
    }

    fn origin(&mut self, origin: bool) {
        self.out
            .extend(if origin { b"\x1b[?6h" } else { b"\x1b[?6l" });
    }

    fn cursors(
        &mut self,
        grid: &Grid<Cell>,
        origin: bool,
        region: (usize, usize),
        charset: CharsetIndex,
    ) {
        self.origin(false);
        self.cursor(&grid.saved_cursor, grid, false, region, charset);
        self.out.extend(b"\x1b7");
        self.origin(origin);
        self.cursor(&grid.cursor, grid, origin, region, charset);
    }

    fn cursor(
        &mut self,
        cursor: &Cursor<Cell>,
        grid: &Grid<Cell>,
        origin: bool,
        region: (usize, usize),
        charset: CharsetIndex,
    ) {
        let mut point = cursor.point;
        if cursor.input_needs_wrap && grid[point].flags.contains(Flags::WIDE_CHAR_SPACER) {
            point.column -= 1;
        }
        let line = if origin {
            assert!(
                (region.0..region.1).contains(&(point.line.0 as usize)),
                "BLOCKED: origin-mode cursor outside scroll region"
            );
            point.line.0 as usize - region.0
        } else {
            point.line.0 as usize
        };
        write!(self.out, "\x1b[{};{}H", line + 1, point.column.0 + 1).unwrap();
        if cursor.input_needs_wrap {
            self.out.extend(b"\x1b(B\x0f");
            self.cell(&grid[point]);
        }
        self.pen(Style::of(&cursor.template), cursor.template.hyperlink());
        for (index, selector) in [
            (CharsetIndex::G0, b'('),
            (CharsetIndex::G1, b')'),
            (CharsetIndex::G2, b'*'),
            (CharsetIndex::G3, b'+'),
        ] {
            let designation = match cursor.charsets[index] {
                StandardCharset::Ascii => b'B',
                StandardCharset::SpecialCharacterAndLineDrawing => b'0',
            };
            self.out.extend([0x1b, selector, designation]);
        }
        self.out.extend(match charset {
            CharsetIndex::G0 => b"\x0f".as_slice(),
            CharsetIndex::G1 => b"\x0e".as_slice(),
            CharsetIndex::G2 => b"\x1bn".as_slice(),
            CharsetIndex::G3 => b"\x1bo".as_slice(),
        });
    }

    fn cursor_style(&mut self, style: CursorStyle) {
        let steady = match style.shape {
            CursorShape::Block => 2,
            CursorShape::Underline => 4,
            CursorShape::Beam => 6,
            CursorShape::HollowBlock | CursorShape::Hidden => {
                panic!("cursor style has no DECSCUSR encoding: {:?}", style.shape);
            }
        };
        let code = steady - u8::from(style.blinking);
        write!(self.out, "\x1b[{code} q").unwrap();
    }

    fn pen(&mut self, style: Style, link: Option<Hyperlink>) {
        if style != self.style {
            self.style = style;
            self.sgr(style);
        }
        if link != self.link {
            match &link {
                Some(link) => {
                    self.out.extend(b"\x1b]8;id=climon2_");
                    self.token_part(link.id());
                    self.out.push(b'_');
                    self.token_part(link.uri());
                    self.out.push(b';');
                    self.uri(link.uri());
                    self.out.extend(b"\x1b\\");
                }
                None => self.out.extend(b"\x1b]8;;\x1b\\"),
            }
            self.link = link;
        }
    }

    fn token_part(&mut self, text: &str) {
        for byte in text.as_bytes() {
            write!(self.out, "{byte:02X}").unwrap();
        }
    }

    fn uri(&mut self, uri: &str) {
        let mut buffer = [0; 4];
        for c in uri.chars() {
            let bytes = c.encode_utf8(&mut buffer).as_bytes();
            if ('\u{7f}'..='\u{9f}').contains(&c) {
                for byte in bytes {
                    write!(self.out, "%{byte:02X}").unwrap();
                }
            } else {
                self.out.extend(bytes);
            }
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
        match style.underline {
            Some(Color::Indexed(index)) => write!(self.out, ";58:5:{index}"),
            Some(Color::Spec(rgb)) => write!(self.out, ";58:2::{}:{}:{}", rgb.r, rgb.g, rgb.b),
            Some(Color::Named(named)) => {
                panic!("named underline colour has no input SGR encoding: {named:?}");
            }
            None => Ok(()),
        }
        .unwrap();
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

/// ORIGIN is restored with the probed scroll region before cursor placement.
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
