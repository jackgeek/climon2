//! The session registry (spec #73, "Session registry"): pure, and the single
//! owner of row state. Discovery outcomes and client events are its inputs,
//! with the time the monitor observed them; it decides which sessions need a
//! client and what each row shows. Rows are keyed by full session ID, which
//! is never shown.
use climon2_core::{SessionId, Timestamp, metadata::SessionMetadata};
use std::{cmp::Reverse, collections::BTreeMap, time::SystemTime};
use unicode_width::UnicodeWidthChar;

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
    /// The daemon title: `None` when unset or unsupported.
    Title(Option<String>),
    /// The client cannot serve this session, with why.
    Unavailable(String),
    /// The connected client's connection ended.
    Ended,
}

/// Whether the row has a client, and how far it got.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Connection {
    /// This monitor's own session: never a client.
    Own,
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
    /// The status column. A connected row shows no effective status yet
    /// (#107), except Unavailable when its daemon lacks `status-v1`.
    fn label(&self) -> &'static str {
        match self {
            Self::Own => "This monitor",
            Self::Starting => "Starting",
            Self::Disconnected => "Disconnected",
            other => other.reachable(),
        }
    }

    /// Connected with `status-v1`: its effective status arrives with #107;
    /// otherwise not reachable for a status.
    fn reachable(&self) -> &'static str {
        if *self == (Self::Connected { status: true }) {
            ""
        } else {
            "Unavailable"
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
}

