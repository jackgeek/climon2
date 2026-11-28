//! What the monitor draws on its own alternate screen: Ratatui renders each
//! frame into a buffer, and [`crate::paint`] writes it to the host.
use crate::{
    introduction::KeyIntroduction,
    keys::EXIT,
    paint::{ANSI, Frame, HostCursor, Underlines, paint},
    selector::Selector,
    toast::{self, LANE},
};
use climon2_core::{SessionId, keys::ControlKey, protocol::TerminalSize};
use climon2_monitor::view::{self as vt, Content, CursorShape, Modes, MouseReporting, Underline};
pub(crate) use climon2_monitor::{Canvas, Placeholder, Session};
use climon2_monitor::{GONE, Recovery, View, toast::Toasts, truncate};
use ratatui::{
    buffer::{Buffer, CellDiffOption},
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Paragraph, Widget},
};
use std::{io, num::NonZeroU16, time::Instant};
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
/// The non-controller placeholder once another client took control, as
/// the client executable words it.
const VIEWED_ELSEWHERE: &str = "This session is being viewed elsewhere.";
/// The non-controller placeholder while no other client is known to have
/// taken control: a session selected without acquiring it (#400).
const NOT_CONTROLLED: &str = "This session is not controlled here.";
/// A selected session with no terminal content to show (spec #73,
/// "Placeholders and states").
const NO_CONTENT: &str = "No terminal content is available.";
/// Over a lost session's retained history while a passive retry is
/// scheduled or running (spec #73, "Placeholders and states").
const RECONNECTING: &str = "Reconnecting...";
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
/// The control slot while the canvas is scrolled back into the session's
/// history (owner decision on #112, 2026-11-16).
const HISTORY: &str = "History";
/// The glossary's truncation notice, above the oldest history row kept
/// once older rows were discarded (owner decision on #112, 2026-11-16).
const NOTICE: &str = "[climon2: older session scrollback was discarded]";
/// SGR 4's subparameter for each underline style but single, which is
/// plain SGR 4.
const UNDERLINES: [(Underline, u8); 4] = [
    (Underline::Double, 2),
    (Underline::Curly, 3),
    (Underline::Dotted, 4),
    (Underline::Dashed, 5),
];

/// What the non-controller placeholder says of who controls the session.
fn says(placeholder: Placeholder) -> &'static str {
    match placeholder {
        Placeholder::TakenOver => VIEWED_ELSEWHERE,
        Placeholder::NotControlled => NOT_CONTROLLED,
    }
}

/// The overlay over a lost session's retained history or its no-content
/// state: exactly `Reconnecting...` while a retry is scheduled or running,
/// otherwise why recovery stopped.
fn overlay(recovery: &Recovery) -> String {
    match recovery {
        Recovery::Retrying(_) => RECONNECTING.to_owned(),
        Recovery::Rejected(why) => format!("Not reconnecting: {why}"),
        Recovery::Gone => GONE.to_owned(),
    }
}

/// Everything one frame shows.
pub(crate) struct Scene<'a> {
    pub(crate) introduction: &'a KeyIntroduction,
    pub(crate) rows: &'a [View],
    pub(crate) selector: &'a Selector,
    /// The open Delete confirmation, over the selector (#117).
    pub(crate) asking: Option<crate::delete::Asking<'a>>,
    /// Why the latest discovery pass was incomplete, if it was.
    pub(crate) incomplete: Option<&'a str>,
    /// The startup progress while the initial collection lasts (#266).
    pub(crate) progress: Option<String>,
    /// Whether the initial collection ended with sessions still starting.
    pub(crate) late: bool,
    /// Prefix+j's notice that no session needs attention, while it shows.
    pub(crate) notice: Option<&'a str>,
    /// The toasts, over everything else, as they look at `now`.
    pub(crate) toasts: &'a Toasts,
    pub(crate) now: Instant,
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

/// Draws frames for the monitor-owned host output, and hands each on once
/// the host has taken the one before (#285). It never asks the host
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
    /// The frame last handed to the host.
    painted: Frame,
    /// The toast, by when it began, whose full title the frame last handed
    /// to the host shows, until the host has taken it ([`Screen::written`]).
    titled: Option<Instant>,
    /// The newest frame drawn since, waiting for the host to take the one
    /// before, and the toast whose full title it shows: a newer one replaces
    /// it, so the host never receives an obsolete frame (#285).
    next: Option<(Frame, Option<Instant>)>,
    /// A frame came due while the host was backed up and was not drawn: it
    /// is drawn once the host has taken what it was handed, unless another
    /// frame is drawn first (#419).
    undrawn: bool,
    /// The selected session's canvas as last painted from a view with no
    /// synchronized update open: it stays while one is.
    coherent: Option<Coherent>,
}

/// A session's canvas as painted from a coherent view, at one host size.
struct Coherent {
    id: SessionId,
    /// The whole frame's buffer; only the rows below the top row are the
    /// canvas.
    buffer: Buffer,
    underlines: Underlines,
    cursor: Option<HostCursor>,
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
            titled: None,
            next: None,
            // Nothing waits yet. Spelled `bool::default()`, false: a line
            // holding only a literal has no coverage region of its own.
            undrawn: bool::default(),
            coherent: None,
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

