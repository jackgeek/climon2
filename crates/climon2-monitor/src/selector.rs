//! The selector (selector document, "Selector"; #106, #113): prefix+s
//! overlays a captionless double-bordered box, one cell in from each edge
//! below the top row, without resizing any application. Its first line is
//! the always-focused search; a single separator joins the side borders with
//! U+255F and U+2562; below it the rows that match the search list in row
//! order, never by score, as borderless status and label columns with a
//! distinct reversed highlight; and, with key hints on, a footer names the
//! select, cancel and help bindings. Its keys come from the input router
//! (#109): typing, Backspace, Left, Right, Home and End edit the search, Up,
//! Down, PageUp and PageDown move the highlight without wrapping, Enter
//! chooses the highlighted row (#108) and Escape cancels. A bracketed paste
//! fills the search as one line. A click on a row highlights it, a click
//! outside the box cancels (owner decision on #113), and the wheel scrolls
//! the list one row a notch.
use crate::{
    mouse::{Gesture, Mouse},
    paint::HostCursor,
    registry::{View, clip},
    router::{Key, Paste, Routed},
    search::{Edit, Search},
};
use climon2_core::{SessionId, keys::ControlKey};
use ratatui::{
    buffer::Buffer,
    layout::{Margin, Position, Rect},
    style::Style,
    widgets::{Block, BorderType, Clear, Widget},
};
use unicode_width::UnicodeWidthStr;

const ESC: u8 = 0x1b;
const ENTER: u8 = b'\r';

/// What a selector key does.
type Handler = fn(&mut Selector, &[View], Rect) -> Option<SessionId>;

/// A key the selector acts on.
enum Command {
    Run(Handler),
    /// One typed byte for the search.
    Type(u8),
}

/// The cursor keys the selector takes, by their final byte.
const CURSOR_KEYS: [(u8, Handler); 6] = [
    (b'A', Selector::up),
    (b'B', Selector::down),
    (b'C', Selector::right),
    (b'D', Selector::left),
    (b'H', Selector::home),
    (b'F', Selector::end),
];

/// The other keys the selector takes, by their bytes: Enter, Backspace
/// (DEL or BS), PageUp, PageDown, and Home and End in their `CSI ~` forms.
const KEYS: [(&[u8], Handler); 7] = [
    (b"\r", Selector::choose),
    (b"\x7f", Selector::backspace),
    (b"\x08", Selector::backspace),
    (b"\x1b[5~", Selector::page_up),
    (b"\x1b[6~", Selector::page_down),
    (b"\x1b[1~", Selector::home),
    (b"\x1b[4~", Selector::end),
];

pub(crate) struct Selector {
    open: bool,
    search: Search,
    /// The highlighted row's identity, kept while rows reorder.
    highlight: Option<SessionId>,
    /// The first matching row in view, as far as the list allows.
    top: usize,
    /// The footer's key hints, unless key hints are off.
    hints: Option<String>,
    /// The latest search edit was refused for its length.
    refused: bool,
}

/// Where the open selector's parts are.
struct Layout {
    /// The border.
    outer: Rect,
    search: Rect,
    /// The separator's row, unless the box has no room for it.
    separator: Rect,
    list: Rect,
    /// The footer's row, while there is a footer and room for it.
    footer: Rect,
}

impl Selector {
    /// A closed selector whose footer names `prefix`'s bindings while
    /// `hints` is on, and whose search holds at most `limit` bytes.
    pub(crate) fn new(prefix: ControlKey, hints: bool, limit: usize) -> Self {
        Self::closed(Search::new(limit), hints.then(|| key_hints(prefix)))
    }

    /// A closed selector with `search` and the footer's `hints`, its list
    /// scrolled to the start, nothing highlighted and no refusal shown:
    /// how [`Selector::new`] makes it and [`Selector::open`] starts again.
    fn closed(search: Search, hints: Option<String>) -> Self {
        Self {
            open: false,
            search,
            highlight: None,
            top: 0,
            hints,
            refused: false,
        }
    }

    /// Opens with an empty search, highlighting `selected` when it is
    /// listed and otherwise the first row; opening an open selector is a
    /// no-op.
    pub(crate) fn open(&mut self, rows: &[View], selected: Option<&SessionId>, area: Rect) {
        if self.open {
            return;
        }
        let search = Search::new(self.search.limit());
        *self = Self {
            open: true,
            ..Self::closed(search, self.hints.take())
        };
        let index = selected
            .and_then(|id| rows.iter().position(|row| &row.id == id))
            .unwrap_or(0);
        self.highlight = rows.get(index).map(|row| row.id.clone());
        self.reveal(index, area);
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
    }

    /// Acts on input for the selector drawn in `area`: keys, pastes and
    /// mouse reports. Enter returns the highlighted row's identity for the
    /// caller to select.
    pub(crate) fn input(&mut self, input: &Routed, rows: &[View], area: Rect) -> Option<SessionId> {
        match input {
            Routed::Key(key) => self.key(key, rows, area),
            Routed::Paste(paste) => self.paste(paste, rows, area),
            Routed::Mouse(mouse) => self.mouse(mouse, rows, area),
            Routed::Action(_) | Routed::Unknown(..) | Routed::Rest(_) => None,
        }
    }

    fn key(&mut self, key: &Key, rows: &[View], area: Rect) -> Option<SessionId> {
        self.refused = false;
        match decode(key)? {
            Command::Run(handler) => handler(self, rows, area),
            Command::Type(byte) => {
                let edit = self.search.typed(byte);
                self.edited(edit, rows, area)
            }
        }
    }

    /// A bracketed paste fills the search as one line once it ends.
    fn paste(&mut self, paste: &Paste, rows: &[View], area: Rect) -> Option<SessionId> {
        match paste {
            Paste::Start => self.search.paste_start(),
            Paste::Data(bytes) => self.search.paste_data(bytes),
            Paste::End => {
                self.refused = false;
                let edit = self.search.paste_end();
                return self.edited(edit, rows, area);
            }
        }
        None
    }

