//! Daemon-core activity and attention: the two state machines of the
//! [session status mechanics](../../../docs/for-humans/behaviour/session-status.md) and their
//! detectors: heuristic, explicit and fallback.
//!
//! The detector is pure. Every event takes the clock reading it happens at,
//! and the random source is injected, so tests drive it without a session.
//! Call protocol for one daemon turn, which reads the clock once:
//!
//! 1. Feed accepted user input with [`Detector::input`] and explicit
//!    acknowledgement with [`Detector::acknowledge`], in acceptance order.
//!    Input therefore comes before the timer and sample work of the turn.
//! 2. Feed each [`Signal`] of the command's output with
//!    [`Detector::signal`], in stream order, as the output is read.
//! 3. Call [`Detector::poll`] once, after reading the command's output.
//! 4. Report a confirmed command result once with [`Detector::finish`].
//!
//! Each event runs to completion, its attention request included, before it
//! reports the effective status, so no intermediate status is ever reported.
//! An event returns the effective status only when it changed; only that
//! change replaces the stored change time.
//!
//! The detection mode selects the activity detector: heuristic sampling,
//! explicit root OSC 7501 reports, or the fallback grace that measures quiet
//! before heuristics resume; OSC 133 shell markers retire explicit scopes.
//! Recognized notifications (#98) request attention through
//! [`Detector::notify`]; progress never does.
mod assembly;
mod clock;
mod notifications;
mod progress;
mod signals;

pub use assembly::Assembler;
use climon2_core::wire::{CommandResult, Counter, Status, StatusState, Utc};
pub use clock::{Clock, SystemRandom, daemon_clock, first_episode};
pub use notifications::Parsed;
pub use progress::Tracker;
pub use signals::{Interpreted, Marker, QUERY_REPLY, Report, Signal, interpret};
use std::time::Duration;

/// The user-work window each qualifying input re-arms.
pub const USER_WINDOW: Duration = Duration::from_millis(2_000);
/// Agent inactivity before attention is requested; hardcoded, never a setting.
pub const INACTIVITY: Duration = Duration::from_millis(10_000);
/// Viewport quiet the fallback grace measures before heuristics resume;
/// hardcoded, never a setting.
pub const FALLBACK: Duration = Duration::from_millis(10_000);
const FOCUS_IN: &[u8] = b"\x1b[I";
const FOCUS_OUT: &[u8] = b"\x1b[O";

/// Whether accepted Input bytes are user-originated. Focus reports, which a
/// host terminal sends on its own once the application enables focus
/// reporting, are not: bytes that hold only focus reports never count.
pub fn user_originated(mut bytes: &[u8]) -> bool {
    while let Some(rest) = bytes
        .strip_prefix(FOCUS_IN)
        .or_else(|| bytes.strip_prefix(FOCUS_OUT))
    {
        bytes = rest;
    }
    !bytes.is_empty()
}

/// Each sampling delay is a fresh random draw from 500 to 1,500 ms.
const SAMPLE_DELAY_MIN_MS: u64 = 500;
const SAMPLE_DELAY_CHOICES: u64 = 1_001;

/// One clock reading: monotonic time for deadlines, and UTC for reports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Now {
    pub monotonic: Duration,
    pub utc: Utc,
}

/// The injected random source.
pub trait Random {
    fn next(&mut self) -> u64;
}

/// What one sample reads: the active viewport and, separately, the title.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sample {
    pub screen: Screen,
    pub title: Option<String>,
}

/// The active viewport's characters, and its size.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Screen {
    pub cols: u16,
    pub rows: u16,
    pub characters: String,
}

