//! Daemon-core activity and attention: the two state machines of the
//! [session status mechanics](../../../docs/session-status.md) and their
//! heuristic detector.
//!
//! The detector is pure. Every event takes the clock reading it happens at,
//! and the random source is injected, so tests drive it without a session.
//! Call protocol for one daemon turn, which reads the clock once:
//!
//! 1. Feed accepted user input with [`Detector::input`] and explicit
//!    acknowledgement with [`Detector::acknowledge`], in acceptance order.
//!    Input therefore comes before the timer and sample work of the turn.
//! 2. Call [`Detector::poll`] once, after reading the command's output.
//! 3. Report a confirmed command result once with [`Detector::finish`].
//!
//! Each event runs to completion, its attention request included, before it
//! reports the effective status, so no intermediate status is ever reported.
//! An event returns the effective status only when it changed; only that
//! change replaces the stored change time.
//!
//! Detection is heuristic only. #92 adds the explicit and fallback detection
//! modes beside it, driven by OSC 7501 and OSC 133, and #98 the notification
//! signals; both request attention through [`Detector::request_attention`].
mod clock;

use climon2_core::wire::{CommandResult, Counter, Status, StatusState, Utc};
pub use clock::{Clock, SystemRandom, daemon_clock, first_episode};
use std::time::Duration;

/// The user-work window each qualifying input re-arms.
pub const USER_WINDOW: Duration = Duration::from_millis(2_000);
/// Agent inactivity before attention is requested; hardcoded, never a setting.
pub const INACTIVITY: Duration = Duration::from_millis(10_000);
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

    /// The user window's end, or a due sample; true when it requests
    /// attention.
    fn poll(
        &mut self,
        now: Duration,
        sample: impl FnOnce() -> Sample,
        random: &mut impl Random,
    ) -> bool {
        if self.window_ended(now) {
            self.activity = Activity::Idle;
            self.baseline = sample();
            self.schedule(now, random);
            return false;
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

    /// No sample is taken during the user window.
    fn sampling_due(&self, now: Duration) -> bool {
        !matches!(self.activity, Activity::User { .. }) && now >= self.next_sample
    }

    /// Changed content, or a resize during agent work, (re)starts the agent
    /// inactivity deadline; only then can it expire. The sample is the next
    /// baseline either way.
    fn observe(&mut self, now: Duration, current: Sample) -> bool {
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

    /// A fresh random delay, never beyond an armed agent deadline.
    fn schedule(&mut self, now: Duration, random: &mut impl Random) {
        let drawn = SAMPLE_DELAY_MIN_MS + random.next() % SAMPLE_DELAY_CHOICES;
        let delay = Duration::from_millis(drawn);
        let delay = match self.activity {
            Activity::Agent { deadline } => delay.min(deadline.saturating_sub(now)),
            _ => delay,
        };
        self.next_sample = now + delay;
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
    pub fn poll(&mut self, now: &Now, sample: impl FnOnce() -> Sample) -> Option<Status> {
        let Phase::Live(live) = &mut self.phase else {
            return None;
        };
        if live.poll(now.monotonic, sample, &mut self.random) {
            self.request_attention();
        }
        self.settle(now)
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
    /// it joins. With the episode identifiers exhausted, attention stays as
    /// it was rather than reuse one.
    pub fn request_attention(&mut self) {
        let Phase::Live(live) = &mut self.phase else {
            return;
        };
        if matches!(live.attention, Attention::Pending(_)) {
            return;
        }
        match self.episodes.issue() {
            Some(episode) => live.attention = Attention::Pending(episode),
            None => tracing::warn!("attention episode identifiers exhausted"),
        }
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
    use super::{Detector, Now, Random, Sample, Screen};
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
        detector.poll(&at(ms), || sample)
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
            assert_eq!(unsampled(&mut detector, 20_000), None);
            assert_eq!(detector.status(), last);
        }
    }

    /// LIM-05 (pure, episodes): episode identifiers are never wrapped or
    /// reused. The last one is issued; then a request that would need another
    /// leaves attention as it was, and activity goes on.
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
        assert_eq!(poll(&mut detector, 11_000, screen("b")), None);
        assert_eq!(poll(&mut detector, 21_000, screen("b")), None);
        assert_eq!(detector.input(&at(21_100)), running(21_100));
        assert_eq!(poll(&mut detector, 23_100, screen("c")), idle(23_100));
        assert_eq!(poll(&mut detector, 23_600, screen("d")), running(23_600));
        assert_eq!(poll(&mut detector, 33_600, screen("d")), idle(33_600));
    }
}