    /// Where the selector is drawn: the overlay rows above the toasts'
    /// lane (owner decision on #115).
    pub(crate) fn menu(&self) -> Rect {
        let overlay = self.overlay();
        Rect {
            height: overlay.height.saturating_sub(LANE),
            ..overlay
        }
    }

    /// Whether a frame shows the selected session's canvas: nothing
    /// `covered` it and the host has room for a session.
    pub(crate) fn shows_canvas(&self, covered: bool) -> bool {
        !covered && self.desire().is_some()
    }

    /// Draws `scene`'s frame to go to the host next, showing the full title
    /// of the toast that began at `titling`, if any.
    pub(crate) fn show(&mut self, scene: &Scene<'_>, titling: Option<Instant>) -> io::Result<()> {
        let frame = self.frame(scene);
        self.present(frame, titling)
    }

    /// The exit screen while shutdown waits for `waiting` sessions (owner
    /// decision on #118), alone in the middle of the host: nothing else is
    /// shown or takes input any more.
    pub(crate) fn exiting(&mut self, waiting: usize) -> io::Result<()> {
        let frame = self.exit_frame(waiting);
        self.present(frame, None)
    }

    /// The exit screen's frame: the count alone, centred on the host.
    fn exit_frame(&self, waiting: usize) -> Frame {
        let area = Rect::new(0, 0, self.size.cols, self.size.rows);
        let mut buffer = Buffer::empty(area);
        centred(&[&exiting(waiting)], &mut buffer, area);
        Frame {
            buffer,
            ..Frame::default()
        }
    }

    /// `frame`, showing the full title of the toast that began at
    /// `titling`, if any, goes to the host next, in place of any frame
    /// still waiting, unless it is the one handed to the host last.
    fn present(&mut self, frame: Frame, titling: Option<Instant>) -> io::Result<()> {
        #[cfg(feature = "test-hooks")]
        climon2_monitor::hooks::fail("draw")?;
        self.undrawn = false;
        if frame == self.painted {
            self.next = None;
            self.titled = self.titled.or(titling);
        } else {
            self.next = Some((frame, titling));
        }
        Ok(())
    }

    /// A frame came due while the host was backed up: it is drawn once the
    /// host has room (#419).
    pub(crate) fn defer(&mut self) {
        self.undrawn = true;
    }

    /// Whether a frame waits to be drawn once the host has room.
    pub(crate) fn undrawn(&self) -> bool {
        self.undrawn
    }

    /// The host has taken the frame last handed to it: the toast whose full
    /// title it shows, once.
    pub(crate) fn written(&mut self) -> Option<Instant> {
        self.titled.take()
    }

    /// The bytes that paint the waiting frame, if any, which the host is
    /// now handed: every cell, so whatever the host showed is overwritten
    /// in place, with nothing erased first and no blank screen between two
    /// frames.
    pub(crate) fn commit(&mut self) -> Option<Vec<u8>> {
        let (frame, titling) = self.next.take()?;
        let mut bytes = keypad_mode(self.painted.keypad, frame.keypad).to_vec();
        bytes.extend(tracking_mode(self.painted.tracking, frame.tracking));
        bytes.extend(paint(&frame));
        self.painted = frame;
        self.titled = titling;
        Some(bytes)
    }

