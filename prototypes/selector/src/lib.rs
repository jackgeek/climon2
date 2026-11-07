pub mod input;
pub mod ui;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    NeedsAttention,
    Acknowledged,
    Running,
    Idle,
    Failed,
    Completed,
    Disconnected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Exit(i32),
    Signal(i32),
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: u128,
    pub title: String,
    pub command: String,
    pub cwd: String,
    pub status: Status,
    pub changed: u64,
    pub connected: bool,
    pub controlled: bool,
    pub cache: Vec<String>,
    pub outcome: Option<Outcome>,
}

impl Session {
    pub fn label(&self) -> &str {
        if self.title.is_empty() {
            &self.command
        } else {
            &self.title
        }
    }

    pub fn effective_status(&self) -> Status {
        if !self.connected && !matches!(self.status, Status::Failed | Status::Completed) {
            Status::Disconnected
        } else {
            self.status
        }
    }

    pub fn status_text(&self) -> String {
        match self.effective_status() {
            Status::NeedsAttention => "Needs attention".into(),
            Status::Acknowledged => "Acknowledged".into(),
            Status::Running => "Running".into(),
            Status::Idle => "Idle".into(),
            Status::Completed => "Completed".into(),
            Status::Disconnected => "Disconnected".into(),
            Status::Failed => match self.outcome {
                Some(Outcome::Exit(code)) => format!("Failed - exit {code}"),
                Some(Outcome::Signal(signal)) => format!("Failed - signal {signal}"),
                None => "Failed - result unavailable".into(),
            },
        }
    }

