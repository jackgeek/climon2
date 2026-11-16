//! Initial selection (spec #73, Session registry, "Initial selection";
//! #266): pure. The monitor collects statuses with bounded progress until the
//! first discovery pass has ended and each client started for it is ready or
//! past its `deadlines.start_ms`, then takes the first eligible row. With
//! nothing eligible then, the first row that becomes eligible later is taken
//! once (owner decision, 2026-11-16). Any selection, automatic or explicit,
//! ends it: nothing is ever chosen automatically again.

use crate::registry::Registry;
use climon2_core::SessionId;
use std::{
    collections::BTreeMap,
    fmt,
    time::{Duration, Instant},
};

/// Where initial selection stands.
pub(crate) struct Initial {
    /// `deadlines.start_ms`: how long collection waits for each client,
    /// from when the monitor started it.
    start: Duration,
    phase: Phase,
    /// The first pass's clients that were still starting when collection
    /// ended at their deadlines.
    late: Vec<SessionId>,
}

enum Phase {
    /// Until the first discovery pass has ended and each client started for
    /// it is ready or past its deadline.
    Collecting {
        passed: bool,
        deadlines: BTreeMap<SessionId, Instant>,
    },
    /// Collection found nothing eligible: the first row to become eligible
    /// is taken.
    Waiting,
    /// A selection exists, automatic or explicit.
    Done,
}

/// The startup progress the body shows while collecting.
pub(crate) struct Progress {
    sessions: usize,
    ready: usize,
}

impl fmt::Display for Progress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { sessions, ready } = self;
        let noun = ["sessions", "session"][usize::from(*sessions == 1)];
        write!(formatter, "Connecting to {sessions} {noun} ({ready} ready)")
    }
}

impl Initial {
    pub(crate) fn new(start: Duration) -> Self {
        Self {
            start,
            phase: Phase::Collecting {
                passed: false,
                deadlines: BTreeMap::new(),
            },
            late: Vec::new(),
        }
    }

    /// The monitor started `id`'s client at `now`: collection waits for it
    /// while the first pass runs.
    pub(crate) fn started(&mut self, id: SessionId, now: Instant) {
        if let Phase::Collecting {
            passed: false,
            deadlines,
        } = &mut self.phase
        {
            deadlines.insert(id, now + self.start);
        }
    }

    /// A discovery pass ended.
    pub(crate) fn passed(&mut self) {
        if let Phase::Collecting { passed, .. } = &mut self.phase {
            *passed = true;
        }
    }

    /// A selection exists: nothing is chosen automatically again.
    pub(crate) fn selected(&mut self) {
        self.phase = Phase::Done;
    }

    /// The row to select and acquire now, if any: the first eligible row
    /// once collection has ended, then never again.
    pub(crate) fn choose(&mut self, registry: &Registry, now: Instant) -> Option<SessionId> {
        self.collect(registry, now);
        let waiting = matches!(self.phase, Phase::Waiting);
        let chosen = waiting.then(|| registry.eligible()).flatten()?;
        self.selected();
        Some(chosen)
    }

    /// Ends collection once the first pass has ended and no client started
    /// for it is still starting before its deadline, keeping those that
    /// were still starting at it.
    fn collect(&mut self, registry: &Registry, now: Instant) {
        let Phase::Collecting { passed, deadlines } = &self.phase else {
            return;
        };
        let holding =
            |(id, deadline): (&SessionId, &Instant)| now < *deadline && registry.starting(id);
        if *passed && !deadlines.iter().any(holding) {
            self.late = starting(deadlines.keys(), registry);
            self.phase = Phase::Waiting;
        }
    }

    /// The startup progress while collecting.
    pub(crate) fn progress(&self, registry: &Registry) -> Option<Progress> {
        let Phase::Collecting { deadlines, .. } = &self.phase else {
            return None;
        };
        let sessions = deadlines.len();
        let ready = sessions - starting(deadlines.keys(), registry).len();
        Some(Progress { sessions, ready })
    }

    /// Whether a client collection stopped waiting for is still starting:
    /// the list it chose from was incomplete.
    pub(crate) fn late(&self, registry: &Registry) -> bool {
        !starting(&self.late, registry).is_empty()
    }
}

