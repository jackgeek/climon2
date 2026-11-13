//! What the monitor draws on its own alternate screen, through Ratatui.
use crate::{introduction::KeyIntroduction, keys::EXIT, registry::View, selector::Selector};
use climon2_core::{keys::ControlKey, protocol::TerminalSize};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::Stylize,
    text::Line,
    widgets::{Paragraph, Widget},
};
use std::io::{self, Stdout};

/// The permanent top row's height.
const TOP_ROW: u16 = 1;
const NO_SELECTION: &str = "No session selected";
const EMPTY: &str = "No sessions to show.";
/// The body once rows exist, while nothing is selected (#107, #108).
const UNSELECTED: &str = "No session selected.";

/// Everything one frame shows.
pub(crate) struct Scene<'a> {
    pub(crate) introduction: &'a KeyIntroduction,
    pub(crate) rows: &'a [View],
    pub(crate) selector: &'a Selector,
    /// Why the latest discovery pass was incomplete, if it was.
    pub(crate) incomplete: Option<&'a str>,
}

/// Ratatui's backend writing to the monitor-owned host output. It is never
/// asked the host's size or cursor, and the cursor stays hidden.
pub(crate) struct Screen {
    backend: CrosstermBackend<Stdout>,
    /// The empty state's key hint, unless key hints are off.
    hint: Option<String>,
    size: TerminalSize,
    /// The frame last painted.
    painted: Buffer,
}

impl Screen {
    /// The empty state names `prefix` in its key hint, unless `hints` is off,
    /// under the key introduction while it is open.
    /// Hides the cursor; nothing is painted until the first [`Self::show`].
    pub(crate) fn new(size: TerminalSize, prefix: ControlKey, hints: bool) -> io::Result<Self> {
        let exit = char::from(EXIT.key);
        let mut screen = Self {
            backend: CrosstermBackend::new(io::stdout()),
            hint: hints.then(|| format!("Press {prefix} then {exit} to exit.")),
            size,
            painted: Buffer::default(),
        };
        screen.backend.hide_cursor()?;
        Ok(screen)
    }

    /// The next frame is drawn at `size`.
    pub(crate) fn resize(&mut self, size: TerminalSize) {
        self.size = size;
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
    fn paint(&mut self, frame: Buffer) -> io::Result<()> {
        #[cfg(feature = "test-hooks")]
        crate::hooks::fail("draw")?;
        let cells = frame.content().iter().enumerate().map(|(index, cell)| {
            let (x, y) = frame.pos_of(index);
            (x, y, cell)
        });
        self.backend.draw(cells)?;
        Backend::flush(&mut self.backend)?;
        self.painted = frame;
        Ok(())
    }

    /// The permanent top row, and below it the body, centred, under the
    /// selector and then the key introduction while they are open. The body
    /// is the empty state until a row exists, then says nothing is selected;
    /// an incomplete discovery pass is said under it, so a list is never
    /// presented as complete. With key hints off, the body names any problem
    /// with the saved choice where its hint would be: a diagnostic, not a
    /// hint.
    fn frame(&self, scene: &Scene<'_>) -> Buffer {
        let introduction = scene.introduction;
        let area = Rect::new(0, 0, self.size.cols, self.size.rows);
        let mut frame = Buffer::empty(area);
        let top = Rect {
            height: TOP_ROW,
            ..area
        };
        let below = Rect {
            y: top.bottom(),
            height: area.height - TOP_ROW,
            ..area
        };
        Paragraph::new(NO_SELECTION)
            .reversed()
            .render(top, &mut frame);
        let note = self.hint.as_deref().or(introduction.problem());
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
            .render(below.centered_vertically(height), &mut frame);
        scene.selector.render(scene.rows, below, &mut frame);
        introduction.render(below, &mut frame);
        frame
    }
}
