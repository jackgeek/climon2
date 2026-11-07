pub mod config;
pub mod input;
pub mod ui;
use std::collections::VecDeque;

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
        if !self.connected && !self.final_outcome() {
            Status::Disconnected
        } else {
            self.status
        }
    }

    pub fn final_outcome(&self) -> bool {
        matches!(self.status, Status::Failed | Status::Completed)
    }

    pub fn status_text(&self) -> String {
        match self.effective_status() {
            Status::NeedsAttention => "Needs attention".into(),
            Status::Acknowledged => "Acknowledged".into(),
            Status::Running => "Running".into(),
            Status::Idle => "Idle".into(),
            Status::Completed => "Succeeded".into(),
            Status::Disconnected => "Disconnected".into(),
            Status::Failed => match self.outcome {
                Some(Outcome::Exit(code)) => format!("Failed - exit {code}"),
                Some(Outcome::Signal(signal)) => format!("Failed - signal {signal}"),
                None => "Failed - result unavailable".into(),
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ToastFrame {
    pub drawn: f64,
    pub erased: f64,
    pub alpha: f64,
}

#[derive(Debug)]
pub struct Toast {
    pub title: String,
    started: u64,
    dismissed: Option<(u64, ToastFrame)>,
}

impl Toast {
    fn new(title: String, started: u64) -> Self {
        Self {
            title,
            started,
            dismissed: None,
        }
    }

    pub fn frame(&self, now: u64) -> Option<ToastFrame> {
        if let Some((at, snapshot)) = self.dismissed {
            let elapsed = now.saturating_sub(at) as f64;
            let fade = if snapshot.alpha > 0.0 { 300.0 } else { 0.0 };
            if elapsed < fade {
                return Some(ToastFrame {
                    alpha: snapshot.alpha * (1.0 - elapsed / fade),
                    ..snapshot
                });
            }
            if elapsed < fade + 400.0 {
                return Some(ToastFrame {
                    drawn: snapshot.drawn,
                    alpha: 0.0,
                    erased: snapshot.erased + (1.0 - snapshot.erased) * (elapsed - fade) / 400.0,
                });
            }
            return None;
        }
        let elapsed = now.saturating_sub(self.started) as f64;
        match elapsed {
            age if age < 400.0 => Some(ToastFrame {
                drawn: age / 400.0,
                erased: 0.0,
                alpha: 0.0,
            }),
            age if age < 2400.0 => Some(ToastFrame {
                drawn: 1.0,
                erased: 0.0,
                alpha: 1.0,
            }),
            age if age < 2700.0 => Some(ToastFrame {
                drawn: 1.0,
                erased: 0.0,
                alpha: 1.0 - (age - 2400.0) / 300.0,
            }),
            age if age < 3100.0 => Some(ToastFrame {
                drawn: 1.0,
                erased: (age - 2700.0) / 400.0,
                alpha: 0.0,
            }),
            _ => None,
        }
    }

    fn dismiss(&mut self, now: u64) {
        if self.dismissed.is_none() {
            self.dismissed = self.frame(now).map(|snapshot| (now, snapshot));
        }
    }
}

#[derive(Debug)]
pub struct State {
    pub sessions: Vec<Session>,
    pub selected: Option<u128>,
    pub highlight: Option<u128>,
    pub menu: bool,
    pub inspecting: bool,
    pub query: Vec<char>,
    pub cursor: usize,
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
    pub now_ms: u64,
    pub toast: Option<Toast>,
    pub toast_queue: VecDeque<String>,
    pub bell: bool,
    bell_pending: bool,
}

impl State {
    pub fn program_status(&self) -> &'static str {
        if self
            .sessions
            .iter()
            .any(|session| session.connected && session.status == Status::NeedsAttention)
        {
            "\x1b]7501;state=blocked:kind=question:app=climon2-monitor\x1b\\"
        } else {
            "\x1b]7501;state=working:app=climon2-monitor\x1b\\"
        }
    }
    pub fn new(sessions: Vec<Session>) -> Self {
        let mut state = Self {
            selected: sessions.first().map(|session| session.id),
            sessions,
            highlight: None,
            menu: false,
            inspecting: false,
            query: Vec::new(),
            cursor: 0,
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
            now_ms: 0,
            toast: None,
            toast_queue: VecDeque::new(),
            bell: true,
            bell_pending: false,
        };
        state.initial_toasts();
        state
    }

    pub fn session(&self, id: u128) -> Option<&Session> {
        self.sessions.iter().find(|session| session.id == id)
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
        self.pending_prefix = false;
        self.query.clear();
        self.cursor = 0;
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
        if self.menu {
            self.menu = false;
            self.message = "Cancelled - selected session and control unchanged".into();
        }
    }

    pub fn insert_search(&mut self, text: &str) {
        if !self.menu {
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

    pub fn search_changed(&mut self) {
        let ids = self.visible_ids();
        self.highlight = self
            .highlight
            .filter(|id| ids.contains(id))
            .or_else(|| ids.first().copied());
        self.manual_scroll = false;
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

    fn select(&mut self, id: u128) {
        let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) else {
            self.message = "Selected session is unavailable".into();
            return;
        };
        self.selected = Some(id);
        self.menu = false;
        self.inspecting = !session.connected || session.final_outcome();
        self.pending_prefix = false;
        self.history_offset = 0;
        if self.inspecting {
            self.message = "Read-only retained history - no control acquired".into();
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

    fn queue_toast(&mut self, title: String) {
        if self.toast.is_none() {
            self.toast = Some(Toast::new(title, self.now_ms));
            self.bell_pending = true;
        } else {
            self.toast_queue.push_back(title);
        }
    }

    fn initial_toasts(&mut self) {
        let titles: Vec<_> = self
            .sessions
            .iter()
            .filter(|s| s.connected && s.status == Status::NeedsAttention)
            .map(|s| s.label().to_owned())
            .collect();
        for title in titles {
            self.queue_toast(title);
        }
    }

    pub fn tick_toast(&mut self, now: u64) {
        self.now_ms = now;
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| toast.frame(now).is_none())
        {
            self.toast = None;
        }
        if self.toast.is_none()
            && let Some(title) = self.toast_queue.pop_front()
        {
            self.toast = Some(Toast::new(title, now));
            self.bell_pending = true;
        }
    }

    pub fn dismiss_toast(&mut self) {
        if let Some(toast) = &mut self.toast {
            toast.dismiss(self.now_ms);
        }
    }

    pub fn take_toast_bell(&mut self, visible: bool) -> bool {
        if visible
            && self.bell_pending
            && self.toast.as_ref().is_some_and(|toast| {
                toast
                    .frame(self.now_ms)
                    .is_some_and(|frame| frame.drawn > 0.0)
            })
        {
            self.bell_pending = false;
            self.bell
        } else {
            false
        }
    }

    pub fn scenario(&mut self, number: u8) {
        self.tick += 1;
        let id = self.highlight.or(self.selected);
        match number {
            4 => {
                let mut title = None;
                if let Some(session) =
                    id.and_then(|id| self.sessions.iter_mut().find(|s| s.id == id))
                    && session.connected
                    && !session.final_outcome()
                    && session.status != Status::NeedsAttention
                {
                    session.status = Status::NeedsAttention;
                    session.changed = self.tick;
                    title = Some(session.label().to_owned());
                }
                if let Some(title) = title {
                    self.queue_toast(title);
                    self.message = "Scenario: new attention episode queued a title toast".into();
                } else {
                    self.message = "Scenario: no new attention episode; no duplicate toast".into();
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
                self.toast = None;
                self.toast_queue.clear();
                self.initial_toasts();
                self.message = "Scenario: fixtures reset; choose a row explicitly".into();
            }
            _ => {}
        }
        self.reconcile();
    }
}

pub fn samples() -> Vec<Session> {
    let rows = [
        ("Shell", Status::Idle, None),
        ("Build", Status::Running, None),
        ("Review needed", Status::NeedsAttention, None),
        ("Build", Status::Failed, Some(Outcome::Exit(2))),
        (
            "Interrupted tests",
            Status::Failed,
            Some(Outcome::Signal(9)),
        ),
        ("Finished", Status::Completed, Some(Outcome::Exit(0))),
        ("Lost connection", Status::Running, None),
        ("Passive - never controlled", Status::Idle, None),
        (
            "界 wide title / cafe\u{301} / a deliberately very long literal title",
            Status::Acknowledged,
            None,
        ),
    ];
    rows.into_iter()
        .enumerate()
        .map(|(index, (title, status, outcome))| Session {
            id: index as u128 + 1,
            title: title.into(),
            command: "cargo test --workspace".into(),
            cwd: "/work/climon2".into(),
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