/// Those of `ids` whose rows still await their clients.
fn starting<'a>(
    ids: impl IntoIterator<Item = &'a SessionId>,
    registry: &Registry,
) -> Vec<SessionId> {
    ids.into_iter()
        .filter(|id| registry.starting(id))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{ClientEvent, Found, Report, State, Status};
    use climon2_core::metadata::SessionMetadata;
    use std::time::{Duration, Instant, SystemTime};

    const START: Duration = Duration::from_secs(10);

    fn metadata(suffix: u8) -> SessionMetadata {
        serde_json::from_value(serde_json::json!({
            "format_version": 1,
            "session_id": format!("00000000-0000-4000-8000-0000000000{suffix:02x}"),
            "socket_path": "/tmp/s", "daemon_pid": 1, "daemon_start_time": 0,
            "command": "cmd", "args": [], "cwd": "/", "depth": 1,
            "created_at": "2026-11-13T10:00:00.000Z", "climon2_version": "0.1.0"
        }))
        .unwrap()
    }

    /// Discovery found `suffix` live; its client starts at `now`.
    fn found(
        registry: &mut Registry,
        initial: &mut Initial,
        suffix: u8,
        now: Instant,
    ) -> SessionId {
        let session = metadata(suffix);
        let id = session.session_id.clone();
        registry.discovered(session, Found::Live, SystemTime::UNIX_EPOCH);
        initial.started(id.clone(), now);
        id
    }

    /// `id`'s client opens with `status-v1` and reports `state`, changed
    /// `changed` seconds after the epoch.
    fn reports(registry: &mut Registry, id: &SessionId, state: State, changed: u64) {
        let now = SystemTime::UNIX_EPOCH;
        registry.client(id, ClientEvent::Opened { status: true }, now);
        let status = Status {
            state,
            changed: now + Duration::from_secs(changed),
            episode: None,
        };
        registry.client(id, ClientEvent::Report(Report::Status(status)), now);
    }

    fn progress(initial: &Initial, registry: &Registry) -> Option<String> {
        initial
            .progress(registry)
            .map(|progress| progress.to_string())
    }

    /// DSC-01 (initial-selection case, pure supplement): collection lasts
    /// until the first pass has ended and every client started for it is
    /// ready, showing its progress, whatever became eligible meanwhile; it
    /// then takes the first eligible row in row order, not the first to
    /// connect, once. Clients started after the first pass are not waited
    /// for.
    #[test]
    fn collection_waits_for_the_first_pass_and_its_clients_then_takes_the_first_eligible_row() {
        let (mut registry, mut initial) = (Registry::new(None), Initial::new(START));
        let t0 = Instant::now();
        let idle = found(&mut registry, &mut initial, 1, t0);
        let attention = found(&mut registry, &mut initial, 2, t0);
        let failed = found(&mut registry, &mut initial, 3, t0);
        assert_eq!(
            progress(&initial, &registry).as_deref(),
            Some("Connecting to 3 sessions (0 ready)")
        );
        reports(&mut registry, &idle, State::Idle, 50);
        assert_eq!(initial.choose(&registry, t0), None);
        initial.passed();
        found(&mut registry, &mut initial, 4, t0);
        registry.client(
            &failed,
            ClientEvent::Unavailable("no".into()),
            SystemTime::UNIX_EPOCH,
        );
        assert_eq!(
            progress(&initial, &registry).as_deref(),
            Some("Connecting to 3 sessions (2 ready)")
        );
        assert_eq!(initial.choose(&registry, t0), None);
        reports(&mut registry, &attention, State::NeedsAttention, 10);
        assert_eq!(initial.choose(&registry, t0), Some(attention));
        assert_eq!(progress(&initial, &registry), None);
        assert!(!initial.late(&registry));
        let later = found(&mut registry, &mut initial, 5, t0);
        reports(&mut registry, &later, State::NeedsAttention, 99);
        assert_eq!(initial.choose(&registry, t0), None);
    }

    /// Awaiting resources rows (#120): a first-pass client that is
    /// resource-blocked is settled, ready in the progress, so collection
    /// ends without waiting for its deadline and takes the eligible row;
    /// one whose release loads slowly is still waited for.
    #[test]
    fn a_resource_blocked_row_is_settled_for_the_startup_wait() {
        let (mut registry, mut initial) = (Registry::new(None), Initial::new(START));
        let t0 = Instant::now();
        let idle = found(&mut registry, &mut initial, 1, t0);
        let blocked = found(&mut registry, &mut initial, 2, t0);
        let slow = found(&mut registry, &mut initial, 3, t0);
        reports(&mut registry, &idle, State::Idle, 50);
        let now = SystemTime::UNIX_EPOCH;
        let short = ClientEvent::AwaitingResources("not enough memory".into());
        registry.client(&blocked, short, now);
        registry.client(&slow, ClientEvent::LoadingSlowly, now);
        initial.passed();
        assert_eq!(
            progress(&initial, &registry).as_deref(),
            Some("Connecting to 3 sessions (2 ready)")
        );
        assert_eq!(initial.choose(&registry, t0), None);
        registry.client(&slow, ClientEvent::Unavailable("refused".into()), now);
        assert_eq!(initial.choose(&registry, t0), Some(idle));
    }

    /// The owner's wording (2026-11-16, on #266): the startup progress says
    /// one session in the singular and any other count in the plural, at
    /// one and one past it.
    #[test]
    fn the_progress_says_one_session_in_the_singular() {
        let shown = |sessions, ready| Progress { sessions, ready }.to_string();
        assert_eq!(shown(1, 0), "Connecting to 1 session (0 ready)");
        assert_eq!(shown(2, 1), "Connecting to 2 sessions (1 ready)");
        assert_eq!(shown(0, 0), "Connecting to 0 sessions (0 ready)");
    }

    /// Confirmed 2026-11-08 (empty start) and the owner decision of
    /// 2026-11-16: with nothing eligible when collection ends, nothing is
    /// chosen; the first row that becomes eligible later, a new arrival or
    /// a row that was Starting, is taken once, and later arrivals never
    /// move it (DSC-03 selection case).
    #[test]
    fn with_nothing_eligible_the_first_row_to_become_eligible_is_taken_once() {
        let (mut registry, mut initial) = (Registry::new(None), Initial::new(START));
        let t0 = Instant::now();
        initial.passed();
        assert_eq!(initial.choose(&registry, t0), None);
        assert_eq!(progress(&initial, &registry), None);
        let slow = found(&mut registry, &mut initial, 1, t0);
        let other = found(&mut registry, &mut initial, 2, t0);
        assert_eq!(initial.choose(&registry, t0 + START * 9), None);
        reports(&mut registry, &slow, State::Idle, 1);
        assert_eq!(initial.choose(&registry, t0), Some(slow));
        reports(&mut registry, &other, State::NeedsAttention, 9);
        assert_eq!(initial.choose(&registry, t0), None);
    }

    /// Spec #73 initial selection: once any selection exists, explicit
    /// included, nothing is chosen automatically, during collection or
    /// after it.
    #[test]
    fn an_explicit_selection_ends_initial_selection() {
        for passed in [false, true] {
            let (mut registry, mut initial) = (Registry::new(None), Initial::new(START));
            let t0 = Instant::now();
            let id = found(&mut registry, &mut initial, 1, t0);
            if passed {
                initial.passed();
                assert_eq!(initial.choose(&registry, t0 + START), None);
            }
            initial.selected();
            initial.passed();
            reports(&mut registry, &id, State::Idle, 1);
            assert_eq!(progress(&initial, &registry), None);
            assert_eq!(initial.choose(&registry, t0), None);
        }
    }

    /// DSC-01 (bounded collection; uncertainty explicit), with the
    /// `deadlines.start_ms` boundary: a client still starting holds
    /// collection until just before its deadline, and at the deadline
    /// collection ends without it, but never before the first pass ends. While such a late client is still
    /// starting, collection is reported late; once it is ready it is not.
    #[test]
    fn a_client_past_its_start_deadline_no_longer_holds_collection_and_is_reported_late() {
        let (mut registry, mut initial) = (Registry::new(None), Initial::new(START));
        let t0 = Instant::now();
        let ready = found(&mut registry, &mut initial, 1, t0);
        let slow = found(&mut registry, &mut initial, 2, t0 + Duration::from_secs(1));
        reports(&mut registry, &ready, State::Running, 5);
        let deadline = t0 + Duration::from_secs(1) + START;
        assert_eq!(initial.choose(&registry, deadline + START), None);
        initial.passed();
        assert_eq!(
            initial.choose(&registry, deadline - Duration::from_nanos(1)),
            None
        );
        assert!(!initial.late(&registry));
        assert_eq!(initial.choose(&registry, deadline), Some(ready));
        assert!(initial.late(&registry));
        registry.client(
            &slow,
            ClientEvent::Opened { status: false },
            SystemTime::UNIX_EPOCH,
        );
        assert!(!initial.late(&registry));
    }
}