    /// The permanent top row, and below it the selected session's canvas or
    /// the body, under the selector and then the key introduction while they
    /// are open, and any toast over them all. Too small a host shows only
    /// why, in the top row.
    fn frame(&mut self, scene: &Scene<'_>) -> Frame {
        let area = Rect::new(0, 0, self.size.cols, self.size.rows);
        let mut buffer = Buffer::empty(area);
        let mut underlines = Underlines::new();
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
                let error = error(scene.notice, session.error, scene.late);
                session_row(&mut buffer, top, session, error, self.top_hint.as_deref());
                let canvas = (&mut buffer, &mut underlines);
                self.canvas(session.id, &session.canvas, canvas, below)
            }
            (Some(_), None) => {
                top_row(&mut buffer, top, NO_SELECTION);
                self.body(scene, &mut buffer, below);
                None
            }
        };
        let search = scene.selector.render(scene.rows, self.menu(), &mut buffer);
        if let Some(asking) = &scene.asking {
            asking.render(below, &mut buffer);
        }
        scene.introduction.render(below, &mut buffer);
        toast::render(scene.toasts, scene.now, below, &mut buffer);
        let modes = self.followed(scene);
        Frame {
            buffer,
            cursor: shown_cursor(scene, cursor.filter(|_| self.desire().is_some()), search),
            keypad: modes.is_some_and(|modes| modes.application_keypad),
            tracking: modes.and_then(|modes| tracking(modes.mouse_reporting)),
            underlines,
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
        let state = state(scene.notice, scene.progress.as_deref(), scene.rows);
        let lines: Vec<_> = std::iter::once(state)
            .chain(note)
            .chain(incomplete.iter().map(String::as_str))
            .collect();
        centred(&lines, buffer, below);
    }

    /// The selected session's canvas below the top row; returns the
    /// cursor its view shows, if any.
    fn canvas(
        &mut self,
        id: &SessionId,
        canvas: &Canvas<'_>,
        (buffer, underlines): (&mut Buffer, &mut Underlines),
        below: Rect,
    ) -> Option<HostCursor> {
        match canvas {
            Canvas::View(view) => return self.view(id, view, buffer, underlines),
            Canvas::Retained(view, recovery) => {
                painted(view, buffer, underlines);
                if let Some(recovery) = recovery {
                    banner(&overlay(recovery), buffer, below);
                }
            }
            text => {
                let lines = self.text(text);
                let lines: Vec<_> = lines.iter().map(String::as_str).collect();
                centred(&lines, buffer, below);
            }
        }
        None
    }

    /// The lines a canvas with no view says, centred: the placeholder with
    /// any takeover hint, or no content, the condition and any overlay.
    fn text(&self, canvas: &Canvas<'_>) -> Vec<String> {
        match canvas {
            Canvas::Placeholder(placeholder) => std::iter::once(says(*placeholder))
                .chain(self.takeover.as_deref())
                .map(str::to_owned)
                .collect(),
            Canvas::NoContent(condition, recovery) => [NO_CONTENT, condition]
                .map(str::to_owned)
                .into_iter()
                .chain(recovery.map(overlay))
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// The host cursor: the search's while the selector is open, otherwise the
/// selected session's while the host has room for it; none while the key
/// introduction or the Delete confirmation covers them.
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
    cursor.filter(|_| !scene.introduction.is_open() && scene.asking.is_none())
}

impl Screen {
    /// The session's view below the top row, unless its application's
    /// synchronized update is open: then the canvas last painted for it
    /// at this host size stays, so a half-drawn screen never shows (the
    /// daemon ends an update still open after 150 ms). The top row and
    /// any overlay are drawn as ever.
    fn view(
        &mut self,
        id: &SessionId,
        view: &vt::View<'_>,
        buffer: &mut Buffer,
        underlines: &mut Underlines,
    ) -> Option<HostCursor> {
        let held = self.coherent.as_ref().filter(|held| {
            view.synchronizing() && held.id == *id && held.buffer.area == buffer.area
        });
        if let Some(held) = held {
            let canvas = usize::from(TOP_ROW * buffer.area.width)..;
            buffer.content[canvas.clone()].clone_from_slice(&held.buffer.content[canvas]);
            underlines.clone_from(&held.underlines);
            return held.cursor;
        }
        let cursor = painted(view, buffer, underlines);
        self.coherent = Some(Coherent {
            id: id.clone(),
            buffer: buffer.clone(),
            underlines: underlines.clone(),
            cursor,
        });
        cursor
    }
}

/// What the exit screen says while shutdown waits for `waiting` sessions
/// (owner decision on #118), one session in the singular.
fn exiting(waiting: usize) -> String {
    let sessions = ["sessions", "session"][usize::from(waiting == 1)];
    format!("Exiting: waiting for {waiting} {sessions}")
}

/// The body's state with nothing selected: prefix+j's `notice` while it
/// shows (owner decision on #114), otherwise the startup progress while the
/// initial collection lasts, otherwise the empty state until a row exists,
/// then that nothing is selected.
fn state<'a>(notice: Option<&'a str>, progress: Option<&'a str>, rows: &[View]) -> &'a str {
    let idle = [EMPTY, UNSELECTED][usize::from(!rows.is_empty())];
    notice.or(progress).unwrap_or(idle)
}

/// The top row's error slot: prefix+j's `notice` while it shows (owner
/// decision on #114), otherwise the session's own error, otherwise, while
/// the initial collection's late sessions still start, that discovery is
/// incomplete (owner decision, 2026-11-16).
fn error<'a>(notice: Option<&'a str>, own: Option<&'a str>, late: bool) -> Option<&'a str> {
    notice.or(own).or(late.then_some(LATE_INCOMPLETE))
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

/// `text` reversed, with a space either side, on the middle row of `area`
/// and centred across it: an overlay that leaves the rest in view.
fn banner(text: &str, buffer: &mut Buffer, area: Rect) {
    let text = format!(" {text} ");
    let width = (text.width() as u16).min(area.width);
    let x = area.x + (area.width - width) / 2;
    let y = area.y + area.height / 2;
    buffer.set_stringn(x, y, text, width.into(), Style::new().reversed());
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
/// the cells they leave from the left, truncated with `…` when it
/// overflows them (nothing marquees), and any error follows it in what
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
        (control(session), CONTROL_SLOT),
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
    let label = truncate(session.label, room.into());
    let (end, _) = buffer.set_stringn(top.x, top.y, label, room.into(), style);
    let left = usize::from(slots_start - end);
    if let Some(error) = error.filter(|_| left > SEPARATOR.width()) {
        buffer.set_stringn(end, top.y, format!("{SEPARATOR}{error}"), left, style);
    }
}

/// The control slot: `History` while the canvas is scrolled back into the
/// session's history, otherwise the control held.
fn control<'a>(session: &Session<'a>) -> &'a str {
    let scrolled = session
        .canvas
        .painted()
        .is_some_and(|view| view.scrolled() > 0);
    if scrolled { HISTORY } else { session.control }
}