impl Screen {
    fn resized(&self, other: &Self) -> bool {
        (self.cols, self.rows) != (other.cols, other.rows)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Activity {
    Idle,
    /// Heuristic agent work, until its inactivity deadline.
    Agent {
        deadline: Duration,
    },
    /// Agent work a root `working` report gave: it has no deadline.
    Reported,
    /// User work, until its window ends.
    User {
        until: Duration,
    },
}

impl Activity {
    /// Both working states project to `running`.
    fn projected(self) -> StatusState {
        match self {
            Self::Idle => StatusState::Idle,
            _ => StatusState::Running,
        }
    }
}

/// The detection mode, which selects the activity detector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Heuristic,
    /// An explicit scope, and whether its root record says `working`.
    Explicit {
        working: bool,
    },
    /// The fallback grace, until its quiet deadline.
    Fallback {
        deadline: Duration,
    },
}

impl Mode {
    /// The activity no deadline holds: the remembered root `working` of an
    /// explicit scope, otherwise idle.
    fn resting(self) -> Activity {
        if self == (Self::Explicit { working: true }) {
            Activity::Reported
        } else {
            Activity::Idle
        }
    }
}

/// Whether prompt rendering is excluded after `marker`: from `A` until `B`
/// or `C`.
fn excluded_after(marker: Marker, excluded: bool) -> bool {
    match marker {
        Marker::A => true,
        Marker::B | Marker::C => false,
        Marker::D => excluded,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Attention {
    Cleared,
    Pending(Counter),
    Acknowledged(Counter),
}

/// The machines while the command runs: observation stops at its final
/// result.
struct Live {
    activity: Activity,
    attention: Attention,
    mode: Mode,
    /// Prompt rendering, between OSC 133 `A` and `B` or `C`.
    excluded: bool,
    baseline: Sample,
    next_sample: Duration,
}

impl Live {
    /// The effective state and its episode: attention masks activity.
    fn effective(&self) -> (StatusState, Option<Counter>) {
        match self.attention {
            Attention::Cleared => (self.activity.projected(), None),
            Attention::Pending(episode) => (StatusState::NeedsAttention, Some(episode)),
            Attention::Acknowledged(episode) => (StatusState::Acknowledged, Some(episode)),
        }
    }

    /// Accepted user input clears attention and re-arms the user window,
    /// cancelling any agent inactivity deadline.
    fn input(&mut self, now: Duration) {
        self.activity = Activity::User {
            until: now + USER_WINDOW,
        };
        self.attention = Attention::Cleared;
    }

    /// The user window's end, then a due sample; true when it requests
    /// attention.
    fn poll(
        &mut self,
        now: Duration,
        sample: &impl Fn() -> Sample,
        random: &mut impl Random,
    ) -> bool {
        if self.window_ended(now) {
            self.end_window(now, sample, random);
        }
        if !self.sampling_due(now) {
            return false;
        }
        let requested = self.observe(now, sample());
        self.schedule(now, random);
        requested
    }

    fn window_ended(&self, now: Duration) -> bool {
        matches!(self.activity, Activity::User { until } if now >= until)
    }

    /// The window ends in the resting activity; heuristics then take a
    /// fresh baseline.
    fn end_window(
        &mut self,
        now: Duration,
        sample: &impl Fn() -> Sample,
        random: &mut impl Random,
    ) {
        self.activity = self.mode.resting();
        if self.mode == Mode::Heuristic {
            self.rebase(now, sample, random);
        }
    }

    fn sampling_due(&self, now: Duration) -> bool {
        now >= self.next_sample && self.observing()
    }

    /// Heuristics sample outside user work and prompt rendering; explicit
    /// detection never samples; the grace measures quiet throughout.
    fn observing(&self) -> bool {
        match self.mode {
            Mode::Heuristic => !self.excluded && !matches!(self.activity, Activity::User { .. }),
            Mode::Explicit { .. } => false,
            Mode::Fallback { .. } => true,
        }
    }

    fn observe(&mut self, now: Duration, current: Sample) -> bool {
        match self.mode {
            Mode::Fallback { deadline } => {
                self.quiet(now, current, deadline);
                false
            }
            _ => self.infer(now, current),
        }
    }

    /// During the grace, changed content or a resize restarts its quiet
    /// deadline, never agent work; checked first, its expiry then resumes
    /// heuristics with this sample as their baseline.
    fn quiet(&mut self, now: Duration, current: Sample, deadline: Duration) {
        let resized = current.screen.resized(&self.baseline.screen);
        let changed = self.changed(&current, resized) || resized;
        self.baseline = current;
        if changed {
            self.mode = Mode::Fallback {
                deadline: now + FALLBACK,
            };
        } else if now >= deadline {
            self.mode = Mode::Heuristic;
        }
    }

    /// Changed content, or a resize during agent work, (re)starts the agent
    /// inactivity deadline; only then can it expire. The sample is the next
    /// baseline either way.
    fn infer(&mut self, now: Duration, current: Sample) -> bool {
        let resized = current.screen.resized(&self.baseline.screen);
        let changed = self.changed(&current, resized);
        self.baseline = current;
        let working = matches!(self.activity, Activity::Agent { .. });
        if changed || (resized && working) {
            self.activity = Activity::Agent {
                deadline: now + INACTIVITY,
            };
            return false;
        }
        self.expire(now)
    }

    /// A changed title counts on its own; characters are compared only at
    /// the same size.
    fn changed(&self, current: &Sample, resized: bool) -> bool {
        let characters = !resized && current.screen.characters != self.baseline.screen.characters;
        characters || current.title != self.baseline.title
    }

    fn expire(&mut self, now: Duration) -> bool {
        let expired = matches!(self.activity, Activity::Agent { deadline } if now >= deadline);
        if expired {
            self.activity = Activity::Idle;
        }
        expired
    }

    /// A fresh random delay, never beyond an armed agent or grace deadline.
    fn schedule(&mut self, now: Duration, random: &mut impl Random) {
        let drawn = SAMPLE_DELAY_MIN_MS + random.next() % SAMPLE_DELAY_CHOICES;
        let delay = Duration::from_millis(drawn);
        let delay = self
            .deadline()
            .map_or(delay, |deadline| delay.min(deadline.saturating_sub(now)));
        self.next_sample = now + delay;
    }

    fn deadline(&self) -> Option<Duration> {
        match (self.activity, self.mode) {
            (Activity::Agent { deadline }, _) | (_, Mode::Fallback { deadline }) => Some(deadline),
            _ => None,
        }
    }

    /// A fresh baseline, and a fresh sampling iteration from it.
    fn rebase(&mut self, now: Duration, sample: &impl Fn() -> Sample, random: &mut impl Random) {
        self.baseline = sample();
        self.schedule(now, random);
    }

    /// A terminal signal; true when it requests attention.
    fn signal(
        &mut self,
        now: Duration,
        signal: Signal,
        sample: &impl Fn() -> Sample,
        random: &mut impl Random,
    ) -> bool {
        match signal {
            Signal::Root(report) => return self.report(now, report, sample, random),
            Signal::Shell(marker) => self.shell(now, marker, sample, random),
            Signal::FullReset => self.reset(now, sample, random),
        }
        false
    }

    /// A root report replaces the previous root-driven decision from any
    /// live activity, user work included. Every report but `working` and
    /// `clear` requests attention.
    fn report(
        &mut self,
        now: Duration,
        report: Report,
        sample: &impl Fn() -> Sample,
        random: &mut impl Random,
    ) -> bool {
        match report {
            Report::Working => self.explicit(true),
            Report::Error | Report::Clear => self.grace(now, sample, random),
            _ => self.explicit(false),
        }
        !matches!(report, Report::Working | Report::Clear)
    }

    fn explicit(&mut self, working: bool) {
        self.mode = Mode::Explicit { working };
        self.activity = self.mode.resting();
    }

    /// The fallback grace starts or restarts, idle, from a fresh baseline.
    fn grace(&mut self, now: Duration, sample: &impl Fn() -> Sample, random: &mut impl Random) {
        self.mode = Mode::Fallback {
            deadline: now + FALLBACK,
        };
        self.activity = Activity::Idle;
        self.rebase(now, sample, random);
    }

    /// `A`, `C` and `D` retire the scope to idle heuristics; `B` only takes
    /// a fresh baseline. Each also updates the prompt-rendering exclusion.
    fn shell(
        &mut self,
        now: Duration,
        marker: Marker,
        sample: &impl Fn() -> Sample,
        random: &mut impl Random,
    ) {
        if marker != Marker::B {
            self.mode = Mode::Heuristic;
            self.activity = Activity::Idle;
        }
        self.excluded = excluded_after(marker, self.excluded);
        self.rebase(now, sample, random);
    }

    /// A full reset refreshes heuristics' baseline; in explicit or grace
    /// detection it behaves as root `clear`, without its attention rule.
    fn reset(&mut self, now: Duration, sample: &impl Fn() -> Sample, random: &mut impl Random) {
        match self.mode {
            Mode::Heuristic => self.rebase(now, sample, random),
            _ => self.grace(now, sample, random),
        }
    }
}

/// Before the command's final result, the live machines; after it, nothing
/// is observed and no later event changes anything.
enum Phase {
    Live(Live),
    Final,
}

/// The effective status last reported, and when it changed.
struct Reported {
    state: StatusState,
    episode: Option<Counter>,
    changed_at: Utc,
}

impl Reported {
    fn status(&self) -> Status {
        Status::new(self.state, self.changed_at.clone(), self.episode)
            .expect("only attention carries an episode")
    }
}

/// Attention episode identifiers: increasing, never reused, never wrapped.
struct Episodes {
    next: Option<Counter>,
}

impl Episodes {
    fn issue(&mut self) -> Option<Counter> {
        let issued = self.next.take()?;
        self.next = issued.get().checked_add(1).and_then(Counter::new);
        Some(issued)
    }
}

/// Both machines of one session and the effective status they report.
pub struct Detector<R> {
    phase: Phase,
    reported: Reported,
    episodes: Episodes,
    /// A request needed an episode after the last one was issued.
    exhausted: bool,
    random: R,
}

impl<R: Random> Detector<R> {
    /// Idle and cleared at `now`, the daemon's initialization, with the
    /// first sample's baseline taken before any command output. Episodes
    /// start at `first_episode`.
    pub fn new(now: &Now, baseline: Sample, first_episode: Counter, mut random: R) -> Self {
        let mut live = Live {
            activity: Activity::Idle,
            attention: Attention::Cleared,
            mode: Mode::Heuristic,
            excluded: false,
            baseline,
            next_sample: now.monotonic,
        };
        live.schedule(now.monotonic, &mut random);
        Self {
            phase: Phase::Live(live),
            reported: Reported {
                state: StatusState::Idle,
                episode: None,
                changed_at: now.utc.clone(),
            },
            episodes: Episodes {
                next: Some(first_episode),
            },
            exhausted: false,
            random,
        }
    }

    /// The current effective status, with its stored change time.
    pub fn status(&self) -> Status {
        self.reported.status()
    }

    /// Accepted user-originated input from the current ready controller.
    pub fn input(&mut self, now: &Now) -> Option<Status> {
        if let Phase::Live(live) = &mut self.phase {
            live.input(now.monotonic);
        }
        self.settle(now)
    }

    /// The turn's timer and sample work; `sample` is read only when a
    /// sample is due.
    pub fn poll(&mut self, now: &Now, sample: impl Fn() -> Sample) -> Option<Status> {
        self.step(now, |live, random| {
            live.poll(now.monotonic, &sample, random)
        })
    }

    /// A terminal signal, at its place in the command's output; `sample`,
    /// the terminal as of that place, is read only for a fresh baseline.
    pub fn signal(
        &mut self,
        now: &Now,
        signal: Signal,
        sample: impl Fn() -> Sample,
    ) -> Option<Status> {
        self.step(now, |live, random| {
            live.signal(now.monotonic, signal, &sample, random)
        })
    }

    /// Runs `event` on the live machines, applies the attention request it
    /// returns, and reports the result.
    fn step(&mut self, now: &Now, event: impl FnOnce(&mut Live, &mut R) -> bool) -> Option<Status> {
        let Phase::Live(live) = &mut self.phase else {
            return None;
        };
        if event(live, &mut self.random) {
            self.request_attention();
        }
        self.settle(now)
    }

    /// A recognized logical notification: it requests attention, which a
    /// pending episode absorbs, without touching activity, timers or the
    /// detection mode.
    pub fn notify(&mut self, now: &Now) -> Option<Status> {
        self.step(now, |_, _| true)
    }

    /// The episode of pending attention, if any: the one a notification
    /// just requested or joined.
    pub fn pending_episode(&self) -> Option<Counter> {
        (self.reported.state == StatusState::NeedsAttention)
            .then_some(self.reported.episode)
            .flatten()
    }

    /// Accepted explicit acknowledgement of pending attention; nothing
    /// pending changes nothing.
    pub fn acknowledge(&mut self, now: &Now) -> Option<Status> {
        if let Phase::Live(live) = &mut self.phase
            && let Attention::Pending(episode) = live.attention
        {
            live.attention = Attention::Acknowledged(episode);
        }
        self.settle(now)
    }

    /// The confirmed command result: the final status, which Command final
    /// carries. It cancels every deadline and stops observation.
    pub fn finish(&mut self, now: &Now, result: &CommandResult) -> Status {
        self.phase = Phase::Final;
        self.reported = Reported {
            state: result.final_state(),
            episode: None,
            changed_at: now.utc.clone(),
        };
        self.reported.status()
    }

    /// An attention request: a new episode unless one is pending, which
    /// it joins. With the episode identifiers exhausted, none is wrapped or
    /// reused: the domain is marked exhausted, and attention stays as it was.
    pub(crate) fn request_attention(&mut self) {
        let Phase::Live(live) = &mut self.phase else {
            return;
        };
        if matches!(live.attention, Attention::Pending(_)) {
            return;
        }
        match self.episodes.issue() {
            Some(episode) => live.attention = Attention::Pending(episode),
            None => {
                tracing::error!(
                    code = "resource_limit",
                    "attention episode identifiers exhausted"
                );
                self.exhausted = true;
            }
        }
    }

    /// Whether a request found the session's episode identifiers exhausted:
    /// attention work that needs one can no longer proceed.
    pub fn exhausted(&self) -> bool {
        self.exhausted
    }

    /// Reports the effective status once the event ran to completion,
    /// when it changed.
    fn settle(&mut self, now: &Now) -> Option<Status> {
        let Phase::Live(live) = &self.phase else {
            return None;
        };
        let (state, episode) = live.effective();
        if (state, episode) == (self.reported.state, self.reported.episode) {
            return None;
        }
        self.reported = Reported {
            state,
            episode,
            changed_at: now.utc.clone(),
        };
        Some(self.reported.status())
    }
}

// Pure state-machine tests (STA-01 supplement, STA-02, STA-07 and LIM-05
// pure cases): the detector driven by hand-written clock readings and
// random draws, against expectations written from the status mechanics.
#[cfg(test)]
mod tests {
    use super::{Detector, Marker, Now, Random, Report, Sample, Screen, Signal};
    use climon2_core::wire::{CommandResult, Counter, Status, StatusState, Utc};
    use std::{num::NonZeroU32, time::Duration};

    /// The UTC milliseconds of the detector's monotonic zero: 2027-01-15T08:00:00.000Z.
    const BASE: u64 = 1_800_000_000_000;

    /// The clock reading `ms` milliseconds after initialization.
    fn at(ms: u64) -> Now {
        Now {
            monotonic: Duration::from_millis(ms),
            utc: Utc::from_unix_millis(BASE + ms).unwrap(),
        }
    }

    /// Random draws in order, the last one repeating.
    struct Draws(Vec<u64>);

    impl Random for Draws {
        fn next(&mut self) -> u64 {
            if self.0.len() > 1 {
                self.0.remove(0)
            } else {
                self.0[0]
            }
        }
    }

    fn sized(cols: u16, rows: u16, characters: &str) -> Sample {
        Sample {
            screen: Screen {
                cols,
                rows,
                characters: characters.into(),
            },
            title: None,
        }
    }

    fn screen(characters: &str) -> Sample {
        sized(80, 24, characters)
    }

    fn titled(characters: &str, title: &str) -> Sample {
        Sample {
            title: Some(title.into()),
            ..screen(characters)
        }
    }

    /// A detector initialized at 0 with an empty screen, drawing `draws`, whose
    /// episodes start at `first`.
    fn detector_from(draws: &[u64], first: u64) -> Detector<Draws> {
        Detector::new(
            &at(0),
            screen(""),
            Counter::new(first).unwrap(),
            Draws(draws.to_vec()),
        )
    }

    /// A detector whose every sampling delay is 500 ms.
    fn fresh() -> Detector<Draws> {
        detector_from(&[0], 1)
    }

    /// The status `state` changed at `ms`, with `episode`.
    fn status(state: StatusState, ms: u64, episode: Option<u64>) -> Status {
        let episode = episode.map(|value| Counter::new(value).unwrap());
        Status::new(state, at(ms).utc, episode).unwrap()
    }

    fn running(ms: u64) -> Option<Status> {
        Some(status(StatusState::Running, ms, None))
    }

    fn idle(ms: u64) -> Option<Status> {
        Some(status(StatusState::Idle, ms, None))
    }

    fn needs(ms: u64, episode: u64) -> Option<Status> {
        Some(status(StatusState::NeedsAttention, ms, Some(episode)))
    }

    fn acknowledged(ms: u64, episode: u64) -> Option<Status> {
        Some(status(StatusState::Acknowledged, ms, Some(episode)))
    }

    fn poll(detector: &mut Detector<Draws>, ms: u64, sample: Sample) -> Option<Status> {
        detector.poll(&at(ms), move || sample.clone())
    }

    /// `signal` at `ms`, with `sample` showing on the terminal.
    fn signalled(
        detector: &mut Detector<Draws>,
        ms: u64,
        signal: Signal,
        sample: Sample,
    ) -> Option<Status> {
        detector.signal(&at(ms), signal, move || sample.clone())
    }

    /// A root `report` at `ms`, which must not take a sample.
    fn root(detector: &mut Detector<Draws>, ms: u64, report: Report) -> Option<Status> {
        detector.signal(&at(ms), Signal::Root(report), || {
            panic!("no sample is due at {ms}")
        })
    }

    /// A root `report` at `ms`, with an empty screen showing.
    fn root_on_empty(detector: &mut Detector<Draws>, ms: u64, report: Report) -> Option<Status> {
        signalled(detector, ms, Signal::Root(report), screen(""))
    }

    fn shell(
        detector: &mut Detector<Draws>,
        ms: u64,
        marker: Marker,
        sample: Sample,
    ) -> Option<Status> {
        signalled(detector, ms, Signal::Shell(marker), sample)
    }

    /// A poll at `ms` that must not take a sample.
    fn unsampled(detector: &mut Detector<Draws>, ms: u64) -> Option<Status> {
        detector.poll(&at(ms), || panic!("no sample is due at {ms}"))
    }

    /// Agent work detected at 500 with "a" on screen: attention at 10,500.
    fn working() -> Detector<Draws> {
        let mut detector = fresh();
        assert_eq!(poll(&mut detector, 500, screen("a")), running(500));
        detector
    }

    /// Pending attention, episode 1, since 10,500.
    fn pending() -> Detector<Draws> {
        let mut detector = working();
        assert_eq!(poll(&mut detector, 10_000, screen("a")), None);
        assert_eq!(poll(&mut detector, 10_500, screen("a")), needs(10_500, 1));
        detector
    }

    /// STA-01 and STA-02 (pure, initial silence): the machines start idle and
    /// cleared at initialization, and a session silent from startup stays so:
    /// it never becomes agent work, so inactivity never requests attention.
    #[test]
    fn a_silent_session_stays_idle_and_cleared() {
        let mut detector = detector_from(&[1_000], 1);
        assert_eq!(detector.status(), status(StatusState::Idle, 0, None));
        for ms in (0..=30_000).step_by(250) {
            assert_eq!(poll(&mut detector, ms, screen("")), None, "{ms}");
        }
        assert_eq!(detector.status(), status(StatusState::Idle, 0, None));
    }

    /// STA-02 (pure, scheduling): each sampling iteration waits a fresh random
    /// delay from 500 to 1,500 ms, from the lowest draw to the highest and
    /// around again, and takes no sample before it.
    #[test]
    fn each_sample_waits_a_fresh_random_delay_of_500_to_1500_ms() {
        let mut detector = detector_from(&[0, 1_000, 1_001, 2_001], 1);
        for (before, due) in [(499, 500), (1_999, 2_000), (2_499, 2_500), (3_999, 4_000)] {
            assert_eq!(unsampled(&mut detector, before), None);
            assert_eq!(poll(&mut detector, due, screen("")), None);
        }
        assert_eq!(unsampled(&mut detector, 5_499), None);
    }

    /// STA-01 and STA-02 (pure, agent inactivity): a changed viewport is agent
    /// work, reported as running at the sample that saw it, and ten seconds
    /// without change after it end the work and request attention in one step:
    /// needs_attention is reported, never an intermediate idle. One millisecond
    /// earlier nothing happens, and a changed title counts on its own.
    #[test]
    fn a_change_is_agent_work_until_ten_seconds_of_inactivity_request_attention() {
        let mut detector = working();
        assert_eq!(poll(&mut detector, 10_000, screen("a")), None);
        assert_eq!(unsampled(&mut detector, 10_499), None);
        assert_eq!(poll(&mut detector, 10_500, screen("a")), needs(10_500, 1));
        assert_eq!(detector.status(), needs(10_500, 1).unwrap());

        let mut detector = fresh();
        assert_eq!(poll(&mut detector, 500, titled("", "t")), running(500));
    }

    /// STA-02 (pure, clamping): the sampling delay is clamped to the remaining
    /// agent deadline, so the deadline is never overshot; after expiry the
    /// expired deadline is disarmed, so nothing spins on it.
    #[test]
    fn the_next_sample_is_clamped_to_the_agent_deadline() {
        let mut detector = detector_from(&[1_000], 1);
        assert_eq!(poll(&mut detector, 1_500, screen("a")), running(1_500));
        for ms in (3_000..=10_500).step_by(1_500) {
            assert_eq!(poll(&mut detector, ms, screen("a")), None, "{ms}");
        }
        assert_eq!(unsampled(&mut detector, 11_499), None);
        assert_eq!(poll(&mut detector, 11_500, screen("a")), needs(11_500, 1));
        assert_eq!(unsampled(&mut detector, 11_500), None);
        assert_eq!(unsampled(&mut detector, 12_999), None);
    }

    /// STA-02 (pure, content before expiry): a change seen at the deadline
    /// keeps the work going and restarts its deadline.
    #[test]
    fn content_is_checked_before_the_deadline_expires() {
        let mut detector = working();
        assert_eq!(poll(&mut detector, 10_500, screen("b")), None);
        assert_eq!(poll(&mut detector, 20_499, screen("b")), None);
        assert_eq!(poll(&mut detector, 20_500, screen("b")), needs(20_500, 1));
    }

    /// STA-02 (pure, resize): a resize replaces the character baseline and
    /// manufactures no work while idle; during agent work it restarts the
    /// inactivity deadline. A title changed at the same sample still counts.
    #[test]
    fn a_resize_replaces_the_baseline_without_manufacturing_work() {
        let mut detector = fresh();
        assert_eq!(poll(&mut detector, 500, sized(100, 30, "x")), None);
        assert_eq!(poll(&mut detector, 1_000, sized(100, 30, "x")), None);
        assert_eq!(poll(&mut detector, 1_500, sized(100, 31, "x")), None);
        assert_eq!(
            poll(
                &mut detector,
                2_000,
                Sample {
                    title: Some("t".into()),
                    ..sized(90, 31, "y")
                }
            ),
            running(2_000)
        );
        let resized = || Sample {
            title: Some("t".into()),
            ..sized(90, 30, "z")
        };
        assert_eq!(poll(&mut detector, 12_000, resized()), None);
        assert_eq!(poll(&mut detector, 21_999, resized()), None);
        assert_eq!(poll(&mut detector, 22_000, resized()), needs(22_000, 1));
    }

    /// STA-01 (pure, user window): accepted input enters user work at once and
    /// arms a two-second window that each further input re-arms. No sample is
    /// taken during it, so its output neither starts agent work nor extends
    /// it. Its end is idle exactly two seconds after the last input, never an
    /// attention request, with a fresh baseline: the output it hid is not a
    /// change afterwards.
    #[test]
    fn input_re_arms_a_two_second_user_window_that_never_requests_attention() {
        let mut detector = fresh();
        assert_eq!(detector.input(&at(100)), running(100));
        assert_eq!(unsampled(&mut detector, 500), None);
        assert_eq!(detector.input(&at(1_500)), None);
        assert_eq!(unsampled(&mut detector, 3_499), None);
        assert_eq!(poll(&mut detector, 3_500, screen("echo")), idle(3_500));
        assert_eq!(unsampled(&mut detector, 3_999), None);
        assert_eq!(poll(&mut detector, 4_000, screen("echo")), None);
        assert_eq!(detector.status(), status(StatusState::Idle, 3_500, None));
    }

    /// STA-01 and STA-07 (pure, working states and stale deadlines): input
    /// during agent work changes only the working state, so the effective
    /// status and its change time stay; it cancels the agent deadline, whose
    /// expiry is then ignored.
    #[test]
    fn input_during_agent_work_keeps_running_and_cancels_its_deadline() {
        let mut detector = working();
        assert_eq!(detector.input(&at(600)), None);
        assert_eq!(detector.status(), status(StatusState::Running, 500, None));
        assert_eq!(poll(&mut detector, 2_600, screen("a")), idle(2_600));
        assert_eq!(poll(&mut detector, 10_500, screen("a")), None);
        assert_eq!(detector.status(), status(StatusState::Idle, 2_600, None));
    }

    /// STA-01 (pure, same-turn dispatch): input accepted in the turn whose
    /// timer work is due comes first, so the user window or agent deadline it
    /// replaced never expires: no idle and no attention.
    #[test]
    fn input_comes_before_the_timer_work_of_its_turn() {
        let mut detector = fresh();
        assert_eq!(detector.input(&at(100)), running(100));
        assert_eq!(detector.input(&at(2_100)), None);
        assert_eq!(unsampled(&mut detector, 2_100), None);

        let mut detector = working();
        assert_eq!(detector.input(&at(10_500)), None);
        assert_eq!(unsampled(&mut detector, 10_500), None);
        assert_eq!(detector.status(), status(StatusState::Running, 500, None));
    }

    /// STA-01 (pure, both clearing paths): accepted input clears pending
    /// attention and acknowledged attention alike, to running with no episode.
    #[test]
    fn accepted_input_clears_pending_and_acknowledged_attention() {
        let mut detector = pending();
        assert_eq!(detector.input(&at(10_600)), running(10_600));
        assert_eq!(poll(&mut detector, 12_600, screen("a")), idle(12_600));
        assert_eq!(poll(&mut detector, 13_100, screen("b")), running(13_100));
        assert_eq!(poll(&mut detector, 23_100, screen("b")), needs(23_100, 2));
        assert_eq!(detector.acknowledge(&at(23_200)), acknowledged(23_200, 2));
        assert_eq!(detector.input(&at(23_300)), running(23_300));
    }

    /// STA-01, STA-07 and NOT-01 (pure, episodes): while attention is pending,
    /// activity goes on hidden and a further request joins the episode, with
    /// no report and no new change time. Acknowledgement keeps the episode;
    /// acknowledging again, or with nothing pending, changes nothing. A
    /// request after acknowledgement starts the next episode.
    #[test]
    fn requests_coalesce_while_pending_and_re_arm_after_acknowledgement() {
        let mut detector = pending();
        assert_eq!(poll(&mut detector, 11_000, screen("b")), None);
        assert_eq!(poll(&mut detector, 21_000, screen("b")), None);
        assert_eq!(detector.status(), needs(10_500, 1).unwrap());
        assert_eq!(detector.acknowledge(&at(21_100)), acknowledged(21_100, 1));
        assert_eq!(detector.acknowledge(&at(21_200)), None);
        assert_eq!(poll(&mut detector, 21_500, screen("c")), None);
        assert_eq!(poll(&mut detector, 31_000, screen("c")), None);
        assert_eq!(poll(&mut detector, 31_500, screen("c")), needs(31_500, 2));

        let mut detector = fresh();
        assert_eq!(detector.acknowledge(&at(100)), None);
        assert_eq!(detector.status(), status(StatusState::Idle, 0, None));
    }

    /// STA-01 (pure, final states): a confirmed exit of zero completes, any
    /// other exit or a signal fails, with no episode and the time of the
    /// result. From then on nothing is observed and no late input, sample,
    /// deadline, acknowledgement or request changes the status.
    #[test]
    fn a_final_result_stops_observation_and_rejects_late_events() {
        let signal = CommandResult::Signal {
            number: NonZeroU32::new(15).unwrap(),
            name: None,
        };
        let results = [
            (CommandResult::Exit { code: 0 }, StatusState::Completed),
            (CommandResult::Exit { code: 143 }, StatusState::Failed),
            (signal, StatusState::Failed),
        ];
        for (result, state) in results {
            let mut detector = working();
            let last = status(state, 9_000, None);
            assert_eq!(detector.finish(&at(9_000), &result), last);
            assert_eq!(unsampled(&mut detector, 10_500), None);
            assert_eq!(detector.input(&at(10_600)), None);
            assert_eq!(detector.acknowledge(&at(10_700)), None);
            detector.request_attention();
            assert_eq!(root(&mut detector, 10_800, Report::Working), None);
            assert_eq!(root_on_empty(&mut detector, 10_900, Report::Error), None);
            let reset = signalled(&mut detector, 11_000, Signal::FullReset, screen(""));
            assert_eq!(reset, None);
            assert_eq!(unsampled(&mut detector, 20_000), None);
            assert_eq!(detector.status(), last);
        }
    }

    /// STA-06 (pure, notifications): a recognized notification requests
    /// attention: from idle it is pending at once with episode 1, and while
    /// pending a further one joins that episode silently. It leaves
    /// activity alone: agent work's inactivity deadline stays armed under
    /// acknowledged attention and starts the next episode. The pending
    /// episode, which a notification record carries, is none once
    /// attention is acknowledged or cleared, and after the final result,
    /// when a notification changes nothing.
    #[test]
    fn notifications_request_attention_and_carry_the_pending_episode() {
        let episode = |value| Some(Counter::new(value).unwrap());
        let mut detector = fresh();
        assert_eq!(detector.pending_episode(), None);
        assert_eq!(detector.notify(&at(100)), needs(100, 1));
        assert_eq!(detector.pending_episode(), episode(1));
        assert_eq!(detector.notify(&at(200)), None);
        assert_eq!(detector.pending_episode(), episode(1));
        assert_eq!(detector.acknowledge(&at(300)), acknowledged(300, 1));
        assert_eq!(detector.pending_episode(), None);
        assert_eq!(detector.notify(&at(400)), needs(400, 2));
        assert_eq!(detector.input(&at(500)), running(500));
        assert_eq!(detector.pending_episode(), None);

        let mut detector = working();
        assert_eq!(detector.notify(&at(1_000)), needs(1_000, 1));
        assert_eq!(detector.acknowledge(&at(1_100)), acknowledged(1_100, 1));
        assert_eq!(poll(&mut detector, 10_000, screen("a")), None);
        assert_eq!(poll(&mut detector, 10_500, screen("a")), needs(10_500, 2));
        detector.finish(&at(11_000), &CommandResult::Exit { code: 0 });
        assert_eq!(detector.notify(&at(11_100)), None);
        assert_eq!(detector.pending_episode(), None);
    }

    /// LIM-05 (pure, episodes): episode identifiers are never wrapped or
    /// reused. The last one is issued without exhausting the domain; then a
    /// request that would need another marks it exhausted and leaves
    /// attention as it was, and activity goes on.
    #[test]
    fn exhausted_episode_identifiers_are_never_wrapped() {
        let mut detector = detector_from(&[0], u64::MAX);
        assert_eq!(poll(&mut detector, 500, screen("a")), running(500));
        assert_eq!(
            poll(&mut detector, 10_500, screen("a")),
            needs(10_500, u64::MAX)
        );
        assert_eq!(
            detector.acknowledge(&at(10_600)),
            acknowledged(10_600, u64::MAX)
        );
        assert!(!detector.exhausted());
        assert_eq!(poll(&mut detector, 11_000, screen("b")), None);
        assert_eq!(poll(&mut detector, 21_000, screen("b")), None);
        assert!(detector.exhausted());
        assert_eq!(detector.input(&at(21_100)), running(21_100));
        assert_eq!(poll(&mut detector, 23_100, screen("c")), idle(23_100));
        assert_eq!(poll(&mut detector, 23_600, screen("d")), running(23_600));
        assert_eq!(poll(&mut detector, 33_600, screen("d")), idle(33_600));
    }

    /// STA-08 (pure, focus reports): Input holding only focus reports, one
    /// or several, in or out, is not user-originated; any other byte, before,
    /// between or after them, is, and so is a partial report.
    #[test]
    fn only_focus_reports_are_not_user_input() {
        for bytes in [&b"\x1b[I"[..], b"\x1b[O", b"\x1b[I\x1b[O\x1b[I", b""] {
            assert!(!super::user_originated(bytes), "{bytes:?}");
        }
        for bytes in [
            &b"x"[..],
            b"\x1b[Ix",
            b"x\x1b[O",
            b"\x1b[I \x1b[O",
            b"\x1b[",
            b"\x1b[A",
            b"\x1b",
        ] {
            assert!(super::user_originated(bytes), "{bytes:?}");
        }
    }

    /// STA-03 (pure, explicit working): a root `working` report is agent
    /// work at once, and explicit detection infers nothing from the viewport
    /// and has no inactivity deadline: it stays running, silently, for as
    /// long as no other signal comes. A root `done` then requests attention,
    /// which is no final state: accepted input still clears it.
    #[test]
    fn explicit_working_runs_silently_without_a_deadline() {
        let mut detector = fresh();
        assert_eq!(root(&mut detector, 100, Report::Working), running(100));
        for ms in (500..=60_000).step_by(500) {
            assert_eq!(unsampled(&mut detector, ms), None, "{ms}");
        }
        assert_eq!(detector.status(), status(StatusState::Running, 100, None));
        assert_eq!(root(&mut detector, 60_000, Report::Done), needs(60_000, 1));
        assert_eq!(detector.input(&at(60_100)), running(60_100));
    }

    /// STA-03 (pure, attention-bearing reports): every root `idle`, `done`,
    /// `blocked` or `error` report requests attention, even while activity
    /// is already idle; `working` and `clear` request none. Repeated
    /// requests while attention is pending join its episode; `clear`
    /// neither requests nor clears attention; after acknowledgement a
    /// further report starts the next episode. None of them is a final
    /// state.
    #[test]
    fn attention_bearing_root_reports_request_attention_and_coalesce() {
        for (report, expected) in [
            (Report::Idle, needs(100, 1)),
            (Report::Done, needs(100, 1)),
            (Report::Blocked, needs(100, 1)),
            (Report::Error, needs(100, 1)),
            (Report::Working, running(100)),
            (Report::Clear, None),
        ] {
            let mut detector = fresh();
            assert_eq!(
                root_on_empty(&mut detector, 100, report),
                expected,
                "{report:?}"
            );
        }

        let mut detector = fresh();
        assert_eq!(root(&mut detector, 100, Report::Idle), needs(100, 1));
        for (ms, report) in [
            (200, Report::Done),
            (300, Report::Blocked),
            (400, Report::Idle),
        ] {
            assert_eq!(root(&mut detector, ms, report), None, "{report:?}");
        }
        assert_eq!(root_on_empty(&mut detector, 500, Report::Error), None);
        assert_eq!(detector.acknowledge(&at(600)), acknowledged(600, 1));
        assert_eq!(root_on_empty(&mut detector, 700, Report::Clear), None);
        assert_eq!(root(&mut detector, 800, Report::Working), None);
        assert_eq!(root(&mut detector, 900, Report::Blocked), needs(900, 2));
        assert_eq!(detector.input(&at(1_000)), running(1_000));
        assert_eq!(root(&mut detector, 1_100, Report::Done), needs(1_100, 3));
    }

    /// STA-03 and STA-05 (pure, remembered working): the user window's end
    /// restores agent work while the current explicit scope's root record
    /// says `working`, without the application repeating it. A shell
    /// boundary retires that scope, so a later window ends idle. A root
    /// report supersedes user work at once.
    #[test]
    fn user_window_expiry_restores_explicit_working_only_in_its_scope() {
        let mut detector = fresh();
        assert_eq!(root(&mut detector, 100, Report::Working), running(100));
        assert_eq!(detector.input(&at(200)), None);
        assert_eq!(unsampled(&mut detector, 2_199), None);
        assert_eq!(unsampled(&mut detector, 2_200), None);
        assert_eq!(unsampled(&mut detector, 30_000), None);
        assert_eq!(
            shell(&mut detector, 30_000, Marker::D, screen("")),
            idle(30_000)
        );
        assert_eq!(detector.input(&at(30_100)), running(30_100));
        assert_eq!(poll(&mut detector, 32_100, screen("")), idle(32_100));

        assert_eq!(detector.input(&at(32_200)), running(32_200));
        assert_eq!(
            root_on_empty(&mut detector, 32_300, Report::Clear),
            idle(32_300)
        );
        assert_eq!(root(&mut detector, 32_400, Report::Idle), needs(32_400, 1));
        assert_eq!(detector.input(&at(32_500)), running(32_500));
        assert_eq!(unsampled(&mut detector, 34_500), idle(34_500));
    }

    /// STA-04 (pure, fallback grace): root `clear` starts a separate ten
    /// seconds of viewport quiet before heuristics resume. During it,
    /// changed characters, a changed title and a resize each restart the
    /// quiet deadline without becoming agent work; it ends at exactly ten
    /// seconds of quiet, not a millisecond earlier, with no attention
    /// request, and heuristic detection then sees the next change.
    #[test]
    fn clear_starts_a_ten_second_fallback_quiet_deadline() {
        let mut detector = fresh();
        assert_eq!(root(&mut detector, 100, Report::Working), running(100));
        assert_eq!(
            root_on_empty(&mut detector, 1_000, Report::Clear),
            idle(1_000)
        );
        assert_eq!(poll(&mut detector, 10_999, screen("x")), None);
        assert_eq!(poll(&mut detector, 20_998, titled("x", "t")), None);
        let resized = |characters: &str| Sample {
            title: Some("t".into()),
            ..sized(100, 30, characters)
        };
        assert_eq!(poll(&mut detector, 30_997, resized("x")), None);
        assert_eq!(poll(&mut detector, 40_996, resized("y")), None);
        assert_eq!(poll(&mut detector, 50_995, resized("y")), None);
        assert_eq!(unsampled(&mut detector, 50_995), None);
        assert_eq!(poll(&mut detector, 50_996, resized("y")), None);
        assert_eq!(poll(&mut detector, 51_496, resized("z")), running(51_496));
    }

    /// STA-03 and STA-04 (pure, error): root `error` requests attention and
    /// starts the same grace; its expiry requests none, so acknowledged
    /// attention stays acknowledged, and heuristics resume after it.
    #[test]
    fn error_requests_attention_and_its_grace_expiry_requests_none() {
        let mut detector = fresh();
        assert_eq!(
            root_on_empty(&mut detector, 1_000, Report::Error),
            needs(1_000, 1)
        );
        assert_eq!(detector.acknowledge(&at(1_100)), acknowledged(1_100, 1));
        assert_eq!(poll(&mut detector, 10_999, screen("")), None);
        assert_eq!(poll(&mut detector, 11_000, screen("")), None);
        assert_eq!(detector.status(), acknowledged(1_100, 1).unwrap());
        assert_eq!(detector.input(&at(11_100)), running(11_100));
        assert_eq!(poll(&mut detector, 13_100, screen("")), idle(13_100));
        assert_eq!(poll(&mut detector, 13_600, screen("x")), running(13_600));
    }

    /// STA-04 (pure, ending grace): a fresh root report ends the grace and
    /// applies explicit detection, which samples nothing; a shell boundary
    /// ends it and resumes heuristics at once. Accepted input neither
    /// restarts nor ends it: quiet is still measured during user work, whose
    /// changes are no agent work and never extend the two-second window.
    #[test]
    fn grace_ends_at_fresh_root_reports_and_shell_boundaries_not_at_input() {
        let mut detector = fresh();
        assert_eq!(root_on_empty(&mut detector, 1_000, Report::Clear), None);
        assert_eq!(root(&mut detector, 2_000, Report::Done), needs(2_000, 1));
        assert_eq!(unsampled(&mut detector, 12_000), None);

        let mut detector = fresh();
        assert_eq!(root_on_empty(&mut detector, 1_000, Report::Clear), None);
        assert_eq!(shell(&mut detector, 2_000, Marker::D, screen("")), None);
        assert_eq!(poll(&mut detector, 2_500, screen("x")), running(2_500));

        let mut detector = fresh();
        assert_eq!(root_on_empty(&mut detector, 1_000, Report::Clear), None);
        assert_eq!(detector.input(&at(2_000)), running(2_000));
        assert_eq!(poll(&mut detector, 2_500, screen("x")), None);
        assert_eq!(poll(&mut detector, 3_999, screen("x")), None);
        assert_eq!(poll(&mut detector, 4_000, screen("x")), idle(4_000));
        assert_eq!(poll(&mut detector, 12_499, screen("x")), None);
        assert_eq!(poll(&mut detector, 12_500, screen("x")), None);
        assert_eq!(poll(&mut detector, 13_000, screen("y")), running(13_000));
    }

    /// STA-04 (pure, full reset): in heuristic detection a full reset only
    /// refreshes the baseline, so the screen it cleared is no change and
    /// running agent work keeps its deadline. In explicit detection it
    /// behaves as root `clear`: idle, no attention, and the grace, which a
    /// reset during grace restarts.
    #[test]
    fn a_full_reset_refreshes_heuristics_and_ends_explicit_detection_like_clear() {
        let mut detector = working();
        let reset = signalled(&mut detector, 1_000, Signal::FullReset, screen(""));
        assert_eq!(reset, None);
        assert_eq!(poll(&mut detector, 1_500, screen("")), None);
        assert_eq!(poll(&mut detector, 10_000, screen("")), None);
        assert_eq!(poll(&mut detector, 10_500, screen("")), needs(10_500, 1));

        let mut detector = fresh();
        assert_eq!(root(&mut detector, 100, Report::Working), running(100));
        let reset = signalled(&mut detector, 200, Signal::FullReset, screen(""));
        assert_eq!(reset, idle(200));
        assert_eq!(poll(&mut detector, 10_199, screen("x")), None);
        assert_eq!(poll(&mut detector, 20_199, screen("x")), None);
        assert_eq!(poll(&mut detector, 20_699, screen("y")), running(20_699));

        let mut detector = fresh();
        assert_eq!(root_on_empty(&mut detector, 1_000, Report::Clear), None);
        let reset = signalled(&mut detector, 5_000, Signal::FullReset, screen(""));
        assert_eq!(reset, None);
        assert_eq!(poll(&mut detector, 11_000, screen("")), None);
        assert_eq!(poll(&mut detector, 11_500, screen("x")), None);
    }

    /// STA-05 (pure, shell boundaries): `A`, `C` and `D` retire the scope,
    /// to idle, with no attention and no deadline left; repeating one is
    /// harmless. Between `A` and `B` prompt rendering is no activity, even
    /// across a `D`; `B` ends that exclusion with a fresh baseline, so the
    /// rendered prompt is no change, and `C` ends it too.
    #[test]
    fn shell_boundaries_retire_the_scope_and_exclude_prompt_rendering() {
        let mut detector = fresh();
        assert_eq!(root(&mut detector, 100, Report::Working), running(100));
        assert_eq!(shell(&mut detector, 200, Marker::A, screen("$")), idle(200));
        assert_eq!(shell(&mut detector, 300, Marker::D, screen("$")), None);
        assert_eq!(unsampled(&mut detector, 5_000), None);
        assert_eq!(shell(&mut detector, 5_100, Marker::B, screen("$ ls")), None);
        assert_eq!(poll(&mut detector, 5_600, screen("$ ls")), None);
        assert_eq!(
            poll(&mut detector, 6_100, screen("$ ls\nout")),
            running(6_100)
        );
        let output = || screen("$ ls\nout");
        assert_eq!(
            shell(&mut detector, 6_200, Marker::C, output()),
            idle(6_200)
        );
        assert_eq!(
            poll(&mut detector, 6_700, screen("$ ls\nmore")),
            running(6_700)
        );
        let output = || screen("$ ls\nmore");
        assert_eq!(
            shell(&mut detector, 7_000, Marker::D, output()),
            idle(7_000)
        );
        assert_eq!(shell(&mut detector, 7_100, Marker::D, output()), None);
        assert_eq!(poll(&mut detector, 7_600, output()), None);
        assert_eq!(poll(&mut detector, 17_100, output()), None);
        assert_eq!(detector.status(), status(StatusState::Idle, 7_000, None));

        let mut detector = fresh();
        assert_eq!(shell(&mut detector, 100, Marker::A, screen("$")), None);
        assert_eq!(shell(&mut detector, 200, Marker::C, screen("$ x")), None);
        assert_eq!(poll(&mut detector, 700, screen("$ x\ny")), running(700));
    }

    /// STA-05 (pure, what boundaries keep): during prompt rendering accepted
    /// input and root reports still apply; a retained `done` does not pin
    /// detection past the next boundary; `B` ends nothing else, neither an
    /// agent deadline nor the grace.
    #[test]
    fn shell_boundaries_keep_input_reports_and_other_deadlines() {
        let mut detector = fresh();
        assert_eq!(shell(&mut detector, 100, Marker::A, screen("$")), None);
        assert_eq!(detector.input(&at(200)), running(200));
        assert_eq!(poll(&mut detector, 2_200, screen("$ x")), idle(2_200));
        assert_eq!(root(&mut detector, 2_300, Report::Done), needs(2_300, 1));

        let mut detector = fresh();
        assert_eq!(root(&mut detector, 100, Report::Done), needs(100, 1));
        assert_eq!(detector.input(&at(200)), running(200));
        assert_eq!(shell(&mut detector, 300, Marker::D, screen("")), idle(300));
        assert_eq!(poll(&mut detector, 800, screen("x")), running(800));

        let mut detector = working();
        assert_eq!(shell(&mut detector, 1_000, Marker::B, screen("a")), None);
        assert_eq!(poll(&mut detector, 10_000, screen("a")), None);
        assert_eq!(poll(&mut detector, 10_500, screen("a")), needs(10_500, 1));

        let mut detector = fresh();
        assert_eq!(root_on_empty(&mut detector, 1_000, Report::Clear), None);
        assert_eq!(shell(&mut detector, 5_000, Marker::B, screen("")), None);
        assert_eq!(poll(&mut detector, 11_000, screen("")), None);
        assert_eq!(poll(&mut detector, 11_500, screen("x")), running(11_500));
    }
}
