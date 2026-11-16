//! Fair start scheduling (#120; operational limits, "Discovery, loading and
//! start scheduling"): pure. Sessions waiting for a client wait here, in two
//! lanes. First starts run newest created first (then ascending session
//! ID), except that after `scheduling.newer_start_burst` newer starts the
//! oldest eligible one gets a turn. Passive retries wait in arrival order.
//! When both lanes have an eligible session they take turns (owner decision
//! on #120, 2026-11-16), so retries share the start lane without
//! monopolizing first starts. Which sessions are eligible (their release
//! loaded, not waiting out an admission interval) is the caller's; a
//! session whose release stalls is simply never eligible, so it blocks no
//! other release's sessions.
//!
//! #117's seam: a Kill-only instance for a resource-blocked row goes
//! through admission at the head of the queue (owner decision on #120), as
//! a lane picked before both of these.
use climon2_core::{SessionId, Timestamp, metadata::SessionMetadata};
use std::{cmp::Reverse, collections::BTreeMap, time::Instant};

/// Which lane a waiting session is in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Lane {
    First,
    Retry,
}

/// What a waiting row was last told about why it waits, if anything.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Condition {
    /// Its release has loaded for longer than `deadlines.load_ms`.
    LoadingSlowly,
    /// It is resource-blocked, with why.
    AwaitingResources(String),
}

/// One waiting session.
#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub(super) metadata: SessionMetadata,
    pub(super) lane: Lane,
    /// When it entered the queue, for the retry lane's order.
    arrival: u64,
    /// Admission refused it with why: it is reconsidered from then on.
    pub(super) blocked: Option<(String, Instant)>,
    /// What its row was last told while it waits.
    pub(super) told: Option<Condition>,
}

type Key = (Reverse<Timestamp>, SessionId);

/// The sessions waiting for a client.
pub(super) struct Queue {
    waiting: BTreeMap<Key, Entry>,
    /// `scheduling.newer_start_burst`.
    burst: u32,
    /// Newer first starts since the oldest last had its turn.
    newer: u32,
    /// The retry lane has the next turn when both lanes have a session.
    retry_next: bool,
    arrivals: u64,
}

impl Queue {
    pub(super) fn new(burst: u32) -> Self {
        Self {
            waiting: BTreeMap::new(),
            burst,
            newer: 0,
            retry_next: false,
            // Only the order of arrivals matters, not where they start.
            arrivals: u64::MIN,
        }
    }

    /// `metadata`'s session waits in `lane`, behind those already waiting
    /// there.
    pub(super) fn push(&mut self, metadata: SessionMetadata, lane: Lane) {
        self.arrivals += 1;
        let entry = Entry {
            metadata,
            lane,
            arrival: self.arrivals,
            blocked: None,
            told: None,
        };
        self.put(entry);
    }

    /// Puts a picked entry back where it waited, as it is now.
    pub(super) fn put(&mut self, entry: Entry) {
        let key = key(&entry.metadata);
        self.waiting.insert(key, entry);
    }

    /// Every waiting session, newest created first.
    pub(super) fn entries(&mut self) -> impl Iterator<Item = &mut Entry> {
        self.waiting.values_mut()
    }

    /// Takes every waiting session `taken` says.
    pub(super) fn take(&mut self, mut taken: impl FnMut(&Entry) -> bool) -> Vec<Entry> {
        let keys: Vec<_> = self
            .waiting
            .iter()
            .filter(|(_, entry)| taken(entry))
            .map(|(key, _)| key.clone())
            .collect();
        keys.iter()
            .filter_map(|key| self.waiting.remove(key))
            .collect()
    }

    /// Takes the next session to start among those `eligible` says, if
    /// any: the lane whose turn it is when both have one.
    pub(super) fn pick(&mut self, eligible: impl Fn(&Entry) -> bool) -> Option<Entry> {
        let firsts: Vec<Key> = self.eligible(Lane::First, &eligible).collect();
        let retry = self
            .eligible(Lane::Retry, &eligible)
            .min_by_key(|key| self.waiting[key].arrival);
        let key = self.choose(&firsts, retry)?;
        let entry = self.waiting.remove(&key)?;
        self.retry_next = entry.lane == Lane::First;
        Some(entry)
    }

    /// The retry when it has its turn or no first start is eligible,
    /// otherwise a first start, if any.
    fn choose(&mut self, firsts: &[Key], retry: Option<Key>) -> Option<Key> {
        let retry_turn = self.retry_next || firsts.is_empty();
        retry
            .filter(|_| retry_turn)
            .or_else(|| (!firsts.is_empty()).then(|| self.first(firsts)))
    }

    /// The keys of `lane`'s eligible sessions, newest created first.
    fn eligible<'a>(
        &'a self,
        lane: Lane,
        eligible: &'a impl Fn(&Entry) -> bool,
    ) -> impl Iterator<Item = Key> + 'a {
        self.waiting
            .iter()
            .filter(move |(_, entry)| entry.lane == lane && eligible(entry))
            .map(|(key, _)| key.clone())
    }

    /// The first start to take from `firsts` (newest first, never empty):
    /// the newest, unless the burst of newer starts is spent, when the
    /// oldest has its turn. Taking the oldest starts a new burst.
    fn first(&mut self, firsts: &[Key]) -> Key {
        let oldest = firsts.len() - 1;
        let index = [0, oldest][usize::from(self.newer >= self.burst)];
        self.newer = [self.newer + 1, 0][usize::from(index == oldest)];
        firsts[index].clone()
    }
}

