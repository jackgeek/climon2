//! What the monitor draws on its own alternate screen: Ratatui renders each
//! frame into a buffer, and [`crate::paint`] writes it to the host.
use crate::{
    introduction::KeyIntroduction,
    keys::EXIT,
    paint::{ANSI, Frame, HostCursor, paint},
    registry::View,
    selector::Selector,
};
use climon2_core::{keys::ControlKey, protocol::TerminalSize};
use climon2_vt::view::{self as vt, Content, CursorShape};
use ratatui::{
    buffer::{Buffer, CellDiffOption},
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Paragraph, Widget},
};
use std::{
    io::{self, Write},
    num::NonZeroU16,
};
use unicode_width::UnicodeWidthStr;

/// The permanent top row's height.
const TOP_ROW: u16 = 1;
const NO_SELECTION: &str = "No session selected";
const EMPTY: &str = "No sessions to show.";
/// The body once rows exist, while nothing is selected (#107).
const UNSELECTED: &str = "No session selected.";
const TOO_SMALL: &str = "Terminal too small";
/// The non-controller placeholder, as the client executable words it.
const VIEWED_ELSEWHERE: &str = "This session is being viewed elsewhere.";
/// A selected session with no terminal content to show (spec #73,
/// "Placeholders and states").
const NO_CONTENT: &str = "No terminal content is available.";
/// The smallest host that shows a session (owner decision, 2026-11-14):
/// any host under four rows or five columns is too small.
const MIN_HOST_ROWS: u16 = 4;
const MIN_HOST_COLS: u16 = 5;
/// Between the top row's slots.
const SEPARATOR: &str = " | ";
/// The status slot's width: the longest status a row shows, "Awaiting
/// resources".
const STATUS_SLOT: u16 = 18;
/// The control slot's width: the longest control, "Not controlling".
const CONTROL_SLOT: u16 = 15;

/// The selected session as the top row and canvas show it.
pub(crate) struct Session<'a> {
    pub(crate) label: &'a str,
    pub(crate) status: &'a str,
    pub(crate) control: &'a str,
    pub(crate) error: Option<&'a str>,
    pub(crate) canvas: Canvas<'a>,
}

/// What the canvas below the top row shows of the selected session.
pub(crate) enum Canvas<'a> {
    /// Its live or frozen model.
    View(vt::View<'a>),
    /// Connected and not controlled by this monitor: viewed elsewhere,
    /// never its cached output.
    Placeholder,
    /// Selected read-only with no terminal content, and its condition.
    Empty(&'a str),
    /// Nothing to show yet: acquiring, before the first restoration, or a
    /// retained session (#116).
    Blank,
}

impl Canvas<'_> {
    fn view(&self) -> Option<&vt::View<'_>> {
        match self {
            Self::View(view) => Some(view),
            _ => None,
        }
    }
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
    /// The placeholder's takeover hint, while a take-control key is bound
    /// and key hints are on.
    takeover: Option<String>,
    size: TerminalSize,
    /// The frame last painted.
    painted: Frame,
}

impl Screen {
    /// The empty state names `prefix` in its key hint, and the top row names
    /// it beside a selected session, and the placeholder names the
    /// take-control key, unless `hints` is off.
    pub(crate) fn new(
        size: TerminalSize,
        prefix: ControlKey,
        take_control: Option<ControlKey>,
        hints: bool,
    ) -> Self {
        let exit = char::from(EXIT.key);
        Self {
            hint: hints.then(|| format!("Press {prefix} then {exit} to exit.")),
            top_hint: hints.then(|| format!("{prefix} then s: sessions")),
            takeover: take_control
                .filter(|_| hints)
                .map(|key| format!("Press {key} to take control")),
            size,
            painted: Frame::default(),
        }
    }

    /// The next frame is drawn at `size`.
    pub(crate) fn resize(&mut self, size: TerminalSize) {
        self.size = size;
    }

