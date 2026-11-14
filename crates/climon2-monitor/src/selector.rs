//! The basic selector (selector document, "Selector"; #106): prefix+s
//! overlays a captionless double-bordered list of every row, with status and
//! title columns and a distinct highlight. Up and Down move the highlight
//! without wrapping, Escape closes and Enter chooses (#108). Its keys come
//! from the input router (#109). Opening it never resizes an application or
//! acquires anything. Search, its separator and the footer arrive with #113.
use crate::{
    registry::{View, clip},
    router::Key,
};
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
    /// Enter: choose the highlighted row.
    Choose,
}

#[derive(Default)]
pub(crate) struct Selector {
    open: bool,
    /// The highlighted row's identity, kept while rows reorder.
    highlight: Option<SessionId>,
}

impl Selector {
    /// Opens with the first row highlighted; opening an open selector is a
    /// no-op.
    pub(crate) fn open(&mut self, rows: &[View]) {
        if self.open {
            return;
        }
        self.open = true;
        self.highlight = rows.first().map(|row| row.id.clone());
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
    }

    /// Acts on one key; Enter returns the highlighted row's identity for the
    /// caller to select.
    pub(crate) fn key(&mut self, key: &Key, rows: &[View]) -> Option<SessionId> {
        match decode(key)? {
            Move::Choose => self.highlighted(rows).map(|index| rows[index].id.clone()),
            found => {
                self.act(found, rows);
                None
            }
        }
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

/// The selector's move for `key`, if it has one: a bare Escape, the Up and
/// Down cursor keys, or Enter.
fn decode(key: &Key) -> Option<Move> {
    match key {
        Key::Escape => Some(Move::Close),
        Key::Cursor(last) => vertical(*last),
        Key::Bytes(bytes) if bytes == b"\r" => Some(Move::Choose),
        _ => None,
    }
}

/// Up or Down, by the cursor key's final byte.
fn vertical(last: u8) -> Option<Move> {
    match last {
        b'A' => Some(Move::Up),
        b'B' => Some(Move::Down),
        _ => None,
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

    const UP: Key = Key::Cursor(b'A');
    const DOWN: Key = Key::Cursor(b'B');

    fn typed(selector: &mut Selector, keys: &[Key], rows: &[View]) {
        for key in keys {
            selector.key(key, rows);
        }
    }

    fn enter() -> Key {
        Key::Bytes(b"\r".to_vec())
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
    /// without wrapping, keeping it in view; Escape closes, while other keys
    /// (Left, a literal Escape byte, an SS3 keypad key) never do; reopening
    /// highlights the first row again.
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
        typed(&mut selector, &[UP], &rows);
        assert_eq!(view(&selector), (" ║ a    ║ ".to_owned(), vec![2]));
        typed(&mut selector, &[DOWN, DOWN, DOWN, DOWN], &rows);
        assert_eq!(view(&selector), (" ║ c    ║ ".to_owned(), vec![2]));
        typed(&mut selector, &[UP], &rows);
        assert_eq!(view(&selector), (" ║ b    ║ ".to_owned(), vec![2]));
        typed(
            &mut selector,
            &[Key::Cursor(b'D'), Key::Bytes(vec![0x1b]), Key::Keypad(b'M')],
            &rows,
        );
        assert_eq!(view(&selector), (" ║ b    ║ ".to_owned(), vec![2]));
        assert!(selector.is_open());
        typed(&mut selector, &[Key::Escape], &rows);
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
        typed(&mut selector, &[DOWN], &rows);
        selector.open(&rows);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 7));
        selector.render(&rows, buffer.area, &mut buffer);
        assert_eq!(highlighted(&buffer), [3]);
        assert!(selector.is_open());
    }

    /// Spec #73 Enter (pure supplement, #108): a typed CR chooses the
    /// highlighted row, leaving the selector to its caller; no other key
    /// (a CR with more bytes, keypad Enter) chooses, and with no rows nothing
    /// is chosen.
    #[test]
    fn enter_chooses_the_highlighted_row() {
        let rows = rows(&[("", "A"), ("", "B")]);
        let mut selector = Selector::default();
        selector.open(&rows);
        let keys = [
            DOWN,
            Key::Bytes(b"\r\r".to_vec()),
            Key::Keypad(b'M'),
            enter(),
        ];
        let chosen: Vec<_> = keys.iter().map(|key| selector.key(key, &rows)).collect();
        assert_eq!(chosen[..3], [None, None, None]);
        assert_eq!(chosen[3], Some(rows[1].id.clone()));
        assert!(selector.is_open());
        selector.close();
        assert!(!selector.is_open());
        let mut empty = Selector::default();
        empty.open(&[]);
        assert_eq!(empty.key(&enter(), &[]), None);
    }
}
