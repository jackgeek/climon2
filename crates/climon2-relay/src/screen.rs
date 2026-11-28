//! The relay's screen: a status bar naming its state, a detail row under it,
//! the link invitation under that, and a key bar on the last row. The
//! dashboard's panes come later (#510); this is the least that says what is
//! bound, how to pair and how to stop.
use crate::{desk::Shown, keys};
use climon2_core::protocol::TerminalSize;
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::Widget,
};
use std::{fs::File, io, net::SocketAddr};

const LISTENING: &str = " Listening";

/// What the screen shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct View {
    /// The address the listener is bound to.
    pub(crate) address: SocketAddr,
    /// The link invitation.
    pub(crate) invitation: Shown,
}

/// The rows that show the invitation. A live one gives its PIN and the
/// listen address as a hint (P4); the hub monitor's user types the address
/// that reaches this relay from their side.
fn invitation(view: &View) -> Vec<String> {
    match &view.invitation {
        Shown::Nothing => Vec::new(),
        Shown::Live { pin } => vec![
            format!(" PIN {pin}"),
            format!(" Pair with: c2 monitor pair {}", view.address),
        ],
        Shown::Paired { name, fingerprint } => vec![
            format!(" Paired with {name}"),
            format!(" Key fingerprint {fingerprint}"),
        ],
        Shown::Ended(why) => vec![format!(" {why}")],
    }
}

/// Draws `view` over the whole of `area`: the status bar, the address and
/// the invitation from the top, and the key bar on the last row, which the
/// rows above never cover. A terminal of one row shows the status bar.
pub(crate) fn render(view: &View, area: Rect, buffer: &mut Buffer) {
    let bar = Style::new().fg(Color::Black).bg(Color::Green);
    let keyed = area.height > 1;
    let above = area.height - u16::from(keyed);
    let row = |offset: u16| Rect::new(area.x, area.y + offset, area.width, 1);
    buffer.set_style(row(0).intersection(area), bar);
    let mut lines = vec![
        Line::styled(LISTENING, bar),
        Line::raw(format!(" {}", view.address)),
        Line::raw(""),
    ];
    lines.extend(invitation(view).into_iter().map(Line::raw));
    for (offset, line) in (0..above).zip(lines) {
        line.render(row(offset), buffer);
    }
    if keyed {
        Line::raw(keys::BAR).render(row(area.height - 1), buffer);
    }
}

fn area(size: TerminalSize) -> Rect {
    Rect::new(0, 0, size.cols, size.rows)
}

/// The relay's screen on its terminal's output.
pub(crate) struct Screen(Terminal<CrosstermBackend<File>>);

impl Screen {
    /// A screen of `size` on the process's terminal output, showing `view`.
    pub(crate) fn drawn(size: TerminalSize, view: &View) -> io::Result<Self> {
        let backend = CrosstermBackend::new(climon2_platform::terminal::output()?);
        let options = TerminalOptions {
            viewport: Viewport::Fixed(area(size)),
        };
        let mut screen = Terminal::with_options(backend, options).map(Self)?;
        screen.draw(view)?;
        Ok(screen)
    }

    /// Shows `view`.
    pub(crate) fn draw(&mut self, view: &View) -> io::Result<()> {
        self.0
            .draw(|frame| render(view, frame.area(), frame.buffer_mut()))
            .map(|_| ())
    }

    /// The terminal is `size` now: everything is drawn again.
    pub(crate) fn redraw(&mut self, size: TerminalSize, view: &View) -> io::Result<()> {
        self.0.resize(area(size))?;
        self.0.clear()?;
        self.draw(view)
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
            invitation: Shown::Nothing,
        }
    }

    fn showing(invitation: Shown) -> View {
        View {
            invitation,
            ..loopback()
        }
    }

    fn trimmed(view: &View) -> Vec<String> {
        let rows = rows(view, 60, 8);
        rows.iter().map(|row| row.trim_end().to_owned()).collect()
    }

    #[test]
    fn the_screen_names_the_state_the_bound_address_and_the_keys() {
        assert_eq!(
            rows(&loopback(), 20, 5),
            [
                " Listening          ",
                " 127.0.0.1:7222     ",
                "                    ",
                "                    ",
                " p Pair  s Stop     ",
            ]
        );
    }

    /// P4-5, X6: a live invitation shows its PIN and the listen address.
    #[test]
    fn a_live_invitation_shows_its_pin_and_the_address_to_pair_with() {
        let view = showing(Shown::Live {
            pin: "1234-5678".into(),
        });
        assert_eq!(
            trimmed(&view),
            [
                " Listening",
                " 127.0.0.1:7222",
                "",
                " PIN 1234-5678",
                " Pair with: c2 monitor pair 127.0.0.1:7222",
                "",
                "",
                " p Pair  s Stop",
            ]
        );
    }

    /// P23: a pairing shows the hub monitor's name and key fingerprint.
    #[test]
    fn a_pairing_shows_the_hub_monitors_name_and_fingerprint() {
        let view = showing(Shown::Paired {
            name: "laptop".into(),
            fingerprint: "630d:cd29:66c4:3366:9112:5448:bbb2:5b4f".into(),
        });
        let rows = trimmed(&view);
        assert_eq!(rows[3], " Paired with laptop");
        assert_eq!(
            rows[4],
            " Key fingerprint 630d:cd29:66c4:3366:9112:5448:bbb2:5b4f"
        );
    }

    /// P8: an invitation that ended unused says how.
    #[test]
    fn an_ended_invitation_says_how() {
        let rows = trimmed(&showing(Shown::Ended("The invitation expired")));
        assert_eq!(rows[3], " The invitation expired");
        assert_eq!(rows[4], "");
    }

    #[test]
    fn an_ipv6_address_is_shown_in_brackets_with_its_port() {
        let view = View {
            address: SocketAddr::from((Ipv6Addr::LOCALHOST, 41000)),
            invitation: Shown::Nothing,
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
