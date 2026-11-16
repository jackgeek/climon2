//! The session registry (spec #73, "Session registry"): pure, and the single
//! owner of row state. Discovery outcomes and client events are its inputs,
//! with the time the monitor observed them; it decides which sessions need a
//! client and what each row shows. Rows are keyed by full session ID, which
//! is never shown.
use crate::terminal::Presentation;
use climon2_core::{SessionId, metadata::SessionMetadata};
use std::{borrow::Cow, cmp::Reverse, collections::BTreeMap, time::SystemTime};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// What one discovery probe found about a session with valid metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Found {
    Live,
    /// Neither proven live nor proven ended.
    Uncertain,
    /// Definitively stale: the daemon is provably gone.
    Stale,
}

/// What a session's passive client reported.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClientEvent {
    /// Monitor Open succeeded, selecting `status-v1` or not.
    Opened { status: bool },
    /// The client cannot serve this session, with why.
    Unavailable(String),
    /// The connected client's connection ended.
    Ended,
    /// What the daemon reported about its session.
    Report(Report),
    /// Control and display, for the session's terminal: never row state.
    Presentation(Presentation),
    /// A request's answer, matched to its request by the supervisor.
    Answer(u64, Result<(), String>),
}

/// What a daemon reports about its session over a connected client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Report {
    /// The daemon title: `None` when unset or unsupported.
    Title(Option<String>),
    /// The effective status changed, or the Initial metadata reported it.
    Status(Status),
    /// The command's final outcome is known.
    Final(Final),
}

/// A session's effective status that is not final, as its daemon reports
/// it through `status-v1`, in ordering-group order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum State {
    NeedsAttention,
    Acknowledged,
    Running,
    Idle,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            Self::NeedsAttention => "Needs attention",
            Self::Acknowledged => "Acknowledged",
            Self::Running => "Running",
            Self::Idle => "Idle",
        }
    }
}

/// The daemon's effective status, with the daemon's stored time of its last
/// effective change: never the time the monitor heard it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Status {
    pub(crate) state: State,
    pub(crate) changed: SystemTime,
}

/// The command's authoritative result: exit 143 is not signal 15.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Exit(u32),
    Signal(u32),
}

impl Outcome {
    /// Its ordering group and display status: Succeeded for exit zero
    /// (`completed`), otherwise Failed with the exact code or signal.
    fn shown(self) -> (Group, String) {
        match self {
            Self::Exit(0) => (Group::Completed, "Succeeded".to_owned()),
            Self::Exit(code) => (Group::Failed, format!("Failed (exit {code})")),
            Self::Signal(number) => (Group::Failed, format!("Failed (signal {number})")),
        }
    }
}

/// A definitive final outcome, with the daemon's terminal status change time
/// when it reported one (only with `status-v1`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Final {
    pub(crate) outcome: Outcome,
    pub(crate) changed: Option<SystemTime>,
}

/// The ordering groups, in order (spec #73, Session registry, Ordering).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Group {
    Status(State),
    Failed,
    Completed,
    /// Not connected, or connected with no current status.
    Disconnected,
}

/// Whether the row has a client, and how far it got.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Connection {
    /// No client: discovery has not proven the session live.
    None,
    Starting,
    Connected {
        status: bool,
    },
    Unavailable(String),
    Disconnected,
}

impl Connection {
    /// The not-connected label of a row with no current status and no final
    /// outcome. A client opened with `status-v1` is still Starting until its
    /// Initial metadata reports the status; one without it is Unavailable.
    fn label(&self) -> &'static str {
        match self {
            Self::Starting | Self::Connected { status: true } => "Starting",
            Self::Disconnected => "Disconnected",
            Self::None | Self::Connected { status: false } | Self::Unavailable(_) => "Unavailable",
        }
    }
}

#[derive(Clone, Debug)]
struct Row {
    metadata: SessionMetadata,
    connection: Connection,
    /// When the monitor recorded entering `connection`.
    since: SystemTime,
    /// Whether a client was ever connected: such a row outlives its
    /// session.
    connected: bool,
    /// The latest nonempty daemon title.
    title: Option<String>,
    /// The latest daemon status: current while connected, otherwise stale
    /// detail that neither labels nor sorts the row.
    status: Option<Status>,
    /// The final outcome and the time it sorts by: the daemon's terminal
    /// status change time, otherwise when the monitor learned it. It
    /// survives disconnection.
    ended: Option<(Outcome, SystemTime)>,
}