    /// A click on a row highlights it and one outside the box cancels; the
    /// wheel scrolls the list.
    fn mouse(&mut self, mouse: &Mouse, rows: &[View], area: Rect) -> Option<SessionId> {
        match mouse.gesture() {
            Gesture::Click => self.click(mouse, rows, area),
            Gesture::WheelUp => self.scroll(rows, area, back),
            Gesture::WheelDown => self.scroll(rows, area, forth),
            Gesture::Other => {}
        }
        None
    }

    /// After rows changed while open: a highlighted row that is gone or no
    /// longer matches loses the highlight for good.
    pub(crate) fn update(&mut self, rows: &[View]) {
        let matching = self.matching(rows);
        self.highlight = self
            .index(&matching)
            .map(|index| matching[index].id.clone());
    }

    /// The rows the search matches, in row order.
    fn matching<'a>(&self, rows: &'a [View]) -> Vec<&'a View> {
        rows.iter().filter(|row| self.search.matches(row)).collect()
    }

    /// The highlighted row's index in `matching`, if it is there.
    fn index(&self, matching: &[&View]) -> Option<usize> {
        let id = self.highlight.as_ref()?;
        matching.iter().position(|row| &row.id == id)
    }

    fn choose(&mut self, rows: &[View], _: Rect) -> Option<SessionId> {
        let matching = self.matching(rows);
        self.index(&matching)
            .map(|index| matching[index].id.clone())
    }

    fn cancel(&mut self, _: &[View], _: Rect) -> Option<SessionId> {
        self.open = false;
        None
    }

    fn up(&mut self, rows: &[View], area: Rect) -> Option<SessionId> {
        self.navigate(rows, area, back, 1)
    }

    fn down(&mut self, rows: &[View], area: Rect) -> Option<SessionId> {
        self.navigate(rows, area, forth, 1)
    }

    fn page_up(&mut self, rows: &[View], area: Rect) -> Option<SessionId> {
        self.navigate(rows, area, back, visible(&self.parts(area)))
    }

    fn page_down(&mut self, rows: &[View], area: Rect) -> Option<SessionId> {
        self.navigate(rows, area, forth, visible(&self.parts(area)))
    }

    /// Moves the highlight `by` rows with `step`, stopping at either end,
    /// and keeps it in view; with nothing highlighted, highlights the first
    /// matching row.
    fn navigate(&mut self, rows: &[View], area: Rect, step: Step, by: usize) -> Option<SessionId> {
        let matching = self.matching(rows);
        let last = matching.len().checked_sub(1)?;
        let target = self
            .index(&matching)
            .map_or(0, |index| step(index, by, last));
        self.highlight = Some(matching[target].id.clone());
        self.reveal(target, area);
        None
    }

    fn backspace(&mut self, rows: &[View], area: Rect) -> Option<SessionId> {
        self.edit(Search::backspace, rows, area)
    }

    fn left(&mut self, rows: &[View], area: Rect) -> Option<SessionId> {
        self.edit(Search::left, rows, area)
    }

    fn right(&mut self, rows: &[View], area: Rect) -> Option<SessionId> {
        self.edit(Search::right, rows, area)
    }

    fn home(&mut self, rows: &[View], area: Rect) -> Option<SessionId> {
        self.edit(Search::home, rows, area)
    }

    fn end(&mut self, rows: &[View], area: Rect) -> Option<SessionId> {
        self.edit(Search::end, rows, area)
    }

    fn edit(
        &mut self,
        edit: fn(&mut Search) -> Edit,
        rows: &[View],
        area: Rect,
    ) -> Option<SessionId> {
        let edit = edit(&mut self.search);
        self.edited(edit, rows, area)
    }

    /// A changed search keeps a highlight that still matches, otherwise
    /// highlights the first match; a refused edit shows why.
    fn edited(&mut self, edit: Edit, rows: &[View], area: Rect) -> Option<SessionId> {
        match edit {
            Edit::Changed => self.refilter(rows, area),
            Edit::Refused => self.refused = true,
            Edit::Unchanged => {}
        }
        None
    }

    fn refilter(&mut self, rows: &[View], area: Rect) {
        let matching = self.matching(rows);
        let index = self.index(&matching).unwrap_or(0);
        self.highlight = matching.get(index).map(|row| row.id.clone());
        self.top = 0;
        self.reveal(index, area);
    }

    /// Scrolls the list so that matching row `index` is in view.
    fn reveal(&mut self, index: usize, area: Rect) {
        let visible = visible(&self.parts(area));
        self.top = self.top.min(index).max((index + 1).saturating_sub(visible));
    }

    /// The first of `count` matching rows in view: never so far down that
    /// rows could show and do not.
    fn first(&self, count: usize, visible: usize) -> usize {
        self.top.min(count.saturating_sub(visible))
    }

    fn click(&mut self, mouse: &Mouse, rows: &[View], area: Rect) {
        let at = Position::new(mouse.column, mouse.row);
        let inside = self.layout(area).filter(|layout| layout.outer.contains(at));
        let Some(list) = inside.map(|layout| layout.list) else {
            self.open = false;
            return;
        };
        let matching = self.matching(rows);
        let first = self.first(matching.len(), usize::from(list.height));
        let row = list
            .contains(at)
            .then(|| matching.get(first + usize::from(mouse.row - list.y)))
            .flatten();
        if let Some(row) = row {
            self.highlight = Some(row.id.clone());
        }
    }

    /// Scrolls the list one row with `step`, as far as it goes.
    fn scroll(&mut self, rows: &[View], area: Rect, step: Step) {
        let count = self.matching(rows).len();
        let visible = visible(&self.parts(area));
        self.top = step(self.first(count, visible), 1, count.saturating_sub(visible));
    }

    /// The selector's parts in `area`, while it is open.
    fn layout(&self, area: Rect) -> Option<Layout> {
        self.open.then(|| self.parts(area))
    }

    /// The selector's parts in `area`, one cell in from each edge: the
    /// border, the search line, the separator, the list and the footer, each
    /// empty where there is no room for it.
    fn parts(&self, area: Rect) -> Layout {
        let outer = area.inner(Margin::new(1, 1));
        let inner = outer.inner(Margin::new(1, 1));
        let below = inner.height.saturating_sub(2);
        let footer = u16::from(self.footer().is_some()).min(below);
        let list = Rect {
            y: inner.y + 2,
            height: below - footer,
            ..inner
        };
        Layout {
            outer,
            search: Rect {
                height: inner.height.min(1),
                ..inner
            },
            separator: Rect {
                y: inner.y + 1,
                height: u16::from(inner.height >= 2),
                ..outer
            },
            list,
            footer: Rect {
                y: list.bottom(),
                height: footer,
                ..inner
            },
        }
    }

    /// The footer's text: why the latest search edit was refused, otherwise
    /// the key hints, if they are on.
    fn footer(&self) -> Option<String> {
        let refused = format!("Search is limited to {} bytes", self.search.limit());
        self.refused
            .then_some(refused)
            .or_else(|| self.hints.clone())
    }

    /// Draws the open selector over `area`; returns the search cursor.
    pub(crate) fn render(
        &self,
        rows: &[View],
        area: Rect,
        buffer: &mut Buffer,
    ) -> Option<HostCursor> {
        let layout = self.layout(area)?;
        Clear.render(layout.outer, buffer);
        Block::bordered()
            .border_type(BorderType::Double)
            .render(layout.outer, buffer);
        separator(layout.separator, buffer);
        self.list(rows, layout.list, buffer);
        for row in layout.footer.rows() {
            text_at(
                buffer,
                row,
                &self.footer().unwrap_or_default(),
                Style::new(),
            );
        }
        self.search_line(layout.search, buffer)
    }

    /// The matching rows in view: a status column as wide as the widest
    /// status, a space, then the label; the highlighted row reversed across
    /// the list.
    fn list(&self, rows: &[View], area: Rect, buffer: &mut Buffer) {
        let status = rows
            .iter()
            .map(|row| row.status.width())
            .max()
            .unwrap_or_default();
        let matching = self.matching(rows);
        let highlighted = self.index(&matching);
        let first = self.first(matching.len(), usize::from(area.height));
        let shown = matching.iter().enumerate().skip(first);
        for (line, (index, row)) in area.rows().zip(shown) {
            let text = format!("{:status$} {}", row.status, row.label);
            let style =
                [Style::new(), Style::new().reversed()][usize::from(Some(index) == highlighted)];
            buffer.set_style(line, style);
            text_at(buffer, line, &text, style);
        }
    }

    /// The search text, scrolled to keep its cursor in view; returns the
    /// cursor, unless the line has no room.
    fn search_line(&self, area: Rect, buffer: &mut Buffer) -> Option<HostCursor> {
        if area.is_empty() {
            return None;
        }
        let (shown, cursor) = self.search.shown(usize::from(area.width));
        buffer.set_stringn(area.x, area.y, shown, usize::from(area.width), Style::new());
        Some(HostCursor {
            column: area.x + cursor as u16,
            row: area.y,
            style: 0,
        })
    }
}

