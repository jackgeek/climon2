//! What the monitor draws on its own alternate screen, through Ratatui.
use climon2_core::{keys::ControlKey, protocol::TerminalSize};
use ratatui::{
    Frame, Terminal, TerminalOptions, Viewport,
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Position, Rect, Size},
    style::Stylize,
    text::Line,
    widgets::Paragraph,
};
use std::io::{self, Stdout};

/// The permanent top row's height.
const TOP_ROW: u16 = 1;
const NO_SELECTION: &str = "No session selected";
const EMPTY: &str = "No sessions to show.";

type Host = Terminal<CrosstermBackend<Stdout>>;

/// Ratatui writing to the monitor-owned host output over a fixed area, so it
/// never asks the host for its size or cursor.
pub(crate) struct Screen {
    terminal: Host,
    lines: [Line<'static>; 2],
}

impl Screen {
    pub(crate) fn new(size: TerminalSize, prefix: ControlKey) -> io::Result<Self> {
        let mut screen = Self {
            terminal: fixed(size)?,
            lines: [
                Line::from(EMPTY),
                Line::from(format!("Press {prefix} then x to exit.")),
            ],
        };
        screen.draw()?;
        Ok(screen)
    }

    /// A resized host keeps nothing of the old frame: clear it, then draw the
    /// whole frame at the new size.
    pub(crate) fn resize(&mut self, size: TerminalSize) -> io::Result<()> {
        self.terminal = fixed(size)?;
        self.terminal.backend_mut().clear()?;
        self.draw()
    }

    fn draw(&mut self) -> io::Result<()> {
        #[cfg(feature = "test-hooks")]
        crate::hooks::fail("draw")?;
        self.terminal.draw(|frame| paint(frame, &self.lines))?;
        Ok(())
    }
}

fn fixed(size: TerminalSize) -> io::Result<Host> {
    let area = Rect::from((Position::ORIGIN, Size::new(size.cols, size.rows)));
    Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )
}

/// The permanent top row, and below it the empty state, centred.
fn paint(frame: &mut Frame, empty: &[Line<'static>; 2]) {
    let area = frame.area();
    let top = Rect {
        height: TOP_ROW,
        ..area
    };
    let below = Rect {
        y: top.bottom(),
        height: area.height - TOP_ROW,
        ..area
    };
    frame.render_widget(Paragraph::new(NO_SELECTION).reversed(), top);
    let height = Constraint::Length(empty.len() as u16);
    frame.render_widget(
        Paragraph::new(empty.to_vec()).centered(),
        below.centered_vertically(height),
    );
}
