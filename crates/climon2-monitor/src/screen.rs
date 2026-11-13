//! What the monitor draws on its own alternate screen, through Ratatui.
use crate::{introduction::KeyIntroduction, keys::EXIT};
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

/// Ratatui's backend writing to the monitor-owned host output. It is never
/// asked the host's size or cursor, and the cursor stays hidden.
pub(crate) struct Screen {
    backend: CrosstermBackend<Stdout>,
    lines: Vec<Line<'static>>,
    size: TerminalSize,
    /// The frame last painted.
    painted: Buffer,
}

impl Screen {
    /// The empty state names `prefix` in its key hint, unless `hints` is off,
    /// under the key introduction while it is open.
    pub(crate) fn new(
        size: TerminalSize,
        prefix: ControlKey,
        hints: bool,
        introduction: &KeyIntroduction,
    ) -> io::Result<Self> {
        let exit = char::from(EXIT.key);
        let hint = hints.then(|| Line::from(format!("Press {prefix} then {exit} to exit.")));
        let mut screen = Self {
            backend: CrosstermBackend::new(io::stdout()),
            lines: std::iter::once(Line::from(EMPTY)).chain(hint).collect(),
            size,
            painted: Buffer::default(),
        };
        screen.backend.hide_cursor()?;
        screen.paint(screen.frame(introduction))?;
        Ok(screen)
    }

    pub(crate) fn resize(
        &mut self,
        size: TerminalSize,
        introduction: &KeyIntroduction,
    ) -> io::Result<()> {
        self.size = size;
        self.paint(self.frame(introduction))
    }

    /// Shows `introduction` over the empty state; an unchanged screen is not
    /// painted again.
    pub(crate) fn show(&mut self, introduction: &KeyIntroduction) -> io::Result<()> {
        let frame = self.frame(introduction);
        if frame == self.painted {
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

    /// The permanent top row, and below it the empty state, centred, under
    /// the key introduction while it is open.
    fn frame(&self, introduction: &KeyIntroduction) -> Buffer {
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
        let height = Constraint::Length(self.lines.len() as u16);
        Paragraph::new(self.lines.clone())
            .centered()
            .render(below.centered_vertically(height), &mut frame);
        introduction.render(below, &mut frame);
        frame
    }
}
