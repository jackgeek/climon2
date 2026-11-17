//! Toasts (spec #73, "Presentation (Toasts)" and "Toast eligibility";
//! #115): pure. A toast announces an attention episode or a final outcome
//! once per run, never while this monitor views the session it is about:
//! the ledger, keyed per session by episode or the one final outcome, lasts
//! the whole run. The queue holds three toasts, the one shown among them
//! (owner decision on #115, 2026-11-16), then one more toast stands for
//! everything after them.
//! One toast shows at a time, yellow, bottom-right in a four-row lane below
//! the selector, its border drawn clockwise, its label held, faded, and its
//! border erased clockwise. The hold, and the toast's one bell, start once
//! the frame showing its full title has been written to the host (owner
//! decision on #115, 2026-11-17). A toast covering the selected session's canvas
//! stops it counting as viewed (`lib.rs`), and prefix+j and Enter dismiss a
//! toast covering their target (owner decision on #114).
use crate::{
    mouse::{Gesture, Mouse},
    registry::{News, truncate},
};
use climon2_core::SessionId;
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
};
use std::{
    collections::{BTreeSet, VecDeque},
    ops::Range,
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthStr;

/// The rows reserved for toasts below the selector (owner decision on
/// #115): the selector's area shrinks by them.
pub(crate) const LANE: u16 = 4;
/// The toast that follows a full queue (owner decision on #115).
pub(crate) const MORE: &str = "More events occurred. Check sessions list";
/// How many toasts the queue holds, the one shown among them.
const QUEUED: usize = 3;
/// The accepted visual timings (selector document, "Toasts and bell").
const DRAW: Duration = Duration::from_millis(400);
const HOLD: Duration = Duration::from_millis(2000);
const FADE: Duration = Duration::from_millis(300);
const ERASE: Duration = Duration::from_millis(400);
/// A toast is at most this percentage of the host's width.
const WIDTH_PERCENT: u16 = 40;
/// The border and one blank cell on each side of the text.
const PADDING: u16 = 4;
/// The border's glyphs, by row (top, middle, bottom) and column (left,
/// middle, right).
const GLYPHS: [[&str; 3]; 3] = [["┌", "─", "┐"], ["│", " ", "│"], ["└", "─", "┘"]];

/// What one toast says: its session's label, or [`MORE`], and the
/// prefix+j hint a needs-attention toast carries while key hints are on.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Content {
    label: String,
    hint: Option<String>,
}

/// The toast on screen.
struct Shown {
    content: Content,
    began: Instant,
    /// How long after it began the frame showing its full title finished
    /// writing to the host: its hold starts then.
    titled: Option<Duration>,
    /// How long after it began it was dismissed.
    dismissed: Option<Duration>,
}

/// The toasts of one monitor run.
pub(crate) struct Toasts {
    /// The prefix+j hint, unless key hints are off.
    hint: Option<String>,
    /// What each session already announced: toasted, or a final outcome
    /// first learned while viewed.
    ledger: BTreeSet<(SessionId, News)>,
    waiting: VecDeque<Content>,
    /// The [`MORE`] toast follows the waiting ones.
    more: bool,
    shown: Option<Shown>,
}

impl Toasts {
    /// Needs-attention toasts carry `hint`, unless it is none.
    pub(crate) fn new(hint: Option<String>) -> Self {
        Self {
            hint,
            ledger: BTreeSet::new(),
            waiting: VecDeque::new(),
            more: false,
            shown: None,
        }
    }

    /// `id`, labelled `label`, announced `news` while this monitor `viewed`
    /// it or not. Each is toasted at most once per run. A final outcome
    /// learned while viewed is never toasted; an episode first seen while
    /// viewed stays eligible, to be announced again unviewed once a menu
    /// opens.
    pub(crate) fn announce(&mut self, id: &SessionId, news: News, label: String, viewed: bool) {
        let attention = matches!(news, News::Attention(_));
        let key = (id.clone(), news);
        if (viewed && attention) || self.ledger.contains(&key) {
            return;
        }
        self.ledger.insert(key);
        let hint = self.hint.clone().filter(|_| attention);
        if !viewed {
            self.queue(Content { label, hint });
        }
    }