    /// The application area a selected controller should have: the host
    /// width by its height less the top row, or none when the host is too
    /// small to show a session.
    pub(crate) fn desire(&self) -> Option<TerminalSize> {
        let TerminalSize { cols, rows } = self.size;
        (cols >= MIN_HOST_COLS && rows >= MIN_HOST_ROWS).then(|| TerminalSize {
            cols,
            rows: rows - TOP_ROW,
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
                top_row(&mut buffer, top, TOO_SMALL);
                None
            }
            (Some(_), Some(session)) => {
                session_row(&mut buffer, top, session, self.top_hint.as_deref());
                self.canvas(&session.canvas, &mut buffer, below)
            }
            (Some(_), None) => {
                top_row(&mut buffer, top, NO_SELECTION);
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
            .and_then(|session| session.canvas.view());
        let shown = !scene.covered() && self.desire().is_some();
        shown && view.is_some_and(|view| view.modes().application_keypad)
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
            .collect();
        centred(&lines, buffer, below);
    }

    /// The selected session's canvas below the top row; returns the
    /// cursor its view shows, if any.
    fn canvas(&self, canvas: &Canvas<'_>, buffer: &mut Buffer, below: Rect) -> Option<HostCursor> {
        match canvas {
            Canvas::View(view) => return painted(view, buffer),
            Canvas::Placeholder => {
                let lines: Vec<_> = std::iter::once(VIEWED_ELSEWHERE)
                    .chain(self.takeover.as_deref())
                    .collect();
                centred(&lines, buffer, below);
            }
            Canvas::Empty(condition) => centred(&[NO_CONTENT, condition], buffer, below),
            Canvas::Blank => {}
        }
        None
    }
}

/// `lines`, each centred, in the middle of `area`.
fn centred(lines: &[&str], buffer: &mut Buffer, area: Rect) {
    let height = Constraint::Length(lines.len() as u16);
    let lines: Vec<_> = lines.iter().copied().map(Line::from).collect();
    Paragraph::new(lines)
        .centered()
        .render(area.centered_vertically(height), buffer);
}

/// DECKPAM or DECKPNM when the host keypad's mode changes, otherwise nothing.
fn keypad_mode(painted: bool, next: bool) -> &'static [u8] {
    match (painted, next) {
        (false, true) => b"\x1b=",
        (true, false) => b"\x1b>",
        _ => b"",
    }
}

/// The reversed top row: `text` from the left, clipped by display cell.
fn top_row(buffer: &mut Buffer, top: Rect, text: &str) {
    let style = Style::new().reversed();
    buffer.set_string(top.x, top.y, " ".repeat(top.width.into()), style);
    buffer.set_stringn(top.x, top.y, text, top.width.into(), style);
}

/// The reversed top row of a selected session. Its status, control and
/// any key hint have fixed-width slots at the right edge, separated by
/// `|`, so none of them ever moves whatever the values. The label fills
/// the cells they leave from the left, and any error follows it in what
/// the label leaves: the error is clipped first, and never squeezes out the
/// label.
fn session_row(buffer: &mut Buffer, top: Rect, session: &Session<'_>, hint: Option<&str>) {
    let style = Style::new().reversed();
    top_row(buffer, top, "");
    let separator = SEPARATOR.width() as u16;
    let slots = [
        (session.status, STATUS_SLOT),
        (session.control, CONTROL_SLOT),
    ]
    .into_iter()
    .chain(hint.map(|hint| (hint, hint.width() as u16)));
    let slots: Vec<_> = slots.collect();
    let width: u16 = slots.iter().map(|(_, width)| separator + width).sum();
    let room = top.width.saturating_sub(width);
    let mut x = top.x + room;
    for (text, slot) in slots {
        buffer.set_stringn(x, top.y, SEPARATOR, usize::from(separator), style);
        buffer.set_stringn(x + separator, top.y, text, usize::from(slot), style);
        x += separator + slot;
    }
    let (end, _) = buffer.set_stringn(top.x, top.y, session.label, room.into(), style);
    let left = usize::from(room - (end - top.x));
    if let Some(error) = session.error.filter(|_| left > SEPARATOR.width()) {
        buffer.set_stringn(end, top.y, format!("{SEPARATOR}{error}"), left, style);
    }
}

/// The session's active screen directly below the top row, at the model's
/// geometry, clipped to the host; the cells it leaves stay blank margins.
/// Returns its cursor if the host shows it.
fn painted(view: &vt::View<'_>, buffer: &mut Buffer) -> Option<HostCursor> {
    let (vt::Screen::Normal { rows, .. } | vt::Screen::Alternate { rows }) = view.screen();
    let area = buffer.area;
    for (y, row) in (TOP_ROW..area.height).zip(rows.iter()) {
        for (x, cell) in (0..area.width).zip(row.cells()) {
            let wide_fits = x + 1 < area.width;
            paint_cell(&mut buffer[(x, y)], &cell, wide_fits);
        }
    }
    cursor(view, area)
}

/// One session cell: its glyph and combining marks, colours and rendition,
/// with the width the engine gave it, never a re-measured one. A wide
/// character's second column has nothing of its own; one that the host's
/// right edge would cut is left blank.
fn paint_cell(target: &mut ratatui::buffer::Cell, cell: &vt::Cell<'_>, wide_fits: bool) {
    let (symbol, width) = match cell.content {
        Content::Narrow(glyph) => (symbol(glyph), NARROW),
        Content::Wide(glyph) if wide_fits => (symbol(glyph), WIDE),
        Content::Wide(_) | Content::Spacer => (" ".to_owned(), NARROW),
    };
    target
        .set_symbol(&symbol)
        .set_diff_option(CellDiffOption::ForcedWidth(width))
        .set_style(style(cell));
}

const NARROW: NonZeroU16 = NonZeroU16::MIN;
const WIDE: NonZeroU16 = NonZeroU16::new(2).expect("two is not zero");

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
    let modifier: Modifier = flags
        .into_iter()
        .filter_map(|(on, modifier)| on.then_some(modifier))
        .collect();
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
    use crate::terminal::Control;
    use climon2_vt::{Geometry, Limits, Model};

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
        let screen = Screen::new(TerminalSize { cols, rows }, prefix, None, true);
        screen.desire().map(|size| (size.cols, size.rows))
    }

    /// TRM-03 and TRM-04 (pure supplement; owner decision 2026-11-14): the
    /// desired application area is the host width by its height less the
    /// top row; a host under four rows or five columns is too small, at
    /// each limit and one below it.
    #[test]
    fn the_application_area_is_the_host_less_its_top_row_unless_too_small() {
        assert_eq!(desire(120, 30), Some((120, 29)));
        assert_eq!(desire(5, 4), Some((5, 3)));
        assert_eq!(desire(4, 30), None);
        assert_eq!(desire(120, 3), None);
        assert_eq!(desire(0, 0), None);
    }

    const HINT: &str = "Ctrl+] then s: sessions";

    /// The top row of a selected session `width` cells wide, as text.
    fn top(width: u16, label: &str, control: &str, error: Option<&str>, hint: bool) -> String {
        let area = Rect::new(0, 0, width, 1);
        let mut buffer = Buffer::empty(area);
        let session = Session {
            label,
            status: "Unavailable",
            control,
            error,
            canvas: Canvas::Blank,
        };
        session_row(&mut buffer, area, &session, hint.then_some(HINT));
        assert!(
            buffer
                .content()
                .iter()
                .all(|cell| cell.modifier == Modifier::REVERSED),
            "the whole top row is reversed"
        );
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    /// Top row (owner decision 2026-11-14): status, control and the key
    /// hint have fixed-width slots at the right edge, separated by `|`, so
    /// no control word moves them or the label.
    #[test]
    fn status_and_control_keep_their_slots_whatever_the_control() {
        let rows = [
            "Paint           | Unavailable        | Acquiring       | Ctrl+] then s: sessions",
            "Paint           | Unavailable        | Restoring       | Ctrl+] then s: sessions",
            "Paint           | Unavailable        | Controlling     | Ctrl+] then s: sessions",
            "Paint           | Unavailable        | Not controlling | Ctrl+] then s: sessions",
            "Paint           | Unavailable        | Ended           | Ctrl+] then s: sessions",
        ];
        let controls = [
            Control::Acquiring,
            Control::Restoring,
            Control::Controlling,
            Control::NotControlling,
            Control::Ended,
        ];
        for (control, row) in controls.into_iter().zip(rows) {
            assert_eq!(top(80, "Paint", control.label(), None, true), row);
        }
        assert_eq!(
            top(60, "Paint", "Controlling", None, false),
            "Paint                 | Unavailable        | Controlling    "
        );
    }

    /// Top row (owner decision 2026-11-14): an error follows the label in
    /// what the label leaves and is clipped first; a long label is never
    /// squeezed out by it, and an error with no room for more than its
    /// separator is not shown.
    #[test]
    fn an_error_is_clipped_first_and_never_squeezes_out_the_label() {
        let error = Some("connection refused by the daemon");
        assert_eq!(
            top(60, "Build", "Controlling", error, false),
            "Build | connection re | Unavailable        | Controlling    "
        );
        assert_eq!(
            top(
                60,
                "A very long session title here",
                "Controlling",
                error,
                false
            ),
            "A very long session t | Unavailable        | Controlling    "
        );
        assert_eq!(
            top(60, "seventeen columns", "Controlling", error, false),
            "seventeen columns | c | Unavailable        | Controlling    "
        );
        assert_eq!(
            top(60, "eighteen columns..", "Controlling", error, false),
            "eighteen columns..    | Unavailable        | Controlling    "
        );
        assert_eq!(
            top(30, "Paint", "Controlling", error, true),
            " | Unavailable        | Contro"
        );
    }

    /// The canvas a `takeover` screen 50 by 6 paints, as text below its top
    /// row: an odd spare row or column falls before the text.
    fn shown(takeover: Option<u8>, hints: bool, canvas: &Canvas<'_>) -> Vec<String> {
        let prefix = ControlKey::from_byte(0x1d).unwrap();
        let takeover = takeover.map(|byte| ControlKey::from_byte(byte).unwrap());
        let size = TerminalSize { cols: 50, rows: 6 };
        let screen = Screen::new(size, prefix, takeover, hints);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 50, 6));
        let below = Rect::new(0, 1, 50, 5);
        assert_eq!(screen.canvas(canvas, &mut buffer, below), None);
        (1..6)
            .map(|y| {
                let row: String = (0..50).map(|x| buffer[(x, y)].symbol()).collect();
                row.trim_end().to_owned()
            })
            .collect()
    }

    /// CTL-02 and INP-01 (pure supplement, #110): the non-controller
    /// placeholder says the session is viewed elsewhere, centred, with the
    /// take-control key's hint only while one is bound and key hints are
    /// on; a row selected read-only says it has no terminal content and its
    /// condition; a blank canvas shows nothing.
    #[test]
    fn the_placeholder_and_the_no_content_state_say_why_nothing_is_painted() {
        assert_eq!(
            shown(Some(0x14), true, &Canvas::Placeholder),
            [
                "",
                "",
                "      This session is being viewed elsewhere.",
                "           Press Ctrl+T to take control",
                "",
            ]
        );
        let alone = [
            "",
            "",
            "      This session is being viewed elsewhere.",
            "",
            "",
        ];
        assert_eq!(shown(None, true, &Canvas::Placeholder), alone);
        assert_eq!(shown(Some(0x14), false, &Canvas::Placeholder), alone);
        assert_eq!(
            shown(Some(0x14), true, &Canvas::Empty("Starting")),
            [
                "",
                "",
                "         No terminal content is available.",
                "                     Starting",
                "",
            ]
        );
        assert_eq!(
            shown(Some(0x14), true, &Canvas::Blank),
            ["", "", "", "", ""]
        );
    }

    /// A model `cols` by `rows` that took `bytes`.
    fn model(cols: u16, rows: u16, bytes: &[u8]) -> Model {
        let limits = Limits {
            bytes: 64 << 20,
            history_rows: 100,
        };
        let mut model = Model::new(Geometry::for_host(cols, rows).unwrap(), limits).unwrap();
        model.feed(bytes).unwrap();
        model
    }

    /// `model`'s canvas on a host `cols` by `rows`: each row's symbols and
    /// the host cursor.
    fn painted(model: &Model, cols: u16, rows: u16) -> (Buffer, Option<HostCursor>) {
        let mut buffer = Buffer::empty(Rect::new(0, 0, cols, rows));
        let cursor = super::painted(&model.view(), &mut buffer);
        (buffer, cursor)
    }

    /// TRM-01 (pure supplement): the cursor shows on the host only where
    /// the host has its cell, below the top row: its last column and row
    /// are in, one past either is out, and a hidden cursor never shows.
    #[test]
    fn the_cursor_shows_only_inside_the_host() {
        let corner = model(10, 3, b"\x1b[3;10H\x1b[6 q");
        let at = |column, row| HostCursor {
            column,
            row,
            style: 6,
        };
        assert_eq!(painted(&corner, 10, 4).1, Some(at(9, 3)));
        assert_eq!(painted(&corner, 9, 4).1, None);
        assert_eq!(painted(&corner, 10, 3).1, None);
        let hidden = model(10, 3, b"\x1b[?25l");
        assert_eq!(painted(&hidden, 10, 4).1, None);
    }

    /// TRM-01 (pure supplement): a wide character in the host's last
    /// column is left blank rather than cut; one with room keeps the
    /// engine's width of two, and its second column is a blank narrow
    /// cell the painter skips.
    #[test]
    fn a_wide_character_the_host_edge_would_cut_is_left_blank() {
        let wide = model(4, 1, "ab界".as_bytes());
        let width = |cell: &ratatui::buffer::Cell| match cell.diff_option {
            CellDiffOption::ForcedWidth(width) => width.get(),
            _ => 0,
        };
        let (fits, _) = painted(&wide, 4, 2);
        let row: Vec<_> = fits.content()[4..]
            .iter()
            .map(|cell| (cell.symbol(), width(cell)))
            .collect();
        assert_eq!(row, [("a", 1), ("b", 1), ("界", 2), (" ", 1)]);
        let (cut, _) = painted(&wide, 3, 2);
        let row: Vec<_> = cut.content()[3..]
            .iter()
            .map(|cell| (cell.symbol(), width(cell)))
            .collect();
        assert_eq!(row, [("a", 1), ("b", 1), (" ", 1)]);
    }

    /// TRM-01 (pure supplement): each rendition the engine reports, and
    /// only those, reaches the cell.
    #[test]
    fn each_rendition_reaches_its_cell() {
        let styled = model(4, 1, b"\x1b[1;3mA\x1b[0;1;2;3;4;7;8;9mB\x1b[0mC");
        let (buffer, _) = painted(&styled, 4, 2);
        let modifiers: Vec<_> = buffer.content()[4..7]
            .iter()
            .map(|cell| cell.modifier)
            .collect();
        let every = Modifier::BOLD
            | Modifier::DIM
            | Modifier::ITALIC
            | Modifier::UNDERLINED
            | Modifier::REVERSED
            | Modifier::HIDDEN
            | Modifier::CROSSED_OUT;
        assert_eq!(
            modifiers,
            [Modifier::BOLD | Modifier::ITALIC, every, Modifier::empty()]
        );
    }

    /// TRM-01 (pure supplement): DECSCUSR's `Ps` for every shape, steady
    /// and blinking; the host's own shape is 0 either way.
    #[test]
    fn each_cursor_shape_has_its_decscusr() {
        let shapes = [
            (CursorShape::Default, 0, 0),
            (CursorShape::Block, 1, 2),
            (CursorShape::Underline, 3, 4),
            (CursorShape::Bar, 5, 6),
        ];
        for (shape, blinking, steady) in shapes {
            assert_eq!(
                (decscusr(shape, true), decscusr(shape, false)),
                (blinking, steady)
            );
        }
    }
}
