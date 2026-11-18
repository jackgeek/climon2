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
use climon2_vt::view::{self as vt, Content, CursorShape, Modes, MouseReporting};
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
/// The monitor's own host mouse tracking: button events (DEC 1002), as it
/// entered the host.
const OWN_TRACKING: u16 = 1002;
const NO_SELECTION: &str = "No session selected";
const EMPTY: &str = "No sessions to show.";
/// The body once rows exist, while nothing is selected (#107).
const UNSELECTED: &str = "No session selected.";
/// The list is incomplete: initial collection ended with sessions still
/// starting (#266).
const LATE_INCOMPLETE: &str = "Discovery incomplete: some sessions are still starting";
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
    /// Selected without a connected client: no terminal content, and its
    /// condition.
    NoContent(&'a str),
    /// Nothing to show yet: acquiring, before the first restoration, or a
    /// retained session (#116).
    Nothing,
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
    /// The startup progress while the initial collection lasts (#266).
    pub(crate) progress: Option<String>,
    /// Whether the initial collection ended with sessions still starting.
    pub(crate) late: bool,
    pub(crate) session: Option<Session<'a>>,
}

/// Whether modal or overlaid UI covers the canvas: the selector or the key
/// introduction.
pub(crate) fn covered(selector: &Selector, introduction: &KeyIntroduction) -> bool {
    selector.is_open() || introduction.is_open()
}

impl Scene<'_> {
    fn covered(&self) -> bool {
        covered(self.selector, self.introduction)
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

    /// The next frame is drawn at `size`, and painted whole: after a resize
    /// the host's screen may no longer hold the frame last painted.
    pub(crate) fn resize(&mut self, size: TerminalSize) {
        self.size = size;
        self.painted = Frame::default();
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

    /// Where overlays are drawn: every row below the top row.
    pub(crate) fn overlay(&self) -> Rect {
        let top = TOP_ROW.min(self.size.rows);
        Rect::new(0, top, self.size.cols, self.size.rows - top)
    }

    /// Whether a frame shows the selected session's canvas: nothing
    /// `covered` it and the host has room for a session.
    pub(crate) fn shows_canvas(&self, covered: bool) -> bool {
        !covered && self.desire().is_some()
    }

    /// Paints `scene` unless its frame is the one painted.
    pub(crate) fn show(&mut self, scene: &Scene<'_>) -> io::Result<()> {
        let frame = self.frame(scene);
        if frame == self.painted {
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
        bytes.extend(tracking_mode(self.painted.tracking, frame.tracking));
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
        let below = self.overlay();
        let top = Rect {
            height: below.y,
            ..area
        };
        let cursor = match (self.desire(), &scene.session) {
            (None, _) => {
                top_row(&mut buffer, top, TOO_SMALL);
                None
            }
            (Some(_), Some(session)) => {
                let error = error(session.error, scene.late);
                session_row(&mut buffer, top, session, error, self.top_hint.as_deref());
                self.canvas(&session.canvas, &mut buffer, below)
            }
            (Some(_), None) => {
                top_row(&mut buffer, top, NO_SELECTION);
                self.body(scene, &mut buffer, below);
                None
            }
        };
        let search = scene.selector.render(scene.rows, below, &mut buffer);
        scene.introduction.render(below, &mut buffer);
        let modes = self.followed(scene);
        Frame {
            buffer,
            cursor: shown_cursor(scene, cursor.filter(|_| self.desire().is_some()), search),
            keypad: modes.is_some_and(|modes| modes.application_keypad),
            tracking: modes.and_then(|modes| tracking(modes.mouse_reporting)),
        }
    }

    /// The modes of the application whose canvas is shown uncovered, which
    /// the host follows: its keypad mode, and its mouse tracking level
    /// (owner decision on #111). Modal UI and every other screen get the
    /// host's own numeric keypad and the monitor's own mouse tracking.
    fn followed(&self, scene: &Scene<'_>) -> Option<Modes> {
        let shown = self.shows_canvas(scene.covered());
        let view = scene
            .session
            .as_ref()
            .and_then(|session| session.canvas.view());
        view.filter(|_| shown).map(|view| view.modes())
    }

    /// The body with nothing selected, centred: its [`state`], then any
    /// [`incomplete`] discovery under it, so a list is never presented as
    /// complete. With key hints off, it names any problem with the saved
    /// choice where its hint would be: a diagnostic, not a hint.
    fn body(&self, scene: &Scene<'_>, buffer: &mut Buffer, below: Rect) {
        let note = self.hint.as_deref().or(scene.introduction.problem());
        let incomplete = incomplete(scene.incomplete, scene.late);
        let lines: Vec<_> = std::iter::once(state(scene.progress.as_deref(), scene.rows))
            .chain(note)
            .chain(incomplete.iter().map(String::as_str))
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
            Canvas::NoContent(condition) => centred(&[NO_CONTENT, condition], buffer, below),
            Canvas::Nothing => {}
        }
        None
    }
}

/// The host cursor: the search's while the selector is open, otherwise the
/// selected session's while the host has room for it; none while the key
/// introduction covers them.
fn shown_cursor(
    scene: &Scene<'_>,
    session: Option<HostCursor>,
    search: Option<HostCursor>,
) -> Option<HostCursor> {
    let cursor = if scene.selector.is_open() {
        search
    } else {
        session
    };
    cursor.filter(|_| !scene.introduction.is_open())
}

/// The body's state with nothing selected: the startup progress while the
/// initial collection lasts, otherwise the empty state until a row exists,
/// then that nothing is selected.
fn state<'a>(progress: Option<&'a str>, rows: &[View]) -> &'a str {
    let idle = [EMPTY, UNSELECTED][usize::from(!rows.is_empty())];
    progress.unwrap_or(idle)
}