impl Row {
    fn new(metadata: SessionMetadata, connection: Connection, now: SystemTime) -> Self {
        Self {
            metadata,
            connection,
            since: now,
            connected: false,
            title: None,
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

    /// Keeps a nonempty title; an empty or unset one changes nothing.
    fn retitle(&mut self, title: Option<String>) {
        if let Some(title) = title.filter(|title| !title.is_empty()) {
            self.title = Some(title);
        }
    }
}

/// One row as the selector shows it. The session ID identifies it and is
/// never displayed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct View {
    pub(crate) id: SessionId,
    pub(crate) status: &'static str,
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
    /// needs a passive client: a live session that has none and is not this
    /// monitor's. Discovery never changes a client's connection. A session
    /// proven gone drops its row unless it was ever connected, and adds none.
    pub(crate) fn discovered(
        &mut self,
        metadata: SessionMetadata,
        found: Found,
        now: SystemTime,
    ) -> Option<SessionMetadata> {
        let id = metadata.session_id.clone();
        if matches!(found, Found::Stale) {
            self.rows.retain(|key, row| *key != id || row.connected);
            return None;
        }
        let own = self.own.as_ref() == Some(&id);
        let connection = if own {
            Connection::Own
        } else {
            Connection::None
        };
        let row = self
            .rows
            .entry(id)
            .or_insert_with(|| Row::new(metadata.clone(), connection, now));
        let wanted = matches!(found, Found::Live) && row.connection == Connection::None;
        wanted.then(|| {
            row.connect(Connection::Starting, now);
            metadata
        })
    }

    /// Records what `id`'s client reported.
    pub(crate) fn client(&mut self, id: &SessionId, event: ClientEvent, now: SystemTime) {
        let Some(row) = self.rows.get_mut(id) else {
            return;
        };
        match event {
            ClientEvent::Opened { status } => row.connect(Connection::Connected { status }, now),
            ClientEvent::Title(title) => row.retitle(title),
            ClientEvent::Unavailable(why) => row.connect(Connection::Unavailable(why), now),
            ClientEvent::Ended => row.connect(Connection::Disconnected, now),
        }
    }

    /// Records whether the latest pass reached every entry.
    pub(crate) fn pass(&mut self, incomplete: Option<String>) {
        self.incomplete = incomplete;
    }

    pub(crate) fn incomplete(&self) -> Option<&str> {
        self.incomplete.as_deref()
    }

    /// Every row, newest created first, then ascending full session ID. The
    /// status order arrives with #107.
    pub(crate) fn rows(&self) -> Vec<View> {
        let mut rows: Vec<_> = self.rows.values().collect();
        rows.sort_by_key(|row| order(&row.metadata.created_at, &row.metadata.session_id));
        rows.into_iter()
            .map(|row| View {
                id: row.metadata.session_id.clone(),
                status: row.connection.label(),
                label: row.label(),
                since: row.since,
            })
            .collect()
    }
}

/// Newest created first, then ascending full session ID: the order of rows
/// and of client starts.
pub(crate) fn order<'a>(
    created: &'a Timestamp,
    id: &'a SessionId,
) -> (Reverse<&'a Timestamp>, &'a SessionId) {
    (Reverse(created), id)
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

/// The leading part of `text` that fits `cells` display cells, never
/// splitting a wide character; zero-width characters stay with the
/// character before them.
pub(crate) fn clip(text: &str, cells: usize) -> &str {
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

    fn shown(registry: &Registry) -> Vec<(&'static str, String)> {
        registry
            .rows()
            .into_iter()
            .map(|row| (row.status, row.label))
            .collect()
    }

    /// DSC-01 (start order, pure supplement) and DSC-03: live sessions need
    /// one client each, asked once however often they are found again; rows
    /// list newest created first with ascending full session ID ties.
    #[test]
    fn live_sessions_need_one_client_each_and_list_newest_first() {
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
        let ids: Vec<_> = registry.rows().into_iter().map(|row| row.id).collect();
        let expected: Vec<SessionId> = [9, 2, 3, 1]
            .map(|suffix| id(suffix).parse().unwrap())
            .to_vec();
        assert_eq!(ids, expected);
        assert!(registry.rows().iter().all(|row| row.status == "Starting"));
    }

    /// Own-session scenario (confirmed 2026-11-08): this monitor's own session
    /// shows as This monitor and never needs a client.
    #[test]
    fn the_own_session_is_this_monitor_and_never_needs_a_client() {
        let mut registry = Registry::new(Some(OWN.parse().unwrap()));
        let own = metadata(OWN, "2026-11-13T10:00:00.000Z", "bash", &[]);
        assert_eq!(registry.discovered(own.clone(), Found::Live, at(1)), None);
        assert_eq!(registry.discovered(own, Found::Live, at(2)), None);
        assert_eq!(shown(&registry), [("This monitor", "bash".to_owned())]);
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
        assert_eq!(
            shown(&registry),
            [
                ("", "live".to_owned()),
                ("Unavailable", "unsure".to_owned())
            ]
        );
        registry.discovered(live.clone(), Found::Uncertain, at(3));
        registry.discovered(live.clone(), Found::Stale, at(3));
        assert_eq!(registry.rows()[0].status, "");
        assert_eq!(registry.rows()[0].since, at(2));
        assert_eq!(
            registry.discovered(unsure.clone(), Found::Live, at(5)),
            Some(unsure)
        );
        assert_eq!(registry.rows()[1].status, "Starting");
        assert_eq!(registry.rows()[1].since, at(5));
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
        assert_eq!(shown(&registry), [("Unavailable", "s4".to_owned())]);
        registry.client(&connected.session_id, ClientEvent::Ended, at(5));
        registry.client(&starting.session_id, ClientEvent::Ended, at(5));
        assert_eq!(shown(&registry), [("Disconnected", "s4".to_owned())]);
        assert_eq!(registry.rows()[0].since, at(5));
    }

    /// COM-01 (Monitor, pure supplement) and the not-connected labels: a
    /// connected row shows no fabricated status, or Unavailable without
    /// `status-v1`; a failed client is Unavailable and an ended one
    /// Disconnected, each recording when it entered that status.
    #[test]
    fn connection_statuses_are_labelled_as_specified() {
        let mut registry = Registry::new(None);
        let session = metadata(&id(1), "2026-11-13T10:00:00.000Z", "cmd", &[]);
        let key = session.session_id.clone();
        registry.discovered(session, Found::Live, at(1));
        let steps = [
            (ClientEvent::Opened { status: false }, "Unavailable"),
            (ClientEvent::Opened { status: true }, ""),
            (ClientEvent::Ended, "Disconnected"),
            (ClientEvent::Unavailable("refused".into()), "Unavailable"),
        ];
        for (second, (event, status)) in (2..).zip(steps) {
            registry.client(&key, event, at(second));
            assert_eq!(registry.rows()[0].status, status);
            assert_eq!(registry.rows()[0].since, at(second));
        }
        registry.client(&key, ClientEvent::Title(Some("t".into())), at(9));
        assert_eq!(registry.rows()[0].since, at(5));
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
            registry.client(&key, ClientEvent::Title(title.map(str::to_owned)), at(2));
            assert_eq!(registry.rows()[0].label, expected);
        }
        assert!(!registry.rows()[0].label.contains(key.as_str()));
    }

    /// TRM-06 / SEL-01 (cell-safe clipping): clipping never splits a wide
    /// character and keeps combining marks with their base.
    #[test]
    fn clipping_is_display_cell_safe() {
        assert_eq!(clip("日本語", 5), "日本");
        assert_eq!(clip("日本語", 4), "日本");
        assert_eq!(clip("e\u{301}x", 1), "e\u{301}");
        assert_eq!(clip("abc", 0), "");
        assert_eq!(clip("abc", 3), "abc");
    }
}
