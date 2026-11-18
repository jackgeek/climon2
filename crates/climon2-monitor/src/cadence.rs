//! The UI's cadence (#119; operational limits, "UI, sampling and
//! responsiveness"): at most `ui.fps` frames painted a second, and client
//! work rotated fairly within `ui.client_quantum_ms` per client and
//! `ui.background_quantum_ms` per UI turn.
use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

/// When the next frame is due: the first at once, then each at least one
/// frame interval after the last painted. A frame not yet due is not
/// painted, never queued: the next one due paints the scene as it is then,
/// so obsolete animation frames are skipped.
pub(crate) struct Frames {
    interval: Duration,
    last: Option<Instant>,
}

impl Frames {
    /// At most `fps` frames a second (1 through 120).
    pub(crate) fn new(fps: u32) -> Self {
        Self {
            interval: Duration::from_secs(1) / fps,
            last: None,
        }
    }

    /// Whether a frame is due at `now`.
    pub(crate) fn due(&self, now: Instant) -> bool {
        self.last
            .is_none_or(|last| now.saturating_duration_since(last) >= self.interval)
    }

    /// A frame was painted at `now`.
    pub(crate) fn painted(&mut self, now: Instant) {
        self.last = Some(now);
    }
}

/// Work waiting per client, done in fair turns: each UI turn does work for
/// at most the background quantum, each client in turn for at most the
/// client quantum before the next client with work has its turn, starting
/// after the client the last turn served. Each item is one bounded step (a
/// display chunk is at most `transfer.chunk_bytes`); a quantum is checked
/// between items, never inside one.
pub(crate) struct Work<K, T> {
    queues: BTreeMap<K, VecDeque<T>>,
    client: Duration,
    background: Duration,
    /// When the current turn began.
    turn: Option<Instant>,
    /// The client served last, and when its quantum began, if in this turn.
    serving: Option<(K, Option<Instant>)>,
}

impl<K: Clone + Ord, T> Work<K, T> {
    pub(crate) fn new(client: Duration, background: Duration) -> Self {
        Self {
            queues: BTreeMap::new(),
            client,
            background,
            turn: None,
            serving: None,
        }
    }

    /// `item` waits behind `key`'s earlier work.
    pub(crate) fn push(&mut self, key: K, item: T) {
        self.queues.entry(key).or_default().push_back(item);
    }

    /// A UI turn begins at `now`: the client served last finishes its
    /// quantum, so the next client with work goes first.
    pub(crate) fn begin(&mut self, now: Instant) {
        self.turn = Some(now);
        if let Some((_, started)) = self.serving.as_mut() {
            *started = None;
        }
    }

    /// The next item to do at `now`, and its client's key: none once the
    /// turn's background quantum is spent or no work waits.
    pub(crate) fn next(&mut self, now: Instant) -> Option<(K, T)> {
        let turn = self.turn?;
        if now.saturating_duration_since(turn) >= self.background {
            return None;
        }
        let key = self.continuing(now).or_else(|| self.switch(now))?;
        let item = self.queues.get_mut(&key).and_then(VecDeque::pop_front);
        item.map(|item| (key, item))
    }

    /// The client being served, while its quantum lasts at `now` and its
    /// work does.
    fn continuing(&self, now: Instant) -> Option<K> {
        let (key, started) = self.serving.as_ref()?;
        let lasting =
            started.is_some_and(|started| now.saturating_duration_since(started) < self.client);
        let working = self.queues.get(key).is_some_and(|queue| !queue.is_empty());
        (lasting && working).then(|| key.clone())
    }