/// The top row's error slot: the session's own error, otherwise, while the
/// initial collection's late sessions still start, that discovery is
/// incomplete (owner decision, 2026-11-16).
fn error(own: Option<&str>, late: bool) -> Option<&str> {
    own.or(late.then_some(LATE_INCOMPLETE))
}

/// Why discovery is incomplete: the latest pass's reason, then whether the
/// initial collection ended with sessions still starting.
fn incomplete(pass: Option<&str>, late: bool) -> Vec<String> {
    pass.map(|why| format!("Discovery incomplete: {why}"))
        .into_iter()
        .chain(late.then(|| LATE_INCOMPLETE.to_owned()))
        .collect()
}

/// `lines`, each centred, in the middle of `area`.
fn centred(lines: &[&str], buffer: &mut Buffer, area: Rect) {
    let height = Constraint::Length(lines.len() as u16);
    let lines: Vec<_> = lines.iter().copied().map(Line::from).collect();
    Paragraph::new(lines)
        .centered()
        .render(area.centered_vertically(height), buffer);
}

/// The host mouse tracking an application with `reporting` asks for: none
/// while it reports nothing, when the monitor's own tracking applies.
fn tracking(reporting: MouseReporting) -> Option<u16> {
    match reporting {
        MouseReporting::Off => None,
        MouseReporting::Clicks => Some(1000),
        MouseReporting::Drag => Some(1002),
        MouseReporting::Motion => Some(1003),
    }
}

