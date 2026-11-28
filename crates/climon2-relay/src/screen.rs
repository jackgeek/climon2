//! The relay's screen: a status bar naming its state, a detail row under it
//! and a key bar on the last row. The dashboard's panes come later; this is
//! the least that says what is bound and how to stop.
use climon2_core::protocol::TerminalSize;
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::Widget,
};
use std::{fs::File, io, net::SocketAddr};

const LISTENING: &str = " Listening";
const KEYS: &str = " s Stop";

/// What the screen shows.
pub(crate) struct View {
    /// The address the listener is bound to.
    pub(crate) address: SocketAddr,
}

/// Draws `view` over the whole of `area`.
pub(crate) fn render(view: &View, area: Rect, buffer: &mut Buffer) {
    let [status, detail, _, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let bar = Style::new().fg(Color::Black).bg(Color::Green);
    buffer.set_style(status, bar);
    Line::styled(LISTENING, bar).render(status, buffer);
    Line::raw(format!(" {}", view.address)).render(detail, buffer);
    Line::raw(KEYS).render(keys, buffer);
}

fn area(size: TerminalSize) -> Rect {
    Rect::new(0, 0, size.cols, size.rows)
}

/// The relay's screen on its terminal's output.
pub(crate) struct Screen(Terminal<CrosstermBackend<File>>);

impl Screen {
    /// A screen of `size`, drawn to the process's terminal output.
    pub(crate) fn new(size: TerminalSize) -> io::Result<Self> {
        let backend = CrosstermBackend::new(climon2_platform::terminal::output()?);
        let options = TerminalOptions {
            viewport: Viewport::Fixed(area(size)),
        };
        Terminal::with_options(backend, options).map(Self)
    }

    pub(crate) fn draw(&mut self, view: &View) -> io::Result<()> {
        self.0
            .draw(|frame| render(view, frame.area(), frame.buffer_mut()))
            .map(|_| ())
    }

    /// The terminal is `size` now: everything is drawn again.
    pub(crate) fn resize(&mut self, size: TerminalSize) -> io::Result<()> {
        self.0.resize(area(size))?;
        self.0.clear()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn rows(view: &View, cols: u16, rows: u16) -> Vec<String> {
        let area = Rect::new(0, 0, cols, rows);
        let mut buffer = Buffer::empty(area);
        render(view, area, &mut buffer);
        (0..rows)
            .map(|row| {
                (0..cols)
                    .map(|col| buffer[(col, row)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    fn loopback() -> View {
        View {
            address: SocketAddr::from((Ipv4Addr::LOCALHOST, 7222)),
        }
    }

    #[test]
    fn the_screen_names_the_state_the_bound_address_and_the_stop_key() {
        assert_eq!(
            rows(&loopback(), 20, 5),
            [
                " Listening          ",
                " 127.0.0.1:7222     ",
                "                    ",
                "                    ",
                " s Stop             ",
            ]
        );
    }

    #[test]
    fn an_ipv6_address_is_shown_in_brackets_with_its_port() {
        let view = View {
            address: SocketAddr::from((Ipv6Addr::LOCALHOST, 41000)),
        };
        assert_eq!(rows(&view, 16, 3)[1], " [::1]:41000    ");
    }

    #[test]
    fn the_status_bar_is_coloured_across_the_whole_top_row_only() {
        let area = Rect::new(0, 0, 12, 4);
        let mut buffer = Buffer::empty(area);
        render(&loopback(), area, &mut buffer);
        for col in 0..12 {
            assert_eq!(buffer[(col, 0)].bg, Color::Green, "column {col}");
            assert_eq!(buffer[(col, 0)].fg, Color::Black, "column {col}");
            for row in 1..4 {
                assert_eq!(buffer[(col, row)].bg, Color::Reset, "{col},{row}");
            }
        }
    }

    #[test]
    fn a_terminal_too_short_for_every_row_keeps_the_status_bar() {
        assert_eq!(rows(&loopback(), 12, 1), [" Listening  "]);
    }

    #[test]
    fn a_screen_covers_its_whole_terminal() {
        let size = TerminalSize { cols: 80, rows: 24 };
        assert_eq!(area(size), Rect::new(0, 0, 80, 24));
    }
}
