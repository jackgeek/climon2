//! The basic selector (selector document, "Selector"; #106): prefix+s
//! overlays a captionless double-bordered list of every row, with status and
//! title columns and a distinct highlight. Up and Down move the highlight
//! without wrapping, and Escape closes. Opening it never resizes an
//! application or acquires anything. Search, its separator, the footer and
//! Enter arrive with #108 and #113.
use crate::registry::{View, clip};
use climon2_core::SessionId;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::{Block, BorderType, Clear, Widget},
};
use unicode_width::UnicodeWidthStr;

/// A key the selector acts on, decoded from typed bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Move {
    Up,
    Down,
    Close,
}

/// Where typed bytes stand in an escape sequence.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Sequence {
    #[default]
    Ground,
    Escape,
    /// After ESC [ or ESC O: arrow keys end here.
    Introduced,
    /// Any other control sequence, skipped to its final byte.
    Skipping,
}

#[derive(Default)]
pub(crate) struct Selector {
    open: bool,
    /// The highlighted row's identity, kept while rows reorder.
    highlight: Option<SessionId>,
    sequence: Sequence,
}

impl Selector {
    /// Opens with the first row highlighted; opening an open selector is a
    /// no-op.
    pub(crate) fn open(&mut self, rows: &[View]) {
        if self.open {
            return;
        }
        self.open = true;
        self.sequence = Sequence::Ground;
        self.highlight = rows.first().map(|row| row.id.clone());
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// Decodes one typed byte, acting on any key it completes.
    pub(crate) fn key(&mut self, byte: u8, rows: &[View]) {
        if let Some(found) = self.decode(byte) {
            self.act(found, rows);
        }
    }

    /// The end of one host read: an Escape with nothing after it in the same
    /// read is a bare Escape. The full input router (#109) replaces this.
    pub(crate) fn read_ended(&mut self, rows: &[View]) {
        if self.sequence == Sequence::Escape {
            self.sequence = Sequence::Ground;
            self.act(Move::Close, rows);
        }
    }

    fn decode(&mut self, byte: u8) -> Option<Move> {
        let (next, found) = match (self.sequence, byte) {
            (_, 0x1b) => (Sequence::Escape, None),
            (Sequence::Escape, b'[' | b'O') => (Sequence::Introduced, None),
            (Sequence::Introduced, b'A') => (Sequence::Ground, Some(Move::Up)),
            (Sequence::Introduced, b'B') => (Sequence::Ground, Some(Move::Down)),
            (Sequence::Introduced | Sequence::Skipping, 0x40..=0x7e) => (Sequence::Ground, None),
            (Sequence::Introduced | Sequence::Skipping, _) => (Sequence::Skipping, None),
            (Sequence::Escape | Sequence::Ground, _) => (Sequence::Ground, None),
        };
        self.sequence = next;
        found
    }

    fn act(&mut self, found: Move, rows: &[View]) {
        let index = self
            .highlight
            .as_ref()
            .and_then(|id| rows.iter().position(|row| &row.id == id));
        let target = match (found, index) {
            (Move::Close, _) => {
                self.open = false;
                return;
            }
            (_, None) => rows.first(),
            (Move::Up, Some(index)) => rows.get(index.saturating_sub(1)),
            (Move::Down, Some(index)) => rows.get(index + 1).or(rows.get(index)),
        };
        self.highlight = target.map(|row| row.id.clone());
    }

    /// Draws the open selector over `area`, one cell in from each edge: no
    /// caption, a status column as wide as its widest status, a space, then
    /// each label clipped to the cells left. The highlighted row is reversed
    /// and kept in view.
    pub(crate) fn render(&self, rows: &[View], area: Rect, buffer: &mut Buffer) {
        if !self.open || area.width < 3 || area.height < 3 {
            return;
        }
        let outer = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width - 2,
            height: area.height - 2,
        };
        Clear.render(outer, buffer);
        let block = Block::bordered().border_type(BorderType::Double);
        let inner = block.inner(outer);
        block.render(outer, buffer);
        let status = rows.iter().map(|row| row.status.width()).max().unwrap_or(0);
        let visible = usize::from(inner.height);
        let highlighted = self
            .highlight
            .as_ref()
            .and_then(|id| rows.iter().position(|row| &row.id == id));
        let first = highlighted.map_or(0, |index| (index + 1).saturating_sub(visible));
        let width = usize::from(inner.width);
        for (line, (index, row)) in rows
            .iter()
            .enumerate()
            .skip(first)
            .take(visible)
            .enumerate()
        {
            let text = format!("{:status$} {}", row.status, row.label);
            let y = inner.y + line as u16;
            let style = if Some(index) == highlighted {
                Style::new().reversed()
            } else {
                Style::new()
            };
            let clipped = clip(&text, width);
            buffer.set_style(Rect::new(inner.x, y, inner.width, 1), style);
            buffer.set_string(inner.x, y, clipped, style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn rows(labels: &[(&'static str, &str)]) -> Vec<View> {
        labels
            .iter()
            .enumerate()
            .map(|(index, (status, label))| View {
                id: format!("00000000-0000-4000-8000-0000000000{index:02x}")
                    .parse()
                    .unwrap(),
                status,
                label: (*label).to_owned(),
                since: SystemTime::UNIX_EPOCH,
            })
            .collect()
    }

    fn lines(buffer: &Buffer) -> Vec<String> {
        let area = buffer.area;
        (0..area.height)
            .map(|y| {
                let mut line = String::new();
                let mut skip = 0;
                for x in 0..area.width {
                    if skip > 0 {
                        skip -= 1;
                        continue;
                    }
                    let symbol = buffer[(x, y)].symbol();
                    skip = symbol.width().saturating_sub(1);
                    line.push_str(symbol);
                }
                line
            })
            .collect()
    }

    fn highlighted(buffer: &Buffer) -> Vec<u16> {
        (0..buffer.area.height)
            .filter(|&y| {
                buffer[(2, y)]
                    .modifier
                    .contains(ratatui::style::Modifier::REVERSED)
            })
            .collect()
    }

    fn typed(selector: &mut Selector, bytes: &[u8], rows: &[View]) {
        for &byte in bytes {
            selector.key(byte, rows);
        }
        selector.read_ended(rows);
    }

    /// SEL-01 basic cases (pure supplement): a captionless double border one
    /// cell in, the status column as wide as its widest status, literal
    /// labels clipped by display cell, and the first row highlighted.
    #[test]
    fn the_selector_draws_a_captionless_double_border_with_two_columns() {
        let rows = rows(&[("Starting", "Alpha"), ("This monitor", "日本語ラベル")]);
        let mut selector = Selector::default();
        selector.open(&rows);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 24, 6));
        selector.render(&rows, buffer.area, &mut buffer);
        assert_eq!(
            lines(&buffer),
            [
                "                        ",
                " ╔════════════════════╗ ",
                " ║Starting     Alpha  ║ ",
                " ║This monitor 日本語 ║ ",
                " ╚════════════════════╝ ",
                "                        ",
            ]
        );
        assert_eq!(highlighted(&buffer), [2]);
    }

    /// SEL-01 basic cases (pure supplement): Up and Down move the highlight
    /// without wrapping, keeping it in view; Escape at the end of a read
    /// closes, while arrow sequences never do; reopening highlights the first
    /// row again.
    #[test]
    fn up_and_down_move_without_wrapping_and_escape_closes() {
        let rows = rows(&[("", "a"), ("", "b"), ("", "c")]);
        let mut selector = Selector::default();
        selector.open(&rows);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 5));
        let view = |selector: &Selector| {
            let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 5));
            selector.render(&rows, buffer.area, &mut buffer);
            (lines(&buffer)[2].clone(), highlighted(&buffer))
        };
        typed(&mut selector, b"\x1b[A", &rows);
        assert_eq!(view(&selector), (" ║ a    ║ ".to_owned(), vec![2]));
        typed(&mut selector, b"\x1b[B\x1bOB\x1b[B\x1b[B", &rows);
        assert_eq!(view(&selector), (" ║ c    ║ ".to_owned(), vec![2]));
        typed(&mut selector, b"\x1b[A", &rows);
        assert_eq!(view(&selector), (" ║ b    ║ ".to_owned(), vec![2]));
        assert!(selector.is_open());
        typed(&mut selector, b"\x1b", &rows);
        assert!(!selector.is_open());
        selector.render(&rows, buffer.area, &mut buffer);
        assert!(lines(&buffer).iter().all(|line| line.trim().is_empty()));
        selector.open(&rows);
        assert_eq!(view(&selector), (" ║ a    ║ ".to_owned(), vec![2]));
    }
}
