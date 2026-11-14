//! What the monitor draws on its own alternate screen: Ratatui renders each
//! frame into a buffer, and [`crate::paint`] writes it to the host.
use crate::{
    introduction::KeyIntroduction,
    keys::EXIT,
    paint::{ANSI, Frame, HostCursor, paint},
    registry::{View, clip},
    selector::Selector,
};
use climon2_core::{keys::ControlKey, protocol::TerminalSize};
use climon2_vt::view::{self as vt, Content, CursorShape};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Paragraph, Widget},
};
use std::io::{self, Write};
use unicode_width::UnicodeWidthStr;

/// The permanent top row's height.
const TOP_ROW: u16 = 1;
const NO_SELECTION: &str = "No session selected";
const EMPTY: &str = "No sessions to show.";
/// The body once rows exist, while nothing is selected (#107).
const UNSELECTED: &str = "No session selected.";
const TOO_SMALL: &str = "Terminal too small";
/// The smallest application area a session supports: two columns (the
/// engine's minimum) by one row.
const MIN_COLS: u16 = 2;
const MIN_ROWS: u16 = 1;
/// Between the top row's stationary fields.
const SEPARATOR: &str = "  ";

/// The selected session as the top row and canvas show it.
pub(crate) struct Session<'a> {
    pub(crate) label: &'a str,
    pub(crate) status: &'a str,
    pub(crate) control: &'a str,
    pub(crate) error: Option<&'a str>,
    /// Its live or frozen model; none while there is nothing to show.
    pub(crate) view: Option<vt::View<'a>>,
}

/// Everything one frame shows.
pub(crate) struct Scene<'a> {
    pub(crate) introduction: &'a KeyIntroduction,
    pub(crate) rows: &'a [View],
    pub(crate) selector: &'a Selector,
    /// Why the latest discovery pass was incomplete, if it was.
    pub(crate) incomplete: Option<&'a str>,
    pub(crate) session: Option<Session<'a>>,
}

impl Scene<'_> {
    /// Whether modal or overlaid UI covers the canvas.
    fn covered(&self) -> bool {
        self.selector.is_open() || self.introduction.is_open()
    }
}

/// Paints frames on the monitor-owned host output. It never asks the host
/// anything.
pub(crate) struct Screen {
    /// The empty state's key hint, unless key hints are off.
    hint: Option<String>,
    /// The top row's key hint while a session is selected, unless key hints
    /// are off.
    top_hint: Option<String>,
    size: TerminalSize,
    /// The frame last painted.
    painted: Frame,
}

impl Screen {
    /// The empty state names `prefix` in its key hint, and the top row names
    /// it beside a selected session, unless `hints` is off.
    pub(crate) fn new(size: TerminalSize, prefix: ControlKey, hints: bool) -> Self {
        let exit = char::from(EXIT.key);
        Self {
            hint: hints.then(|| format!("Press {prefix} then {exit} to exit.")),
            top_hint: hints.then(|| format!("{prefix} then s: sessions")),
            size,
            painted: Frame::default(),
        }
    }

    /// The next frame is drawn at `size`.
    pub(crate) fn resize(&mut self, size: TerminalSize) {
        self.size = size;
    }

    /// The application area a selected controller should have: the host
    /// width by its height less the top row, or none when that is too small
    /// for any session.
    pub(crate) fn desire(&self) -> Option<TerminalSize> {
        let rows = self.size.rows.saturating_sub(TOP_ROW);
        (self.size.cols >= MIN_COLS && rows >= MIN_ROWS).then_some(TerminalSize {
            cols: self.size.cols,
            rows,
        })
    }

    /// Paints `scene`: always when `force`, otherwise only when its frame
    /// differs from the one painted.
    pub(crate) fn show(&mut self, scene: &Scene<'_>, force: bool) -> io::Result<()> {
        let frame = self.frame(scene);
        if !force && frame == self.painted {
            return Ok(());
        }
        self.paint(frame)
    }

    /// Paints every cell of `frame`, so whatever the host showed is
    /// overwritten in place: nothing is erased first, and no blank screen
    /// shows between two frames.
    fn paint(&mut self, frame: Frame) -> io::Result<()> {
        #[cfg(feature = "test-hooks")]
        crate::hooks::fail("draw")?;
        let mut bytes = keypad_mode(self.painted.keypad, frame.keypad).to_vec();
        bytes.extend(paint(&frame));
        let mut output = io::stdout().lock();
        output.write_all(&bytes)?;
        output.flush()?;
        self.painted = frame;
        Ok(())
    }