/// A row the canvas shows.
enum Shown<'a> {
    Row(vt::Row<'a>),
    /// The truncation notice, above the oldest history row kept, within
    /// the session's columns.
    Notice(usize),
}

/// The rows the canvas shows, top first, as many as the active screen
/// has: the alternate screen's own, or the normal screen's scrolled back
/// [`vt::View::scrolled`] rows into its history, with the truncation notice
/// one row past the oldest kept row.
fn shown<'a>(view: &vt::View<'a>) -> Vec<Shown<'a>> {
    let (rows, history) = match view.screen() {
        vt::Screen::Normal { rows, history } => (rows, Some(history.rows)),
        vt::Screen::Alternate { rows } => (rows, None),
    };
    let history = history.map_or_else(Vec::new, |history| history.iter().collect());
    let scrolled = view.scrolled();
    let first = rows.iter().next().expect("every screen has a row");
    let cols = first.cells().len();
    let notice = (scrolled > history.len()).then_some(Shown::Notice(cols));
    let back = history.len().saturating_sub(scrolled);
    notice
        .into_iter()
        .chain(history.into_iter().skip(back).map(Shown::Row))
        .chain(rows.iter().map(Shown::Row))
        .take(rows.len())
        .collect()
}

/// The session's screen directly below the top row, at the model's
/// geometry, clipped to the host; the cells it leaves stay blank margins.
/// Returns its cursor if the host shows it.
fn painted(
    view: &vt::View<'_>,
    buffer: &mut Buffer,
    underlines: &mut Underlines,
) -> Option<HostCursor> {
    let rows = shown(view);
    let height = buffer.area.height;
    for (y, shown) in (TOP_ROW..height).zip(&rows) {
        match shown {
            Shown::Row(row) => paint_row(buffer, y, row, underlines),
            Shown::Notice(cols) => {
                buffer.set_stringn(0, y, NOTICE, *cols, Style::new());
            }
        }
    }
    cursor(view, rows.len(), buffer.area)
}

/// One row of session cells on host row `y`.
fn paint_row(buffer: &mut Buffer, y: u16, row: &vt::Row<'_>, underlines: &mut Underlines) {
    let width = buffer.area.width;
    for (x, cell) in (0..width).zip(row.cells()) {
        paint_cell(&mut buffer[(x, y)], &cell, x + 1 < width);
        if let Some(style) = underline(&cell) {
            underlines.insert((x, y), style);
        }
    }
}

