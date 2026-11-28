//! Where the terminal interface shows the monitor library's toasts (spec
//! #73, "Presentation (Toasts)"; #115): pure. One toast shows at a time,
//! yellow, bottom-right in a four-row lane below the selector, its border
//! drawn clockwise, its label held, faded, and its border erased clockwise.
//! The library keeps the queue, the ledger and the timing; placement,
//! hit-testing and drawing are this interface's (ADR 0011). A toast
//! covering the selected session's canvas stops it counting as viewed, and
//! prefix+j and Enter dismiss a toast covering their target (owner decision
//! on #114): the library asks this module whether it does.
use crate::mouse::{Gesture, Mouse};
use climon2_monitor::{
    toast::{DRAW, ERASE, Schedule, Text, Toast, Toasts},
    truncate,
};
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
};
use std::{ops::Range, time::Duration};
use unicode_width::UnicodeWidthStr;

/// The rows reserved for toasts below the selector (owner decision on
/// #115): the selector's area shrinks by them.
pub(crate) const LANE: u16 = 4;
/// A toast is at most this percentage of the host's width.
const WIDTH_PERCENT: u16 = 40;
/// The border and one blank cell on each side of the text.
const PADDING: u16 = 4;
/// The border's glyphs, by row (top, middle, bottom) and column (left,
/// middle, right).
const GLYPHS: [[&str; 3]; 3] = [["┌", "─", "┐"], ["│", " ", "│"], ["└", "─", "┘"]];

/// What the shown toast does with a mouse report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Taken {
    /// It is not the toast's.
    No,
    /// The toast keeps it and does nothing.
    Kept,
    /// A click on the toast: it only dismisses it.
    Dismissing,
}

impl Taken {
    /// Whether the toast took the report.
    pub(crate) fn taken(self) -> bool {
        self != Self::No
    }
}

/// Where `toast` shows in `area`: flush with its bottom-right corner,
/// three rows high, or four with a hint, as wide as its widest line and
/// padding, but at most [`WIDTH_PERCENT`] of the host's width. None when
/// it has no room for one cell of text.
pub(crate) fn place(toast: &Toast, area: Rect) -> Option<Rect> {
    let lines = std::iter::once(&toast.label).chain(&toast.hint);
    let widest = lines.map(|line| line.width()).max().unwrap_or_default();
    let widest = u16::try_from(widest).unwrap_or(u16::MAX);
    let width = widest
        .saturating_add(PADDING)
        .min(area.width * WIDTH_PERCENT / 100);
    let height = 3 + u16::from(toast.hint.is_some());
    let fits = width > PADDING && area.height >= height;
    fits.then(|| Rect::new(area.right() - width, area.bottom() - height, width, height))
}

/// Whether `toast`, placed in `area`, covers any cell of `canvas` there.
pub(crate) fn covers(toast: &Toast, canvas: Rect, area: Rect) -> bool {
    place(toast, area).is_some_and(|rect| rect.intersects(canvas.intersection(area)))
}

/// Whether the host cell `column`, `row` (from 0) is on `toast` in `area`.
fn hit(toast: &Toast, (column, row): (u16, u16), area: Rect) -> bool {
    place(toast, area).is_some_and(|rect| rect.contains(Position::new(column, row)))
}

/// What the `shown` toast does with `mouse` (spec #73, "Modals and
/// mouse"): it takes a report outside any gesture (`idle`) that lands on
/// it, so a toast never takes a gesture begun before it appeared. A click
/// it takes only dismisses it.
pub(crate) fn take(shown: Option<&Toast>, mouse: Mouse, idle: bool, area: Rect) -> Taken {
    let taken = idle && shown.is_some_and(|toast| hit(toast, mouse.host_cell(), area));
    match (taken, mouse.gesture()) {
        (false, _) => Taken::No,
        (true, Gesture::Click) => Taken::Dismissing,
        (true, _) => Taken::Kept,
    }
}