    /// The permanent top row, and below it the selected session's canvas or
    /// the body, under the selector and then the key introduction while they
    /// are open. Too small a host shows only why, in the top row.
    fn frame(&self, scene: &Scene<'_>) -> Frame {
        let area = Rect::new(0, 0, self.size.cols, self.size.rows);
        let mut buffer = Buffer::empty(area);
        let top = Rect {
            height: TOP_ROW.min(area.height),
            ..area
        };
        let below = Rect {
            y: top.bottom(),
            height: area.height - top.height,
            ..area
        };
        let cursor = match (self.desire(), &scene.session) {
            (None, _) => {
                top_row(&mut buffer, top, TOO_SMALL, &[]);
                None
            }
            (Some(_), Some(session)) => self.session(session, &mut buffer, top),
            (Some(_), None) => {
                top_row(&mut buffer, top, NO_SELECTION, &[]);
                self.body(scene, &mut buffer, below);
                None
            }
        };
        scene.selector.render(scene.rows, below, &mut buffer);
        scene.introduction.render(below, &mut buffer);
        Frame {
            buffer,
            cursor: cursor.filter(|_| !scene.covered()),
            keypad: self.keypad(scene),
        }
    }

    /// Whether the host keypad should be in application mode: while the
    /// selected session's canvas is shown uncovered and its application
    /// asked for it. Modal UI gets the host's own numeric keypad.
    fn keypad(&self, scene: &Scene<'_>) -> bool {
        let view = scene
            .session
            .as_ref()
            .and_then(|session| session.view.as_ref());
        let shown = !scene.covered() && self.desire().is_some();
        shown && view.is_some_and(|view| view.modes().application_keypad)
    }

    /// The selected session's top row and canvas, and its cursor if visible
    /// on the host.
    fn session(&self, session: &Session<'_>, buffer: &mut Buffer, top: Rect) -> Option<HostCursor> {
        let fields: Vec<&str> = [session.status, session.control]
            .into_iter()
            .chain(session.error)
            .chain(self.top_hint.as_deref())
            .collect();
        top_row(buffer, top, session.label, &fields);
        let view = session.view?;
        canvas(&view, buffer);
        cursor(&view, buffer.area)
    }

    /// The body with nothing selected, centred: the empty state until a row
    /// exists, then that nothing is selected; an incomplete discovery pass
    /// is said under it, so a list is never presented as complete. With key
    /// hints off, it names any problem with the saved choice where its hint
    /// would be: a diagnostic, not a hint.
    fn body(&self, scene: &Scene<'_>, buffer: &mut Buffer, below: Rect) {
        let note = self.hint.as_deref().or(scene.introduction.problem());
        let state = if scene.rows.is_empty() {
            EMPTY
        } else {
            UNSELECTED
        };
        let incomplete = scene
            .incomplete
            .map(|why| format!("Discovery incomplete: {why}"));
        let lines: Vec<_> = std::iter::once(state)
            .chain(note)
            .chain(incomplete.as_deref())
            .map(Line::from)
            .collect();
        let height = Constraint::Length(lines.len() as u16);
        Paragraph::new(lines)
            .centered()
            .render(below.centered_vertically(height), buffer);
    }
}

/// DECKPAM or DECKPNM when the host keypad's mode changes, otherwise nothing.
fn keypad_mode(painted: bool, next: bool) -> &'static [u8] {
    match (painted, next) {
        (false, true) => b"\x1b=",
        (true, false) => b"\x1b>",
        _ => b"",
    }
}

/// The reversed top row: `label` from the left, clipped by display cell to
/// what the stationary `fields` leave, and the fields at the right edge.
fn top_row(buffer: &mut Buffer, top: Rect, label: &str, fields: &[&str]) {
    let style = Style::new().reversed();
    buffer.set_style(top, style);
    let width = usize::from(top.width);
    let fields = fields.join(SEPARATOR);
    let fields = clip(&fields, width);
    let room = width - fields.width();
    let label = clip(label, room.saturating_sub(1));
    buffer.set_string(top.x, top.y, label, style);
    buffer.set_string(top.x + room as u16, top.y, fields, style);
}