    /// A toast queues behind at most two others, the shown one included,
    /// so however frames batch arrivals the same toasts show; past them,
    /// and until it shows, the [`MORE`] toast stands for every later one.
    fn queue(&mut self, content: Content) {
        let queued = self.waiting.len() + usize::from(self.shown.is_some());
        if self.more || queued == QUEUED {
            self.more = true;
        } else {
            self.waiting.push_back(content);
        }
    }

    /// Advances to `now` in `area`, the rows below the top row: a toast
    /// that has disappeared goes, and the next one begins drawing if none
    /// shows and it fits.
    pub(crate) fn frame(&mut self, now: Instant, area: Rect) {
        self.shown = self.shown.take().filter(|shown| !shown.ended(now));
        if self.shown.is_some() {
            return;
        }
        let next = self.waiting.front().cloned();
        let next = next.or_else(|| self.more.then(more));
        let Some(content) = next.filter(|content| place(content, area).is_some()) else {
            return;
        };
        if self.waiting.pop_front().is_none() {
            self.more = false;
        }
        self.shown = Some(Shown {
            content,
            began: now,
            titled: None,
            dismissed: None,
        });
    }

    /// Whether the frame drawn at `now` in `area` is the first to show the
    /// shown toast's full title: once it has been written to the host, the
    /// hold starts and the bell rings ([`Toasts::titled`]).
    pub(crate) fn titling(&self, now: Instant, area: Rect) -> bool {
        let shown = self.shown.as_ref().filter(|_| self.rect(area).is_some());
        shown.is_some_and(|shown| shown.titled.is_none() && shown.text(now) == Text::Shown)
    }

    /// The frame showing the shown toast's full title finished writing to
    /// the host at `at`: its hold starts then, once.
    pub(crate) fn titled(&mut self, at: Instant) {
        if let Some(shown) = &mut self.shown {
            let after = at.saturating_duration_since(shown.began);
            shown.titled.get_or_insert(after);
        }
    }

    /// The cells the shown toast occupies in `area`, if one shows there.
    fn rect(&self, area: Rect) -> Option<Rect> {
        place(&self.shown.as_ref()?.content, area)
    }

    /// Whether the shown toast covers any cell of `canvas` in `area`.
    pub(crate) fn covers(&self, canvas: Rect, area: Rect) -> bool {
        self.rect(area)
            .is_some_and(|rect| rect.intersects(canvas.intersection(area)))
    }

    /// Whether the host cell `column`, `row` (from 0) is on the shown toast.
    fn hit(&self, (column, row): (u16, u16), area: Rect) -> bool {
        self.rect(area)
            .is_some_and(|rect| rect.contains(Position::new(column, row)))
    }

    /// Whether the shown toast takes `mouse` at `now` (spec #73, "Modals and
    /// mouse"): a report outside any gesture (`idle`) that lands on it, so
    /// a toast never takes a gesture begun before it appeared. A click it
    /// takes only dismisses it.
    pub(crate) fn take(&mut self, mouse: Mouse, idle: bool, area: Rect, now: Instant) -> bool {
        let taken = idle && self.hit(mouse.host_cell(), area);
        if taken && mouse.gesture() == Gesture::Click {
            self.dismiss(now);
        }
        taken
    }

    /// Dismisses the shown toast at `now` if it covers `canvas` in `area`:
    /// what prefix+j and Enter do for their target (owner decision on
    /// #114).
    pub(crate) fn dismiss_over(&mut self, canvas: Rect, area: Rect, now: Instant) {
        if self.covers(canvas, area) {
            self.dismiss(now);
        }
    }

