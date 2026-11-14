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
use std::ops::RangeInclusive;
use unicode_width::UnicodeWidthStr;

/// A key the selector acts on, decoded from typed bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Move {
    Up,
    Down,
    Close,
    /// Enter: choose the highlighted row.
    Choose,
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

    pub(crate) fn close(&mut self) {
        self.open = false;
    }

    /// Decodes one typed byte, acting on any key it completes; Enter returns
    /// the highlighted row's identity for the caller to select.
    pub(crate) fn key(&mut self, byte: u8, rows: &[View]) -> Option<SessionId> {
        match self.decode(byte)? {
            Move::Choose => self.highlighted(rows).map(|index| rows[index].id.clone()),
            found => {
                self.act(found, rows);
                None
            }
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
        let rule = RULES
            .iter()
            .find(|rule| rule.from.contains(&self.sequence) && rule.bytes.contains(&byte))
            .expect("the last rules cover every byte in every state");
        self.sequence = rule.next;
        rule.found
    }

    /// The highlighted row's index in `rows`, if it is there.
    fn highlighted(&self, rows: &[View]) -> Option<usize> {
        let id = self.highlight.as_ref()?;
        rows.iter().position(|row| &row.id == id)
    }

    fn act(&mut self, found: Move, rows: &[View]) {
        let index = self.highlighted(rows);
        let target = match (found, index) {
            (Move::Close | Move::Choose, _) => {
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
        let Some(outer) = self.frame(area) else {
            return;
        };
        Clear.render(outer, buffer);
        let block = Block::bordered().border_type(BorderType::Double);
        let inner = block.inner(outer);
        block.render(outer, buffer);
        let status = rows.iter().map(|row| row.status.width()).max().unwrap_or(0);
        let visible = usize::from(inner.height);
        let highlighted = self.highlighted(rows);
        let first = highlighted.map_or(0, |index| (index + 1).saturating_sub(visible));
        let shown = rows.iter().enumerate().skip(first).take(visible);
        for (line, (index, row)) in (0..).zip(shown) {
            let text = format!("{:status$} {}", row.status, row.label);
            let area = Rect::new(inner.x, inner.y + line, inner.width, 1);
            line_at(buffer, area, &text, Some(index) == highlighted);
        }
    }

    /// The selector's border, one cell in from each edge of `area`, while
    /// open and when it fits.
    fn frame(&self, area: Rect) -> Option<Rect> {
        let fits = area.width >= 3 && area.height >= 3;
        (self.open && fits).then(|| Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width - 2,
            height: area.height - 2,
        })
    }
}

/// One row of the list in `area`, clipped by display cell, reversed when
/// highlighted.
fn line_at(buffer: &mut Buffer, area: Rect, text: &str, highlighted: bool) {
    let style = if highlighted {
        Style::new().reversed()
    } else {
        Style::new()
    };
    buffer.set_style(area, style);
    buffer.set_string(area.x, area.y, clip(text, usize::from(area.width)), style);
}

/// One transition of the escape decoder: in a `from` state, a byte in
/// `bytes` moves to `next`, completing `found`. The first matching rule
/// applies.
struct Rule {
    from: &'static [Sequence],
    bytes: RangeInclusive<u8>,
    next: Sequence,
    found: Option<Move>,
}

const ANY: &[Sequence] = &[
    Sequence::Ground,
    Sequence::Escape,
    Sequence::Introduced,
    Sequence::Skipping,
];
const SEQUENCE: &[Sequence] = &[Sequence::Introduced, Sequence::Skipping];

const RULES: [Rule; 9] = [
    rule(ANY, 0x1b..=0x1b, Sequence::Escape, None),
    rule(&[Sequence::Escape], b'['..=b'[', Sequence::Introduced, None),
    rule(&[Sequence::Escape], b'O'..=b'O', Sequence::Introduced, None),
    rule(
        &[Sequence::Introduced],
        b'A'..=b'A',
        Sequence::Ground,
        Some(Move::Up),
    ),
    rule(
        &[Sequence::Introduced],
        b'B'..=b'B',
        Sequence::Ground,
        Some(Move::Down),
    ),
    rule(
        &[Sequence::Ground],
        b'\r'..=b'\r',
        Sequence::Ground,
        Some(Move::Choose),
    ),
    // A final byte ends any other control sequence.
    rule(SEQUENCE, 0x40..=0x7e, Sequence::Ground, None),
    rule(SEQUENCE, 0x00..=0xff, Sequence::Skipping, None),
    rule(ANY, 0x00..=0xff, Sequence::Ground, None),
];

const fn rule(
    from: &'static [Sequence],
    bytes: RangeInclusive<u8>,
    next: Sequence,
    found: Option<Move>,
) -> Rule {
    Rule {
        from,
        bytes,
        next,
        found,
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

    /// SEL-01 basic cases (pure supplement): prefix+s while the selector is
    /// open changes nothing: the highlight stays where it was moved.
    #[test]
    fn opening_an_open_selector_keeps_its_highlight() {
        let rows = rows(&[("", "a"), ("", "b"), ("", "c")]);
        let mut selector = Selector::default();
        selector.open(&rows);
        typed(&mut selector, b"\x1b[B", &rows);
        selector.open(&rows);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 7));
        selector.render(&rows, buffer.area, &mut buffer);
        assert_eq!(highlighted(&buffer), [3]);
        assert!(selector.is_open());
    }

    /// Spec #73 Enter (pure supplement, #108): a typed CR chooses the
    /// highlighted row, leaving the selector to its caller; no other key
    /// chooses, a CR inside an escape sequence is no Enter, and with no
    /// rows nothing is chosen.
    #[test]
    fn enter_chooses_the_highlighted_row() {
        let rows = rows(&[("", "A"), ("", "B")]);
        let mut selector = Selector::default();
        selector.open(&rows);
        let chosen: Vec<_> = b"\x1b[B\x1b\r\r"
            .iter()
            .map(|&byte| selector.key(byte, &rows))
            .collect();
        let none = [None, None, None, None, None];
        assert_eq!(chosen[..5], none);
        assert_eq!(chosen[5], Some(rows[1].id.clone()));
        assert!(selector.is_open());
        selector.close();
        assert!(!selector.is_open());
        let mut empty = Selector::default();
        empty.open(&[]);
        assert_eq!(empty.key(b'\r', &[]), None);
    }
}