/// The session's active screen directly below the top row, at the model's
/// geometry, clipped to the host; the cells it leaves stay blank margins.
fn canvas(view: &vt::View<'_>, buffer: &mut Buffer) {
    let (vt::Screen::Normal { rows, .. } | vt::Screen::Alternate { rows }) = view.screen();
    let area = buffer.area;
    for (y, row) in (TOP_ROW..area.height).zip(rows.iter()) {
        for (x, cell) in (0..area.width).zip(row.cells()) {
            let wide_fits = x + 1 < area.width;
            paint_cell(&mut buffer[(x, y)], &cell, wide_fits);
        }
    }
}

/// One session cell: its glyph and combining marks, colours and rendition.
/// A wide character's second column has nothing of its own; one that the
/// host's right edge would cut is left blank.
fn paint_cell(target: &mut ratatui::buffer::Cell, cell: &vt::Cell<'_>, wide_fits: bool) {
    match cell.content {
        Content::Narrow(glyph) => target.set_symbol(&symbol(glyph)),
        Content::Wide(glyph) if wide_fits => target.set_symbol(&symbol(glyph)),
        Content::Wide(_) | Content::Spacer => target.set_symbol(" "),
    };
    target.set_style(style(cell));
}

fn symbol(glyph: vt::Glyph<'_>) -> String {
    std::iter::once(glyph.character)
        .chain(glyph.combining.iter().copied())
        .collect()
}

fn style(cell: &vt::Cell<'_>) -> Style {
    let rendition = cell.rendition;
    let flags = [
        (rendition.bold, Modifier::BOLD),
        (rendition.dim, Modifier::DIM),
        (rendition.italic, Modifier::ITALIC),
        (rendition.underline.is_some(), Modifier::UNDERLINED),
        (rendition.inverse, Modifier::REVERSED),
        (rendition.hidden, Modifier::HIDDEN),
        (rendition.strikeout, Modifier::CROSSED_OUT),
    ];
    let modifier = flags
        .into_iter()
        .filter(|(on, _)| *on)
        .fold(Modifier::empty(), |all, (_, modifier)| all | modifier);
    Style::new()
        .fg(colour(cell.foreground))
        .bg(colour(cell.background))
        .underline_color(colour(cell.underline))
        .add_modifier(modifier)
}

fn colour(colour: vt::Colour) -> Color {
    match colour {
        vt::Colour::Default => Color::Reset,
        vt::Colour::Ansi(ansi) => ANSI[ansi as usize],
        vt::Colour::Indexed(index) => Color::Indexed(index),
        vt::Colour::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

/// The session's cursor on the host, below the top row, when the
/// application shows it and the host has the cell; never in the top row.
fn cursor(view: &vt::View<'_>, area: Rect) -> Option<HostCursor> {
    let cursor = view.cursor();
    let row = cursor.row.checked_add(TOP_ROW)?;
    let shown = cursor.visible && row < area.height && cursor.column < area.width;
    shown.then(|| HostCursor {
        column: cursor.column,
        row,
        style: decscusr(cursor.shape, cursor.blinking),
    })
}

/// DECSCUSR's `Ps` for a shape: 0 for the host's own, otherwise odd when
/// blinking.
fn decscusr(shape: CursorShape, blinking: bool) -> u8 {
    let steady = match shape {
        CursorShape::Default => return 0,
        CursorShape::Block => 2,
        CursorShape::Underline => 4,
        CursorShape::Bar => 6,
    };
    steady - u8::from(blinking)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// INP-02 (pure supplement): the host keypad's mode is written only when
    /// it changes.
    #[test]
    fn the_host_keypad_mode_is_written_when_it_changes() {
        assert_eq!(keypad_mode(false, true), b"\x1b=");
        assert_eq!(keypad_mode(true, false), b"\x1b>");
        assert_eq!(keypad_mode(true, true), b"");
        assert_eq!(keypad_mode(false, false), b"");
    }

    fn desire(cols: u16, rows: u16) -> Option<(u16, u16)> {
        let prefix = ControlKey::from_byte(0x1d).unwrap();
        let screen = Screen::new(TerminalSize { cols, rows }, prefix, true);
        screen.desire().map(|size| (size.cols, size.rows))
    }

    /// TRM-03 and TRM-04 (pure supplement): the desired application area is
    /// the host width by its height less the top row; one column, or one
    /// row (no room below the top row), is too small, and two of each are
    /// room enough.
    #[test]
    fn the_application_area_is_the_host_less_its_top_row_unless_too_small() {
        assert_eq!(desire(120, 30), Some((120, 29)));
        assert_eq!(desire(2, 2), Some((2, 1)));
        assert_eq!(desire(1, 30), None);
        assert_eq!(desire(120, 1), None);
        assert_eq!(desire(0, 0), None);
    }
}