    pub fn final_outcome(&self) -> bool {
        matches!(self.status, Status::Failed | Status::Completed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Search,
    Select,
    Inspect,
    Details,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    A,
    B,
    C,
}

#[derive(Debug)]
pub struct State {
    pub sessions: Vec<Session>,
    pub selected: Option<u128>,
    pub highlight: Option<u128>,
    pub menu: bool,
    pub details: bool,
    pub inspecting: bool,
    pub query: Vec<char>,
    pub cursor: usize,
    pub focus: Focus,
    pub variant: Variant,
    pub hints: bool,
    pub message: String,
    pub input: Vec<String>,
    pub tick: u64,
    pub pending_prefix: bool,
    pub exiting: bool,
    pub history_offset: usize,
    pub list_offset: usize,
    pub manual_scroll: bool,
    pub prefix_label: String,
    pub details_offset: u16,
}

impl State {
    pub fn new(sessions: Vec<Session>) -> Self {
        Self {
            selected: sessions.first().map(|session| session.id),
            sessions,
            highlight: None,
            menu: false,
            details: false,
            inspecting: false,
            query: Vec::new(),
            cursor: 0,
            focus: Focus::Search,
            variant: Variant::A,
            hints: true,
            message: String::new(),
            input: Vec::new(),
            tick: 0,
            pending_prefix: false,
            exiting: false,
            history_offset: 0,
            list_offset: 0,
            manual_scroll: false,
            prefix_label: "Ctrl+]".into(),
            details_offset: 0,
        }
    }

    pub fn visible_ids(&self) -> Vec<u128> {
        let query = self.query.iter().collect::<String>().to_lowercase();
        let mut sessions: Vec<_> = self
            .sessions
            .iter()
            .filter(|session| {
                session.label().to_lowercase().contains(&query)
                    || session.command.to_lowercase().contains(&query)
            })
            .collect();
        sessions.sort_by_key(|session| {
            (
                session.effective_status(),
                std::cmp::Reverse(session.changed),
                session.id,
            )
        });
        sessions.into_iter().map(|session| session.id).collect()
    }

    pub fn open(&mut self) {
        if self.menu {
            return;
        }
        self.menu = true;
        self.details = false;
        self.details_offset = 0;
        self.pending_prefix = false;
        self.query.clear();
        self.cursor = 0;
        self.focus = Focus::Search;
        self.list_offset = 0;
        self.manual_scroll = false;
        let ids = self.visible_ids();
        self.highlight = self
            .selected
            .filter(|id| ids.contains(id))
            .or_else(|| ids.first().copied());
        self.message = "Highlighting does not select or acquire control".into();
    }

    pub fn cancel(&mut self) {
        self.pending_prefix = false;
        if self.details {
            self.details = false;
            self.focus = Focus::Search;
        } else if self.menu {
            self.menu = false;
            self.message = "Cancelled - selected session and control unchanged".into();
        } else if self.inspecting {
            self.inspecting = false;
            self.history_offset = 0;
            self.focus = Focus::Search;
            self.message = "Left read-only inspection - no control acquired".into();
        }
    }

    pub fn insert_search(&mut self, text: &str) {
        if !self.menu || self.details || self.focus != Focus::Search {
            return;
        }
        for ch in text.chars() {
            let ch = match ch {
                '\n' | '\r' | '\t' => ' ',
                ch if ch.is_control() => continue,
                ch => ch,
            };
            self.query.insert(self.cursor, ch);
            self.cursor += 1;
        }
        self.search_changed();
    }

    pub fn move_highlight(&mut self, delta: isize) {
        let ids = self.visible_ids();
        if ids.is_empty() {
            self.highlight = None;
            return;
        }
        let index = self
            .highlight
            .and_then(|id| ids.iter().position(|candidate| *candidate == id));
        let index = match index {
            Some(index) => index.saturating_add_signed(delta).min(ids.len() - 1),
            None if delta < 0 => ids.len() - 1,
            None => 0,
        };
        self.highlight = Some(ids[index]);
        self.manual_scroll = false;
    }

    pub fn reconcile(&mut self) {
        if self
            .highlight
            .is_some_and(|id| !self.visible_ids().contains(&id))
        {
            self.highlight = None;
            self.message =
                "Highlighted session changed or disappeared; choose a row before selecting".into();
        }
        self.manual_scroll = false;
    }

    pub fn confirm(&mut self) {
        let Some(id) = self.highlight.filter(|id| self.visible_ids().contains(id)) else {
            self.message = "Choose a session before selecting".into();
            return;
        };
        self.select(id);
    }

    pub fn inspect(&mut self) {
        let Some(session) = self.highlight.and_then(|id| self.session(id)) else {
            self.message = "Choose a session before inspecting".into();
            return;
        };
        if session.cache.is_empty() {
            self.message = "No cached terminal content is available".into();
            return;
        }
        self.selected = Some(session.id);
        self.inspecting = true;
        self.history_offset = 0;
        self.menu = false;
        self.details = false;
        self.pending_prefix = false;
        self.message = "READ-ONLY cached inspection - no control acquired".into();
    }

    pub fn application_input(&mut self, text: &str) {
        if self.menu || self.inspecting || self.exiting {
            return;
        }
        if self
            .selected
            .and_then(|id| self.session(id))
            .is_some_and(|session| {
                session.connected && session.controlled && !session.final_outcome()
            })
        {
            self.history_offset = 0;
            self.input.push(text.into());
            self.message = format!("Synthetic application received: {text:?}");
        }
    }

    pub fn jump_attention(&mut self) {
        let target = self
            .sessions
            .iter()
            .filter(|session| session.connected && session.status == Status::NeedsAttention)
            .min_by_key(|session| (std::cmp::Reverse(session.changed), session.id))
            .map(|session| session.id);
        if let Some(id) = target {
            self.select(id);
        } else {
            self.message = "No sessions need attention".into();
        }
    }

    pub fn session(&self, id: u128) -> Option<&Session> {
        self.sessions.iter().find(|session| session.id == id)
    }

    fn select(&mut self, id: u128) {
        let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) else {
            self.message = "Selected session is unavailable".into();
            return;
        };
        self.selected = Some(id);
        self.menu = false;
        self.details = false;
        self.inspecting = false;
        self.pending_prefix = false;
        self.history_offset = 0;
        if session.final_outcome() {
            self.message = "Retained final terminal - read-only".into();
        } else if !session.connected {
            self.message = "Selected session is disconnected - no automatic takeover".into();
        } else {
            session.controlled = true;
            if session.status == Status::NeedsAttention {
                session.status = Status::Acknowledged;
                self.tick += 1;
                session.changed = self.tick;
            }
            self.message =
                "Selected explicitly; synthetic control/restoration/paint completed".into();
        }
    }

    pub fn search_changed(&mut self) {
        let ids = self.visible_ids();
        self.highlight = self
            .highlight
            .filter(|id| ids.contains(id))
            .or_else(|| ids.first().copied());
        self.manual_scroll = false;
    }

    pub fn page_start(&self, height: usize) -> usize {
        let ids = self.visible_ids();
        let start = self.list_offset.min(ids.len().saturating_sub(height));
        if self.manual_scroll || height == 0 {
            return start;
        }
        match self
            .highlight
            .and_then(|id| ids.iter().position(|candidate| *candidate == id))
        {
            Some(index) if index < start => index,
            Some(index) if index >= start + height => index + 1 - height,
            _ => start,
        }
    }

    pub fn advance(&mut self) {
        self.tick += 1;
        for session in &mut self.sessions {
            if session.connected && session.controlled && !session.final_outcome() {
                session.cache.push(format!(
                    "Synthetic output {:04}: {}",
                    self.tick,
                    session.label()
                ));
                if session.cache.len() > 200 {
                    session.cache.remove(0);
                }
            }
        }
    }

    pub fn scenario(&mut self, number: u8) {
        self.tick += 1;
        let id = self.highlight.or(self.selected);
        match number {
            4 => {
                if let Some(session) =
                    id.and_then(|id| self.sessions.iter_mut().find(|s| s.id == id))
                {
                    session.status = Status::NeedsAttention;
                    session.changed = self.tick;
                    self.message =
                        "Scenario: highlighted session requests attention and reorders".into();
                }
            }
            5 => {
                if let Some(session) =
                    id.and_then(|id| self.sessions.iter_mut().find(|s| s.id == id))
                {
                    session.title = format!("Renamed title at {}", self.tick);
                    self.message =
                        "Scenario: daemon title changed (not a user-assigned name)".into();
                }
            }
            6 => {
                if let Some(session) =
                    id.and_then(|id| self.sessions.iter_mut().find(|s| s.id == id))
                {
                    session.connected = !session.connected;
                    session.controlled = false;
                    self.message = "Scenario: connection toggled; no automatic acquisition".into();
                }
            }
            7 => {
                self.sessions.retain(|session| Some(session.id) != id);
                self.message = "Scenario: row removed; no automatic selection fallback".into();
            }
            8 => {
                self.sessions = samples();
                self.selected = Some(1);
                self.highlight = None;
                self.inspecting = false;
                self.message = "Scenario: fixtures reset; choose a row explicitly".into();
            }
            _ => {}
        }
        self.reconcile();
    }
}

pub fn samples() -> Vec<Session> {
    let rows = [
        ("Shell", "/work/climon2", Status::Idle, None),
        ("Build", "/work/climon2", Status::Running, None),
        (
            "Review needed",
            "/work/service",
            Status::NeedsAttention,
            None,
        ),
        (
            "Build",
            "/work/other-project",
            Status::Failed,
            Some(Outcome::Exit(2)),
        ),
        (
            "Interrupted tests",
            "/work/service",
            Status::Failed,
            Some(Outcome::Signal(9)),
        ),
        (
            "Finished",
            "/work/docs",
            Status::Completed,
            Some(Outcome::Exit(0)),
        ),
        ("Lost connection", "/work/archive", Status::Running, None),
        (
            "Passive - never controlled",
            "/work/passive",
            Status::Idle,
            None,
        ),
        (
            "界 wide title / cafe\u{301} / a deliberately very long literal title",
            "/work/unicode",
            Status::Acknowledged,
            None,
        ),
    ];
    rows.into_iter()
        .enumerate()
        .map(|(index, (title, cwd, status, outcome))| Session {
            id: index as u128 + 1,
            title: title.into(),
            command: if index == 0 {
                "bash --noprofile --norc".into()
            } else {
                "cargo test --workspace".into()
            },
            cwd: cwd.into(),
            status,
            changed: 100 + index as u64,
            connected: index != 6 && !matches!(status, Status::Failed | Status::Completed),
            controlled: index == 0,
            cache: if index == 7 {
                Vec::new()
            } else {
                (0..30)
                    .map(|row| format!("{title}: retained synthetic terminal row {row:02}"))
                    .collect()
            },
            outcome,
        })
        .collect()
}