    /// Dismisses the shown toast at `now`: its text fades, or, before its
    /// title has been shown, what was drawn is erased at once.
    fn dismiss(&mut self, now: Instant) {
        if let Some(shown) = &mut self.shown {
            let at = now.saturating_duration_since(shown.began);
            shown.dismissed.get_or_insert(at);
        }
    }

    /// Draws the shown toast as it looks at `now` in `area`.
    pub(crate) fn render(&self, now: Instant, area: Rect, buffer: &mut Buffer) {
        let shown = self.shown.as_ref();
        let Some((shown, rect)) = shown.zip(self.rect(area)) else {
            return;
        };
        let look = look(shown.elapsed(now), shown.schedule(), perimeter(rect));
        draw(&shown.content, rect, &look, buffer);
    }
}

impl Shown {
    fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.began)
    }

    fn schedule(&self) -> Schedule {
        schedule(self.titled, self.dismissed)
    }

    fn ended(&self, now: Instant) -> bool {
        self.elapsed(now) >= self.schedule().end
    }

    /// How its text shows at `now`.
    fn text(&self, now: Instant) -> Text {
        text(self.elapsed(now), &self.schedule())
    }
}

fn more() -> Content {
    Content {
        label: MORE.to_owned(),
        hint: None,
    }
}

/// Where `content` shows in `area`: flush with its bottom-right corner,
/// three rows high, or four with a hint, as wide as its widest line and
/// padding, but at most [`WIDTH_PERCENT`] of the host's width. None when
/// it has no room for one cell of text.
fn place(content: &Content, area: Rect) -> Option<Rect> {
    let lines = std::iter::once(&content.label).chain(&content.hint);
    let widest = lines.map(|line| line.width()).max().unwrap_or_default();
    let widest = u16::try_from(widest).unwrap_or(u16::MAX);
    let width = widest
        .saturating_add(PADDING)
        .min(area.width * WIDTH_PERCENT / 100);
    let height = 3 + u16::from(content.hint.is_some());
    let fits = width > PADDING && area.height >= height;
    fits.then(|| Rect::new(area.right() - width, area.bottom() - height, width, height))
}

/// When a toast's text fades, its border starts erasing, and it has gone,
/// after it began.
#[derive(Debug, Eq, PartialEq)]
struct Schedule {
    fade: Duration,
    erase: Duration,
    end: Duration,
}

/// The schedule of a toast whose full title was shown when `titled` says
/// and that was dismissed when `dismissed` says: drawn, its title shown
/// until it has been, held from then, faded and erased in turn. Dismissed
/// before its title was shown, it erases what was drawn at once, as fast as
/// it drew it; dismissed while held, its text fades at once. Dismissed
/// later, it was already disappearing.
fn schedule(titled: Option<Duration>, dismissed: Option<Duration>) -> Schedule {
    match (titled, dismissed) {
        (Some(titled), dismissed) => fading(dismissed.unwrap_or(Duration::MAX).min(titled + HOLD)),
        (None, Some(at)) => Schedule {
            fade: at,
            erase: at,
            end: at + at.min(DRAW),
        },
        (None, None) => Schedule {
            fade: Duration::MAX,
            erase: Duration::MAX,
            end: Duration::MAX,
        },
    }
}

fn fading(fade: Duration) -> Schedule {
    Schedule {
        fade,
        erase: fade + FADE,
        end: fade + FADE + ERASE,
    }
}

/// How a toast's text shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Text {
    Hidden,
    Shown,
    Faded,
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
        text: text(elapsed, &schedule),
        filled: elapsed < schedule.erase,
    }
}