impl Row {
    fn new(metadata: SessionMetadata, connection: Connection, now: SystemTime) -> Self {
        Self {
            metadata,
            connection,
            since: now,
            connected: false,
            title: None,
            status: None,
            ended: None,
        }
    }

    /// The row's ordering group, the time it sorts by within it, and its
    /// display status: a final outcome first, then a connected row's
    /// current status, otherwise its not-connected condition since the
    /// monitor recorded entering it.
    fn standing(&self) -> (Group, SystemTime, String) {
        let connected = matches!(self.connection, Connection::Connected { .. });
        match (self.ended, self.status.filter(|_| connected)) {
            (Some((outcome, at)), _) => {
                let (group, label) = outcome.shown();
                (group, at, label)
            }
            (None, Some(status)) => (
                Group::Status(status.state),
                status.changed,
                status.state.label().to_owned(),
            ),
            (None, None) => (
                Group::Disconnected,
                self.since,
                self.connection.label().to_owned(),
            ),
        }
    }

    /// The latest nonempty title, otherwise command plus arguments.
    fn label(&self) -> String {
        let fallback = || {
            std::iter::once(self.metadata.command.as_str())
                .chain(self.metadata.args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" ")
        };
        literal(&self.title.clone().unwrap_or_else(fallback))
    }

    fn connect(&mut self, connection: Connection, now: SystemTime) {
        self.connected |= matches!(connection, Connection::Connected { .. });
        if self.connection != connection {
            self.connection = connection;
            self.since = now;
        }
    }

    /// Whether the row awaits its client: no status or outcome yet, and a
    /// client starting or opened with `status-v1`.
    fn starting(&self) -> bool {
        let waiting = matches!(
            self.connection,
            Connection::Starting | Connection::Connected { status: true }
        );
        waiting && self.standing().0 == Group::Disconnected
    }

    /// What Enter does on this row, and what its canvas shows without a view.
    fn kind(&self) -> Kind {
        match (&self.connection, self.ended.is_some()) {
            (Connection::Disconnected, _) | (_, true) => Kind::Retained,
            (Connection::Connected { .. }, false) => Kind::Connected,
            _ => Kind::Unconnected,
        }
    }

    /// A report changes what the row shows, a connection event its
    /// connection; control, display and answers are never row state.
    fn apply(&mut self, event: ClientEvent, now: SystemTime) {
        match event {
            ClientEvent::Report(report) => self.learn(report, now),
            event => {
                if let Some(connection) = connection(event) {
                    self.connect(connection, now);
                }
            }
        }
    }

    /// Keeps the latest nonempty title and the latest status; keeps the
    /// first final outcome, sorting by its terminal status time or else
    /// `now`, when the monitor learned it.
    fn learn(&mut self, report: Report, now: SystemTime) {
        match report {
            Report::Title(title) => self.retitle(title),
            Report::Status(status) => self.status = Some(status),
            Report::Final(known) => {
                let at = known.changed.unwrap_or(now);
                self.ended.get_or_insert((known.outcome, at));
            }
        }
    }

    /// Keeps a nonempty title; an empty or unset one changes nothing.
    fn retitle(&mut self, title: Option<String>) {
        if let Some(title) = title.filter(|title| !title.is_empty()) {
            self.title = Some(title);
        }
    }
}

/// What Enter does on a row (spec #73, "Enter"), and what the canvas
/// shows for a selected row whose terminal has no view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    /// Connected and not known to be final: selected and acquired, and the
    /// non-controller placeholder while the monitor does not control it.
    Connected,
    /// Starting, or Unavailable with no connected client: selected
    /// but never acquired, with no terminal content to show.
    Unconnected,
    /// Disconnected or known final: its retained history arrives with #116;
    /// until then Enter ignores it.
    Retained,
}

/// One row as the selector shows it. The session ID identifies it and is
/// never displayed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct View {
    pub(crate) id: SessionId,
    pub(crate) status: String,
    /// Literal: control characters escaped, never interpreted.
    pub(crate) label: String,
    /// When the monitor recorded the row's current connection status.
    pub(crate) since: SystemTime,
}

pub(crate) struct Registry {
    own: Option<SessionId>,
    rows: BTreeMap<SessionId, Row>,
    /// Why the latest discovery pass was incomplete, if it was.
    incomplete: Option<String>,
}

impl Registry {
    /// `own` is this monitor's enclosing session (`CLIMON2_SESSION_ID`).
    pub(crate) fn new(own: Option<SessionId>) -> Self {
        Self {
            own,
            rows: BTreeMap::new(),
            incomplete: None,
        }
    }

