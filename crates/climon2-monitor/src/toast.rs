//! The toast queue (spec #73, "Presentation (Toasts)" and "Toast
//! eligibility"; #115): pure. A toast announces an attention episode or a
//! final outcome once per run, never while this monitor views the session
//! it is about: the ledger, keyed per session by episode or the one final
//! outcome, lasts the whole run. The queue holds three toasts, the one
//! shown among them (owner decision on #115, 2026-11-16), then one more
//! toast stands for everything after them.
//! One toast shows at a time: drawn, its label held, faded, and erased. The
//! hold, and the toast's one bell, start once the frame showing its full
//! title has been written to the consumer's device (owner decision on #115,
//! 2026-11-17). Where a toast sits, whether it fits and what it covers are
//! its consumer's to say (ADR 0011): the queue asks, and never places one.
use crate::registry::News;
use climon2_core::SessionId;
use std::{
    collections::{BTreeSet, VecDeque},
    time::{Duration, Instant},
};

/// The toast that follows a full queue (owner decision on #115).
pub const MORE: &str = "More events occurred. Check sessions list";
/// How many toasts the queue holds, the one shown among them.
const QUEUED: usize = 3;
/// The accepted visual timings (selector document, "Toasts and bell").
pub const DRAW: Duration = Duration::from_millis(400);
const HOLD: Duration = Duration::from_millis(2000);
const FADE: Duration = Duration::from_millis(300);
pub const ERASE: Duration = Duration::from_millis(400);

/// What one toast says: its session's label, or [`MORE`], and the
/// prefix+j hint a needs-attention toast carries while key hints are on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Toast {
    pub label: String,
    pub hint: Option<String>,
}

/// The toast on screen.
struct Shown {
    content: Toast,
    began: Instant,
    /// How long after it began the frame showing its full title finished
    /// writing to the host: its hold starts then.
    titled: Option<Duration>,
    /// How long after it began it was dismissed.
    dismissed: Option<Duration>,
}

/// The toasts of one monitor run.
pub struct Toasts {
    /// The prefix+j hint, unless key hints are off.
    hint: Option<String>,
    /// What each session already announced: toasted, or a final outcome
    /// first learned while viewed.
    ledger: BTreeSet<(SessionId, News)>,
    waiting: VecDeque<Toast>,
    /// The [`MORE`] toast follows the waiting ones.
    more: bool,
    shown: Option<Shown>,
}

/// The shown toast as it is at one moment, for a consumer to draw.
pub struct Showing<'a> {
    pub toast: &'a Toast,
    /// How long ago it began.
    pub elapsed: Duration,
    pub schedule: Schedule,
}