/// How a toast's text shows `elapsed` after it began, by `schedule`: from
/// when its border is drawn until it fades, then faded until its border
/// starts erasing.
fn text(elapsed: Duration, schedule: &Schedule) -> Text {
    if !(DRAW..schedule.erase).contains(&elapsed) {
        Text::Hidden
    } else if elapsed < schedule.fade {
        Text::Shown
    } else {
        Text::Faded
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

/// Draws `content` in `rect` as `look` says, yellow.
fn draw(content: &Content, rect: Rect, look: &Look, buffer: &mut Buffer) {
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
        write(content, rect, style, buffer);
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

/// Writes `content`'s lines inside `rect`'s border, in `style`, each after
/// one blank cell and truncated with `…` before the last.
fn write(content: &Content, rect: Rect, style: Style, buffer: &mut Buffer) {
    let lines = std::iter::once(&content.label).chain(&content.hint);
    let room = usize::from(rect.width - PADDING);
    for (y, line) in (rect.y + 1..).zip(lines) {
        buffer.set_stringn(rect.x + 2, y, truncate(line, room), room, style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Whether a toast begins in the frame at `now`.
    fn begins(toasts: &mut Toasts, now: Instant) -> bool {
        let before = toasts.shown.as_ref().map(|shown| shown.began);
        toasts.frame(now, area(120, 30));
        let after = toasts.shown.as_ref().map(|shown| shown.began);
        after.is_some() && after != before
    }

    /// The label of each toast shown, in turn, until none is left, each
    /// title written as soon as it is drawn.
    fn shown_in_turn(toasts: &mut Toasts, start: Instant) -> Vec<String> {
        let mut labels = Vec::new();
        let mut now = start;
        while begins(toasts, now) {
            labels.push(toasts.shown.as_ref().unwrap().content.label.clone());
            toasts.titled(now + DRAW);
            now += ms(3100);
        }
        labels
    }

    /// NOT-01 (pure supplement): each attention episode and each final
    /// outcome is toasted at most once per session for the whole run, so a
    /// repeat, as a reconnection brings, toasts nothing; the same episode
    /// of another session toasts. An episode first seen while viewed is not
    /// toasted but stays eligible, toasting once announced unviewed, as a
    /// menu opening announces it; a final outcome learned while viewed
    /// never toasts. Needs-attention toasts carry the hint, final ones do
    /// not.
    #[test]
    fn each_episode_and_final_outcome_toasts_once_unless_viewed() {
        let mut toasts = Toasts::new(Some(HINT.to_owned()));
        let (alpha, beta) = (id(1), id(2));
        let announce = |toasts: &mut Toasts, id: &SessionId, news, viewed| {
            toasts.announce(id, news, format!("{news:?} {id}"), viewed);
        };
        announce(&mut toasts, &alpha, News::Attention(1), false);
        announce(&mut toasts, &alpha, News::Attention(1), false);
        announce(&mut toasts, &beta, News::Attention(1), false);
        announce(&mut toasts, &alpha, News::Attention(2), true);
        announce(&mut toasts, &alpha, News::Final, true);
        announce(&mut toasts, &alpha, News::Final, false);
        let first: Vec<_> = toasts.waiting.iter().cloned().collect();
        let hinted = |label: &str| Content {
            label: label.to_owned(),
            hint: Some(HINT.to_owned()),
        };
        assert_eq!(
            first,
            [
                hinted(&format!("Attention(1) {alpha}")),
                hinted(&format!("Attention(1) {beta}")),
            ]
        );
        announce(&mut toasts, &alpha, News::Attention(2), false);
        announce(&mut toasts, &alpha, News::Attention(2), false);
        announce(&mut toasts, &beta, News::Final, false);
        let labels = shown_in_turn(&mut toasts, Instant::now());
        assert_eq!(
            labels,
            [
                format!("Attention(1) {alpha}"),
                format!("Attention(1) {beta}"),
                format!("Attention(2) {alpha}"),
                MORE.to_owned(),
            ]
        );
        let mut unhinted = Toasts::new(None);
        announce(&mut unhinted, &beta, News::Final, false);
        announce(&mut unhinted, &alpha, News::Attention(3), false);
        let hints: Vec<_> = unhinted.waiting.iter().map(|toast| &toast.hint).collect();
        assert_eq!(hints, [&None, &None]);
        let mut finals = Toasts::new(Some(HINT.to_owned()));
        announce(&mut finals, &beta, News::Final, false);
        assert_eq!(finals.waiting[0].hint, None);
    }

    /// The label of the toast shown at `now`, if one began then; its title
    /// is written as soon as it is drawn.
    fn began(toasts: &mut Toasts, now: Instant) -> Option<String> {
        let began = begins(toasts, now);
        toasts.titled(now + DRAW);
        began.then(|| toasts.shown.as_ref().unwrap().content.label.clone())
    }

    fn episode(toasts: &mut Toasts, episode: u64) {
        let label = format!("E{episode}");
        toasts.announce(&id(1), News::Attention(episode), label, false);
    }

    /// NOT-02 (pure supplement; owner decision on #115): the queue holds
    /// three toasts, the one shown among them, so the same toasts show
    /// whether frames meet arrivals together or one by one: a fourth makes
    /// the `More events occurred` toast follow the three, every later one
    /// coalescing into it, and once it shows toasts queue again. A toast
    /// begins only in a frame where none shows, one per frame.
    #[test]
    fn the_queue_holds_three_toasts_and_one_more_stands_for_the_rest() {
        let start = Instant::now();
        let mut together = Toasts::new(None);
        for number in 1..=5 {
            episode(&mut together, number);
        }
        let mut apart = Toasts::new(None);
        episode(&mut apart, 1);
        assert_eq!(began(&mut apart, start).as_deref(), Some("E1"));
        assert_eq!(began(&mut apart, start), None);
        episode(&mut apart, 2);
        episode(&mut apart, 3);
        assert!(!apart.more);
        episode(&mut apart, 4);
        assert!(apart.more);
        episode(&mut apart, 5);
        let shown = |toasts: &mut Toasts, first: Instant| {
            let times = (0..5).map(|turn| first + ms(3100) * turn);
            times.map(|now| began(toasts, now)).collect::<Vec<_>>()
        };
        let expected = [Some("E2".to_owned()), Some("E3".to_owned())];
        let after = shown(&mut apart, start + ms(3100));
        assert_eq!(after[..2], expected);
        assert_eq!(after[2..], [Some(MORE.to_owned()), None, None]);
        let all = shown(&mut together, start);
        assert_eq!(all[0].as_deref(), Some("E1"));
        assert_eq!(all[1..3], expected);
        assert_eq!(all[3..], [Some(MORE.to_owned()), None]);
        let mut again = Toasts::new(None);
        for number in 1..=4 {
            episode(&mut again, number);
        }
        assert_eq!(began(&mut again, start).as_deref(), Some("E1"));
        assert_eq!(began(&mut again, start + ms(3099)), None);
        assert_eq!(began(&mut again, start + ms(3100)).as_deref(), Some("E2"));
        assert_eq!(began(&mut again, start + ms(6200)).as_deref(), Some("E3"));
        let more = start + ms(9300);
        assert_eq!(began(&mut again, more).as_deref(), Some(MORE));
        episode(&mut again, 5);
        assert_eq!(began(&mut again, more + ms(3100)).as_deref(), Some("E5"));
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

    /// NOT-02 (pure supplement; owner decision on #115, 2026-11-17): the
    /// hold starts when the frame first showing the full title has been
    /// written, however late: until then the title stays, and the toast
    /// never ends. Only that first frame is titling, never one before the
    /// border is drawn, a later one, a redraw or a resize, nor one where
    /// the toast does not fit or none shows; a later write never moves
    /// the hold.
    #[test]
    fn the_hold_starts_when_the_title_has_been_written() {
        let mut toasts = Toasts::new(None);
        let start = Instant::now();
        let host = area(120, 30);
        assert!(!toasts.titling(start + DRAW, host));
        toasts.announce(&id(1), News::Final, "Build".to_owned(), false);
        toasts.frame(start, host);
        assert!(!toasts.titling(start + ms(399), host));
        assert!(toasts.titling(start + DRAW, host));
        assert!(!toasts.titling(start + DRAW, area(12, 30)));
        let late = start + ms(5000);
        toasts.frame(late, host);
        let shown = toasts.shown.as_ref().unwrap();
        assert_eq!(shown.text(late), Text::Shown);
        assert!(toasts.titling(late, host));
        toasts.titled(late);
        toasts.titled(late + ms(1000));
        assert!(!toasts.titling(late + ms(1), host));
        assert!(!toasts.titling(late + ms(1), area(100, 20)));
        let shown = toasts.shown.as_ref().unwrap();
        assert_eq!(shown.titled, Some(ms(5000)));
        assert_eq!(shown.text(late + ms(1999)), Text::Shown);
        assert_eq!(shown.text(late + HOLD), Text::Faded);
        assert_eq!(shown.text(late + ms(2300)), Text::Hidden);
        toasts.frame(late + ms(2699), host);
        assert!(toasts.shown.is_some());
        toasts.frame(late + ms(2700), host);
        assert!(toasts.shown.is_none());
    }

    /// NOT-02 (pure supplement; owner decision on #115, 2026-11-17): a
    /// toast dismissed before its title was shown never titles, so never
    /// rings: dismissed while drawing, or once drawn but before the frame
    /// with its title was written, it erases what was drawn, as fast as it
    /// drew it.
    #[test]
    fn a_toast_dismissed_before_its_title_never_titles() {
        let host = area(120, 30);
        let start = Instant::now();
        for (dismissed, end) in [(200, 400), (450, 850)] {
            let mut toasts = Toasts::new(None);
            toasts.announce(&id(1), News::Final, "Build".to_owned(), false);
            toasts.frame(start, host);
            toasts.dismiss(start + ms(dismissed));
            for later in [dismissed, 400.max(dismissed), 600] {
                assert!(!toasts.titling(start + ms(later), host), "{later}");
            }
            assert_eq!(toasts.shown.as_ref().unwrap().schedule().end, ms(end));
        }
        let titled = schedule(Some(ms(400)), Some(ms(450)));
        assert_eq!(titled, fading(ms(450)));
    }

    /// NOT-02 (pure supplement): an early dismissal while the border draws
    /// erases only what was drawn, from then, as fast as it drew; one while
    /// the text holds fades it at once, then erases; one while it fades or
    /// erases changes nothing.
    #[test]
    fn an_early_dismissal_erases_only_what_was_drawn() {
        let drawing = schedule(None, Some(ms(200)));
        assert_eq!(
            drawing,
            Schedule {
                fade: ms(200),
                erase: ms(200),
                end: ms(400),
            }
        );
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
        assert_eq!(schedule(None, Some(ms(399))).end, ms(798));
        assert_eq!(titled(400), fading(ms(400)));
        assert_eq!(titled(1000).end, ms(1700));
        let held = |millis| look(ms(millis), titled(1000), 20);
        assert_eq!(held(999).text, Text::Shown);
        assert_eq!(held(1000).text, Text::Faded);
        assert_eq!(held(1300).text, Text::Hidden);
        assert_eq!(titled(2399).end, ms(3099));
        for late in [2400, 2900, 5000] {
            assert_eq!(titled(late), schedule(Some(DRAW), None));
        }
    }

    /// NOT-02 (pure supplement): a dismissal counts from when the toast
    /// began, only the first one counts, and with no toast shown nothing
    /// happens; a dismissed toast ends early, and the next begins then.
    #[test]
    fn a_dismissed_toast_ends_early_and_the_next_begins() {
        let mut toasts = Toasts::new(None);
        let start = Instant::now();
        toasts.dismiss(start);
        for episode in 1..=2 {
            toasts.announce(
                &id(1),
                News::Attention(episode),
                format!("E{episode}"),
                false,
            );
        }
        assert!(begins(&mut toasts, start));
        toasts.dismiss(start + ms(100));
        toasts.dismiss(start + ms(150));
        assert!(!begins(&mut toasts, start + ms(199)));
        assert!(begins(&mut toasts, start + ms(200)));
        assert_eq!(toasts.shown.as_ref().unwrap().content.label, "E2");
    }

    fn hinted(label: &str) -> Content {
        Content {
            label: label.to_owned(),
            hint: Some(HINT.to_owned()),
        }
    }

    fn plain(label: &str) -> Content {
        Content {
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
        toasts.frame(start, area(12, 30));
        assert!(toasts.shown.is_none());
        toasts.frame(start, area(13, 30));
        assert!(toasts.shown.is_some());
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
        toasts.render(start + ms(elapsed), area(20, 6), &mut buffer);
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
        toasts.frame(start, area(20, 6));
        toasts.titled(start + DRAW);
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
        none.frame(start, area(20, 6));
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
    /// cells are hits; with no toast shown, nothing is covered or hit.
    #[test]
    fn the_shown_toast_covers_and_takes_only_its_own_cells() {
        let mut toasts = Toasts::new(None);
        let host = area(120, 30);
        let canvas = Rect::new(0, 1, 120, 29);
        assert!(!toasts.covers(canvas, host));
        assert!(!toasts.hit((119, 29), host));
        toasts.announce(&id(1), News::Final, "Build".to_owned(), false);
        toasts.frame(Instant::now(), host);
        assert!(toasts.covers(canvas, host));
        assert!(toasts.covers(Rect::new(0, 1, 112, 27), host));
        assert!(!toasts.covers(Rect::new(0, 1, 111, 29), host));
        assert!(!toasts.covers(Rect::new(0, 1, 120, 26), host));
        assert!(toasts.hit((111, 27), host));
        assert!(toasts.hit((119, 29), host));
        assert!(!toasts.hit((110, 29), host));
        assert!(!toasts.hit((119, 26), host));
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
        let mut toasts = Toasts::new(None);
        let host = area(120, 30);
        let start = Instant::now();
        let click = report(0, 120, 30, false);
        assert!(!toasts.take(click, true, host, start));
        toasts.announce(&id(1), News::Final, "Build".to_owned(), false);
        toasts.frame(start, host);
        let now = start + ms(1000);
        assert!(!toasts.take(click, false, host, now));
        assert!(!toasts.take(report(0, 111, 30, false), true, host, now));
        for other in [report(0, 120, 30, true), report(64, 120, 30, false)] {
            assert!(toasts.take(other, true, host, now));
        }
        assert_eq!(toasts.shown.as_ref().unwrap().dismissed, None);
        assert!(toasts.take(report(2, 112, 28, false), true, host, now));
        assert_eq!(toasts.shown.as_ref().unwrap().dismissed, None);
        assert!(toasts.take(report(0, 112, 28, false), true, host, now));
        assert_eq!(toasts.shown.as_ref().unwrap().dismissed, Some(ms(1000)));
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
        toasts.frame(start, host);
        toasts.dismiss_over(canvas(host, Some((111, 29))), host, start + ms(500));
        assert_eq!(toasts.shown.as_ref().unwrap().dismissed, None);
        toasts.dismiss_over(canvas(host, Some((112, 27))), host, start + ms(600));
        assert_eq!(toasts.shown.as_ref().unwrap().dismissed, Some(ms(600)));
    }

    /// TRM-01 (pure supplement): a wide character that the toast's left
    /// edge would cut is blanked, so the border shows whole; a narrow one
    /// stays.
    #[test]
    fn a_wide_character_cut_by_the_toast_is_blanked() {
        let mut toasts = Toasts::new(None);
        toasts.announce(&id(1), News::Final, "Build".to_owned(), false);
        let start = Instant::now();
        toasts.frame(start, area(20, 6));
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 6));
        buffer.set_string(11, 3, "界", Style::new());
        buffer.set_string(11, 4, "a", Style::new());
        toasts.render(start + ms(400), area(20, 6), &mut buffer);
        assert_eq!(buffer[(11, 3)].symbol(), " ");
        assert_eq!(buffer[(11, 4)].symbol(), "a");
        assert_eq!(buffer[(12, 3)].symbol(), "┌");
    }
}