    /// Records a probe's outcome. Returns the session's metadata when it now
    /// needs a passive client: a live session that has none. Discovery never
    /// changes a client's connection. A session proven gone drops its row
    /// unless it was ever connected, and adds none; this monitor's own
    /// session never has a row (owner decision on #110), so it can be
    /// neither listed nor selected.
    pub(crate) fn discovered(
        &mut self,
        metadata: SessionMetadata,
        found: Found,
        now: SystemTime,
    ) -> Option<SessionMetadata> {
        let id = metadata.session_id.clone();
        let own = self.own.as_ref() == Some(&id);
        if own || matches!(found, Found::Stale) {
            self.rows.retain(|key, row| *key != id || row.connected);
            return None;
        }
        let row = self
            .rows
            .entry(id)
            .or_insert_with(|| Row::new(metadata.clone(), Connection::None, now));
        let wanted = matches!(found, Found::Live) && row.connection == Connection::None;
        wanted.then(|| {
            row.connect(Connection::Starting, now);
            metadata
        })
    }

    /// Records what `id`'s client reported.
    pub(crate) fn client(&mut self, id: &SessionId, event: ClientEvent, now: SystemTime) {
        if let Some(row) = self.rows.get_mut(id) {
            row.apply(event, now);
        }
    }

    /// What Enter does on `id`'s row, if there is one.
    pub(crate) fn kind(&self, id: &SessionId) -> Option<Kind> {
        self.rows.get(id).map(Row::kind)
    }

    /// Records whether the latest pass reached every entry.
    pub(crate) fn pass(&mut self, incomplete: Option<String>) {
        self.incomplete = incomplete;
    }

    pub(crate) fn incomplete(&self) -> Option<&str> {
        self.incomplete.as_deref()
    }

    /// Every row by ordering group, then newest time first, then ascending
    /// full session ID.
    pub(crate) fn rows(&self) -> Vec<View> {
        self.ordered().into_iter().map(|(_, view)| view).collect()
    }

    /// The first row in row order that initial selection may take (spec
    /// #73, "Initial selection"): connected, its status known and not
    /// final. The own session has no row.
    pub(crate) fn eligible(&self) -> Option<SessionId> {
        self.ordered()
            .into_iter()
            .find(|(group, _)| matches!(group, Group::Status(_)))
            .map(|(_, view)| view.id)
    }

    /// Whether `id`'s row still awaits its client: Starting, also once
    /// opened with `status-v1` until its status arrives.
    pub(crate) fn starting(&self, id: &SessionId) -> bool {
        self.rows.get(id).is_some_and(Row::starting)
    }

    /// Every row as shown, with its ordering group, in row order.
    fn ordered(&self) -> Vec<(Group, View)> {
        let mut rows: Vec<_> = self
            .rows
            .values()
            .map(|row| {
                let (group, at, status) = row.standing();
                let view = View {
                    id: row.metadata.session_id.clone(),
                    status,
                    label: row.label(),
                    since: row.since,
                };
                ((group, Reverse(at)), view)
            })
            .collect();
        rows.sort_by(|(left, a), (right, b)| (left, &a.id).cmp(&(right, &b.id)));
        rows.into_iter()
            .map(|((group, _), view)| (group, view))
            .collect()
    }
}

/// The connection a client event moves its row to, if any.
fn connection(event: ClientEvent) -> Option<Connection> {
    match event {
        ClientEvent::Opened { status } => Some(Connection::Connected { status }),
        ClientEvent::Unavailable(why) => Some(Connection::Unavailable(why)),
        ClientEvent::Ended => Some(Connection::Disconnected),
        _ => None,
    }
}

/// Session text shown literally: control characters are escaped, so the
/// host never interprets them.
fn literal(text: &str) -> String {
    text.chars()
        .flat_map(|character| {
            let escaped: Vec<char> = if character.is_control() {
                character.escape_default().collect()
            } else {
                vec![character]
            };
            escaped
        })
        .collect()
}

/// `text` as it fits `cells` display cells: whole when it fits, otherwise
/// statically truncated to end with `…` within them (owner decision on
/// #112, 2026-11-16: nothing marquees).
/// Text with no cell at all is empty.
pub(crate) fn truncate(text: &str, cells: usize) -> Cow<'_, str> {
    if text.width() <= cells {
        Cow::Borrowed(text)
    } else if cells == 0 {
        Cow::Borrowed("")
    } else {
        // The ellipsis takes one cell.
        Cow::Owned(format!("{}…", clip(text, cells - 1)))
    }
}