impl Toasts {
    /// Needs-attention toasts carry `hint`, unless it is none.
    pub fn new(hint: Option<String>) -> Self {
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
    pub fn announce(&mut self, id: &SessionId, news: News, label: String, viewed: bool) {
        let attention = matches!(news, News::Attention(_));
        let key = (id.clone(), news);
        if (viewed && attention) || self.ledger.contains(&key) {
            return;
        }
        self.ledger.insert(key);
        let hint = self.hint.clone().filter(|_| attention);
        if !viewed {
            self.queue(Toast { label, hint });
        }
    }

    /// A toast queues behind at most two others, the shown one included,
    /// so however frames batch arrivals the same toasts show; past them,
    /// and until it shows, the [`MORE`] toast stands for every later one.
    fn queue(&mut self, content: Toast) {
        let queued = self.waiting.len() + usize::from(self.shown.is_some());
        if self.more || queued == QUEUED {
            self.more = true;
        } else {
            self.waiting.push_back(content);
        }
    }

    /// Advances to `now`: a toast that has disappeared goes, and the next
    /// one begins drawing if none shows and its consumer says it `fits`.
    pub fn advance(&mut self, now: Instant, fits: impl Fn(&Toast) -> bool) {
        self.shown = self.shown.take().filter(|shown| !shown.ended(now));
        if self.shown.is_some() {
            return;
        }
        let next = self.waiting.front().cloned();
        let next = next.or_else(|| self.more.then(more));
        let Some(content) = next.filter(fits) else {
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

    /// The shown toast, by when it began, while the frame drawn at `now`
    /// shows its full title for the first time, which only a frame it is
    /// `placed` in can: once that frame has been written to the host, the
    /// hold starts and the bell rings ([`Toasts::titled`]).
    pub fn titling(&self, now: Instant, placed: bool) -> Option<Instant> {
        let shown = self.shown.as_ref().filter(|_| placed);
        let titling =
            shown.filter(|shown| shown.titled.is_none() && shown.text(now) == Text::Shown);
        titling.map(|shown| shown.began)
    }

    /// A frame showing the full title of the toast that began at `toast`
    /// finished writing to the host at `at`: its hold starts then, once,
    /// unless it was dismissed first. Whether it did, so the bell rings.
    pub fn titled(&mut self, toast: Instant, at: Instant) -> bool {
        let shown = self.shown.as_mut().filter(|shown| shown.began == toast);
        let Some(shown) = shown.filter(|shown| shown.titled.is_none() && shown.dismissed.is_none())
        else {
            return false;
        };
        shown.titled = Some(at.saturating_duration_since(shown.began));
        true
    }

    /// The shown toast, if one shows.
    pub fn shown(&self) -> Option<&Toast> {
        self.shown.as_ref().map(|shown| &shown.content)
    }

    /// The shown toast as it is at `now`, if one shows.
    pub fn showing(&self, now: Instant) -> Option<Showing<'_>> {
        self.shown.as_ref().map(|shown| Showing {
            toast: &shown.content,
            elapsed: shown.elapsed(now),
            schedule: shown.schedule(),
        })
    }

    /// Dismisses the shown toast at `now`: its text fades, or, before its
    /// title has been shown, what was drawn is erased at once.
    pub fn dismiss(&mut self, now: Instant) {
        if let Some(shown) = &mut self.shown {
            let at = now.saturating_duration_since(shown.began);
            shown.dismissed.get_or_insert(at);
        }
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

fn more() -> Toast {
    Toast {
        label: MORE.to_owned(),
        hint: None,
    }
}

/// When a toast's text fades, its border starts erasing, and it has gone,
/// after it began.
#[derive(Debug, Eq, PartialEq)]
pub struct Schedule {
    pub fade: Duration,
    pub erase: Duration,
    pub end: Duration,
}

/// The schedule of a toast whose full title was shown when `titled` says
/// and that was dismissed when `dismissed` says: drawn, its title shown
/// until it has been, held from then, faded and erased in turn. Dismissed
/// before its title was shown, it erases what was drawn at once, as fast as
/// it drew it; dismissed while held, its text fades at once. Dismissed
/// later, it was already disappearing.
pub fn schedule(titled: Option<Duration>, dismissed: Option<Duration>) -> Schedule {
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
pub enum Text {
    Hidden,
    Shown,
    Faded,
}

/// How a toast's text shows `elapsed` after it began, by `schedule`: from
/// when its border is drawn until it fades, then faded until its border
/// starts erasing.
pub fn text(elapsed: Duration, schedule: &Schedule) -> Text {
    if !(DRAW..schedule.erase).contains(&elapsed) {
        Text::Hidden
    } else if elapsed < schedule.fade {
        Text::Shown
    } else {
        Text::Faded
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

    /// More toasts than any test queues: the waiting ones, then [`MORE`].
    const SHOWN_AT_MOST: usize = 8;

    /// The frame showing the shown toast's full title finished writing at
    /// `at`.
    fn title(toasts: &mut Toasts, at: Instant) -> bool {
        let began = toasts.shown.as_ref().unwrap().began;
        toasts.titled(began, at)
    }

    /// Whether a toast begins in the frame at `now`.
    fn begins(toasts: &mut Toasts, now: Instant) -> bool {
        let before = toasts.shown.as_ref().map(|shown| shown.began);
        toasts.advance(now, |_| true);
        let after = toasts.shown.as_ref().map(|shown| shown.began);
        after.is_some() && after != before
    }

    /// The label of each toast shown, in turn, until none is left, each
    /// title written as soon as it is drawn. It stops after [`SHOWN_AT_MOST`],
    /// so a queue that never empties fails its test instead of growing
    /// without bound.
    fn shown_in_turn(toasts: &mut Toasts, start: Instant) -> Vec<String> {
        let mut labels = Vec::new();
        let mut now = start;
        while labels.len() < SHOWN_AT_MOST && begins(toasts, now) {
            labels.push(toasts.shown.as_ref().unwrap().content.label.clone());
            title(toasts, now + DRAW);
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
        let hinted = |label: &str| Toast {
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
        if began {
            title(toasts, now + DRAW);
        }
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
        // The consumer places the toast.
        let host = true;
        assert_eq!(toasts.titling(start + DRAW, host), None);
        assert!(!toasts.titled(start, start + DRAW));
        toasts.announce(&id(1), News::Final, "Build".to_owned(), false);
        toasts.advance(start, |_| host);
        assert_eq!(toasts.titling(start + ms(399), host), None);
        assert_eq!(toasts.titling(start + DRAW, host), Some(start));
        assert_eq!(toasts.titling(start + DRAW, false), None);
        let late = start + ms(5000);
        toasts.advance(late, |_| host);
        let shown = toasts.shown.as_ref().unwrap();
        assert_eq!(shown.text(late), Text::Shown);
        assert_eq!(toasts.titling(late, host), Some(start));
        assert!(!toasts.titled(start + ms(1), late));
        assert!(toasts.titled(start, late));
        assert!(!toasts.titled(start, late + ms(1000)));
        assert_eq!(toasts.titling(late + ms(1), host), None);
        assert_eq!(toasts.titling(late + ms(1), host), None);
        let shown = toasts.shown.as_ref().unwrap();
        assert_eq!(shown.titled, Some(ms(5000)));
        assert_eq!(shown.text(late + ms(1999)), Text::Shown);
        assert_eq!(shown.text(late + HOLD), Text::Faded);
        assert_eq!(shown.text(late + ms(2300)), Text::Hidden);
        toasts.advance(late + ms(2699), |_| host);
        assert!(toasts.shown.is_some());
        toasts.advance(late + ms(2700), |_| host);
        assert!(toasts.shown.is_none());
    }

    /// NOT-02 (pure supplement; owner decision on #115, 2026-11-17): a
    /// toast dismissed before its title was shown never titles, so never
    /// rings: dismissed while drawing, or once drawn but before the frame
    /// with its title was written, it erases what was drawn, as fast as it
    /// drew it.
    #[test]
    fn a_toast_dismissed_before_its_title_never_titles() {
        // The consumer places the toast.
        let host = true;
        let start = Instant::now();
        for (dismissed, end) in [(200, 400), (450, 850)] {
            let mut toasts = Toasts::new(None);
            toasts.announce(&id(1), News::Final, "Build".to_owned(), false);
            toasts.advance(start, |_| host);
            toasts.dismiss(start + ms(dismissed));
            for later in [dismissed, 400.max(dismissed), 600] {
                assert_eq!(toasts.titling(start + ms(later), host), None, "{later}");
            }
            // A frame with its title, drawn before the dismissal, finished
            // writing after it.
            assert!(!toasts.titled(start, start + ms(600)));
            assert_eq!(toasts.shown.as_ref().unwrap().schedule().end, ms(end));
        }
        let titled = schedule(Some(ms(400)), Some(ms(450)));
        assert_eq!(titled, fading(ms(450)));
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

    /// NOT-02 (pure supplement): the accepted timings, at each boundary and
    /// one before it, for a title written as soon as it is drawn: no text
    /// while the border draws for 400 ms, the text then holds for 2,000 ms,
    /// fades for 300 ms, and is hidden while the border erases for 400 ms.
    #[test]
    fn a_toast_shows_holds_and_fades_its_text_on_time() {
        let at = |millis| text(ms(millis), &schedule(Some(DRAW), None));
        assert_eq!(at(0), Text::Hidden);
        assert_eq!(at(399), Text::Hidden);
        assert_eq!(at(400), Text::Shown);
        assert_eq!(at(2399), Text::Shown);
        assert_eq!(at(2400), Text::Faded);
        assert_eq!(at(2699), Text::Faded);
        assert_eq!(at(2700), Text::Hidden);
        assert_eq!(
            schedule(Some(DRAW), None),
            Schedule {
                fade: ms(2400),
                erase: ms(2700),
                end: ms(3100),
            }
        );
    }

    /// NOT-02 (pure supplement): an early dismissal while the border draws
    /// erases only what was drawn, from then, as fast as it drew; one while
    /// the text holds fades it at once, then erases; one while it fades or
    /// erases changes nothing.
    #[test]
    fn an_early_dismissal_ends_a_toast_early() {
        let drawing = schedule(None, Some(ms(200)));
        assert_eq!(
            drawing,
            Schedule {
                fade: ms(200),
                erase: ms(200),
                end: ms(400),
            }
        );
        let titled = |dismissed| schedule(Some(DRAW), Some(ms(dismissed)));
        assert_eq!(schedule(None, Some(ms(399))).end, ms(798));
        assert_eq!(titled(400), fading(ms(400)));
        assert_eq!(titled(1000).end, ms(1700));
        let held = |millis| text(ms(millis), &titled(1000));
        assert_eq!(held(999), Text::Shown);
        assert_eq!(held(1000), Text::Faded);
        assert_eq!(held(1300), Text::Hidden);
        assert_eq!(titled(2399).end, ms(3099));
        for late in [2400, 2900, 5000] {
            assert_eq!(titled(late), schedule(Some(DRAW), None));
        }
    }

    /// What a consumer draws: the shown toast, how long ago it began and
    /// its schedule, and nothing while none shows; a toast its consumer
    /// has no room for waits.
    #[test]
    fn the_shown_toast_is_offered_as_it_is_now() {
        let mut toasts = Toasts::new(None);
        let start = Instant::now();
        assert!(toasts.shown().is_none());
        assert!(toasts.showing(start).is_none());
        toasts.announce(&id(1), News::Final, "Build".to_owned(), false);
        toasts.advance(start, |_| false);
        assert!(toasts.shown().is_none());
        toasts.advance(start, |toast| toast.label == "Build");
        assert_eq!(
            toasts.shown().map(|toast| toast.label.as_str()),
            Some("Build")
        );
        toasts.dismiss(start + ms(100));
        let showing = toasts.showing(start + ms(150)).unwrap();
        assert_eq!(showing.toast.label, "Build");
        assert_eq!(showing.elapsed, ms(150));
        assert_eq!(showing.schedule, schedule(None, Some(ms(100))));
    }
}