/// The host cells of a canvas `geometry` (columns, rows) shows from the
/// top-left of `below`, the rows below the top row: all of them before it
/// has a geometry.
pub(crate) fn canvas(below: Rect, geometry: Option<(u16, u16)>) -> Rect {
    geometry.map_or(below, |(width, height)| Rect {
        width,
        height,
        ..below
    })
}

/// Draws the shown toast of `toasts` as it looks at `now` in `area`.
pub(crate) fn render(toasts: &Toasts, now: std::time::Instant, area: Rect, buffer: &mut Buffer) {
    let Some(showing) = toasts.showing(now) else {
        return;
    };
    let Some(rect) = place(showing.toast, area) else {
        return;
    };
    let look = look(showing.elapsed, showing.schedule, perimeter(rect));
    draw(showing.toast, rect, &look, buffer);
}

/// What a toast shows at one moment.
#[derive(Debug, Eq, PartialEq)]
struct Look {
    /// The border cells drawn, counted clockwise from the top-left corner.
    border: Range<usize>,
    text: Text,
    /// Whether its inside is blank over what lies beneath: until its border
    /// starts erasing.
    filled: bool,
}

/// What a toast with a border of `perimeter` cells shows `elapsed` after
/// it began, by `schedule`: its border drawn clockwise in [`DRAW`], its
/// text from then until it fades, faded until the border starts erasing,
/// and the border erased clockwise in [`ERASE`] for a whole border.
fn look(elapsed: Duration, schedule: Schedule, perimeter: usize) -> Look {
    let drawn = share(perimeter, elapsed.min(schedule.erase), DRAW);
    let erased = share(perimeter, elapsed.saturating_sub(schedule.erase), ERASE);
    Look {
        border: erased.min(drawn)..drawn,
        text: climon2_monitor::toast::text(elapsed, &schedule),
        filled: elapsed < schedule.erase,
    }
}

/// The cells of `perimeter` that `elapsed` of `span` reaches.
fn share(perimeter: usize, elapsed: Duration, span: Duration) -> usize {
    let reached = perimeter as u128 * elapsed.as_nanos() / span.as_nanos();
    usize::try_from(reached)
        .unwrap_or(usize::MAX)
        .min(perimeter)
}

/// How many cells a border around `rect` has.
fn perimeter(rect: Rect) -> usize {
    let (width, height) = (usize::from(rect.width), usize::from(rect.height));
    2 * (width + height) - 4
}

/// The border cell `index` cells clockwise from `rect`'s top-left corner:
/// along the top, down the right, back along the bottom and up the left.
fn border(rect: Rect, index: usize) -> (u16, u16) {
    let (width, height) = (usize::from(rect.width), usize::from(rect.height));
    let (right, bottom) = (rect.right() - 1, rect.bottom() - 1);
    let offset = |count: usize| u16::try_from(count).unwrap_or(u16::MAX);
    if index < width {
        (rect.x + offset(index), rect.y)
    } else if index < width + height - 1 {
        (right, rect.y + offset(index + 1 - width))
    } else if index < 2 * width + height - 2 {
        (right - offset(index + 2 - width - height), bottom)
    } else {
        (rect.x, bottom - offset(index + 3 - 2 * width - height))
    }
}

/// The border glyph at `x`, `y` on `rect`'s edge.
fn glyph(rect: Rect, x: u16, y: u16) -> &'static str {
    let along = |at: u16, start: u16, end: u16| usize::from(at != start) + usize::from(at == end);
    GLYPHS[along(y, rect.y, rect.bottom() - 1)][along(x, rect.x, rect.right() - 1)]
}

/// Draws `toast` in `rect` as `look` says, yellow.
fn draw(toast: &Toast, rect: Rect, look: &Look, buffer: &mut Buffer) {
    let yellow = Style::new().fg(Color::Yellow);
    unwiden(rect, buffer);
    if look.filled {
        fill(rect, buffer);
    }
    for index in look.border.clone() {
        let (x, y) = border(rect, index);
        buffer.set_string(x, y, glyph(rect, x, y), yellow);
    }
    if look.text != Text::Hidden {
        let faded = usize::from(look.text == Text::Faded);
        let style = [yellow, yellow.add_modifier(Modifier::DIM)][faded];
        write(toast, rect, style, buffer);
    }
}

