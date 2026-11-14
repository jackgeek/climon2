//! The interactive monitor (spec #73): one terminal that tracks every live
//! session. It validates its start before touching the host terminal, then
//! owns the host with one permanent top row, discovers sessions off its UI
//! thread, keeps one passive client per live session, and lists them in the
//! selector on prefix+s, under the key introduction when it is due, until
//! prefix+x or a catchable termination signal. Nothing is selected yet.

mod discovery;
#[cfg(feature = "test-hooks")]
mod hooks;
mod host;
mod introduction;
mod keys;
mod paint;
mod preference;
mod registry;
mod screen;
mod selector;
mod startup;
mod supervisor;
mod terminal;

use climon2_core::{SessionId, protocol::TerminalSize};
use climon2_discovery::{Batch, Incomplete, Outcome, Progress};
use climon2_vt::Limits;
use discovery::{Discovery, Found};
use host::{Ending, Event, Host};
use introduction::KeyIntroduction;
use keys::{Action, Key, Keys};
use registry::{ClientEvent, Registry};
use screen::{Scene, Screen, Session};
use selector::Selector;
use startup::Startup;
use std::{
    collections::BTreeMap,
    fmt, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use supervisor::{Context, Load, Refusal, Supervisor};
use terminal::{Presentation, Request, Terminal};

/// A one-line diagnostic that the thin main prints, after the host is
/// restored, with the climon2 error code.
#[derive(Debug)]
pub struct Error {
    message: String,
    logs: Option<PathBuf>,
}

impl Error {
    fn new(message: impl fmt::Display) -> Self {
        Self {
            message: message.to_string(),
            logs: None,
        }
    }

    fn logged(mut self, path: &Path) -> Self {
        self.logs = Some(path.to_owned());
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&one_line(&self.message))?;
        if let Some(logs) = &self.logs {
            write!(out, " (logs: {})", one_line(&logs.to_string_lossy()))?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

fn one_line(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                character.escape_default().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}

/// The CLI execs the monitor with no arguments. Returns the exit status of an
/// orderly exit: 0 for prefix+x, 128 + signal for a termination signal.
pub fn run() -> Result<u32, Error> {
    let startup = Startup::read()?;
    let log = startup.log.path().to_owned();
    let filter = std::env::var("CLIMON2_LOG").ok();
    let dispatch = startup
        .log
        .dispatch(filter.as_deref())
        .map_err(|error| Error::new(error).logged(&log))?;
    let filter = filter.unwrap_or_else(|| DEFAULT_FILTER.to_owned());
    tracing::dispatcher::with_default(&dispatch, || monitor(startup, &dispatch, filter))
        .map_err(|error| Error::new(error).logged(&log))
}

/// Records how the monitor ended: the host terminal may be gone, so a failure
/// is logged as well as returned for main to print.
/// The library log filter when `CLIMON2_LOG` sets none.
const DEFAULT_FILTER: &str = "info";

fn monitor(startup: Startup, dispatch: &tracing::Dispatch, filter: String) -> io::Result<u32> {
    tracing::info!(version = %startup.version, "monitor started");
    tracing::info!(effective = %startup.config, "monitor configuration");
    if let Some(problem) = startup.preference.problem() {
        tracing::warn!(%problem, "monitor preference unusable");
    }
    let status = owned(startup, dispatch, filter)
        .inspect_err(|error| tracing::error!(%error, "monitor failed"))?;
    tracing::info!(status, "monitor stopped");
    Ok(status)
}

/// Owns the host until an ending, and releases it before any diagnostic.
fn owned(startup: Startup, dispatch: &tracing::Dispatch, filter: String) -> io::Result<u32> {
    let (mut host, size) = Host::enter()?;
    let served = Monitor::start(size, startup, dispatch, filter)
        .and_then(|monitor| monitor.serve(&mut host));
    let released = host.release();
    served.and_then(|ending| released.map(|()| ending.status()))
}

/// The monitor's own enclosing session, if it runs inside one.
fn own_session() -> Option<SessionId> {
    let value = std::env::var_os("CLIMON2_SESSION_ID")?;
    let parsed = value.to_str().and_then(|text| text.parse().ok());
    if parsed.is_none() {
        tracing::warn!("CLIMON2_SESSION_ID is not a session ID; no row is this monitor");
    }
    parsed
}

/// The monitor: what it draws, the rows it tracks, each session's terminal,
/// and the keys that act on them.
struct Monitor {
    screen: Screen,
    keys: Keys,
    introduction: KeyIntroduction,
    registry: Registry,
    selector: Selector,
    discovery: Discovery,
    supervisor: Supervisor,
    /// The selected session: none until the user selects one, and no
    /// arrival ever moves it.
    selected: Option<SessionId>,
    /// One terminal per session ID, kept for the whole run.
    terminals: BTreeMap<SessionId, Terminal>,
    /// What bounds each terminal's model.
    model: Limits,
}

impl Monitor {
    fn start(
        size: TerminalSize,
        startup: Startup,
        dispatch: &tracing::Dispatch,
        filter: String,
    ) -> io::Result<Self> {
        let config = &startup.config;
        let prefix = config.escape_prefix();
        let hints = config.settings().show_key_hints;
        let supervisor = Supervisor::start(context(&startup, filter)?, dispatch.clone())?;
        let introduction = KeyIntroduction::new(prefix, startup.preference, hints);
        let limits = &config.settings().limits;
        let discovery = Discovery::start(startup.home, limits.discovery())?;
        let model = limits.model();
        let mut monitor = Self {
            screen: Screen::new(size, prefix, hints),
            keys: Keys::new(prefix),
            introduction,
            registry: Registry::new(own_session()),
            selector: Selector::default(),
            discovery,
            supervisor,
            selected: None,
            terminals: BTreeMap::new(),
            model: Limits {
                bytes: usize::try_from(model.bytes).unwrap_or(usize::MAX),
                history_rows: model.history_rows as usize,
            },
        };
        monitor.show(true).map(|()| monitor)
    }

    /// Acts on host events until an ending, then shuts every client down.
    /// Consuming the monitor drops its screen before the host is released.
    fn serve(mut self, host: &mut Host) -> io::Result<Ending> {
        let ending = self.events(host);
        self.supervisor.stop();
        ending
    }

    fn events(&mut self, host: &mut Host) -> io::Result<Ending> {
        loop {
            if let Some(ending) = self.step(host)? {
                return Ok(ending);
            }
        }
    }

    /// Handles the next host event, then, unless it ends the monitor, what
    /// discovery and the supervisor reported meanwhile.
    fn step(&mut self, host: &mut Host) -> io::Result<Option<Ending>> {
        let event = host.next()?;
        let ending = self.handle(event)?;
        if ending.is_none() {
            self.collect();
            self.show(false)?;
        }
        Ok(ending)
    }

    fn handle(&mut self, event: Event<'_>) -> io::Result<Option<Ending>> {
        match event {
            Event::End(ending) => Ok(Some(ending)),
            Event::Resize(size) => {
                self.screen.resize(size);
                self.resize();
                self.show(true).map(|()| None)
            }
            Event::Input(bytes) => Ok(self.input(bytes)),
            Event::Idle => Ok(None),
        }
    }

    /// Paints the current scene: always when `force`, otherwise only when it
    /// changed. Each terminal first learns whether this frame shows its
    /// applied restoration, so its control and input readiness describe the
    /// frame being painted.
    fn show(&mut self, force: bool) -> io::Result<()> {
        self.mark_shown();
        let rows = self.registry.rows();
        let row = self
            .selected
            .as_ref()
            .and_then(|id| rows.iter().find(|row| &row.id == id));
        let terminal = self.selected.as_ref().and_then(|id| self.terminals.get(id));
        let session = row.map(|row| selected(row, terminal));
        let scene = Scene {
            introduction: &self.introduction,
            rows: &rows,
            selector: &self.selector,
            incomplete: self.registry.incomplete(),
            session,
        };
        self.screen.show(&scene, force)
    }

    /// Only the selected terminal can be shown, and only while nothing
    /// covers its canvas and the host has room for it.
    fn mark_shown(&mut self) {
        let covered = self.selector.is_open() || self.introduction.is_open();
        let visible = self.screen.desire().is_some() && !covered;
        for (id, terminal) in &mut self.terminals {
            terminal.painted(visible && self.selected.as_ref() == Some(id));
        }
    }

    /// `id`'s terminal, created the first time it is needed.
    fn terminal(&mut self, id: &SessionId) -> &mut Terminal {
        let model = self.model;
        self.terminals
            .entry(id.clone())
            .or_insert_with(|| Terminal::new(model))
    }

    /// Submits `request` through `id`'s client, if there is one.
    fn request(&self, id: &SessionId, request: Option<Request>) {
        if let Some(request) = request {
            self.supervisor.request(id.clone(), request);
        }
    }

    /// Resize goes only to the selected controller, and only while the host
    /// leaves room for a session.
    fn resize(&mut self) {
        let Some(desire) = self.screen.desire() else {
            return;
        };
        let Some(id) = self.selected.clone() else {
            return;
        };
        let request = self.terminal(&id).resize(desire);
        self.request(&id, request);
    }

    /// Enter on a connected row not known to be final selects it, closes
    /// the selector and acquires it; Enter on any other row is ignored until
    /// #110 and #116, which own those cases.
    fn enter(&mut self, id: SessionId) {
        if !self.registry.acquirable(&id) {
            return;
        }
        self.selector.close();
        let request = self.terminal(&id).acquire();
        self.request(&id, Some(request));
        self.selected = Some(id);
        self.resize();
    }

    /// Applies a client's control and display report to its terminal,
    /// submitting the Ready that follows an applied restoration, then
    /// keeps the selected controller's size.
    fn present(&mut self, id: &SessionId, presentation: Presentation) {
        let ready = self.terminal(id).present(presentation);
        self.request(id, ready);
        self.resize();
    }

    /// Applies what discovery and the supervisor reported since the last
    /// event, asking for a client for each session that now needs one.
    fn collect(&mut self) {
        let now = SystemTime::now();
        while let Ok(found) = self.discovery.found.try_recv() {
            match found {
                Found::Batch(batch) => self.batch(batch, now),
                Found::Pass(progress) => self.registry.pass(incomplete(&progress)),
            }
        }
        while let Ok((id, event)) = self.supervisor.reports.try_recv() {
            self.report(&id, event, now);
        }
    }

    /// Control and display go to the session's terminal, everything else to
    /// its row; the command's final result and the end of the client to
    /// both.
    fn report(&mut self, id: &SessionId, event: ClientEvent, now: SystemTime) {
        match event {
            ClientEvent::Presentation(presentation) => self.present(id, presentation),
            event => {
                if let Some(presentation) = ending(&event) {
                    self.present(id, presentation);
                }
                self.registry.client(id, event, now);
            }
        }
    }

    fn batch(&mut self, batch: Batch, now: SystemTime) {
        for invalid in &batch.invalid {
            tracing::warn!(path = ?invalid.path, reason = ?invalid.reason, "invalid session metadata");
        }
        for outcome in batch.sessions {
            self.discovered(outcome, now);
        }
    }

    fn discovered(&mut self, outcome: Outcome, now: SystemTime) {
        let (metadata, found) = match outcome {
            Outcome::Live(live) => (live.metadata, registry::Found::Live),
            Outcome::Uncertain(metadata, uncertainty) => {
                tracing::info!(session = %metadata.session_id, ?uncertainty, "session uncertain");
                (metadata, registry::Found::Uncertain)
            }
            Outcome::Stale(metadata, _) => (metadata, registry::Found::Stale),
        };
        if let Some(metadata) = self.registry.discovered(metadata, found, now) {
            self.supervisor.start_client(metadata);
        }
    }

    /// Acts on each byte in turn until one ends the monitor.
    fn input(&mut self, bytes: &[u8]) -> Option<Ending> {
        let ending = bytes.iter().find_map(|&byte| self.act(byte));
        if ending.is_none() && !self.introduction.is_open() && self.selector.is_open() {
            self.selector.read_ended(&self.registry.rows());
        }
        ending
    }

    /// Acts on the key `byte` completes: prefix+x ends the monitor, and
    /// other actions and typed keys go to [`Monitor::action`] and
    /// [`Monitor::typed`].
    fn act(&mut self, byte: u8) -> Option<Ending> {
        match self.keys.key(byte)? {
            Key::Action(Action::Exit) => Some(Ending::Exit),
            Key::Action(action) => {
                self.action(action);
                None
            }
            Key::Typed(byte) => {
                self.typed(byte);
                None
            }
        }
    }

    /// Prefix+? opens the key introduction, and prefix+s the selector
    /// unless the modal key introduction is open.
    fn action(&mut self, action: Action) {
        match action {
            Action::KeyIntroduction => self.introduction.open(),
            Action::SessionList if !self.introduction.is_open() => {
                self.selector.open(&self.registry.rows());
            }
            _ => {}
        }
    }

    /// A typed key goes to the key introduction while it is open, otherwise
    /// to the open selector, otherwise nowhere.
    fn typed(&mut self, byte: u8) {
        if self.introduction.is_open() {
            self.introduction.key(byte);
        } else if self.selector.is_open()
            && let Some(id) = self.selector.key(byte, &self.registry.rows())
        {
            self.enter(id);
        }
    }
}

/// What a row event means for the session's terminal: the view freezes at
/// the command's final result, and control ends with the client.
fn ending(event: &ClientEvent) -> Option<Presentation> {
    match event {
        ClientEvent::Final => Some(Presentation::Final),
        ClientEvent::Ended | ClientEvent::Unavailable(_) => Some(Presentation::Ended),
        _ => None,
    }
}

/// The selected row as the top row shows it: its label, its status (or
/// Unavailable until status reaches the monitor, #95 and #107), its control,
/// any error, and what its terminal shows.
fn selected<'a>(row: &'a registry::View, terminal: Option<&'a Terminal>) -> Session<'a> {
    let status = Some(row.status).filter(|status| !status.is_empty());
    Session {
        label: &row.label,
        status: status.unwrap_or("Unavailable"),
        control: terminal
            .map_or(terminal::Control::NotControlling, Terminal::control)
            .label(),
        error: terminal.and_then(Terminal::error),
        view: terminal.and_then(Terminal::view),
    }
}

/// What the supervisor's instances share, from the started configuration.
fn context(startup: &Startup, filter: String) -> io::Result<Context> {
    let config = &startup.config;
    let limits = &config.settings().limits;
    let home = startup.home.root();
    let runtime = climon2_platform::runtime::RuntimeDirectory::prepare(home)?;
    let lanes = limits.lanes();
    Ok(Context {
        load: loader(startup.executable.clone()),
        home: startup.home.clone(),
        runtime: runtime.path().to_owned(),
        escape: config.escape_prefix(),
        take_control: config.take_control_key(),
        limits: limits.library(),
        filter,
        lanes,
        shutdown: Duration::from_millis(lanes.shutdown_ms.into()),
    })
}

/// The monitor's one validated loader, shared by every release it loads.
fn loader(executables: climon2_core::layout::ExecutableHome) -> Load {
    let loader = climon2_platform::library::Loader::new(executables);
    Box::new(move |version| {
        loader
            .load(version)
            .map(climon2_platform::library::Library::functions)
            .map_err(|error| Refusal::Library(error.failure(), error.to_string()))
    })
}

/// Why a pass was incomplete, as the monitor shows it; `None` when complete.
fn incomplete(progress: &Progress) -> Option<String> {
    match progress {
        Progress::Complete => None,
        Progress::Incomplete(Incomplete::Deadline) => {
            tracing::warn!("discovery pass incomplete at its deadline");
            Some("the pass deadline passed".into())
        }
        Progress::Incomplete(Incomplete::Inaccessible(error)) => {
            tracing::warn!(%error, "discovery pass incomplete");
            Some(format!("cannot list sessions: {error}"))
        }
    }
}