/// Moves an index `by` rows, stopping at 0 or `last`.
type Step = fn(usize, usize, usize) -> usize;

fn back(index: usize, by: usize, _: usize) -> usize {
    index.saturating_sub(by)
}

fn forth(index: usize, by: usize, last: usize) -> usize {
    (index + by).min(last)
}

/// How many list rows `layout` shows, counting at least one.
fn visible(layout: &Layout) -> usize {
    usize::from(layout.list.height).max(1)
}

/// The footer's key hints with `prefix` (owner decision on #113: only
/// select, cancel and help).
fn key_hints(prefix: ControlKey) -> String {
    let cancel = ["", "  Esc: cancel"][usize::from(escape_cancels(prefix))];
    format!(
        "{}: select{cancel}  {prefix} then ?: help",
        select_key(prefix)
    )
}

/// The selector's keys as the key introduction lists them, under their own
/// heading (owner decision on #113: the footer names only select, cancel
/// and help, and help lists everything else).
pub(crate) fn help(prefix: ControlKey) -> Vec<String> {
    let mut lines: Vec<String> = [
        "In the session list:",
        "Type or paste  Search by status and title",
        "Left, Right, Home, End, Backspace  Edit the search",
        "Up, Down, PageUp, PageDown  Move the highlight",
    ]
    .map(str::to_owned)
    .to_vec();
    lines.push(format!(
        "{}  Select the highlighted session",
        select_key(prefix)
    ));
    lines.extend(escape_cancels(prefix).then(|| "Esc  Cancel".to_owned()));
    lines.extend(
        [
            "Click a row  Highlight it",
            "Click outside  Cancel",
            "Wheel  Scroll the list",
        ]
        .map(str::to_owned),
    );
    lines
}

/// What selects: Enter, or the prefix twice when Enter is the prefix.
fn select_key(prefix: ControlKey) -> String {
    if prefix.byte() == ENTER {
        format!("{prefix} then {prefix}")
    } else {
        "Enter".to_owned()
    }
}

/// Whether a bare Escape cancels: not when the prefix consumes it.
fn escape_cancels(prefix: ControlKey) -> bool {
    prefix.byte() != ESC
}

/// The separator across `area`'s width, on its row if it has one.
fn separator(area: Rect, buffer: &mut Buffer) {
    let line = format!(
        "╟{}╢",
        "─".repeat(usize::from(area.width.saturating_sub(2)))
    );
    for row in area.rows() {
        buffer.set_stringn(row.x, row.y, &line, usize::from(row.width), Style::new());
    }
}

/// `text` on the one-row `area`, ending in "…" when it does not fit
/// (owner decision on #112), never splitting a wide character.
fn text_at(buffer: &mut Buffer, area: Rect, text: &str, style: Style) {
    let cells = usize::from(area.width);
    let fitted = if clip(text, cells).len() == text.len() {
        text.to_owned()
    } else {
        format!("{}…", clip(text, cells.saturating_sub(1)))
    };
    buffer.set_stringn(area.x, area.y, fitted, cells, style);
}