/// SGR 4's subparameter for `cell`'s underline style, unless it has none
/// or a single one.
fn underline(cell: &vt::Cell<'_>) -> Option<u8> {
    UNDERLINES
        .iter()
        .find(|(style, _)| cell.rendition.underline == Some(*style))
        .map(|(_, code)| *code)
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
/// application shows it, the canvas's `rows` show its row (scrolling back
/// moves it down, and out of view) and the host has the cell; never in the
/// top row.
fn cursor(view: &vt::View<'_>, rows: usize, area: Rect) -> Option<HostCursor> {
    let cursor = view.cursor();
    let below = usize::from(cursor.row) + view.scrolled();
    let row = u16::try_from(below).ok().filter(|_| below < rows)? + TOP_ROW;
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
    use climon2_monitor::{Control, diagnostic};
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

    fn id() -> SessionId {
        "00000000-0000-4000-8000-000000000001".parse().unwrap()
    }

    /// The top row of a selected session `width` cells wide, as text.
    fn top(width: u16, label: &str, control: &str, error: Option<&str>, hint: bool) -> String {
        let area = Rect::new(0, 0, width, 1);
        let mut buffer = Buffer::empty(area);
        let id = id();
        let session = Session {
            id: &id,
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
        // The cell a wide character covers is never painted.
        let mut covered = false;
        let mut text = String::new();
        for cell in buffer.content() {
            if !covered {
                assert_eq!(cell.modifier, Modifier::REVERSED, "{cell:?}");
                text.push_str(cell.symbol());
            }
            covered = cell.symbol().width() > 1;
        }
        text
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
    /// separator is not shown. TRM-06 (owner decision on #112): a label
    /// that fits its room exactly is whole, and one a cell longer is
    /// truncated statically with `…`, display-cell safe.
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
            "A very long session … | Unavailable        | Controlling    "
        );
        assert_eq!(
            top(60, "A long title 日本語 here", "Controlling", error, false),
            "A long title 日本語 … | Unavailable        | Controlling    "
        );
        assert_eq!(
            top(60, "Twenty-one columns!!!", "Controlling", error, false),
            "Twenty-one columns!!! | Unavailable        | Controlling    "
        );
        assert_eq!(
            top(60, "Twenty-two columns!!!!", "Controlling", error, false),
            "Twenty-two columns!!… | Unavailable        | Controlling    "
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
        let mut screen = screen;
        let mut buffer = Buffer::empty(Rect::new(0, 0, 50, 6));
        let below = Rect::new(0, 1, 50, 5);
        let canvas_of = (&mut buffer, &mut Underlines::new());
        assert_eq!(screen.canvas(&id(), canvas, canvas_of, below), None);
        (1..6)
            .map(|y| {
                let row: String = (0..50).map(|x| buffer[(x, y)].symbol()).collect();
                row.trim_end().to_owned()
            })
            .collect()
    }

    /// CTL-02 and INP-01 (pure supplement, #110): the non-controller
    /// placeholder says the session is viewed elsewhere once another client
    /// took control, and otherwise only that it is not controlled here
    /// (#400), centred, with the take-control key's hint only while one is
    /// bound and key hints are on; a row selected without a connected
    /// client says it has no terminal content and its condition; a blank
    /// canvas shows nothing.
    #[test]
    fn the_placeholder_and_the_no_content_state_say_why_nothing_is_painted() {
        let says = [
            (
                Placeholder::TakenOver,
                "      This session is being viewed elsewhere.",
            ),
            (
                Placeholder::NotControlled,
                "       This session is not controlled here.",
            ),
        ];
        for (placeholder, line) in says {
            let canvas = Canvas::Placeholder(placeholder);
            assert_eq!(
                shown(Some(0x14), true, &canvas),
                ["", "", line, "           Press Ctrl+T to take control", ""]
            );
            let alone = ["", "", line, "", ""];
            assert_eq!(shown(None, true, &canvas), alone);
            assert_eq!(shown(Some(0x14), false, &canvas), alone);
        }
        assert_eq!(
            shown(Some(0x14), true, &Canvas::NoContent("Starting", None)),
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

    /// REC-02 (pure supplement; owner decisions on #116): a lost session's
    /// retained history is painted read-only, with no cursor, under exactly
    /// `Reconnecting...` while a retry is scheduled or running, why
    /// recovery stopped once rejected, or that the session no longer
    /// exists; with no history, the no-content state carries the same
    /// overlay under its condition. The top row's error slot gets the
    /// diagnostic, and the control slot says `History` once scrolled back.
    #[test]
    fn a_lost_session_shows_its_history_read_only_under_its_overlay() {
        let mut model = model(50, 5, b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix");
        let retrying = Recovery::Retrying("transport io_error: reset".into());
        let rejected = Recovery::Rejected("unsupported_capability".into());
        let history = |recovery| shown(None, true, &Canvas::Retained(model.view(), recovery));
        assert_eq!(
            history(Some(&retrying)),
            [
                "two",
                "three",
                &format!("{:<17}Reconnecting...", "four"),
                "five",
                "six"
            ]
        );
        assert_eq!(history(None), ["two", "three", "four", "five", "six"]);
        let long = Recovery::Rejected("x".repeat(60));
        let clipped = format!(" Not reconnecting: {}", "x".repeat(31));
        assert_eq!(history(Some(&long))[2], clipped, "clipped to the canvas");
        assert_eq!(
            history(Some(&Recovery::Gone))[2],
            format!("{:<13}Session no longer exists", "four")
        );
        let none = |recovery| shown(None, true, &Canvas::NoContent("Disconnected", recovery));
        let centred =
            |text: &str| format!("{:>width$}", text, width = (50 + text.len()).div_ceil(2));
        let content = |overlay: &str| {
            vec![
                String::new(),
                "         No terminal content is available.".to_owned(),
                centred("Disconnected"),
                centred(overlay),
                String::new(),
            ]
        };
        assert_eq!(none(Some(&retrying)), content("Reconnecting..."));
        assert_eq!(
            none(Some(&rejected)),
            content("Not reconnecting: unsupported_capability")
        );
        assert_eq!(
            none(Some(&Recovery::Gone)),
            content("Session no longer exists")
        );
        assert_eq!(diagnostic(&retrying), "transport io_error: reset");
        assert_eq!(diagnostic(&rejected), "unsupported_capability");
        assert_eq!(diagnostic(&Recovery::Gone), "Session no longer exists");
        model.scroll(1);
        let session = Session {
            id: &id(),
            label: "Lost",
            status: "Disconnected",
            control: "Not controlling",
            error: Some(diagnostic(&retrying)),
            canvas: Canvas::Retained(model.view(), Some(&retrying)),
        };
        let area = Rect::new(0, 0, 90, 1);
        let mut buffer = Buffer::empty(area);
        session_row(&mut buffer, area, &session, session.error, None);
        let top: String = (0..90).map(|x| buffer[(x, 0)].symbol()).collect();
        let label = "Lost | transport io_error: reset";
        let expected = format!("{label:<51} | {:<18} | {:<15}", "Disconnected", "History");
        assert_eq!(top, expected);
    }

    /// Owner decision on #118 (pure supplement): the exit screen counts the
    /// sessions shutdown still waits for, one in the singular.
    #[test]
    fn the_exit_screen_counts_the_sessions_it_waits_for() {
        assert_eq!(exiting(0), "Exiting: waiting for 0 sessions");
        assert_eq!(exiting(1), "Exiting: waiting for 1 session");
        assert_eq!(exiting(2), "Exiting: waiting for 2 sessions");
    }

    /// What `bytes`, a painted frame, say.
    fn painted_text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    /// #285 (pure supplement): frames drawn before the host takes the one
    /// before are not queued: the newest replaces any waiting, one the host
    /// was handed last replaces nothing and cancels a waiting one, and
    /// each frame is handed over once.
    #[test]
    fn only_the_newest_frame_waits_for_the_host() {
        let prefix = ControlKey::from_byte(0x1d).unwrap();
        let mut screen = Screen::new(TerminalSize { cols: 40, rows: 9 }, prefix, None, true);
        screen.exiting(3).unwrap();
        let first = painted_text(&screen.commit().unwrap());
        assert!(first.contains("for 3 sessions"), "{first}");
        assert_eq!(screen.commit(), None);
        screen.exiting(2).unwrap();
        screen.exiting(1).unwrap();
        let newest = painted_text(&screen.commit().unwrap());
        assert!(newest.contains("for 1 session"), "{newest}");
        assert!(!newest.contains("for 2"), "{newest}");
        screen.exiting(0).unwrap();
        screen.exiting(1).unwrap();
        assert_eq!(screen.commit(), None);
        // After a resize the host may no longer show what it was handed.
        screen.resize(TerminalSize { cols: 40, rows: 9 });
        screen.exiting(1).unwrap();
        assert!(screen.commit().is_some());
    }

    /// #419 (pure supplement): a frame deferred while the host is backed
    /// up waits to be drawn until any frame is drawn, the exit screen
    /// included, which then goes to the host instead.
    #[test]
    fn a_deferred_frame_waits_until_another_is_drawn() {
        let prefix = ControlKey::from_byte(0x1d).unwrap();
        let mut screen = Screen::new(TerminalSize { cols: 40, rows: 9 }, prefix, None, true);
        assert!(!screen.undrawn());
        screen.defer();
        assert!(screen.undrawn());
        assert_eq!(screen.commit(), None);
        screen.exiting(2).unwrap();
        assert!(!screen.undrawn());
        let painted = painted_text(&screen.commit().unwrap());
        assert!(painted.contains("for 2 sessions"), "{painted}");
    }

    /// #285 and the owner decision on #115 (pure supplement): the toast
    /// whose full title a frame shows is told once the host has taken that
    /// frame: once that frame is handed over, never for a frame that was
    /// replaced first, and for the frame handed over last when a newer one
    /// is the same.
    #[test]
    fn a_frame_showing_a_title_is_told_once_it_is_written() {
        let prefix = ControlKey::from_byte(0x1d).unwrap();
        let mut screen = Screen::new(TerminalSize { cols: 40, rows: 9 }, prefix, None, true);
        let toast = Instant::now();
        let later = toast + std::time::Duration::from_secs(1);
        screen.present(screen.exit_frame(1), Some(toast)).unwrap();
        assert_eq!(screen.written(), None);
        screen.present(screen.exit_frame(2), None).unwrap();
        screen.commit().unwrap();
        assert_eq!(screen.written(), None);
        screen.present(screen.exit_frame(3), Some(toast)).unwrap();
        screen.commit().unwrap();
        assert_eq!(screen.written(), Some(toast));
        assert_eq!(screen.written(), None);
        screen.present(screen.exit_frame(3), Some(later)).unwrap();
        assert_eq!(screen.commit(), None);
        assert_eq!(screen.written(), Some(later));
    }

    /// Owner decision on #118 (pure supplement): the exit screen covers the
    /// whole host with its count alone, centred across and down it.
    #[test]
    fn the_exit_screen_is_its_count_alone_in_the_middle_of_the_host() {
        let prefix = ControlKey::from_byte(0x1d).unwrap();
        let screen = Screen::new(TerminalSize { cols: 40, rows: 9 }, prefix, None, true);
        let buffer = screen.exit_frame(1).buffer;
        assert_eq!(buffer.area, Rect::new(0, 0, 40, 9));
        let rows: Vec<String> = (0..9)
            .map(|y| (0..40).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        let middle = format!("{:5}{:<35}", "", "Exiting: waiting for 1 session");
        for (y, row) in rows.iter().enumerate() {
            let expected = if y == 4 { middle.as_str() } else { "" };
            assert_eq!(row.trim_end(), expected.trim_end(), "row {y}");
        }
    }

    /// DSC-01 (initial-selection case, pure supplement) and the owner's
    /// wording of 2026-11-16: with nothing selected the body reads the
    /// startup progress while collecting, otherwise the empty state until a
    /// row exists and then that nothing is selected; prefix+j's notice
    /// replaces either while it shows (NOT-03, owner decision on #114); an
    /// incomplete pass and
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
        assert_eq!(state(None, Some(progress), &[]), progress);
        let rows = std::slice::from_ref(&row);
        assert_eq!(state(None, Some(progress), rows), progress);
        assert_eq!(state(None, None, &[]), "No sessions to show.");
        assert_eq!(state(None, None, rows), "No session selected.");
        let notice = "No sessions need attention";
        assert_eq!(state(Some(notice), Some(progress), rows), notice);
        assert_eq!(state(Some(notice), None, &[]), notice);
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
    /// Prefix+j's notice shows there first while it shows (NOT-03, owner
    /// decision on #114).
    #[test]
    fn a_late_collection_shows_in_the_error_slot_unless_the_session_has_its_own() {
        let late = "Discovery incomplete: some sessions are still starting";
        assert_eq!(error(None, None, false), None);
        assert_eq!(error(None, None, true), Some(late));
        assert_eq!(error(None, Some("timed out"), true), Some("timed out"));
        assert_eq!(error(None, Some("timed out"), false), Some("timed out"));
        let notice = "No sessions need attention";
        assert_eq!(error(Some(notice), Some("timed out"), true), Some(notice));
        assert_eq!(error(Some(notice), None, false), Some(notice));
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
        let mut screen = Screen::new(TerminalSize { cols, rows: 4 }, prefix, None, false);
        let mut list = Selector::new(prefix, false, 4096);
        if selector {
            list.open(&[], None, screen.overlay());
        }
        let id = id();
        let toasts = Toasts::new(None);
        let scene = Scene {
            introduction: &key_introduction,
            rows: &[],
            selector: &list,
            asking: None,
            incomplete: None,
            progress: None,
            late: false,
            notice: None,
            toasts: &toasts,
            now: Instant::now(),
            session: Some(Session {
                id: &id,
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
        let (buffer, _, cursor) = drawn(model, cols, rows);
        (buffer, cursor)
    }

    /// [`painted`], with the frame's underline styles.
    fn drawn(model: &Model, cols: u16, rows: u16) -> (Buffer, Underlines, Option<HostCursor>) {
        let mut buffer = Buffer::empty(Rect::new(0, 0, cols, rows));
        let mut underlines = Underlines::new();
        let cursor = super::painted(&model.view(), &mut buffer, &mut underlines);
        (buffer, underlines, cursor)
    }

    /// The text of each host row below the top row, trailing blanks
    /// trimmed.
    fn below(buffer: &Buffer) -> Vec<String> {
        (1..buffer.area.height)
            .map(|y| {
                let row: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                row.trim_end().to_owned()
            })
            .collect()
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

    /// TRM-05, TRM-01 (pure supplement; owner decision on #112): a canvas
    /// scrolled back shows that many history rows above the screen's top
    /// rows, as many rows as the screen has; the cursor moves down with the
    /// screen and is hidden once scrolled out of view, at the canvas's last
    /// row and one past it. The alternate screen shows its own rows.
    #[test]
    fn a_scrolled_canvas_shows_history_and_hides_the_cursor_out_of_view() {
        let mut model = model(6, 3, b"h0\r\nh1\r\nh2\r\ns0\r\ns1\r\ns2");
        let (buffer, cursor) = painted(&model, 6, 4);
        assert_eq!(below(&buffer), ["s0", "s1", "s2"]);
        assert_eq!(cursor.map(|cursor| cursor.row), Some(3));
        model.scroll(1);
        let (buffer, cursor) = painted(&model, 6, 4);
        assert_eq!(below(&buffer), ["h2", "s0", "s1"]);
        assert_eq!(cursor, None, "one row past the canvas");
        let (buffer, cursor) = painted(&model, 6, 6);
        assert_eq!(below(&buffer), ["h2", "s0", "s1", "", ""], "blank margins");
        assert_eq!(cursor, None, "one row past the canvas, above the margin");
        model.feed(&mut Parser::new(), b"\x1b[2;1H").unwrap();
        let (_, cursor) = painted(&model, 6, 4);
        assert_eq!(cursor.map(|cursor| cursor.row), Some(3), "its last row");
        model.scroll(2);
        let (buffer, cursor) = painted(&model, 6, 4);
        assert_eq!(below(&buffer), ["h0", "h1", "h2"]);
        assert_eq!(cursor, None);
        model.feed(&mut Parser::new(), b"\x1b[?1049halt").unwrap();
        let (buffer, cursor) = painted(&model, 6, 4);
        assert_eq!(below(&buffer), ["", "alt", ""]);
        assert_eq!(cursor.map(|cursor| cursor.row), Some(2));
    }

    /// TRM-05 (pure supplement; owner decision on #112): once the rows a
    /// scrolled canvas showed are evicted, it shows the glossary's
    /// truncation notice, clipped to the session's columns, above the
    /// oldest row kept.
    #[test]
    fn an_evicted_view_shows_the_notice_above_the_oldest_kept_row() {
        let limits = Limits {
            bytes: 64 << 20,
            history_rows: 2,
        };
        let geometry = Geometry::for_host(60, 2).unwrap();
        let mut model = Model::new(geometry, limits).unwrap();
        let mut parser = Parser::new();
        model.feed(&mut parser, b"h0\r\nh1\r\ns0\r\ns1").unwrap();
        model.scroll(2);
        model.feed(&mut parser, b"\r\ns2").unwrap();
        let (buffer, _) = painted(&model, 70, 3);
        assert_eq!(below(&buffer), [NOTICE, "h1"]);
        let (buffer, _) = painted(&model, 30, 3);
        assert_eq!(below(&buffer), ["[climon2: older session scroll", "h1"]);
        let narrow = Geometry::for_host(9, 2).unwrap();
        let mut model = Model::new(narrow, limits).unwrap();
        model
            .feed(&mut parser, b"h0\r\nh1\r\nh2\r\ns0\r\ns1")
            .unwrap();
        model.scroll(3);
        let (buffer, _) = painted(&model, 70, 3);
        assert_eq!(below(&buffer), ["[climon2:", "h1"]);
    }

    /// TRM-05 (pure supplement; owner decision on #112): while the canvas is
    /// scrolled back into history the control slot says `History`; live, it
    /// says the control held.
    #[test]
    fn while_scrolled_back_the_control_slot_says_history() {
        let mut model = model(4, 1, b"a\r\nb");
        let id = id();
        let slot = |model: &Model| {
            let view = model.view();
            let session = Session {
                id: &id,
                label: "",
                status: "",
                control: "Controlling",
                error: None,
                canvas: Canvas::View(view),
            };
            control(&session).to_owned()
        };
        assert_eq!(slot(&model), "Controlling");
        model.scroll(1);
        assert_eq!(slot(&model), "History");
    }

    /// TRM-01 (pure supplement; note from #108 on #112): each underline
    /// style reaches its cell: underlined, with SGR 4's subparameter for
    /// double, curly, dotted and dashed, and none for single or none.
    #[test]
    fn each_underline_style_reaches_its_cell() {
        let styled = model(
            6,
            1,
            b"\x1b[4mA\x1b[4:2mB\x1b[4:3mC\x1b[4:4mD\x1b[4:5mE\x1b[0mF",
        );
        let (buffer, underlines, _) = drawn(&styled, 6, 2);
        let underlined: Vec<_> = buffer.content()[6..]
            .iter()
            .map(|cell| cell.modifier.contains(Modifier::UNDERLINED))
            .collect();
        assert_eq!(underlined, [true, true, true, true, true, false]);
        let expected: Underlines = [((1, 1), 2), ((2, 1), 3), ((3, 1), 4), ((4, 1), 5)]
            .into_iter()
            .collect();
        assert_eq!(underlines, expected);
    }

    /// TRM-01 (pure supplement): synchronized updates. While the session's
    /// update is open the canvas painted before it stays, cursor and
    /// underline styles included, and once it ends the whole update shows.
    /// A canvas held for one session or host size is never shown for
    /// another.
    #[test]
    fn an_open_synchronized_update_keeps_the_last_coherent_canvas() {
        let prefix = ControlKey::from_byte(0x1d).unwrap();
        let mut screen = Screen::new(TerminalSize { cols: 6, rows: 3 }, prefix, None, true);
        let mut model = model(6, 2, b"\x1b[4:3mold\x1b[0m");
        let mut parser = Parser::new();
        let calls = std::cell::Cell::new(0);
        let view = |screen: &mut Screen, model: &Model, id: &SessionId, cols| {
            let mut buffer = Buffer::empty(Rect::new(0, 0, cols, 3));
            // Each frame's top row is its own.
            calls.set(calls.get() + 1);
            let top = format!("top{}", calls.get());
            buffer.set_string(0, 0, &top, Style::new());
            let mut underlines = Underlines::new();
            let cursor = screen.view(id, &model.view(), &mut buffer, &mut underlines);
            let drawn: String = (0..cols).map(|x| buffer[(x, 0)].symbol()).collect();
            assert_eq!(drawn.trim_end(), top, "the top row stays");
            (
                below(&buffer),
                underlines,
                cursor.map(|cursor| cursor.column),
            )
        };
        let old = (
            vec!["old".to_owned(), String::new()],
            [((0, 1), 3), ((1, 1), 3), ((2, 1), 3)]
                .into_iter()
                .collect(),
            Some(3),
        );
        assert_eq!(view(&mut screen, &model, &id(), 6), old);
        model
            .feed(&mut parser, b"\x1b[?2026h\x1b[2J\x1b[Hne")
            .unwrap();
        assert_eq!(view(&mut screen, &model, &id(), 6), old);
        let other: SessionId = "00000000-0000-4000-8000-000000000002".parse().unwrap();
        let half = (
            vec!["ne".to_owned(), String::new()],
            Underlines::new(),
            Some(2),
        );
        assert_eq!(view(&mut screen, &model, &other, 6), half);
        assert_eq!(view(&mut screen, &model, &id(), 5).0, half.0);
        model.feed(&mut parser, b"w\x1b[?2026l").unwrap();
        let new = (
            vec!["new".to_owned(), String::new()],
            Underlines::new(),
            Some(3),
        );
        assert_eq!(view(&mut screen, &model, &id(), 6), new);
    }
}