fn key(metadata: &SessionMetadata) -> Key {
    (
        Reverse(metadata.created_at.clone()),
        metadata.session_id.clone(),
    )
}

#[cfg(test)]
mod tests {
    //! DSC-02 and LIM-06 (Monitor, start case), deterministically: which
    //! session each free start slot goes to.
    use super::*;

    fn metadata(index: u8, created: &str) -> SessionMetadata {
        serde_json::from_value(serde_json::json!({
            "format_version": 1,
            "session_id": format!("00000000-0000-4000-8000-0000000000{index:02x}"),
            "socket_path": "/tmp/s", "daemon_pid": 1, "daemon_start_time": 0,
            "command": "fake", "args": [], "cwd": "/", "depth": 1,
            "created_at": created, "climon2_version": "0.1.0"
        }))
        .unwrap()
    }

    /// Session `index`, created `second` seconds into the minute.
    fn at(index: u8, second: u8) -> SessionMetadata {
        metadata(index, &format!("2026-11-16T10:00:{second:02}.000Z"))
    }

    fn index(entry: &Entry) -> u8 {
        let id = entry.metadata.session_id.as_str();
        u8::from_str_radix(&id[id.len() - 2..], 16).unwrap()
    }

    /// The sessions picked, in order, until none is eligible.
    fn order(queue: &mut Queue, eligible: impl Fn(&Entry) -> bool) -> Vec<u8> {
        std::iter::from_fn(|| queue.pick(&eligible))
            .map(|entry| index(&entry))
            .collect()
    }

    fn all(_: &Entry) -> bool {
        true
    }

    /// First starts run newest created first, then by ascending session ID,
    /// while no burst is spent.
    #[test]
    fn first_starts_run_newest_first_with_ascending_id_ties() {
        let mut queue = Queue::new(8);
        queue.push(at(1, 1), Lane::First);
        queue.push(at(4, 3), Lane::First);
        queue.push(at(3, 3), Lane::First);
        queue.push(at(2, 2), Lane::First);
        assert_eq!(order(&mut queue, all), [3, 4, 2, 1]);
    }

    /// After exactly `newer_start_burst` newer starts the oldest waiting
    /// session has its turn, not one start sooner; then a new burst begins.
    #[test]
    fn the_oldest_gets_a_turn_after_each_burst_of_newer_starts() {
        let mut queue = Queue::new(2);
        for index in 1..=5 {
            queue.push(at(index, index), Lane::First);
        }
        assert_eq!(order(&mut queue, all), [5, 4, 1, 3, 2]);
        let mut queue = Queue::new(1);
        for index in 1..=4 {
            queue.push(at(index, index), Lane::First);
        }
        assert_eq!(order(&mut queue, all), [4, 1, 3, 2]);
    }

    /// Continuous arrivals of newer sessions never keep the oldest from
    /// starting: it starts once the burst after its arrival is spent.
    #[test]
    fn continuous_newer_arrivals_never_starve_the_oldest() {
        let mut queue = Queue::new(3);
        queue.push(at(1, 1), Lane::First);
        let mut started = Vec::new();
        for arrival in 2..=5 {
            queue.push(at(arrival, arrival), Lane::First);
            started.extend(queue.pick(all).map(|entry| index(&entry)));
        }
        assert_eq!(started, [2, 3, 4, 1]);
    }

    /// Retries wait in arrival order, whatever their creation time, and
    /// take turns with first starts while both wait; either lane alone
    /// takes every slot.
    #[test]
    fn retries_and_first_starts_take_turns() {
        let mut queue = Queue::new(8);
        queue.push(at(1, 1), Lane::First);
        queue.push(at(2, 2), Lane::First);
        queue.push(at(3, 3), Lane::First);
        queue.push(at(8, 8), Lane::Retry);
        queue.push(at(6, 6), Lane::Retry);
        queue.push(at(9, 9), Lane::Retry);
        queue.push(at(7, 7), Lane::Retry);
        assert_eq!(order(&mut queue, all), [3, 8, 2, 6, 1, 9, 7]);
        queue.push(at(5, 5), Lane::Retry);
        queue.push(at(4, 4), Lane::First);
        assert_eq!(order(&mut queue, all), [4, 5]);
    }

    /// A session that is not eligible (its release still loading, or
    /// waiting out its admission interval) is passed over and keeps its
    /// place; with none eligible nothing is picked.
    #[test]
    fn ineligible_sessions_keep_their_place_and_block_no_other() {
        let mut queue = Queue::new(8);
        queue.push(at(1, 1), Lane::First);
        queue.push(at(2, 2), Lane::First);
        queue.push(at(3, 3), Lane::Retry);
        let stalled = |entry: &Entry| index(entry) != 2 && index(entry) != 3;
        assert_eq!(order(&mut queue, stalled), [1]);
        assert!(queue.pick(|_| false).is_none());
        let mut entries: Vec<_> = queue.entries().map(|entry| index(entry)).collect();
        entries.sort_unstable();
        assert_eq!(entries, [2, 3]);
        let mut picked = queue.pick(|entry| entry.lane == Lane::First).unwrap();
        picked.blocked = Some(("short".into(), Instant::now()));
        assert_eq!(index(&picked), 2);
        queue.put(picked);
        let taken = queue.take(|entry| entry.blocked.is_some());
        assert_eq!(taken.iter().map(index).collect::<Vec<_>>(), [2]);
        let rest = queue.take(|_| true);
        assert_eq!(rest.iter().map(index).collect::<Vec<_>>(), [3]);
        assert!(queue.pick(all).is_none());
    }
}