/// When the host's mouse tracking changes, the old mode reset, then the new
/// one set, so a terminal that keeps one tracking mode and one that keeps
/// each separately both end in the new one; otherwise nothing.
fn tracking_mode(painted: Option<u16>, next: Option<u16>) -> Vec<u8> {
    let from = painted.unwrap_or(OWN_TRACKING);
    let to = next.unwrap_or(OWN_TRACKING);
    if from == to {
        return Vec::new();
    }
    format!("\x1b[?{from}l\x1b[?{to}h").into_bytes()
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
fn session_row(
    buffer: &mut Buffer,
    top: Rect,
    session: &Session<'_>,
    error: Option<&str>,
    hint: Option<&str>,
) {
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
    let slots_start = top.x + room;
    let mut x = slots_start;
    for (text, slot) in slots {
        buffer.set_stringn(x, top.y, SEPARATOR, usize::from(separator), style);
        buffer.set_stringn(x + separator, top.y, text, usize::from(slot), style);
        x += separator + slot;
    }
    let (end, _) = buffer.set_stringn(top.x, top.y, session.label, room.into(), style);
    let left = usize::from(slots_start - end);
    if let Some(error) = error.filter(|_| left > SEPARATOR.width()) {
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
    use climon2_vt::{Geometry, Limits, Model, Parser};

    /// INP-05 (pure supplement; owner decision on #111): the host's mouse
    /// tracking is switched only when it changes, the old mode reset and
    /// the new one set; an application that reports nothing leaves the
    /// monitor's own button-event tracking.
    #[test]
    fn the_host_mouse_tracking_is_written_when_it_changes() {
        assert_eq!(tracking_mode(None, Some(1003)), b"\x1b[?1002l\x1b[?1003h");
        assert_eq!(tracking_mode(Some(1003), None), b"\x1b[?1003l\x1b[?1002h");
        assert_eq!(
            tracking_mode(Some(1000), Some(1003)),
            b"\x1b[?1000l\x1b[?1003h"
        );
        assert_eq!(tracking_mode(None, Some(1002)), b"");
        assert_eq!(tracking_mode(Some(1000), Some(1000)), b"");
        assert_eq!(tracking_mode(None, None), b"");
        let levels = [
            MouseReporting::Off,
            MouseReporting::Clicks,
            MouseReporting::Drag,
            MouseReporting::Motion,
        ];
        assert_eq!(
            levels.map(tracking),
            [None, Some(1000), Some(1002), Some(1003)]
        );
    }

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
            canvas: Canvas::Nothing,
        };
        session_row(
            &mut buffer,
            area,
            &session,
            session.error,
            hint.then_some(HINT),
        );
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
    /// on; a row selected without a connected client says it has no terminal content and its
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
            shown(Some(0x14), true, &Canvas::NoContent("Starting")),
            [
                "",
                "",
                "         No terminal content is available.",
                "                     Starting",
                "",
            ]
        );
        assert_eq!(
            shown(Some(0x14), true, &Canvas::Nothing),
            ["", "", "", "", ""]
        );
    }

    /// DSC-01 (initial-selection case, pure supplement) and the owner's
    /// wording of 2026-11-16: with nothing selected the body reads the
    /// startup progress while collecting, otherwise the empty state until a
    /// row exists and then that nothing is selected; an incomplete pass and
    /// a collection that ended with sessions still starting are each said
    /// as discovery incomplete, the pass first.
    #[test]
    fn the_body_says_the_startup_progress_or_the_state_and_any_incompleteness() {
        let row = View {
            id: "00000000-0000-4000-8000-000000000001".parse().unwrap(),
            status: "Idle".into(),
            label: "Row".into(),
            since: std::time::SystemTime::UNIX_EPOCH,
        };
        let progress = "Connecting to 2 sessions (1 ready)";
        assert_eq!(state(Some(progress), &[]), progress);
        assert_eq!(state(Some(progress), std::slice::from_ref(&row)), progress);
        assert_eq!(state(None, &[]), "No sessions to show.");
        assert_eq!(state(None, &[row]), "No session selected.");
        assert_eq!(incomplete(None, false), Vec::<String>::new());
        assert_eq!(
            incomplete(Some("the pass deadline passed"), true),
            [
                "Discovery incomplete: the pass deadline passed",
                "Discovery incomplete: some sessions are still starting",
            ]
        );
        assert_eq!(
            incomplete(None, true),
            ["Discovery incomplete: some sessions are still starting"]
        );
    }

    /// The owner's decision of 2026-11-16 on #266 (pure supplement): while
    /// the initial collection's late sessions still start, the top row's
    /// error slot says discovery is incomplete with a row selected too,
    /// unless the session has its own error, which it never hides.
    #[test]
    fn a_late_collection_shows_in_the_error_slot_unless_the_session_has_its_own() {
        let late = "Discovery incomplete: some sessions are still starting";
        assert_eq!(error(None, false), None);
        assert_eq!(error(None, true), Some(late));
        assert_eq!(error(Some("timed out"), true), Some("timed out"));
        assert_eq!(error(Some("timed out"), false), Some("timed out"));
    }

    /// What a host `cols` by 4 receives of `model`'s session in one frame:
    /// whether its keypad is in application mode, and the cursor, under the
    /// selector or the key introduction when they are open.
    fn received(
        cols: u16,
        model: &Model,
        selector: bool,
        introduction: bool,
    ) -> (bool, Option<HostCursor>) {
        let prefix = ControlKey::from_byte(0x1d).unwrap();
        let home = tempfile::tempdir().unwrap();
        let layout = climon2_core::layout::HomeLayout::at(home.path()).unwrap();
        let preference = crate::preference::Preference::read(&layout);
        let mut key_introduction = KeyIntroduction::new(prefix, preference, false);
        if introduction {
            key_introduction.open();
        }
        let screen = Screen::new(TerminalSize { cols, rows: 4 }, prefix, None, false);
        let mut list = Selector::new(prefix, false, 4096);
        if selector {
            list.open(&[], None, screen.overlay());
        }
        let scene = Scene {
            introduction: &key_introduction,
            rows: &[],
            selector: &list,
            incomplete: None,
            progress: None,
            late: false,
            session: Some(Session {
                label: "Paint",
                status: "Idle",
                control: "Controlling",
                error: None,
                canvas: Canvas::View(model.view()),
            }),
        };
        let frame = screen.frame(&scene);
        (frame.keypad, frame.cursor)
    }

    /// INP-02 and TRM-01 (pure supplement): the session's application
    /// keypad mode and its cursor reach the host only while its canvas
    /// shows. Either modal alone over it gives the host its own numeric
    /// keypad and no session cursor, and so does a host too small for a
    /// session.
    #[test]
    fn a_modal_or_a_small_host_keeps_the_sessions_keypad_mode_and_cursor() {
        let session = model(10, 3, b"\x1b=\x1b[2;3H");
        let cursor = Some(HostCursor {
            column: 2,
            row: 2,
            style: 0,
        });
        assert_eq!(received(10, &session, false, false), (true, cursor));
        assert_eq!(received(10, &session, true, false), (false, None));
        assert_eq!(received(10, &session, false, true), (false, None));
        assert_eq!(received(4, &session, false, false), (false, None));
    }

    /// A model `cols` by `rows` that took `bytes`.
    fn model(cols: u16, rows: u16, bytes: &[u8]) -> Model {
        let limits = Limits {
            bytes: 64 << 20,
            history_rows: 100,
        };
        let mut model = Model::new(Geometry::for_host(cols, rows).unwrap(), limits).unwrap();
        model.feed(&mut Parser::new(), bytes).unwrap();
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

    /// TRM-01 (pure supplement): each colour the engine reports reaches its
    /// cell as the same kind of colour: the 16 ANSI colours (normal and
    /// bright) as themselves, palette entries as indexes, direct colours as
    /// RGB, and the default as the host's own, for the foreground, the
    /// background and the underline alike.
    #[test]
    fn each_colour_reaches_its_cell_as_the_same_kind() {
        let coloured = model(
            4,
            1,
            b"\x1b[31;44mA\x1b[0;38;5;200;48;2;1;2;3mB\x1b[0;4;95;58;2;9;8;7mC\x1b[0mD",
        );
        let (buffer, _) = painted(&coloured, 4, 2);
        let colours: Vec<_> = buffer.content()[4..8]
            .iter()
            .map(|cell| (cell.fg, cell.bg, cell.underline_color))
            .collect();
        assert_eq!(
            colours,
            [
                (Color::Red, Color::Blue, Color::Reset),
                (Color::Indexed(200), Color::Rgb(1, 2, 3), Color::Reset),
                (Color::LightMagenta, Color::Reset, Color::Rgb(9, 8, 7)),
                (Color::Reset, Color::Reset, Color::Reset),
            ]
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