/// The selector's command for `key`, if it has one: a bare Escape, a cursor
/// key or another key it takes, or one typed byte.
fn decode(key: &Key) -> Option<Command> {
    match key {
        Key::Escape => Some(Command::Run(Selector::cancel)),
        Key::Cursor(last) => found(&CURSOR_KEYS, last),
        Key::Bytes(bytes) => found(&KEYS, &bytes.as_slice()).or_else(|| typed(bytes)),
        Key::Keypad(_) => None,
    }
}

fn found<T: PartialEq>(keys: &[(T, Handler)], key: &T) -> Option<Command> {
    keys.iter()
        .find(|(bytes, _)| bytes == key)
        .map(|(_, handler)| Command::Run(*handler))
}

fn typed(bytes: &[u8]) -> Option<Command> {
    match bytes {
        [byte] => Some(Command::Type(*byte)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;
    use std::time::SystemTime;

    fn rows(labels: &[(&str, &str)]) -> Vec<View> {
        labels
            .iter()
            .enumerate()
            .map(|(index, (status, label))| View {
                id: format!("00000000-0000-4000-8000-0000000000{index:02x}")
                    .parse()
                    .unwrap(),
                status: (*status).to_owned(),
                label: (*label).to_owned(),
                since: SystemTime::UNIX_EPOCH,
            })
            .collect()
    }

    fn letters(count: u8) -> Vec<View> {
        let labels: Vec<String> = (b'a'..b'a' + count)
            .map(|letter| char::from(letter).to_string())
            .collect();
        let labels: Vec<_> = labels.iter().map(|label| ("", label.as_str())).collect();
        rows(&labels)
    }

    fn selector(prefix: &str, hints: bool) -> Selector {
        Selector::new(ControlKey::from_name(prefix).unwrap(), hints, 4096)
    }

    fn lines(buffer: &Buffer) -> Vec<String> {
        let area = buffer.area;
        (area.y..area.bottom())
            .map(|y| {
                let mut line = String::new();
                let mut skip = 0;
                for x in area.x..area.right() {
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

    /// The rows whose cells are reversed, by the cell just inside the
    /// border.
    fn highlighted(buffer: &Buffer) -> Vec<u16> {
        let area = buffer.area;
        let x = area.x + 2;
        (area.y..area.bottom())
            .filter(|&y| x < area.right() && buffer[(x, y)].modifier.contains(Modifier::REVERSED))
            .collect()
    }

    const UP: Key = Key::Cursor(b'A');
    const DOWN: Key = Key::Cursor(b'B');
    const LEFT: Key = Key::Cursor(b'D');

    fn bytes(bytes: &[u8]) -> Key {
        Key::Bytes(bytes.to_vec())
    }

    fn enter() -> Key {
        bytes(b"\r")
    }

    fn page_up() -> Key {
        bytes(b"\x1b[5~")
    }

    fn page_down() -> Key {
        bytes(b"\x1b[6~")
    }

    /// An area 12 cells wide whose list shows two rows without a footer.
    const SMALL: Rect = Rect::new(0, 0, 12, 8);

    fn typed(
        selector: &mut Selector,
        keys: &[Key],
        rows: &[View],
        area: Rect,
    ) -> Vec<Option<SessionId>> {
        keys.iter()
            .map(|key| selector.key(key, rows, area))
            .collect()
    }

    fn text(selector: &mut Selector, text: &str, rows: &[View]) {
        for byte in text.bytes() {
            selector.key(&Key::Bytes(vec![byte]), rows, SMALL);
        }
    }

    fn drawn(
        selector: &Selector,
        rows: &[View],
        area: Rect,
    ) -> (Vec<String>, Vec<u16>, Option<HostCursor>) {
        let mut buffer = Buffer::empty(area);
        let cursor = selector.render(rows, area, &mut buffer);
        (lines(&buffer), highlighted(&buffer), cursor)
    }

    /// The list rows of [`SMALL`], trimmed, and which of its two rows is
    /// highlighted.
    fn list(selector: &Selector, rows: &[View]) -> (Vec<String>, Vec<u16>) {
        let (lines, highlighted, _) = drawn(selector, rows, SMALL);
        let shown = lines[4..6]
            .iter()
            .map(|line| line.trim_matches([' ', '║']).to_owned())
            .collect();
        (shown, highlighted)
    }

    fn cursor(column: u16, row: u16) -> Option<HostCursor> {
        Some(HostCursor {
            column,
            row,
            style: 0,
        })
    }

    /// SEL-01 (pure supplement): a captionless double border one cell in;
    /// the empty search on its first line with the cursor at its start; a
    /// single separator joined with U+255F and U+2562; the status column as
    /// wide as the widest status, then literal labels, an over-long one
    /// ending in "…" without splitting a wide character (owner decision on
    /// #112); the first row highlighted across the list; with key hints off,
    /// no footer.
    #[test]
    fn the_selector_draws_search_separator_and_two_columns() {
        let rows = rows(&[("Starting", "Alpha"), ("Disconnected", "日本語ラベル")]);
        let mut selector = selector("Ctrl+]", false);
        selector.open(&rows, None, Rect::new(0, 0, 24, 8));
        let (lines, highlighted, cursor) = drawn(&selector, &rows, Rect::new(0, 0, 24, 8));
        assert_eq!(
            lines,
            [
                "                        ",
                " ╔════════════════════╗ ",
                " ║                    ║ ",
                " ╟────────────────────╢ ",
                " ║Starting     Alpha  ║ ",
                " ║Disconnected 日本語…║ ",
                " ╚════════════════════╝ ",
                "                        ",
            ]
        );
        assert_eq!(highlighted, [4]);
        assert_eq!(cursor, super::tests::cursor(2, 2));
    }

    /// SEL-01 (pure supplement, footer): with key hints on, the footer at the
    /// bottom inside the box names select, cancel and help with the actual
    /// prefix (owner decision on #113); it costs the list one row, which
    /// hints off reclaim. A prefix that consumes Escape is never advertised
    /// as cancelling, Enter as the prefix is named as typed twice, and a
    /// footer too long for the box ends in "…".
    #[test]
    fn the_footer_names_the_actual_bindings_and_its_row_is_reclaimed_without_hints() {
        let rows = letters(3);
        let area = Rect::new(0, 0, 51, 8);
        let footer = |prefix: &str, hints: bool, area: Rect| {
            let mut selector = selector(prefix, hints);
            selector.open(&rows, None, area);
            let (lines, ..) = drawn(&selector, &rows, area);
            lines[4..6]
                .iter()
                .map(|line| line.trim_matches([' ', '║']).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            footer("Ctrl+]", true, area),
            ["a", "Enter: select  Esc: cancel  Ctrl+] then ?: help"]
        );
        assert_eq!(footer("Ctrl+]", false, area), ["a", "b"]);
        assert_eq!(
            footer("Ctrl+[", true, area),
            ["a", "Enter: select  Ctrl+[ then ?: help"]
        );
        assert_eq!(
            footer("Ctrl+M", true, Rect::new(0, 0, 64, 8)),
            [
                "a",
                "Ctrl+M then Ctrl+M: select  Esc: cancel  Ctrl+M then ?: help"
            ]
        );
        assert_eq!(
            footer("Ctrl+]", true, Rect::new(0, 0, 50, 8)),
            ["a", "Enter: select  Esc: cancel  Ctrl+] then ?: he…"]
        );
    }

    /// HLP-02 and SEL-02 (pure supplement; owner decision on #113, "help
    /// lists everything else"): the key introduction lists the selector's
    /// keys under their own heading, naming Enter as the prefix twice when it
    /// is the prefix and leaving out Escape when the prefix consumes it.
    #[test]
    fn help_lists_every_selector_key_with_the_actual_prefix() {
        let help = |prefix: &str| help(ControlKey::from_name(prefix).unwrap());
        let lines = |select: &str, cancel: &[&str]| {
            let mut lines = vec![
                "In the session list:".to_owned(),
                "Type or paste  Search by status and title".to_owned(),
                "Left, Right, Home, End, Backspace  Edit the search".to_owned(),
                "Up, Down, PageUp, PageDown  Move the highlight".to_owned(),
                format!("{select}  Select the highlighted session"),
            ];
            lines.extend(cancel.iter().map(|line| (*line).to_owned()));
            lines.extend(
                [
                    "Click a row  Highlight it",
                    "Click outside  Cancel",
                    "Wheel  Scroll the list",
                ]
                .map(str::to_owned),
            );
            lines
        };
        assert_eq!(help("Ctrl+]"), lines("Enter", &["Esc  Cancel"]));
        assert_eq!(help("Ctrl+["), lines("Enter", &[]));
        assert_eq!(
            help("Ctrl+M"),
            lines("Ctrl+M then Ctrl+M", &["Esc  Cancel"])
        );
    }

    /// SEL-02 (pure supplement, navigation): Up and Down move the highlight
    /// one row and PageUp and PageDown a list's height, never wrapping and
    /// stopping at either end; the list scrolls to keep the highlight in
    /// view; keypad keys, Shift+Tab and other keys move nothing.
    #[test]
    fn navigation_moves_without_wrapping_and_keeps_the_highlight_in_view() {
        let rows = letters(5);
        let mut selector = selector("Ctrl+]", false);
        selector.open(&rows, None, SMALL);
        let steps: [(Key, [&str; 2], u16); 13] = [
            (UP, ["a", "b"], 4),
            (DOWN, ["a", "b"], 5),
            (DOWN, ["b", "c"], 5),
            (page_down(), ["d", "e"], 5),
            (page_down(), ["d", "e"], 5),
            (DOWN, ["d", "e"], 5),
            (page_up(), ["c", "d"], 4),
            (Key::Keypad(b'x'), ["c", "d"], 4),
            (bytes(b"\x1b[Z"), ["c", "d"], 4),
            (UP, ["b", "c"], 4),
            (DOWN, ["b", "c"], 5),
            (page_up(), ["a", "b"], 4),
            (page_up(), ["a", "b"], 4),
        ];
        for (key, shown, row) in steps {
            assert_eq!(selector.key(&key, &rows, SMALL), None);
            assert_eq!(
                list(&selector, &rows),
                (shown.map(str::to_owned).to_vec(), vec![row]),
                "{key:?}"
            );
        }
    }

    /// SEL-02 (pure supplement, editing and highlight): a search edit keeps
    /// a highlight that still matches, otherwise highlights the first match
    /// or, with none, nothing; rows never reorder by score; clearing the
    /// search never restores an older highlight; Enter chooses only a
    /// highlighted matching row; Tab and Left do not change the matches.
    #[test]
    fn search_edits_keep_a_matching_highlight_or_take_the_first_match() {
        let rows = rows(&[
            ("Idle", "alpha"),
            ("Idle", "beta"),
            ("Idle", "gamma"),
            ("Idle", "delta"),
        ]);
        let mut selector = selector("Ctrl+]", false);
        let area = Rect::new(0, 0, 20, 10);
        let shown = |selector: &Selector| {
            let (lines, highlighted, _) = drawn(selector, &rows, area);
            let listed: Vec<_> = lines[4..8]
                .iter()
                .map(|line| line.trim_matches([' ', '║']).to_owned())
                .filter(|line| !line.is_empty())
                .collect();
            (listed, highlighted)
        };
        selector.open(&rows, None, area);
        text(&mut selector, "a", &rows);
        assert_eq!(shown(&selector).1, [4]);
        typed(&mut selector, &[DOWN], &rows, area);
        text(&mut selector, "\tm", &rows);
        assert_eq!(shown(&selector), (vec!["Idle gamma".to_owned()], vec![4]));
        typed(
            &mut selector,
            &[bytes(b"\x7f"), bytes(b"\x08")],
            &rows,
            area,
        );
        assert_eq!(shown(&selector).1, [6]);
        text(&mut selector, "t", &rows);
        assert_eq!(
            shown(&selector),
            (
                vec!["Idle beta".to_owned(), "Idle delta".to_owned()],
                vec![4]
            )
        );
        typed(&mut selector, &[DOWN, LEFT, Key::Cursor(b'C')], &rows, area);
        assert_eq!(shown(&selector).1, [5]);
        text(&mut selector, "zz", &rows);
        assert_eq!(shown(&selector), (vec![], vec![]));
        assert_eq!(selector.key(&enter(), &rows, area), None);
        typed(
            &mut selector,
            &[bytes(b"\x1b[4~"), bytes(b"\x7f"), bytes(b"\x7f")],
            &rows,
            area,
        );
        assert_eq!(shown(&selector).1, [4]);
        assert_eq!(
            selector.key(&enter(), &rows, area),
            Some(rows[1].id.clone())
        );
    }

    /// SEL-02 and DSC-05 (pure supplement; selector document, "Status/
    /// title/discovery changes apply while the selector is open"): reordered
    /// rows keep the highlighted identity; a highlighted row that leaves or
    /// stops matching loses the highlight for good, so Enter chooses nothing
    /// until Up or Down highlights the first match.
    #[test]
    fn a_highlighted_row_that_leaves_or_stops_matching_is_unhighlighted_for_good() {
        let listed = rows(&[("Idle", "alpha"), ("Idle", "beta"), ("Idle", "gamma")]);
        let mut selector = selector("Ctrl+]", false);
        let area = Rect::new(0, 0, 20, 9);
        selector.open(&listed, None, area);
        typed(&mut selector, &[DOWN], &listed, area);
        let reordered = [listed[2].clone(), listed[1].clone(), listed[0].clone()];
        selector.update(&reordered);
        assert_eq!(drawn(&selector, &reordered, area).1, [5]);
        assert_eq!(
            selector.key(&enter(), &reordered, area),
            Some(listed[1].id.clone())
        );
        let without = [listed[0].clone(), listed[2].clone()];
        selector.update(&without);
        selector.update(&listed);
        assert_eq!(drawn(&selector, &listed, area).1, Vec::<u16>::new());
        assert_eq!(selector.key(&enter(), &listed, area), None);
        typed(&mut selector, &[UP], &listed, area);
        assert_eq!(
            selector.key(&enter(), &listed, area),
            Some(listed[0].id.clone())
        );

        text(&mut selector, "be", &listed);
        assert_eq!(
            selector.key(&enter(), &listed, area),
            Some(listed[1].id.clone())
        );
        let mut retitled = listed.clone();
        retitled[1].label = "zeta".to_owned();
        selector.update(&retitled);
        assert_eq!(selector.key(&enter(), &retitled, area), None);
        retitled[1].label = "beta".to_owned();
        selector.update(&retitled);
        assert_eq!(selector.key(&enter(), &retitled, area), None);
        typed(&mut selector, &[DOWN], &retitled, area);
        assert_eq!(
            selector.key(&enter(), &retitled, area),
            Some(listed[1].id.clone())
        );
    }

    /// SEL-01 (pure supplement, opening): the selector opens with an empty
    /// search on the selected session, scrolled into view, or else on the
    /// first row; opening an open selector changes nothing; Escape closes
    /// it and Enter leaves closing to its caller.
    #[test]
    fn opening_highlights_the_selected_session_or_the_first_row() {
        let rows = letters(5);
        let mut selector = selector("Ctrl+]", false);
        selector.open(&rows, Some(&rows[4].id), SMALL);
        assert_eq!(
            list(&selector, &rows),
            (vec!["d".to_owned(), "e".to_owned()], vec![5])
        );
        text(&mut selector, "e", &rows);
        selector.open(&rows, None, SMALL);
        assert_eq!(
            list(&selector, &rows),
            (vec!["e".to_owned(), String::new()], vec![4])
        );
        assert_eq!(
            selector.key(&enter(), &rows, SMALL),
            Some(rows[4].id.clone())
        );
        assert!(selector.is_open());
        selector.key(&Key::Escape, &rows, SMALL);
        assert!(!selector.is_open());
        assert_eq!(drawn(&selector, &rows, SMALL).0, ["            "; 8]);
        let unknown = letters(6).remove(5).id;
        selector.open(&rows, Some(&unknown), SMALL);
        assert_eq!(
            list(&selector, &rows),
            (vec!["a".to_owned(), "b".to_owned()], vec![4])
        );
        selector.close();
        assert!(!selector.is_open());
        selector.open(&rows, Some(&rows[1].id), SMALL);
        assert_eq!(
            list(&selector, &rows),
            (vec!["a".to_owned(), "b".to_owned()], vec![5])
        );
        selector.close();
        // Opened where no list row shows, then drawn where two do, the
        // selected row is still in view.
        for hidden in [Rect::new(0, 0, 2, 2), Rect::new(0, 0, 12, 6)] {
            selector.open(&rows, Some(&rows[2].id), hidden);
            assert_eq!(
                list(&selector, &rows),
                (vec!["c".to_owned(), "d".to_owned()], vec![4]),
                "{hidden:?}"
            );
            selector.close();
        }
        let mut empty = Selector::new(ControlKey::from_name("Ctrl+]").unwrap(), false, 4096);
        empty.open(&[], None, SMALL);
        assert_eq!(
            typed(&mut empty, &[DOWN, page_down(), enter()], &[], SMALL),
            [None, None, None]
        );
    }

    fn mouse(code: u16, column: u16, row: u16) -> Mouse {
        Mouse {
            code,
            column,
            row,
            release: false,
        }
    }

    /// SEL-02 and INP-05 (pure supplement, mouse): a left click on a listed
    /// row only highlights it, a click elsewhere inside the box does
    /// nothing, and one outside it cancels (owner decision on #113); the
    /// wheel scrolls the list a row a notch, as far as it goes, without
    /// moving the highlight; releases and other buttons do nothing.
    #[test]
    fn clicks_highlight_rows_or_cancel_outside_and_the_wheel_scrolls() {
        let rows = letters(4);
        let mut selector = selector("Ctrl+]", false);
        selector.open(&rows, None, SMALL);
        selector.mouse(&mouse(0, 3, 5), &rows, SMALL);
        assert_eq!(
            list(&selector, &rows),
            (vec!["a".to_owned(), "b".to_owned()], vec![5])
        );
        for (code, column, row) in [
            (0, 3, 2),
            (0, 1, 4),
            (0, 3, 3),
            (0, 10, 6),
            (2, 3, 4),
            (1, 3, 4),
            (32, 3, 4),
        ] {
            selector.mouse(&mouse(code, column, row), &rows, SMALL);
            assert_eq!(list(&selector, &rows).1, [5], "{code} {column} {row}");
        }
        let mut released = mouse(0, 3, 4);
        released.release = true;
        selector.mouse(&released, &rows, SMALL);
        assert_eq!(list(&selector, &rows).1, [5]);
        let wheel =
            |selector: &mut Selector, code| selector.mouse(&mouse(code, 0, 0), &rows, SMALL);
        wheel(&mut selector, 65);
        assert_eq!(
            list(&selector, &rows),
            (vec!["b".to_owned(), "c".to_owned()], vec![4])
        );
        wheel(&mut selector, 65);
        wheel(&mut selector, 65);
        assert_eq!(
            list(&selector, &rows),
            (vec!["c".to_owned(), "d".to_owned()], vec![])
        );
        selector.mouse(&mouse(0, 9, 5), &rows, SMALL);
        assert_eq!(
            selector.key(&enter(), &rows, SMALL),
            Some(rows[3].id.clone())
        );
        wheel(&mut selector, 64);
        assert_eq!(
            list(&selector, &rows),
            (vec!["b".to_owned(), "c".to_owned()], vec![])
        );
        wheel(&mut selector, 64);
        wheel(&mut selector, 64);
        assert_eq!(
            list(&selector, &rows),
            (vec!["a".to_owned(), "b".to_owned()], vec![])
        );
        assert!(selector.is_open());
        for (column, row) in [(0, 4), (11, 4), (5, 0), (5, 7)] {
            selector.mouse(&mouse(0, column, row), &rows, SMALL);
            assert!(!selector.is_open(), "{column} {row}");
            selector.open(&rows, None, SMALL);
        }
        let few = letters(1);
        selector.mouse(&mouse(0, 3, 5), &few, SMALL);
        wheel(&mut selector, 65);
        assert_eq!(
            list(&selector, &few),
            (vec!["a".to_owned(), String::new()], vec![4])
        );
        selector.mouse(&mouse(0, 0, 0), &rows, Rect::new(0, 0, 2, 2));
        assert!(!selector.is_open());
    }

    /// SEL-02 and operational limits (pure supplement, paste and
    /// `ui.search_bytes`): a bracketed paste fills the search as one line
    /// and refilters; a paste or typed character past the limit is refused
    /// visibly in the footer's row, which hints off lend it, until the next
    /// key or paste.
    #[test]
    fn pastes_fill_the_search_and_refusals_show_in_the_footer() {
        let rows = rows(&[("Idle", "a b"), ("Idle", "ab")]);
        let mut selector = Selector::new(ControlKey::from_name("Ctrl+]").unwrap(), false, 5);
        let area = Rect::new(0, 0, 32, 8);
        selector.open(&rows, None, area);
        let shown = |selector: &Selector| {
            let (lines, highlighted, cursor) = drawn(selector, &rows, area);
            let inside: Vec<_> = lines[1..7]
                .iter()
                .map(|line| {
                    line.trim_matches([' ', '║', '╔', '╗', '╚', '╝', '═', '╟', '╢', '─'])
                        .to_owned()
                })
                .collect();
            (inside, highlighted, cursor)
        };
        let paste = |selector: &mut Selector, parts: &[Paste]| {
            for part in parts {
                selector.paste(part, &rows, area);
            }
        };
        paste(
            &mut selector,
            &[
                Paste::Start,
                Paste::Data(b"a\r".to_vec()),
                Paste::Data(b"\nb".to_vec()),
                Paste::End,
            ],
        );
        assert_eq!(
            shown(&selector),
            (
                ["", "a b", "", "Idle a b", "", ""]
                    .map(str::to_owned)
                    .to_vec(),
                vec![4],
                cursor(5, 2)
            )
        );
        text(&mut selector, "cde", &rows);
        let refused = ["", "a bcd", "", "", "Search is limited to 5 bytes", ""]
            .map(str::to_owned)
            .to_vec();
        assert_eq!(shown(&selector), (refused, vec![], cursor(7, 2)));
        selector.key(&LEFT, &rows, area);
        assert_eq!(shown(&selector).0[4], "");
        paste(
            &mut selector,
            &[Paste::Start, Paste::Data(b"xy".to_vec()), Paste::End],
        );
        assert_eq!(shown(&selector).0[4], "Search is limited to 5 bytes");
        paste(&mut selector, &[Paste::Start, Paste::End]);
        assert_eq!(shown(&selector).0[4], "");
        selector.paste(&Paste::Data(b"x".to_vec()), &rows, area);
        assert_eq!(shown(&selector).0[1], "a bcd");
    }

    /// SEL-02 and HLP-02 (pure supplement, modal input): the selector takes
    /// keys, pastes and mouse reports; a global action, a prefix with a key
    /// that completes none, or the rest of a released key does nothing to
    /// it.
    #[test]
    fn the_selector_takes_keys_pastes_and_mouse_reports_only() {
        let rows = letters(3);
        let mut selector = selector("Ctrl+]", false);
        selector.open(&rows, None, SMALL);
        let ignored = [
            Routed::Action(crate::keys::Action::SessionList),
            Routed::Unknown(0x1d, Key::Escape),
            Routed::Rest(Key::Bytes(b"b".to_vec())),
        ];
        for input in &ignored {
            assert_eq!(selector.input(input, &rows, SMALL), None);
        }
        assert!(selector.is_open());
        assert_eq!(drawn(&selector, &rows, SMALL).0[2], " ║        ║ ");
        let taken = [
            Routed::Key(DOWN),
            Routed::Paste(Paste::Start),
            Routed::Paste(Paste::Data(b"c".to_vec())),
            Routed::Paste(Paste::End),
            Routed::Mouse(mouse(0, 0, 0)),
        ];
        for input in &taken {
            assert_eq!(selector.input(input, &rows, SMALL), None);
        }
        assert!(!selector.is_open());
        selector.open(&rows, None, SMALL);
        assert_eq!(
            selector.input(&Routed::Key(enter()), &rows, SMALL),
            Some(rows[0].id.clone())
        );
    }

    /// SEL-02 (pure supplement): a search edit scrolls the list back to its
    /// start, as far as the highlight allows, and reopening forgets an
    /// earlier refusal and scrolling.
    #[test]
    fn search_edits_and_reopening_scroll_back_to_the_start() {
        let rows = rows(&[("Idle", "a"), ("Idle", "b"), ("Idle", "c"), ("Idle", "d")]);
        let mut selector = Selector::new(ControlKey::from_name("Ctrl+]").unwrap(), false, 1);
        selector.open(&rows, None, SMALL);
        typed(&mut selector, &[DOWN], &rows, SMALL);
        for _ in 0..3 {
            selector.mouse(&mouse(65, 0, 0), &rows, SMALL);
        }
        text(&mut selector, "i", &rows);
        assert_eq!(
            list(&selector, &rows),
            (vec!["Idle a".to_owned(), "Idle b".to_owned()], vec![5])
        );
        text(&mut selector, "x", &rows);
        assert_eq!(drawn(&selector, &rows, SMALL).0[5], " ║Search …║ ");
        selector.mouse(&mouse(65, 0, 0), &rows, SMALL);
        selector.close();
        selector.open(&rows, None, SMALL);
        assert_eq!(
            list(&selector, &rows),
            (vec!["Idle a".to_owned(), "Idle b".to_owned()], vec![4])
        );
    }

    /// SEL-01 (pure supplement, search line): the search cursor follows
    /// editing; a box too small for a search line shows no cursor, and a
    /// closed selector draws nothing.
    #[test]
    fn the_search_cursor_follows_the_text() {
        let rows = letters(1);
        let mut selector = selector("Ctrl+]", false);
        selector.open(&rows, None, SMALL);
        text(&mut selector, "日b", &rows);
        assert_eq!(drawn(&selector, &rows, SMALL).2, cursor(5, 2));
        typed(&mut selector, &[LEFT], &rows, SMALL);
        assert_eq!(drawn(&selector, &rows, SMALL).2, cursor(4, 2));
        typed(&mut selector, &[Key::Cursor(b'H')], &rows, SMALL);
        assert_eq!(drawn(&selector, &rows, SMALL).2, cursor(2, 2));
        typed(&mut selector, &[Key::Cursor(b'C')], &rows, SMALL);
        assert_eq!(drawn(&selector, &rows, SMALL).2, cursor(4, 2));
        typed(&mut selector, &[Key::Cursor(b'F')], &rows, SMALL);
        assert_eq!(drawn(&selector, &rows, SMALL).2, cursor(5, 2));
        typed(&mut selector, &[bytes(b"\x1b[1~")], &rows, SMALL);
        assert_eq!(drawn(&selector, &rows, SMALL).2, cursor(2, 2));
        let (lines, _, cursor) = drawn(&selector, &rows, Rect::new(3, 0, 6, 5));
        assert_eq!(lines[2], " ║日║ ");
        assert_eq!(cursor, super::tests::cursor(5, 2));
        typed(&mut selector, &[bytes(b"\x1b[4~")], &rows, SMALL);
        let (lines, _, cursor) = drawn(&selector, &rows, Rect::new(3, 0, 6, 5));
        assert_eq!(lines[2], " ║b ║ ");
        assert_eq!(cursor, super::tests::cursor(6, 2));
        assert_eq!(drawn(&selector, &rows, Rect::new(0, 0, 4, 9)).2, None);
        assert_eq!(drawn(&selector, &rows, Rect::new(0, 0, 9, 4)).2, None);
        selector.close();
        assert_eq!(drawn(&selector, &rows, SMALL).2, None);
    }

    /// SEL-01 (pure supplement; the box at its smallest): an area 3 cells
    /// wide and high holds a one-cell border and nothing else; one cell
    /// narrower or lower holds no selector at all; a box with two inner rows
    /// has its search and separator, and one with one inner row only its
    /// search; drawing never fails.
    #[test]
    fn the_selector_draws_only_what_fits() {
        let rows = rows(&[("Starting", "Alpha")]);
        let mut selector = selector("Ctrl+]", true);
        selector.open(&rows, None, SMALL);
        let drawn =
            |width: u16, height: u16| drawn(&selector, &rows, Rect::new(0, 0, width, height)).0;
        assert_eq!(drawn(3, 3), ["   ", " ╔ ", "   "]);
        assert_eq!(drawn(2, 3), ["  "; 3]);
        assert_eq!(drawn(3, 2), ["   "; 2]);
        assert_eq!(drawn(1, 4), [" "; 4]);
        assert_eq!(drawn(4, 1), ["    "]);
        assert_eq!(
            drawn(6, 5),
            ["      ", " ╔══╗ ", " ║  ║ ", " ╚══╝ ", "      "]
        );
        assert_eq!(
            drawn(6, 6),
            ["      ", " ╔══╗ ", " ║  ║ ", " ╟──╢ ", " ╚══╝ ", "      "]
        );
        assert_eq!(
            drawn(6, 7),
            [
                "      ",
                " ╔══╗ ",
                " ║  ║ ",
                " ╟──╢ ",
                " ║E…║ ",
                " ╚══╝ ",
                "      "
            ]
        );
    }
}