    /// The next client with work after the one served last, wrapping round
    /// to it last, whose quantum begins at `now`.
    fn switch(&mut self, now: Instant) -> Option<K> {
        use std::ops::Bound::{Excluded, Unbounded};
        let waiting = |(key, queue): (&K, &VecDeque<T>)| (!queue.is_empty()).then(|| key.clone());
        let after = self.serving.as_ref().map(|(key, _)| key);
        let later = after.and_then(|after| {
            self.queues
                .range((Excluded(after), Unbounded))
                .find_map(waiting)
        });
        let next = later.or_else(|| self.queues.iter().find_map(waiting))?;
        self.serving = Some((next.clone(), Some(now)));
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LIM-06 (pure supplement, render cadence): the first frame is due at
    /// once; at 30 fps the next only 33,333,333 ns after the last painted,
    /// not a nanosecond sooner, however often it is asked; at 1 and 120 fps
    /// the interval is one second and 8,333,333 ns.
    #[test]
    fn frames_are_due_at_most_once_per_frame_interval() {
        let start = Instant::now();
        let nanosecond = Duration::from_nanos(1);
        for (fps, nanos) in [(30, 33_333_333), (1, 1_000_000_000), (120, 8_333_333)] {
            let mut frames = Frames::new(fps);
            assert!(frames.due(start));
            frames.painted(start);
            let interval = Duration::from_nanos(nanos);
            assert!(!frames.due(start + interval - nanosecond));
            assert!(!frames.due(start + interval - nanosecond));
            assert!(frames.due(start + interval));
            frames.painted(start + interval);
            assert!(!frames.due(start + 2 * interval - nanosecond));
            assert!(frames.due(start + 2 * interval));
        }
    }

    /// LIM-06 (pure supplement): a clock that steps backwards never makes a
    /// frame due early.
    #[test]
    fn an_earlier_instant_is_never_due() {
        let start = Instant::now() + Duration::from_secs(1);
        let mut frames = Frames::new(30);
        frames.painted(start);
        assert!(!frames.due(start - Duration::from_millis(500)));
    }

    /// Work items taken from `work` in one turn begun at `start`, each
    /// taking `step`: (client, item) in order.
    fn turn(work: &mut Work<char, u32>, start: Instant, step: Duration) -> Vec<(char, u32)> {
        work.begin(start);
        let mut done = Vec::new();
        let mut now = start;
        while let Some(item) = work.next(now) {
            done.push(item);
            now += step;
        }
        done
    }

    /// LIM-06 (pure supplement, work quanta): with a 2 ms client quantum and
    /// 1 ms items, each client does two items, at 0 and 1 ms, and its third
    /// waits: at exactly 2 ms the next client with work has its turn. A
    /// client whose work runs out hands on early. The turn ends at exactly
    /// its 8 ms background quantum, one item short of it still doing work;
    /// the next turn starts with the client after the one served last.
    #[test]
    fn clients_take_fair_turns_within_both_quanta() {
        let ms = Duration::from_millis(1);
        let mut work = Work::new(2 * ms, 8 * ms);
        for item in 1..=5 {
            work.push('a', item);
            work.push('b', item);
        }
        work.push('c', 1);
        let start = Instant::now();
        assert_eq!(
            turn(&mut work, start, ms),
            [
                ('a', 1),
                ('a', 2),
                ('b', 1),
                ('b', 2),
                ('c', 1),
                ('a', 3),
                ('a', 4),
                ('b', 3),
            ]
        );
        // Begun at once, the next turn starts with the client after b,
        // although b's quantum would still last.
        assert_eq!(
            turn(&mut work, start + 8 * ms, ms),
            [('a', 5), ('b', 4), ('b', 5)]
        );
        assert_eq!(work.next(start + 9 * ms), None);
    }

    /// LIM-06 (pure supplement): one client alone keeps its turns, quantum
    /// after quantum, until the background quantum; nothing is done before
    /// a turn begins, and at exactly the background quantum nothing more.
    #[test]
    fn a_lone_client_is_bounded_by_the_background_quantum() {
        let ms = Duration::from_millis(1);
        let mut work = Work::new(2 * ms, 3 * ms);
        let start = Instant::now();
        for item in 1..=9 {
            work.push('a', item);
        }
        assert_eq!(work.next(start), None);
        assert_eq!(turn(&mut work, start, ms), [('a', 1), ('a', 2), ('a', 3)]);
        work.begin(start);
        assert_eq!(
            work.next(start + 3 * ms - Duration::from_nanos(1)),
            Some(('a', 4))
        );
        assert_eq!(work.next(start + 3 * ms), None);
    }
}