/// Blanks each wide character that `rect`'s left edge would cut, so the
/// host never covers the border with it.
fn unwiden(rect: Rect, buffer: &mut Buffer) {
    let Some(x) = rect.x.checked_sub(1) else {
        return;
    };
    for y in rect.y..rect.bottom() {
        let cell = &mut buffer[(x, y)];
        if cell.symbol().width() > 1 {
            cell.reset();
        }
    }
}

/// Blanks the inside of `rect`'s border over what lies beneath.
fn fill(rect: Rect, buffer: &mut Buffer) {
    let blank = " ".repeat(usize::from(rect.width - 2));
    for y in rect.y + 1..rect.bottom() - 1 {
        buffer.set_string(rect.x + 1, y, &blank, Style::new());
    }
}

/// Writes `toast`'s lines inside `rect`'s border, in `style`, each after
/// one blank cell and truncated with `…` before the last.
fn write(toast: &Toast, rect: Rect, style: Style, buffer: &mut Buffer) {
    let lines = std::iter::once(&toast.label).chain(&toast.hint);
    let room = usize::from(rect.width - PADDING);
    for (y, line) in (rect.y + 1..).zip(lines) {
        buffer.set_stringn(rect.x + 2, y, truncate(line, room), room, style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use climon2_core::SessionId;
    use climon2_monitor::{News, toast::schedule};
    use std::time::Instant;

    fn id(suffix: u8) -> SessionId {
        format!("00000000-0000-4000-8000-0000000000{suffix:02x}")
            .parse()
            .unwrap()
    }

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    const HINT: &str = "Ctrl+] then j: jump to attention";

    /// The rows below the top row of a host `cols` by `rows`.
    fn area(cols: u16, rows: u16) -> Rect {
        Rect::new(0, 1, cols, rows - 1)
    }

    /// `toasts` advanced to `now` as the interface advances them for a
    /// host whose rows below the top row are `area`.
    fn frame(toasts: &mut Toasts, now: Instant, area: Rect) {
        toasts.advance(now, |toast| place(toast, area).is_some());
    }

    /// How long after it began the shown toast was dismissed, if it was,
    /// before its title was written.
    fn dismissed(toasts: &Toasts, now: Instant) -> Option<Duration> {
        let showing = toasts.showing(now).unwrap();
        let dismissed = showing.schedule != schedule(None, None);
        dismissed.then_some(showing.schedule.fade)
    }

    /// NOT-02 (pure supplement): the accepted timings, at each boundary and
    /// one before it, for a title written as soon as it is drawn: the
    /// border draws clockwise for 400 ms with no text, the text then holds
    /// for 2,000 ms, fades for 300 ms, and the border erases clockwise for
    /// 400 ms, the inside blank until it starts.
    #[test]
    fn a_toast_draws_holds_fades_and_erases_on_time() {
        let at = |millis| look(ms(millis), schedule(Some(DRAW), None), 20);
        let look_of = |border: Range<usize>, text, filled| Look {
            border,
            text,
            filled,
        };
        assert_eq!(at(0), look_of(0..0, Text::Hidden, true));
        assert_eq!(at(20), look_of(0..1, Text::Hidden, true));
        assert_eq!(at(399), look_of(0..19, Text::Hidden, true));
        assert_eq!(at(400), look_of(0..20, Text::Shown, true));
        assert_eq!(at(2399), look_of(0..20, Text::Shown, true));
        assert_eq!(at(2400), look_of(0..20, Text::Faded, true));
        assert_eq!(at(2699), look_of(0..20, Text::Faded, true));
        assert_eq!(at(2700), look_of(0..20, Text::Hidden, false));
        assert_eq!(at(2720), look_of(1..20, Text::Hidden, false));
        assert_eq!(at(3099), look_of(19..20, Text::Hidden, false));
        assert_eq!(at(3100), look_of(20..20, Text::Hidden, false));
        assert_eq!(schedule(Some(DRAW), None).end, ms(3100));
    }

    /// NOT-02 (pure supplement): an early dismissal while the border draws
    /// erases only what was drawn, from then, as fast as it drew; one while
    /// the text holds fades it at once, then erases.
    #[test]
    fn an_early_dismissal_erases_only_what_was_drawn() {
        let at = |millis| look(ms(millis), schedule(None, Some(ms(200))), 20);
        assert_eq!(at(199).border, 0..9);
        assert_eq!(at(200).border, 0..10);
        assert_eq!(at(300).border, 5..10);
        assert_eq!(at(399).border, 9..10);
        assert!(
            [at(200), at(500)]
                .iter()
                .all(|look| look.text == Text::Hidden)
        );
        let titled = |dismissed| schedule(Some(DRAW), Some(ms(dismissed)));
        let held = |millis| look(ms(millis), titled(1000), 20);
        assert_eq!(held(999).text, Text::Shown);
        assert_eq!(held(1000).text, Text::Faded);
        assert_eq!(held(1300).text, Text::Hidden);
    }

    fn hinted(label: &str) -> Toast {
        Toast {
            label: label.to_owned(),
            hint: Some(HINT.to_owned()),
        }
    }

    fn plain(label: &str) -> Toast {
        Toast {
            label: label.to_owned(),
            hint: None,
        }
    }

    /// NOT-02 (pure supplement): a toast sits flush with the bottom-right
    /// corner, three rows high or four with its hint, as wide as its widest
    /// line and padding, at most 40% of the host's width, here 48 and 47
    /// cells; it needs room for one cell of text and its rows, at each
    /// limit and one under it.
    #[test]
    fn a_toast_sits_bottom_right_at_most_forty_percent_wide() {
        let label = "Build";
        assert_eq!(
            place(&plain(label), area(120, 30)),
            Some(Rect::new(111, 27, 9, 3))
        );
        assert_eq!(
            place(&hinted(label), area(120, 30)),
            Some(Rect::new(84, 26, 36, 4))
        );
        let wide = plain(&"w".repeat(60));
        assert_eq!(place(&wide, area(120, 30)), Some(Rect::new(72, 27, 48, 3)));
        assert_eq!(place(&wide, area(119, 30)), Some(Rect::new(72, 27, 47, 3)));
        assert_eq!(place(&wide, area(13, 30)).map(|rect| rect.width), Some(5));
        assert_eq!(place(&wide, area(12, 30)), None);
        assert_eq!(place(&hinted(label), area(120, 5)).map(|r| r.y), Some(1));
        assert_eq!(place(&hinted(label), area(120, 4)), None);
        assert_eq!(place(&plain(label), area(120, 4)).map(|r| r.y), Some(1));
        assert_eq!(place(&plain(label), area(120, 3)), None);
    }

    /// NOT-02 (pure supplement): no toast begins where it does not fit; it
    /// waits until it does.
    #[test]
    fn a_toast_waits_until_it_fits() {
        let mut toasts = Toasts::new(None);
        toasts.announce(&id(1), News::Final, "Build".to_owned(), false);
        let start = Instant::now();
        frame(&mut toasts, start, area(12, 30));
        assert!(toasts.shown().is_none());
        frame(&mut toasts, start, area(13, 30));
        assert!(toasts.shown().is_some());
    }

    /// The text rows of `buffer`.
    fn text(buffer: &Buffer) -> Vec<String> {
        let width = usize::from(buffer.area.width);
        buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }

    /// `toasts` drawn at `elapsed` on a 20 by 6 host whose rows below the
    /// top row are all `x`.
    fn drawn(toasts: &Toasts, start: Instant, elapsed: u64) -> (Buffer, Vec<String>) {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 6));
        for y in 1..6 {
            buffer.set_string(0, y, "x".repeat(20), Style::new());
        }
        render(toasts, start + ms(elapsed), area(20, 6), &mut buffer);
        let text = text(&buffer);
        (buffer, text)
    }

    /// NOT-02 (pure supplement): the toast in its cells, yellow: its border
    /// clockwise from the top-left corner, its blank inside, its label and
    /// hint truncated with `…` within the padding, faded dim, and once it
    /// erases the inside shows what lies beneath.
    #[test]
    fn a_toast_draws_its_border_label_and_hint_in_yellow() {
        let mut toasts = Toasts::new(Some("C-] then j".to_owned()));
        toasts.announce(&id(1), News::Attention(1), "Long label".to_owned(), false);
        let start = Instant::now();
        frame(&mut toasts, start, area(20, 6));
        assert!(toasts.titled(start, start + DRAW));
        let (_, early) = drawn(&toasts, start, 100);
        assert_eq!(
            early[1..],
            [
                "xxxxxxxxxxxxxxxxxxxx",
                "xxxxxxxxxxxx┌────xxx",
                "xxxxxxxxxxxxx      x",
                "xxxxxxxxxxxxx      x",
                "xxxxxxxxxxxxxxxxxxxx",
            ]
        );
        let (held, shown) = drawn(&toasts, start, 400);
        assert_eq!(
            shown[2..],
            [
                "xxxxxxxxxxxx┌──────┐",
                "xxxxxxxxxxxx│ Lon… │",
                "xxxxxxxxxxxx│ C-]… │",
                "xxxxxxxxxxxx└──────┘",
            ]
        );
        assert_eq!(held[(12, 2)].fg, Color::Yellow);
        assert_eq!(held[(14, 3)].fg, Color::Yellow);
        assert_eq!(held[(14, 3)].modifier, Modifier::empty());
        assert_eq!(held[(13, 3)].fg, Color::Reset);
        let (faded, _) = drawn(&toasts, start, 2400);
        assert_eq!(faded[(14, 3)].modifier, Modifier::DIM);
        assert_eq!(faded[(14, 4)].modifier, Modifier::DIM);
        let (_, erasing) = drawn(&toasts, start, 2900);
        assert_eq!(
            erasing[2..],
            [
                "xxxxxxxxxxxxxxxxxxxx",
                "xxxxxxxxxxxx│xxxxxxx",
                "xxxxxxxxxxxx│xxxxxxx",
                "xxxxxxxxxxxx└──────┘",
            ]
        );
        let mut none = Toasts::new(None);
        frame(&mut none, start, area(20, 6));
        let untouched = drawn(&none, start, 500).1;
        assert!(untouched[1..].iter().all(|row| *row == "x".repeat(20)));
    }

    /// NOT-02 (pure supplement): the border's cells in clockwise order from
    /// the top-left corner, each with its glyph.
    #[test]
    fn the_border_runs_clockwise_from_the_top_left_corner() {
        let rect = Rect::new(2, 1, 4, 3);
        let cells: Vec<_> = (0..perimeter(rect))
            .map(|index| {
                let (x, y) = border(rect, index);
                (x, y, glyph(rect, x, y))
            })
            .collect();
        assert_eq!(
            cells,
            [
                (2, 1, "┌"),
                (3, 1, "─"),
                (4, 1, "─"),
                (5, 1, "┐"),
                (5, 2, "│"),
                (5, 3, "┘"),
                (4, 3, "─"),
                (3, 3, "─"),
                (2, 3, "└"),
                (2, 2, "│"),
            ]
        );
    }

    /// FOC-01 and INP-05 (pure supplement): the shown toast covers a canvas
    /// that shares a cell with it and none that does not, and only its own
    /// cells are hits; a toast with no room covers and takes nothing.
    #[test]
    fn the_shown_toast_covers_and_takes_only_its_own_cells() {
        let host = area(120, 30);
        let canvas = Rect::new(0, 1, 120, 29);
        let toast = plain("Build");
        assert!(covers(&toast, canvas, host));
        assert!(covers(&toast, Rect::new(0, 1, 112, 27), host));
        assert!(!covers(&toast, Rect::new(0, 1, 111, 29), host));
        assert!(!covers(&toast, Rect::new(0, 1, 120, 26), host));
        assert!(hit(&toast, (111, 27), host));
        assert!(hit(&toast, (119, 29), host));
        assert!(!hit(&toast, (110, 29), host));
        assert!(!hit(&toast, (119, 26), host));
        assert!(!covers(&toast, canvas, area(12, 30)));
        assert!(!hit(&toast, (11, 29), area(12, 30)));
    }

    /// A report at the host cell `column`, `row` (from 1).
    fn report(code: u16, column: u16, row: u16, released: bool) -> Mouse {
        Mouse {
            code,
            column,
            row,
            released,
        }
    }

    /// INP-05 (pure supplement): outside any gesture, the shown toast takes
    /// reports that land on it, and a left click it takes dismisses it;
    /// within a gesture begun before, or off its cells, it takes nothing.
    /// With no toast shown nothing is taken.
    #[test]
    fn the_toast_takes_reports_outside_gestures_and_a_click_dismisses_it() {
        let host = area(120, 30);
        let click = report(0, 120, 30, false);
        assert_eq!(take(None, click, true, host), Taken::No);
        let toast = plain("Build");
        let shown = Some(&toast);
        assert_eq!(take(shown, click, false, host), Taken::No);
        let off = report(0, 111, 30, false);
        assert_eq!(take(shown, off, true, host), Taken::No);
        for other in [report(0, 120, 30, true), report(64, 120, 30, false)] {
            assert_eq!(take(shown, other, true, host), Taken::Kept);
        }
        let middle = report(2, 112, 28, false);
        assert_eq!(take(shown, middle, true, host), Taken::Kept);
        let clicked = take(shown, report(0, 112, 28, false), true, host);
        assert_eq!(clicked, Taken::Dismissing);
        assert!(clicked.taken() && Taken::Kept.taken() && !Taken::No.taken());
    }

    /// NOT-03 (pure supplement; owner decision on #114): prefix+j and
    /// Enter dismiss the toast only when it covers their target's canvas,
    /// which takes its geometry's cells from the top-left below the top
    /// row, or all of them before it has a geometry.
    #[test]
    fn a_toast_is_dismissed_over_a_canvas_it_covers() {
        let host = area(120, 30);
        assert_eq!(canvas(host, None), host);
        assert_eq!(canvas(host, Some((80, 24))), Rect::new(0, 1, 80, 24));
        let start = Instant::now();
        let mut toasts = Toasts::new(None);
        toasts.announce(&id(1), News::Final, "Build".to_owned(), false);
        frame(&mut toasts, start, host);
        let toast = toasts.shown().unwrap();
        assert!(!covers(toast, canvas(host, Some((111, 29))), host));
        assert!(covers(toast, canvas(host, Some((112, 27))), host));
        assert_eq!(dismissed(&toasts, start), None);
        toasts.dismiss(start + ms(600));
        assert_eq!(dismissed(&toasts, start), Some(ms(600)));
    }

    /// TRM-01 (pure supplement): a wide character that the toast's left
    /// edge would cut is blanked, so the border shows whole; a narrow one
    /// stays.
    #[test]
    fn a_wide_character_cut_by_the_toast_is_blanked() {
        let mut toasts = Toasts::new(None);
        toasts.announce(&id(1), News::Final, "Build".to_owned(), false);
        let start = Instant::now();
        frame(&mut toasts, start, area(20, 6));
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 6));
        buffer.set_string(11, 3, "界", Style::new());
        buffer.set_string(11, 4, "a", Style::new());
        render(&toasts, start + ms(400), area(20, 6), &mut buffer);
        assert_eq!(buffer[(11, 3)].symbol(), " ");
        assert_eq!(buffer[(11, 4)].symbol(), "a");
        assert_eq!(buffer[(12, 3)].symbol(), "┌");
    }
}
