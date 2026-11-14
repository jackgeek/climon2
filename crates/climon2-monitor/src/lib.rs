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
mod preference;
mod registry;
mod screen;
mod selector;
mod startup;
mod supervisor;

use climon2_core::{SessionId, protocol::TerminalSize};
use climon2_discovery::{Incomplete, Outcome, Progress, Uncertainty};
use discovery::{Discovery, Found};
use host::{Ending, Event, Host};
use introduction::KeyIntroduction;
use keys::{Action, Key, Keys};
use registry::Registry;
use screen::{Scene, Screen};
use selector::Selector;
use startup::Startup;
use std::{
    fmt, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use supervisor::{Context, Load, Refusal, Supervisor};

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

/// The monitor: what it draws, the rows it tracks, and the keys that act on
/// them.
struct Monitor {
    screen: Screen,
    keys: Keys,
    introduction: KeyIntroduction,
    registry: Registry,
    selector: Selector,
    discovery: Discovery,
    supervisor: Option<Supervisor>,
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
        let limits = &config.settings().limits;
        let introduction = KeyIntroduction::new(prefix, startup.preference, hints);
        let runtime = climon2_platform::runtime::RuntimeDirectory::prepare(startup.home.root())?
            .path()
            .to_owned();
        let lanes = limits.lanes();
        let context = Context {
            load: loader(startup.executable),
            home: startup.home.clone(),
            runtime,
            escape: prefix,
            take_control: config.take_control_key(),
            limits: limits.library(),
            filter,
            lanes,
            shutdown: Duration::from_millis(lanes.shutdown_ms.into()),
        };
        let supervisor = Supervisor::start(context, dispatch.clone())?;
        let discovery = Discovery::start(startup.home, limits.discovery())?;
        let registry = Registry::new(own_session());
        let selector = Selector::default();
        let screen = Screen::new(size, prefix, hints)?;
        let mut monitor = Self {
            screen,
            keys: Keys::new(prefix),
            introduction,
            registry,
            selector,
            discovery,
            supervisor: Some(supervisor),
        };
        monitor.show(true)?;
        Ok(monitor)
    }

    /// Acts on host events until an ending, then shuts every client down.
    /// Consuming the monitor drops its screen before the host is released.
    fn serve(mut self, host: &mut Host) -> io::Result<Ending> {
        let ending = self.events(host);
        if let Some(mut supervisor) = self.supervisor.take() {
            supervisor.stop();
        }
        ending
    }

    fn events(&mut self, host: &mut Host) -> io::Result<Ending> {
        loop {
            let event = host.next()?;
            if let Some(ending) = self.handle(event)? {
                return Ok(ending);
            }
            self.collect();
            self.show(false)?;
        }
    }

    fn handle(&mut self, event: Event<'_>) -> io::Result<Option<Ending>> {
        match event {
            Event::End(ending) => Ok(Some(ending)),
            Event::Resize(size) => {
                self.screen.resize(size);
                self.show(true).map(|()| None)
            }
            Event::Input(bytes) => Ok(self.input(bytes)),
            Event::Idle => Ok(None),
        }
    }

    /// Paints the current scene: always when `force`, otherwise only when it
    /// changed.
    fn show(&mut self, force: bool) -> io::Result<()> {
        let rows = self.registry.rows();
        let scene = Scene {
            introduction: &self.introduction,
            rows: &rows,
            selector: &self.selector,
            incomplete: self.registry.incomplete(),
        };
        self.screen.show(&scene, force)
    }

    /// Applies what discovery and the supervisor reported since the last
    /// event, asking for a client for each session that now needs one.
    fn collect(&mut self) {
        let now = SystemTime::now();
        while let Ok(found) = self.discovery.found.try_recv() {
            match found {
                Found::Batch(batch) => {
                    for invalid in &batch.invalid {
                        tracing::warn!(path = ?invalid.path, reason = ?invalid.reason, "invalid session metadata");
                    }
                    for outcome in batch.sessions {
                        self.discovered(outcome, now);
                    }
                }
                Found::Pass(progress) => self.registry.pass(incomplete(&progress)),
            }
        }
        if let Some(supervisor) = &self.supervisor {
            while let Ok((id, event)) = supervisor.reports.try_recv() {
                self.registry.client(&id, event, now);
            }
        }
    }

    fn discovered(&mut self, outcome: Outcome, now: SystemTime) {
        let (metadata, found) = match outcome {
            Outcome::Live(live) => (live.metadata, registry::Found::Live),
            Outcome::Uncertain(metadata, uncertainty) => {
                let why = uncertain(&uncertainty);
                tracing::info!(session = %metadata.session_id, %why, "session uncertain");
                (metadata, registry::Found::Uncertain)
            }
            Outcome::Stale(metadata, _) => (metadata, registry::Found::Stale),
        };
        if let Some(metadata) = self.registry.discovered(metadata, found, now)
            && let Some(supervisor) = &self.supervisor
        {
            supervisor.start_client(metadata);
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

    /// Acts on the key `byte` completes: prefix+x ends the monitor, prefix+?
    /// opens the key introduction, and prefix+s the selector unless the
    /// modal key introduction is open; a typed key goes to the key
    /// introduction while it is open, otherwise to the selector.
    fn act(&mut self, byte: u8) -> Option<Ending> {
        match self.keys.key(byte) {
            Some(Key::Action(Action::Exit)) => return Some(Ending::Exit),
            Some(Key::Action(Action::KeyIntroduction)) => self.introduction.open(),
            Some(Key::Action(Action::SessionList)) if !self.introduction.is_open() => {
                self.selector.open(&self.registry.rows());
            }
            Some(Key::Typed(byte)) if self.introduction.is_open() => self.introduction.key(byte),
            Some(Key::Typed(byte)) if self.selector.is_open() => {
                self.selector.key(byte, &self.registry.rows());
            }
            Some(Key::Action(Action::SessionList) | Key::Typed(_)) | None => {}
        }
        None
    }
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

fn uncertain(uncertainty: &Uncertainty) -> String {
    match uncertainty {
        Uncertainty::Mismatched => "another session answered".into(),
        Uncertainty::Unanswered(error) => format!("no answer: {error}"),
        Uncertainty::Unreachable(error) => format!("unreachable: {error}"),
        Uncertainty::DaemonAlive => "its daemon runs but does not answer".into(),
        Uncertainty::IdentityUnknown(error) => format!("daemon identity unknown: {error}"),
    }
}