/// The leading part of `text` that fits `cells` display cells, never
/// splitting a wide character; zero-width characters stay with the
/// character before them.
fn clip(text: &str, cells: usize) -> &str {
    let mut used = 0;
    for (index, character) in text.char_indices() {
        used += character.width().unwrap_or_default();
        if used > cells {
            return &text[..index];
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const OWN: &str = "00000000-0000-4000-8000-0000000000aa";

    fn metadata(id: &str, created: &str, command: &str, args: &[&str]) -> SessionMetadata {
        serde_json::from_value(serde_json::json!({
            "format_version": 1, "session_id": id, "socket_path": "/tmp/s",
            "daemon_pid": 1, "daemon_start_time": 0, "command": command, "args": args,
            "cwd": "/", "depth": 1, "created_at": created, "climon2_version": "0.1.0"
        }))
        .unwrap()
    }

    fn id(suffix: u8) -> String {
        format!("00000000-0000-4000-8000-0000000000{suffix:02x}")
    }

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    /// Each row as `status|label`, in order.
    fn shown(registry: &Registry) -> Vec<String> {
        registry
            .rows()
            .into_iter()
            .map(|row| format!("{}|{}", row.status, row.label))
            .collect()
    }

    /// Finds `suffix`'s live session, with `command` as its label, at
    /// `second`, when its client also opens, with `status-v1` or without it.
    fn opened(
        registry: &mut Registry,
        (suffix, command): (u8, &str),
        status: bool,
        second: u64,
    ) -> SessionId {
        let session = metadata(&id(suffix), "2026-11-13T10:00:00.000Z", command, &[]);
        let key = session.session_id.clone();
        registry.discovered(session, Found::Live, at(second));
        registry.client(&key, ClientEvent::Opened { status }, at(second));
        key
    }

    fn titled(title: Option<&str>) -> ClientEvent {
        ClientEvent::Report(Report::Title(title.map(str::to_owned)))
    }

    fn status(state: State, changed: u64) -> ClientEvent {
        ClientEvent::Report(Report::Status(Status {
            state,
            changed: at(changed),
        }))
    }

    fn ended(outcome: Outcome, changed: Option<u64>) -> ClientEvent {
        ClientEvent::Report(Report::Final(Final {
            outcome,
            changed: changed.map(at),
        }))
    }

    /// DSC-03 (pure supplement): live sessions need one client each, asked
    /// once however often they are found again. Rows waiting together for
    /// their clients tie on the time they started, so ascending full session
    /// ID orders them, whatever their creation times.
    #[test]
    fn live_sessions_need_one_client_each() {
        let mut registry = Registry::new(None);
        let tied = "2026-11-13T10:00:00.000Z";
        let found = [
            metadata(&id(3), tied, "c", &[]),
            metadata(&id(1), "2026-11-13T09:00:00.000Z", "old", &[]),
            metadata(&id(2), tied, "b", &[]),
            metadata(&id(9), "2026-11-13T11:00:00.000Z", "new", &[]),
        ];
        for metadata in &found {
            let wanted = registry.discovered(metadata.clone(), Found::Live, at(1));
            assert_eq!(wanted.as_ref(), Some(metadata));
        }
        for metadata in &found {
            assert_eq!(
                registry.discovered(metadata.clone(), Found::Live, at(2)),
                None
            );
        }
        assert_eq!(
            shown(&registry),
            ["Starting|old", "Starting|b", "Starting|c", "Starting|new"]
        );
    }

    /// DSC-01 (status order, pure supplement) and STA-07 (Monitor, pure
    /// supplement): rows order by group (needs attention, acknowledged,
    /// running, idle, failed, completed, then not connected), within a group
    /// by the daemon's change time, newest first, never the time the monitor
    /// heard it or the creation time, then by ascending full session ID on a
    /// tie. Only a new effective change time moves a row; a disconnected row
    /// keeps its last status as stale detail, sorting and labelled by its
    /// disconnection instead.
    #[test]
    fn rows_order_by_group_then_newest_daemon_change_then_session_id() {
        let mut registry = Registry::new(None);
        let events = [
            (9, "attention older", status(State::NeedsAttention, 100)),
            (8, "attention newer", status(State::NeedsAttention, 200)),
            (7, "acknowledged", status(State::Acknowledged, 50)),
            (5, "running tied low", status(State::Running, 300)),
            (6, "running tied high", status(State::Running, 300)),
            (4, "idle", status(State::Idle, 400)),
            (3, "failed", ended(Outcome::Exit(3), Some(10))),
            (2, "succeeded", ended(Outcome::Exit(0), Some(500))),
            (1, "disconnected", status(State::Running, 700)),
        ];
        let mut keys = BTreeMap::new();
        for (suffix, command, event) in events {
            let key = opened(&mut registry, (suffix, command), true, 1);
            registry.client(&key, event, at(900 - u64::from(suffix)));
            keys.insert(command, key);
        }
        registry.client(&keys["disconnected"], ClientEvent::Ended, at(600));
        let starting = metadata(&id(10), "2026-11-13T12:00:00.000Z", "starting", &[]);
        registry.discovered(starting, Found::Live, at(2));
        assert_eq!(
            shown(&registry),
            [
                "Needs attention|attention newer",
                "Needs attention|attention older",
                "Acknowledged|acknowledged",
                "Running|running tied low",
                "Running|running tied high",
                "Idle|idle",
                "Failed (exit 3)|failed",
                "Succeeded|succeeded",
                "Disconnected|disconnected",
                "Starting|starting",
            ]
        );
        let older = &keys["attention older"];
        registry.client(older, status(State::NeedsAttention, 100), at(950));
        assert_eq!(shown(&registry)[0], "Needs attention|attention newer");
        registry.client(older, status(State::NeedsAttention, 250), at(951));
        assert_eq!(shown(&registry)[0], "Needs attention|attention older");
    }

    /// COM-06 (Monitor labels, pure supplement) and final rows (derived):
    /// exit zero is Succeeded, a nonzero exit or a signal Failed with its
    /// exact code or signal, exit 143 apart from signal 15. A final row sorts
    /// by the daemon's terminal status time when it reported one, otherwise
    /// by when the monitor learned the outcome, with or without `status-v1`.
    /// The outcome survives disconnection, a later status never replaces it,
    /// and a repeated final changes nothing.
    #[test]
    fn final_outcomes_are_labelled_exactly_and_sort_by_their_time() {
        let mut registry = Registry::new(None);
        let outcomes = [
            (1, "signal", true, ended(Outcome::Signal(15), Some(25)), 99),
            (2, "exit 143", true, ended(Outcome::Exit(143), None), 20),
            (3, "exit 1", false, ended(Outcome::Exit(1), None), 10),
            (4, "exit 0", false, ended(Outcome::Exit(0), None), 30),
        ];
        let mut keys = Vec::new();
        for (suffix, command, selected, event, learned) in outcomes {
            let key = opened(&mut registry, (suffix, command), selected, 1);
            registry.client(&key, event, at(learned));
            keys.push(key);
        }
        let expected = [
            "Failed (signal 15)|signal",
            "Failed (exit 143)|exit 143",
            "Failed (exit 1)|exit 1",
            "Succeeded|exit 0",
        ];
        assert_eq!(shown(&registry), expected);
        registry.client(&keys[1], ClientEvent::Ended, at(40));
        registry.client(&keys[0], status(State::Running, 50), at(41));
        registry.client(&keys[2], ended(Outcome::Exit(0), Some(99)), at(42));
        assert_eq!(shown(&registry), expected);
    }

    /// Not-connected rows (confirmed 2026-11-08) and STA-07 (Monitor, pure
    /// supplement): a row with no current status and no final outcome sorts
    /// after every status and final row, by the newest time it entered its
    /// condition: Starting (also a client opened with `status-v1` awaiting
    /// its Initial metadata), Unavailable (uncertain, failed, or connected
    /// without `status-v1`) and Disconnected, whose stale status time neither
    /// sorts nor labels it. The own session has no row (owner decision on
    /// #110).
    #[test]
    fn not_connected_rows_sort_by_when_they_entered_their_condition() {
        let mut registry = Registry::new(Some(OWN.parse().unwrap()));
        let own = metadata(OWN, "2026-11-13T10:00:00.000Z", "own", &[]);
        registry.discovered(own, Found::Live, at(1));
        let starting = metadata(&id(1), "2026-11-13T10:00:00.000Z", "starting", &[]);
        registry.discovered(starting, Found::Live, at(2));
        let unsure = metadata(&id(2), "2026-11-13T10:00:00.000Z", "unsure", &[]);
        registry.discovered(unsure, Found::Uncertain, at(3));
        opened(&mut registry, (3, "without status"), false, 4);
        opened(&mut registry, (4, "awaiting"), true, 5);
        let lost = opened(&mut registry, (5, "lost"), true, 1);
        registry.client(&lost, status(State::Running, 100), at(5));
        registry.client(&lost, ClientEvent::Ended, at(6));
        let failed = opened(&mut registry, (6, "failed"), true, 1);
        registry.client(&failed, ClientEvent::Unavailable("no".into()), at(7));
        let idle = opened(&mut registry, (7, "idle"), true, 1);
        registry.client(&idle, status(State::Idle, 0), at(8));
        assert_eq!(
            shown(&registry),
            [
                "Idle|idle",
                "Unavailable|failed",
                "Disconnected|lost",
                "Starting|awaiting",
                "Unavailable|without status",
                "Unavailable|unsure",
                "Starting|starting",
            ]
        );
    }

    /// Own-session scenario (owner decision on #110, 2026-11-14): this
    /// monitor's own session never needs a client and has no row, live or
    /// uncertain, so it cannot be selected; other sessions still do.
    #[test]
    fn the_own_session_is_hidden_and_never_needs_a_client() {
        let mut registry = Registry::new(Some(OWN.parse().unwrap()));
        let own = metadata(OWN, "2026-11-13T10:00:00.000Z", "bash", &[]);
        assert_eq!(registry.discovered(own.clone(), Found::Live, at(1)), None);
        assert_eq!(
            registry.discovered(own.clone(), Found::Uncertain, at(2)),
            None
        );
        assert!(shown(&registry).is_empty());
        assert_eq!(registry.kind(&own.session_id), None);
        let other = metadata(&id(1), "2026-11-13T10:00:00.000Z", "other", &[]);
        assert_eq!(
            registry.discovered(other.clone(), Found::Live, at(3)),
            Some(other)
        );
        assert_eq!(shown(&registry), ["Starting|other"]);
    }

    /// DSC-04 (pure supplement): uncertainty and staleness never change a
    /// client's connection, an uncertain session shows Unavailable until
    /// proven live, and a stale session nobody knew adds no row.
    #[test]
    fn uncertainty_never_removes_rows_or_tears_down_clients() {
        let mut registry = Registry::new(None);
        let live = metadata(&id(1), "2026-11-13T10:00:00.000Z", "live", &[]);
        let unsure = metadata(&id(2), "2026-11-13T09:00:00.000Z", "unsure", &[]);
        let gone = metadata(&id(3), "2026-11-13T08:00:00.000Z", "gone", &[]);
        registry.discovered(live.clone(), Found::Live, at(1));
        registry.client(
            &live.session_id,
            ClientEvent::Opened { status: true },
            at(2),
        );
        assert_eq!(
            registry.discovered(unsure.clone(), Found::Uncertain, at(2)),
            None
        );
        assert_eq!(registry.discovered(gone.clone(), Found::Stale, at(2)), None);
        assert_eq!(shown(&registry), ["Starting|live", "Unavailable|unsure"]);
        registry.discovered(live.clone(), Found::Uncertain, at(3));
        registry.discovered(live.clone(), Found::Stale, at(3));
        assert_eq!(shown(&registry), ["Starting|live", "Unavailable|unsure"]);
        assert_eq!(registry.rows()[0].since, at(2));
        assert_eq!(
            registry.discovered(unsure.clone(), Found::Live, at(5)),
            Some(unsure)
        );
        assert_eq!(shown(&registry), ["Starting|unsure", "Starting|live"]);
        assert_eq!(registry.rows()[0].since, at(5));
    }

    /// Stale rows (owner decision on #106): a session proven gone drops its
    /// row unless it was ever seen connected, whatever its status. A row that
    /// was connected stays, and shows Disconnected once its connection ends,
    /// with when that happened.
    #[test]
    fn a_session_proven_gone_drops_its_row_unless_it_was_ever_connected() {
        let mut registry = Registry::new(None);
        let sessions = [1, 2, 3, 4].map(|suffix| {
            let created = format!("2026-11-13T10:00:0{suffix}.000Z");
            metadata(&id(suffix), &created, &format!("s{suffix}"), &[])
        });
        let [unsure, starting, failed, connected] = &sessions;
        registry.discovered(unsure.clone(), Found::Uncertain, at(1));
        for live in [starting, failed, connected] {
            registry.discovered(live.clone(), Found::Live, at(1));
        }
        registry.client(
            &failed.session_id,
            ClientEvent::Unavailable("refused".into()),
            at(2),
        );
        registry.client(
            &connected.session_id,
            ClientEvent::Opened { status: false },
            at(2),
        );
        registry.client(
            &connected.session_id,
            ClientEvent::Unavailable("lost".into()),
            at(3),
        );
        for session in &sessions {
            assert_eq!(
                registry.discovered(session.clone(), Found::Stale, at(4)),
                None
            );
        }
        assert_eq!(shown(&registry), ["Unavailable|s4"]);
        registry.client(&connected.session_id, ClientEvent::Ended, at(5));
        registry.client(&starting.session_id, ClientEvent::Ended, at(5));
        assert_eq!(shown(&registry), ["Disconnected|s4"]);
        assert_eq!(registry.rows()[0].since, at(5));
    }

    /// COM-01 (Monitor, pure supplement) and the not-connected labels: a
    /// connected row shows no fabricated status: Starting until its status
    /// arrives, or Unavailable without `status-v1`; a failed client is Unavailable and an ended one
    /// Disconnected, each recording when it entered that status.
    #[test]
    fn connection_statuses_are_labelled_as_specified() {
        let mut registry = Registry::new(None);
        let session = metadata(&id(1), "2026-11-13T10:00:00.000Z", "cmd", &[]);
        let key = session.session_id.clone();
        registry.discovered(session, Found::Live, at(1));
        let steps = [
            (ClientEvent::Opened { status: false }, "Unavailable"),
            (ClientEvent::Opened { status: true }, "Starting"),
            (ClientEvent::Ended, "Disconnected"),
            (ClientEvent::Unavailable("refused".into()), "Unavailable"),
        ];
        for (second, (event, status)) in (2..).zip(steps) {
            registry.client(&key, event, at(second));
            assert_eq!(registry.rows()[0].status, status);
            assert_eq!(registry.rows()[0].since, at(second));
        }
        registry.client(&key, titled(Some("t")), at(9));
        assert_eq!(registry.rows()[0].since, at(5));
    }

    /// Enter (spec #73, Session registry; #108, #110, pure supplement): a
    /// connected row not known to be final, with or without `status-v1`, is
    /// connected (selected and acquired); a Starting row and an Unavailable
    /// row without a connected client, uncertain or failed, are unconnected
    /// (selected without acquiring); Disconnected and final rows are retained
    /// (#116); an unknown row has none.
    /// Control and display change nothing.
    #[test]
    fn enter_kinds_follow_each_row_condition() {
        let mut registry = Registry::new(Some(OWN.parse().unwrap()));
        let sessions = [1, 2, 3, 4, 5, 6].map(|suffix| {
            let created = format!("2026-11-13T10:00:0{suffix}.000Z");
            metadata(&id(suffix), &created, "cmd", &[])
        });
        for session in &sessions {
            registry.discovered(session.clone(), Found::Live, at(1));
        }
        let uncertain = metadata(&id(8), "2026-11-13T10:00:08.000Z", "cmd", &[]);
        registry.discovered(uncertain, Found::Uncertain, at(1));
        let key = |index: usize| sessions[index].session_id.clone();
        registry.client(&key(0), ClientEvent::Opened { status: false }, at(2));
        registry.client(&key(1), ClientEvent::Opened { status: true }, at(2));
        registry.client(&key(2), ClientEvent::Opened { status: true }, at(2));
        registry.client(&key(2), ended(Outcome::Exit(0), None), at(3));
        registry.client(&key(3), ClientEvent::Unavailable("no".into()), at(2));
        registry.client(&key(4), ClientEvent::Opened { status: false }, at(2));
        registry.client(&key(4), ClientEvent::Ended, at(3));
        for event in [
            ClientEvent::Presentation(Presentation::Revoked),
            ClientEvent::Answer(2, Ok(())),
        ] {
            registry.client(&key(0), event, at(4));
        }
        let kinds: Vec<_> = (0..6).map(|index| registry.kind(&key(index))).collect();
        assert_eq!(
            kinds,
            [
                Some(Kind::Connected),
                Some(Kind::Connected),
                Some(Kind::Retained),
                Some(Kind::Unconnected),
                Some(Kind::Retained),
                Some(Kind::Unconnected),
            ]
        );
        assert_eq!(
            registry.kind(&id(8).parse().unwrap()),
            Some(Kind::Unconnected)
        );
        assert_eq!(registry.kind(&id(7).parse().unwrap()), None);
        let first = registry.rows().into_iter().find(|row| row.id == key(0));
        assert_eq!(first.map(|row| row.since), Some(at(2)));
    }

    /// DSC-01 (initial-selection case, pure supplement) and the confirmed
    /// exclusion: initial selection may take only a connected row whose
    /// status is known and not final, the first in row order; a final row,
    /// a disconnected row with a stale status and every not-connected row
    /// are never eligible. A row awaits its client while Starting, also
    /// once opened with `status-v1` until its status arrives.
    #[test]
    fn only_a_connected_row_with_a_known_nonfinal_status_is_eligible() {
        let mut registry = Registry::new(None);
        let starting = metadata(&id(1), "2026-11-13T10:00:00.000Z", "starting", &[]);
        let unsure = metadata(&id(2), "2026-11-13T10:00:00.000Z", "unsure", &[]);
        registry.discovered(starting.clone(), Found::Live, at(1));
        registry.discovered(unsure.clone(), Found::Uncertain, at(1));
        let awaiting = opened(&mut registry, (3, "awaiting"), true, 1);
        let without = opened(&mut registry, (4, "without"), false, 1);
        let lost = opened(&mut registry, (5, "lost"), true, 1);
        registry.client(&lost, status(State::NeedsAttention, 90), at(2));
        registry.client(&lost, ClientEvent::Ended, at(3));
        let failed = opened(&mut registry, (6, "failed"), true, 1);
        registry.client(&failed, ended(Outcome::Exit(1), Some(95)), at(2));
        assert_eq!(registry.eligible(), None);
        let starts: Vec<_> = [&starting.session_id, &unsure.session_id, &awaiting]
            .into_iter()
            .chain([&without, &lost, &failed])
            .map(|key| registry.starting(key))
            .collect();
        assert_eq!(starts, [true, false, true, false, false, false]);
        let idle = opened(&mut registry, (7, "idle"), true, 1);
        registry.client(&idle, status(State::Idle, 99), at(4));
        assert_eq!(registry.eligible(), Some(idle.clone()));
        registry.client(&awaiting, status(State::Running, 10), at(5));
        assert!(!registry.starting(&awaiting));
        assert_eq!(registry.eligible(), Some(awaiting));
        assert!(!registry.starting(&id(9).parse().unwrap()));
    }

    /// TRM-06 label cases: the latest nonempty title, otherwise command and
    /// arguments, shown literally with controls escaped and never the
    /// session ID; an empty or unset title keeps the latest nonempty one.
    #[test]
    fn labels_are_the_latest_nonempty_title_or_command_and_arguments_literally() {
        let mut registry = Registry::new(None);
        let session = metadata(
            &id(1),
            "2026-11-13T10:00:00.000Z",
            "/bin/bash",
            &["-c", "echo\thi\x1b[2J"],
        );
        let key = session.session_id.clone();
        registry.discovered(session, Found::Live, at(1));
        assert_eq!(registry.rows()[0].label, "/bin/bash -c echo\\thi\\u{1b}[2J");
        for (title, expected) in [
            (Some("Build 日本"), "Build 日本"),
            (Some(""), "Build 日本"),
            (None, "Build 日本"),
            (Some("a\x07b"), "a\\u{7}b"),
        ] {
            registry.client(&key, titled(title), at(2));
            assert_eq!(registry.rows()[0].label, expected);
        }
        assert!(!registry.rows()[0].label.contains(key.as_str()));
    }

    /// TRM-06 / SEL-01 (cell-safe truncation; owner decision on #112,
    /// 2026-11-16: nothing marquees): text that fits its cells is kept
    /// whole, at the limit too; one cell past it, it ends with `…` within
    /// them. Truncation never splits a wide character, so a wide character
    /// that would cross the limit leaves a blank cell's room, and combining
    /// marks stay with their base. No cell holds nothing at all.
    #[test]
    fn overflowing_text_is_truncated_display_cell_safe_with_an_ellipsis() {
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcd", 3), "ab…");
        assert_eq!(truncate("日本語", 6), "日本語");
        assert_eq!(truncate("日本語", 5), "日本…");
        assert_eq!(truncate("日本語", 4), "日…");
        assert_eq!(truncate("e\u{301}xy", 2), "e\u{301}…");
        assert_eq!(truncate("e\u{301}x", 2), "e\u{301}x");
        assert_eq!(truncate("ab", 1), "…");
        assert_eq!(truncate("ab", 0), "");
        assert_eq!(truncate("", 0), "");
    }
}
