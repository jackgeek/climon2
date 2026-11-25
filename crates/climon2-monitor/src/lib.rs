//! The interactive monitor (spec #73): one terminal that tracks every live
//! session. It validates its start before touching the host terminal, then
//! owns the host with one permanent top row, discovers sessions off its UI
//! thread, keeps one passive client per live session, and lists them in the
//! selector on prefix+s, under the key introduction when it is due, until
//! prefix+x or a catchable termination signal. Once its first discovery
//! pass's clients are ready, it selects and acquires the first eligible row
//! (#266).

mod attention;
mod cadence;
mod delete;
mod discovery;
mod failstop;
#[cfg(feature = "test-hooks")]
mod hooks;
mod host;
mod initial;
mod introduction;
mod keys;
mod mouse;
mod paint;
mod preference;
mod pressure;
mod registry;
mod router;
mod screen;
mod search;
mod selector;
mod shutdown;
mod startup;
mod supervisor;
mod terminal;
mod toast;

use attention::{Deferred, HostReport, Next, Notice};
use climon2_core::{SessionId, protocol::TerminalSize};
use climon2_discovery::{Batch, Incomplete, Outcome, Progress};
use climon2_vt::{
    Limits, Parser,
    view::{self as vt, Modes, MouseReporting},
};
use delete::{Asking, Confirmation, Decision};
use discovery::{Discovery, Found};
use host::{Ending, Event, Host, Output};
use initial::Initial;
use introduction::KeyIntroduction;
use keys::Action;
use mouse::{Area, Gestures, Mouse, Wheel};
use ratatui::layout::Rect;
use registry::{ClientEvent, Kind, Offer, Recovery, Registry, Report, View};
use router::{Begin, Key, Pasted, Routed, Router};
use screen::{Canvas, Scene, Screen, Session};
use selector::Selector;
use shutdown::Verdict;
use startup::Startup;
use std::{
    collections::BTreeMap,
    io, mem,
    path::Path,
    thread,
    time::{Duration, Instant, SystemTime},
};
use supervisor::{Context, Load, Refusal, Supervisor};
use terminal::{Authority, Control, Paste, Presentation, Request, Terminal};
use toast::Toasts;

/// A one-line diagnostic that the thin main prints, after the host is
/// restored, with the climon2 error code.
pub use climon2_core::failure::Failure as Error;

/// The CLI execs the monitor with no arguments. Returns the exit status of an
/// orderly exit: 0 for prefix+x, 128 + signal for a termination signal,
/// never a session's own status. An exit whose cleanup cannot be confirmed
/// never returns: it fail-stops the process with the climon2 error code
/// (#118).
pub fn run() -> Result<u32, Error> {
    let startup = Startup::read()?;
    let log = startup.log.path().to_owned();
    let filter = std::env::var("CLIMON2_LOG").ok();
    let dispatch = startup
        .log
        .dispatch(filter.as_deref())
        .map_err(|error| Error::from(error).logged(&log))?;
    let filter = filter.unwrap_or_else(|| DEFAULT_FILTER.to_owned());
    tracing::dispatcher::with_default(&dispatch, || monitor(startup, &dispatch, filter))
        .map_err(|error| Error::from(error).logged(&log))
}

/// Records how the monitor ended: the host terminal may be gone, so a failure
/// is logged as well as returned for main to print.
/// The host bell a toast rings once the frame first showing its full title
/// has been written (docs/for-humans/protocol/monitor-host/bell.md).
const BEL: &[u8] = b"\x07";
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

/// One wait for host input during shutdown, and how long shutdown waits
/// past its deadline for the supervisor's last round.
const TICK: Duration = Duration::from_millis(20);

/// Owns the host until an ending, shuts every client down, and releases the
/// host before any diagnostic, all within `deadlines.final_host_ms` of the
/// shutdown's end (#118). A monitor that started told the host its
/// attention state, so the host hears the root clear once released (#114),
/// even after a failure.
fn owned(startup: Startup, dispatch: &tracing::Dispatch, filter: String) -> io::Result<u32> {
    let lanes = startup.config.settings().limits.lanes();
    let final_host = Duration::from_millis(lanes.final_host_ms.into());
    let log = startup.log.path().to_owned();
    let (mut host, size) = Host::enter()?;
    let monitor = Monitor::start(size, startup, dispatch, filter);
    let reported = monitor.is_ok();
    let (served, verdict, output) = match monitor {
        Ok(monitor) => monitor.serve(&mut host),
        Err(error) => (Err(error), Verdict::Clean, Output::default()),
    };
    let watchdog = failstop::Watchdog::arm(final_host, dispatch.clone());
    let released = host.release(output);
    let cleared = cleared(reported);
    let status = served.and_then(|ending| released.and(cleared).map(|()| ending.status()));
    let status = judged(status, verdict, &log);
    watchdog.disarm();
    status
}

/// The root clear, once the host heard an attention report.
fn cleared(reported: bool) -> io::Result<()> {
    if reported {
        return host::write_host(attention::CLEAR);
    }
    Ok(())
}

/// The exit a shutdown's `verdict` leaves: an unconfirmed departure fails
/// it, and unconfirmed cleanup fail-stops the whole process, either way
/// with any earlier failure's diagnostic first.
fn judged(status: io::Result<u32>, verdict: Verdict, log: &Path) -> io::Result<u32> {
    match verdict {
        Verdict::Clean => status,
        Verdict::Failed(why) => Err(also(status, &why)),
        Verdict::FailStop(why) => failstop::stop(&also(status, &why), log),
    }
}

/// `why`, after the failure `status` holds, if any.
fn also(status: io::Result<u32>, why: &str) -> io::Error {
    let earlier = status.err().map(|error| format!("{error}; "));
    io::Error::other(format!("{}{why}", earlier.unwrap_or_default()))
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
    router: Router,
    introduction: KeyIntroduction,
    registry: Registry,
    selector: Selector,
    /// The Delete confirmation, while it is open over the selector (#117).
    deleting: Option<Confirmation>,
    discovery: Discovery,
    supervisor: Supervisor,
    /// The selected session: none until initial selection or the user
    /// selects one, and no arrival ever moves it.
    selected: Option<SessionId>,
    /// Initial selection, until a selection exists.
    initial: Initial,
    /// One terminal per session ID, kept for the whole run.
    terminals: BTreeMap<SessionId, Terminal>,
    /// What bounds each terminal's model.
    model: Limits,
    /// The take-control key's byte, if one is bound.
    take_control: Option<u8>,
    /// The one vte parser every terminal's model is fed through (#166),
    /// counted once beside their footprints at startup.
    parser: Parser,
    /// The paste under way and the target it goes to, while that target
    /// takes input; none while none is, or once its target changed.
    pasting: Option<Pasting>,
    /// The mouse gesture under way and its owner.
    gestures: Gestures<Target>,
    /// Whether the host has focus: assumed until it reports otherwise, since
    /// a host that never reports focus cannot be detected (owner decision on
    /// #111).
    host_focused: bool,
    /// What received application input after the last read; a change
    /// before the next read is a target change.
    context: InputContext,
    /// The acknowledgement an explicit action deferred until its target is
    /// viewed (#114).
    acknowledging: Option<Deferred>,
    /// Prefix+j's notice that no session needs attention.
    notice: Notice,
    /// The attention state the host last heard.
    host_report: HostReport,
    toasts: Toasts,
    /// Whether a toast rings the host bell as it begins (`monitor.bell`).
    bell: bool,
    /// When the next frame may be painted: at most `ui.fps` a second.
    frames: cadence::Frames,
    /// What the host has been handed and not yet taken (#285).
    output: Output,
    /// Client reports waiting to be applied, in fair turns within the work
    /// quanta (#119).
    work: cadence::Work<SessionId, ClientEvent>,
    /// The memory budgets history is kept within (#119).
    memory: climon2_core::config::MemoryLimits,
    /// When each session was last inspected: painted selected.
    inspected: BTreeMap<SessionId, Instant>,
}

/// A session under the authority it takes input with.
type Target = (SessionId, Authority);

/// The selected session, whether a modal covers it, and the authority its
/// control period and restoration hold, if any: any change is a target
/// change (spec #73, "Target changes").
type InputContext = (Option<SessionId>, bool, Option<Authority>);

/// A paste going to its target.
struct Pasting {
    target: Target,
    /// The application had bracketed paste on when the paste began.
    bracketed: bool,
}

/// The target taking application input now, its modes, whether its
/// alternate screen is active and the area of its canvas that the host
/// shows.
struct Taking {
    target: Target,
    modes: Modes,
    alternate: bool,
    area: Area,
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
        let take_control = config.take_control_key();
        let hints = config.settings().show_key_hints;
        let jump = hints.then(|| format!("{prefix} then j: jump to attention"));
        let supervisor = Supervisor::start(context(&startup, filter)?, dispatch.clone())?;
        let introduction = KeyIntroduction::new(prefix, startup.preference, hints);
        let limits = &config.settings().limits;
        let search = usize::try_from(limits.search_bytes()).unwrap_or(usize::MAX);
        let discovery = Discovery::start(startup.home, limits.discovery())?;
        let model = limits.model();
        let start = Duration::from_millis(limits.library().start_ms.into());
        let mut monitor = Self {
            screen: Screen::new(size, prefix, take_control, hints),
            router: Router::new(prefix),
            introduction,
            registry: Registry::new(own_session()),
            selector: Selector::new(prefix, hints, search),
            deleting: None,
            discovery,
            supervisor,
            selected: None,
            initial: Initial::new(start),
            terminals: BTreeMap::new(),
            model: Limits {
                bytes: usize::try_from(model.bytes).unwrap_or(usize::MAX),
                history_rows: model.history_rows as usize,
            },
            take_control: take_control.map(climon2_core::keys::ControlKey::byte),
            parser: Parser::new(),
            pasting: None,
            gestures: Gestures::default(),
            host_focused: true,
            context: InputContext::default(),
            acknowledging: None,
            notice: Notice::default(),
            host_report: HostReport::default(),
            toasts: Toasts::new(jump),
            bell: config.settings().bell,
            frames: cadence::Frames::new(limits.cadence().fps),
            output: Output::default(),
            work: work(limits.cadence()),
            memory: limits.memory(),
            inspected: BTreeMap::new(),
        };
        monitor.show().map(|()| monitor)
    }

    /// Acts on host events until an ending, or a failure of the host or
    /// rendering, then shuts down: why it ended, what the shutdown proved,
    /// and what the host has yet to take. Consuming the monitor drops its
    /// screen before the host is released.
    fn serve(mut self, host: &mut Host) -> (io::Result<Ending>, Verdict, Output) {
        let ending = self.events(host);
        let (hosted, report) = self.shut_down(host, ending.is_ok());
        report.log();
        (
            ending.and_then(|ending| hosted.map(|()| ending)),
            report.verdict(),
            self.output,
        )
    }

    /// The coordinated shutdown (#118): application input, acquisition,
    /// discovery and retries stop at once, and the supervisor shuts every
    /// client down concurrently, until one deadline that nothing restarts.
    /// Meanwhile the exit screen counts the sessions it waits for, and host
    /// input is read and discarded: repeated exits and signals do nothing.
    /// A host that failed is left alone. Returns whether the host still
    /// worked, and what shutdown proved, discovery's end included.
    fn shut_down(&mut self, host: &mut Host, hosting: bool) -> (io::Result<()>, shutdown::Report) {
        let deadline = self.supervisor.shut_down() + TICK;
        self.discovery.stop();
        let mut hosted = Ok(());
        let mut waiting = Vec::new();
        let mut report = None;
        while !(report.is_some() && self.discovery.join())
            && deadline.checked_duration_since(Instant::now()).is_some()
        {
            waiting = self.supervisor.waiting().unwrap_or(waiting);
            hosted = hosted.and_then(|()| self.exiting(host, hosting, waiting.len()));
            report = report.or_else(|| self.supervisor.report());
        }
        let mut report = report.unwrap_or_else(|| unfinished(waiting));
        if !self.discovery.join() {
            report.unfinished.push("discovery pass".to_owned());
        }
        (hosted, report)
    }

    /// One round of the exit screen: painted, then one wait for host
    /// input, whose resizes alone count; without a working host, one tick.
    fn exiting(&mut self, host: &mut Host, hosting: bool, waiting: usize) -> io::Result<()> {
        if !hosting {
            thread::sleep(TICK);
            return Ok(());
        }
        self.screen.exiting(waiting)?;
        self.pump(host)?;
        self.resized(host)
    }

    /// One wait for host input during exit: a resize resizes the exit
    /// screen, and anything else is ignored.
    fn resized(&mut self, host: &mut Host) -> io::Result<()> {
        if let Event::Resize(size) = host.next(&self.output)? {
            self.screen.resize(size);
        }
        Ok(())
    }

    fn events(&mut self, host: &mut Host) -> io::Result<Ending> {
        loop {
            if let Some(ending) = self.step(host)? {
                return Ok(ending);
            }
        }
    }

    /// Handles the next host event, then, unless it ends the monitor, what
    /// discovery and the supervisor reported meanwhile, and writes what the
    /// host takes of the frame.
    fn step(&mut self, host: &mut Host) -> io::Result<Option<Ending>> {
        let event = host.next(&self.output)?;
        let read = matches!(event, Event::Input(_));
        let ending = self.handle(event)?;
        if ending.is_none() {
            self.collect();
            self.relieve();
            self.retarget();
            self.frame(read)?;
            self.pump(host)?;
        }
        Ok(ending)
    }

    /// A target change since the last read discards a pending prefix (spec
    /// #73, "Target changes"); one the read itself made came before any
    /// prefix it left pending. An open paste or gesture learns of a change
    /// when its next part arrives.
    fn retarget(&mut self) {
        let current = self.taking().map(|taking| taking.target);
        self.cut(current.as_ref());
        let context = self.context();
        if context != self.context {
            self.router.retarget();
        }
        self.context = context;
    }

    fn context(&self) -> InputContext {
        let id = self.selected.clone();
        let authority = id
            .as_ref()
            .and_then(|id| self.terminals.get(id))
            .and_then(Terminal::authority);
        (id, self.covered(), authority)
    }

    fn handle(&mut self, event: Event<'_>) -> io::Result<Option<Ending>> {
        match event {
            Event::End(ending) => Ok(Some(ending)),
            Event::Resize(size) => {
                self.screen.resize(size);
                self.resize();
                self.show().map(|()| None)
            }
            Event::Input(bytes) => self.input(bytes),
            Event::Idle => Ok(None),
        }
    }

    /// Each step's frame: at once after a host read, so input, the
    /// selector and exit respond without waiting, and otherwise once one is
    /// due (at most `ui.fps` a second, #119): one not due is skipped, and a
    /// later step's paints the scene as it is then. Frames an action needs
    /// at once (a revealed canvas that the rest of a read may type into, a
    /// resize) are painted when they happen. Every frame counts toward the
    /// cadence, so output alone never paints faster than it.
    fn frame(&mut self, read: bool) -> io::Result<()> {
        if read || self.frames.due(Instant::now()) {
            return self.show();
        }
        Ok(())
    }

    /// Paints the current scene when it differs from what the screen shows.
    /// Each terminal first learns whether this frame shows its applied
    /// restoration, so its control and input readiness describe the frame
    /// being painted.
    fn show(&mut self) -> io::Result<()> {
        let now = Instant::now();
        self.frames.painted(now);
        if let Some(id) = self.selected.clone() {
            self.inspected.insert(id, now);
        }
        self.menus();
        self.toasts.frame(now, self.screen.overlay());
        let titling = self.toasts.titling(now, self.screen.overlay());
        self.mark_shown();
        self.synchronise();
        let rows = self.registry.rows();
        let row = self
            .selected
            .as_ref()
            .and_then(|id| rows.iter().find(|row| &row.id == id));
        let session = row.and_then(|row| {
            let shown = shown(&self.registry, &row.id)?;
            Some(selected(row, shown, self.terminals.get(&row.id)))
        });
        let scene = Scene {
            introduction: &self.introduction,
            rows: &rows,
            selector: &self.selector,
            asking: asking(self.deleting.as_ref(), &self.registry, &rows),
            incomplete: self.registry.incomplete(),
            progress: self
                .initial
                .progress(&self.registry)
                .map(|progress| progress.to_string()),
            late: self.initial.late(&self.registry),
            notice: self.notice.text(now),
            toasts: &self.toasts,
            now,
            session,
        };
        self.screen.show(&scene, titling)?;
        self.attend();
        Ok(())
    }

    /// Writes what the host takes now, never waiting for it (#285). Each
    /// time it has taken everything it was handed, a toast whose full title
    /// that showed begins its hold, and the newest frame is handed on, so
    /// frames drawn meanwhile never queue.
    fn pump(&mut self, host: &mut Host) -> io::Result<()> {
        host.write(&mut self.output)?;
        while self.output.idle() && self.hand_on() {
            host.write(&mut self.output)?;
        }
        Ok(())
    }

    /// The host has written everything it was handed: hands it the newest
    /// frame and any bell, and says whether it handed anything.
    fn hand_on(&mut self) -> bool {
        if let Some(toast) = self.screen.written() {
            self.title(toast);
        }
        if let Some(bytes) = self.screen.commit() {
            self.output.queue(&bytes);
        }
        !self.output.idle()
    }

    /// While a menu is open, every pending attention episode that was
    /// never toasted is eligible (spec #73, "Toast eligibility"): one first
    /// seen while viewed toasts as a menu opens, and the toasts' ledger
    /// keeps any from toasting twice.
    fn menus(&mut self) {
        if self.covered() {
            for (id, news) in self.registry.pending() {
                self.announce(&id, news, false);
            }
        }
    }

    /// Hands `id`'s `news` to the toasts, with its label and whether this
    /// monitor viewed it when it arrived.
    fn announce(&mut self, id: &SessionId, news: registry::News, viewed: bool) {
        let label = self.registry.label(id).unwrap_or_default();
        self.toasts.announce(id, news, label, viewed);
    }

    /// Once a frame showing the full title of the toast that began at
    /// `toast` has been written to the host, the toast's hold starts and,
    /// while `monitor.bell` is on, one BEL rings, unless it was dismissed
    /// first (owner decision on #115, 2026-11-17;
    /// docs/for-humans/protocol/monitor-host/bell.md).
    fn title(&mut self, toast: Instant) {
        if self.toasts.titled(toast, Instant::now()) {
            self.ring();
        }
    }

    /// One BEL, while `monitor.bell` is on.
    fn ring(&mut self) {
        if self.bell {
            self.output.queue(BEL);
        }
    }

    /// Once a frame is painted (spec #73, "Viewing, focus, acknowledgement
    /// and host signals"; #114): each daemon hears whether the monitor
    /// views its session, a deferred acknowledgement is submitted once its
    /// target is viewed, after that viewing was submitted (Set viewing is
    /// coalesced, so it can lag the frame: owner decision on #114), and the
    /// host hears whether any connected session needs attention.
    fn attend(&mut self) {
        self.reacknowledge();
        let viewed = self.viewed().cloned();
        for (id, terminal) in &mut self.terminals {
            if let Some(request) = terminal.viewing(viewed.as_ref() == Some(id)) {
                self.supervisor.request(id.clone(), request);
            }
        }
        let reported = viewed.and_then(|id| self.terminals.get(&id));
        self.acknowledge(reported.is_some_and(Terminal::viewing_reported));
        let pending = self.registry.attention().is_some();
        if let Some(report) = self.host_report.report(pending) {
            self.output.queue(report);
        }
    }

    /// An acknowledgement that a newer restoration of its control period
    /// superseded, as a resize can bring, is deferred again while its
    /// session stays selected. Reselecting a session is an explicit action
    /// that defers one anyway.
    fn reacknowledge(&mut self) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let owed = self.terminals.get_mut(&id);
        if owed.is_some_and(Terminal::take_unacknowledged) {
            self.acknowledging = Some(Deferred::new(id));
        }
    }

    /// The selected session while the monitor views it: its target takes
    /// input (controlling, ready, painted uncovered with room, not ended),
    /// its view is live, the host has focus (assumed until it reports
    /// otherwise) and no toast covers its canvas.
    fn viewed(&self) -> Option<&SessionId> {
        let (id, terminal, _) = self.target()?;
        let live = terminal.view().is_some_and(|view| view.scrolled() == 0);
        let covered = self.toasts.covers(self.canvas(id), self.screen.overlay());
        (live && self.host_focused && !covered).then_some(id)
    }

    /// The host cells `id`'s canvas takes below the top row.
    fn canvas(&self, id: &SessionId) -> Rect {
        let terminal = self.terminals.get(id);
        let geometry = terminal.and_then(Terminal::geometry);
        let geometry = geometry.map(|geometry| (geometry.cols(), geometry.rows()));
        toast::canvas(self.screen.overlay(), geometry)
    }

    /// Prefix+j and Enter dismiss a toast covering their target's canvas
    /// (owner decision on #114).
    fn dismiss_toast(&mut self, id: &SessionId) {
        let canvas = self.canvas(id);
        let area = self.screen.overlay();
        self.toasts.dismiss_over(canvas, area, Instant::now());
    }

    /// Advances a deferred acknowledgement against the selected target as
    /// this frame left it: submitted once `viewing`, kept while not, dropped
    /// once the target context changed.
    fn acknowledge(&mut self, viewing: bool) {
        let Some(deferred) = self.acknowledging.take() else {
            return;
        };
        match deferred.next(&self.attended(viewing)) {
            Next::Wait(deferred) => self.acknowledging = Some(deferred),
            Next::Submit(id, authority) => {
                self.supervisor.request(id, Request::Acknowledge(authority));
            }
            Next::Cancel => {}
        }
    }

    /// The selected target as the frame just painted left it.
    fn attended(&self, viewing: bool) -> attention::Target<'_> {
        let id = self.selected.as_ref();
        let terminal = id.and_then(|id| self.terminals.get(id));
        attention::Target {
            id,
            covered: self.covered(),
            control: control(terminal),
            authority: terminal.and_then(Terminal::authority),
            viewing,
        }
    }

    /// An explicit action on the selected session acknowledges its pending
    /// attention once it is viewed (#114).
    fn defer_acknowledgement(&mut self) {
        self.acknowledging = self.selected.clone().map(Deferred::new);
    }

    /// What each terminal owes its application before the frame: the end
    /// marker of a paste cut short ([`Terminal::paste_end`]), then
    /// application focus (spec #73, "Application focus"). The selected
    /// session is focused while no modal covers it and the host has focus;
    /// every other is not. Each ready controller with `focus-v1` is told of
    /// each change once, a newly ready one of the current desire.
    fn synchronise(&mut self) {
        let focused = self.host_focused && !self.covered();
        for (id, terminal) in &mut self.terminals {
            let desired = focused && self.selected.as_ref() == Some(id);
            let owed = terminal.paste_end();
            for request in owed.into_iter().chain(terminal.focus(desired)) {
                self.supervisor.request(id.clone(), request);
            }
        }
    }

    /// Only the [`Monitor::shown`] terminal is shown.
    fn mark_shown(&mut self) {
        let shown = self.shown().cloned();
        for (id, terminal) in &mut self.terminals {
            terminal.painted(shown.as_ref() == Some(id));
        }
    }

    /// The selected session while the screen shows its canvas: nothing
    /// covers it and the host has room for it.
    fn shown(&self) -> Option<&SessionId> {
        let shows = self.screen.shows_canvas(self.covered());
        self.selected.as_ref().filter(|_| shows)
    }

    /// `id`'s terminal, created the first time it is needed.
    fn terminal(&mut self, id: &SessionId) -> &mut Terminal {
        terminal(&mut self.terminals, self.model, id)
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

    /// Enter acts on the row's condition when pressed (spec #73, "Enter"):
    /// a connected row not known to be final is selected and acquired; a
    /// Starting or Unavailable row with no connected client, and a
    /// disconnected or final row, are selected without acquiring, read-only
    /// (#116). This monitor's own session has no row (owner decision on
    /// #110).
    /// Enter on a connected row also dismisses a toast covering it and
    /// acknowledges its attention once viewed (#114).
    fn enter(&mut self, id: SessionId) {
        match self.registry.kind(&id) {
            Some(Kind::Connected) => {
                self.dismiss_toast(&id);
                self.choose(id.clone());
                self.acquire(&id);
                self.defer_acknowledgement();
            }
            Some(Kind::Unconnected | Kind::Retained) => self.choose(id),
            None => {}
        }
    }

    /// Selects `id` and closes the selector. A session selected before
    /// keeps any control in the background, and nothing is selected
    /// automatically any more.
    fn choose(&mut self, id: SessionId) {
        self.selector.close();
        self.initial.selected();
        self.selected = Some(id);
    }

    /// Initial selection (spec #73): once collection has ended, the first
    /// eligible row is selected and acquired, at most once and never over
    /// an existing selection. Any open modal stays open over it.
    fn initialise(&mut self) {
        if let Some(id) = self.initial.choose(&self.registry, Instant::now()) {
            self.selected = Some(id.clone());
            self.acquire(&id);
        }
    }

    /// Takes control of `id` explicitly, then keeps the selected
    /// controller's size.
    fn acquire(&mut self, id: &SessionId) {
        let request = self.terminal(id).acquire();
        self.request(id, Some(request));
        self.resize();
    }

    /// The take-control key acquires the selected session while it shows
    /// the non-controller placeholder, and is consumed either way; the
    /// monitor never takes control back by itself.
    fn take_control(&mut self) {
        let revoked = self.selected.clone().filter(|id| {
            self.registry
                .kind(id)
                .is_some_and(|kind| placeholder(kind, self.control(id)))
        });
        if let Some(id) = revoked {
            self.acquire(&id);
        }
    }

    /// The control the monitor holds over `id`.
    fn control(&self, id: &SessionId) -> Control {
        control(self.terminals.get(id))
    }

    /// Applies a client's control and display report to its terminal,
    /// submitting the Ready that follows an applied restoration, then
    /// keeps the selected controller's size. An update the terminal's model
    /// refused drops the session's client, to be retried passively, while
    /// its last safe content stays read-only (#119).
    fn present(&mut self, id: &SessionId, presentation: Presentation) {
        let terminal = terminal(&mut self.terminals, self.model, id);
        let ready = terminal.present(&mut self.parser, presentation);
        if let Some(why) = terminal.take_rejection() {
            self.supervisor.drop_client(id.clone(), why);
        }
        self.request(id, ready);
        self.resize();
    }

    /// Keeps history within its budgets (#119): each session's within
    /// `memory.history_bytes_per_session`, shortening its own, then all
    /// histories together within `history_bytes_total` and everything the
    /// monitor tracks within `total_bytes`, shortening caches in the
    /// pressure order ([`pressure::order`]). No cache ever gives up the
    /// chunk holding its newest page.
    fn relieve(&mut self) {
        let budgets = pressure::Budgets {
            per_session: within(self.memory.history_bytes_per_session),
            histories: within(self.memory.history_bytes_total),
            total: within(self.memory.total_bytes),
        };
        // Beside the models: the one parser, every unapplied report and
        // every admitted reservation (#120).
        let reserved = within(self.supervisor.reserved());
        let fixed = [Parser::BYTES, self.supervisor.backlog(), reserved]
            .into_iter()
            .sum();
        let caches = self.caches();
        pressure::relieve(&mut self.terminals, &caches, fixed, budgets);
    }

    /// Every terminal's cache as the pressure policy sees it.
    fn caches(&self) -> Vec<pressure::Cache<SessionId>> {
        let selected = self.selected.as_ref();
        let cache = |id: &SessionId| {
            let inspected = self.inspected.get(id).copied();
            cache(id, self.registry.kind(id), selected, inspected)
        };
        self.terminals.keys().map(cache).collect()
    }

    /// Applies what discovery reported since the last event, asking for a
    /// client for each session that now needs one, then client reports in
    /// fair turns: each client's at most `ui.client_quantum_ms` at a time,
    /// all at most `ui.background_quantum_ms` this turn (#119). The rest
    /// waits for the next turn, still charged to its client, which is not
    /// drained meanwhile once its reports fill its allowance.
    fn collect(&mut self) {
        let now = SystemTime::now();
        self.discover(now);
        while let Ok((id, event)) = self.supervisor.reports.try_recv() {
            self.work.push(id, event);
        }
        self.work.begin(Instant::now());
        while let Some((id, event)) = self.work.next(Instant::now()) {
            let bytes = supervisor::footprint(&id, &event);
            self.report(&id, event, now);
            self.supervisor.processed(&id, bytes);
        }
        let registry = &self.registry;
        self.deleting = self
            .deleting
            .take()
            .filter(|confirmation| registry.kind(confirmation.target()).is_some());
        self.selector.update(&self.registry.rows());
        self.initialise();
    }

    /// What discovery found since the last turn: batches of sessions, and
    /// the end of each pass.
    fn discover(&mut self, now: SystemTime) {
        while let Ok(found) = self.discovery.found.try_recv() {
            match found {
                Found::Batch(batch) => self.batch(batch, now),
                Found::Pass(progress) => {
                    self.registry.pass(incomplete(&progress));
                    self.initial.passed();
                }
            }
        }
    }

    /// Control and display go to the session's terminal, a confirmed
    /// Delete's end dismisses its row (#117), and every other event goes to
    /// [`Monitor::client`].
    fn report(&mut self, id: &SessionId, event: ClientEvent, now: SystemTime) {
        match event {
            ClientEvent::Presentation(presentation) => self.present(id, presentation),
            ClientEvent::Removed => self.dismiss(id),
            event => self.client(id, event, now),
        }
    }

    /// Any other client event goes to the session's row, and the command's
    /// final result and the end of the client to its terminal too. What
    /// the row announces goes to the toasts, with whether the monitor
    /// viewed the session before the event changed it.
    fn client(&mut self, id: &SessionId, event: ClientEvent, now: SystemTime) {
        let viewed = self.viewed() == Some(id);
        if let Some(presentation) = ending(&event) {
            self.present(id, presentation);
        }
        if let Some(news) = self.registry.client(id, event, now) {
            self.announce(id, news, viewed);
        }
    }

    /// A confirmed Delete is done (#117): the row is dismissed for the run
    /// and its retained cache discarded. A selected row's selection falls
    /// back to the row now at its place, else the one before it, else none,
    /// and so does the selector's highlight; neither acquires anything.
    fn dismiss(&mut self, id: &SessionId) {
        let before = self.registry.rows();
        self.registry.dismiss(id);
        self.terminals.remove(id);
        let after = self.registry.rows();
        self.selector.dismissed(id, &before, &after);
        if self.selected.as_ref() == Some(id) {
            self.selected = delete::fallback(id, &ids(&before), &ids(&after));
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
            Outcome::Stale(metadata, _) => {
                self.supervisor.forget(metadata.session_id.clone());
                (metadata, registry::Found::Stale)
            }
        };
        if let Some(metadata) = self.registry.discovered(metadata, found, now) {
            self.initial
                .started(metadata.session_id.clone(), Instant::now());
            self.supervisor.start_client(metadata);
        }
    }

    /// Routes one host read; a lone Escape that ends it then goes at once
    /// to a session taking input, while a modal waits for the next byte.
    fn input(&mut self, bytes: &[u8]) -> io::Result<Option<Ending>> {
        let routed = self.router.read(bytes);
        let ending = self.route(routed)?;
        if ending.is_none() && self.accepting().is_some() {
            let released = self.router.release();
            self.route(released.into_iter().collect())?;
        }
        self.context = self.context();
        Ok(ending)
    }

    /// Acts on each routed input in turn until one ends the monitor, each
    /// against the state the ones before it left. Application bytes are
    /// gathered and submitted together, before anything else acts, so
    /// nothing typed after an action can reach the target before it, and
    /// nothing after prefix+x reaches any.
    fn route(&mut self, routed: Vec<Routed>) -> io::Result<Option<Ending>> {
        let mut typed = Vec::new();
        let mut ending = None;
        for routed in routed {
            ending = self.routed(routed, &mut typed)?;
            if ending.is_some() {
                break;
            }
        }
        self.submit(&mut typed);
        Ok(ending)
    }

    /// Pastes, mouse reports and host focus reports bypass keys.
    fn routed(&mut self, routed: Routed, typed: &mut Vec<u8>) -> io::Result<Option<Ending>> {
        match routed {
            Routed::Paste(pasted) => self.paste(pasted, typed).map(|()| None),
            Routed::Mouse(mouse) => self.mouse(mouse, typed).map(|()| None),
            Routed::Focus(focused) => {
                self.host_focused = focused;
                Ok(None)
            }
            keyed => self.keyed(keyed, typed),
        }
    }

    /// A paste is data for the target that took input when it began, sent
    /// with that target's markers while it still takes input; once it does
    /// not, the paste is [cut](Monitor::cut) (spec #73, "Paste rules",
    /// "Target changes"). A modal owns a paste: the selector's search takes
    /// it (#113), the key introduction discards it; a paste a modal covers
    /// part-way is cut, and the search ignores the rest of it, which it
    /// never saw begin. A prefix the paste discarded reaches its target as
    /// one literal prefix first.
    fn paste(&mut self, pasted: Pasted, typed: &mut Vec<u8>) -> io::Result<()> {
        if self.covered() {
            self.cut(None);
            return self.modal(&Routed::Paste(pasted));
        }
        let taking = self.taking();
        let starts = pasted.begin != Begin::Continued;
        if starts {
            self.pasting = taking.as_ref().map(|taking| Pasting {
                target: taking.target.clone(),
                bracketed: taking.modes.bracketed_paste,
            });
            typed.extend(pasted.begin.prefix());
        }
        let current = taking.map(|taking| taking.target);
        self.cut(current.as_ref());
        self.submit(typed);
        let ends = pasted.ends;
        if let Some(Pasting { target, bracketed }) = &self.pasting {
            terminal(&mut self.terminals, self.model, &target.0).return_live();
            let part = Paste {
                bracketed: *bracketed,
                starts,
                bytes: pasted.bytes,
                ends,
            };
            self.supervisor
                .request(target.0.clone(), Request::Paste(target.1, part));
        }
        self.pasting = self.pasting.take().filter(|_| !ends);
        Ok(())
    }

    /// A paste whose target no longer takes input as `current` (spec #73,
    /// "Target changes") is cut: the rest of it is discarded through its
    /// end, never sent elsewhere, even if the target takes input again. An
    /// application that had bracketed paste on is owed the end marker
    /// (owner decision on #111), which its terminal pays.
    fn cut(&mut self, current: Option<&Target>) {
        let cut = self
            .pasting
            .take_if(|pasting| Some(&pasting.target) != current);
        if let Some(Pasting {
            target: (id, authority),
            bracketed: true,
        }) = cut
        {
            self.terminal(&id).cut_paste(authority);
        }
    }

    /// A mouse report goes to its gesture's owner (`mouse.rs`): to the
    /// target taking input, at its cell below the top row, as its modes ask,
    /// or nowhere; chrome and overlays keep theirs, and an open modal
    /// takes them (#113); with none open, a wheel notch scrolls a selected
    /// read-only session's history (#116). Over an application that does
    /// not report the mouse, a wheel notch scrolls its history a row, or on
    /// its alternate screen is a cursor key with alternate scroll on (#112).
    fn mouse(&mut self, mouse: Mouse, typed: &mut Vec<u8>) -> io::Result<()> {
        let taking = self.taking();
        let (idle, area) = (self.gestures.idle(), self.screen.overlay());
        let toasted = self.toasts.take(mouse, idle, area, Instant::now());
        let target = taking.as_ref().map(|taking| (&taking.target, taking.area));
        let cell = self.gestures.route(mouse, target.filter(|_| !toasted));
        let Some((cell, taking)) = cell.zip(taking) else {
            return self.local(mouse, toasted);
        };
        match mouse::wheel(mouse, &taking.modes, taking.alternate) {
            Some(Wheel::History(rows)) => self.terminal(&taking.target.0).scroll(rows),
            Some(Wheel::Key(key)) => Key::Cursor(key).encode(&taking.modes, typed),
            Some(Wheel::Nothing) => {}
            None => mouse::encode(mouse, cell, &taking.modes, typed),
        }
        Ok(())
    }

    /// A mouse report the monitor keeps: one the shown toast took is its
    /// own, never selecting, acquiring or acknowledging; any other scrolls
    /// a selected read-only history with no modal open (#116), or goes to
    /// the open modal.
    fn local(&mut self, mouse: Mouse, toasted: bool) -> io::Result<()> {
        if toasted {
            return Ok(());
        }
        self.inspect(mouse);
        self.modal(&Routed::Mouse(mouse))
    }

    /// A vertical wheel notch, with no modal open, scrolls the selected
    /// session's read-only history a row (#116), as it does a live one.
    fn inspect(&mut self, mouse: Mouse) {
        if let Some((id, rows)) = self.scrolls(mouse) {
            self.terminal(&id).scroll(rows);
        }
    }

    /// The selected session whose read-only history `mouse` scrolls, with
    /// no modal open, and by how many rows.
    fn scrolls(&self, mouse: Mouse) -> Option<(SessionId, isize)> {
        let id = self.selected.as_ref().filter(|_| !self.covered())?;
        let shown = shown(&self.registry, id)?;
        let view = retained(shown.kind, shown.finished, self.terminals.get(id))?;
        inspected(mouse, &view).map(|rows| (id.clone(), rows))
    }

    fn keyed(&mut self, routed: Routed, typed: &mut Vec<u8>) -> io::Result<Option<Ending>> {
        if let Some(modes) = self
            .accepting()
            .filter(|_| !matches!(routed, Routed::Action(_)))
        {
            routed.encode(&modes, typed);
            return Ok(None);
        }
        self.submit(typed);
        match routed {
            Routed::Action(action) => Ok(self.action(action)),
            Routed::Unknown(..) | Routed::Rest(_) => Ok(None),
            input => self.modal(&input).map(|()| None),
        }
    }

    /// The selected session's terminal modes while it accepts application
    /// input: no modal is open, the host has room for it, and its control
    /// period's restoration is applied, ready and painted.
    fn accepting(&self) -> Option<Modes> {
        self.taking().map(|taking| taking.modes)
    }

    /// The target taking application input, its modes and its canvas's
    /// area: its geometry, which the host shows below the top row as far as
    /// it has room, so the host reports no cell beyond it.
    fn taking(&self) -> Option<Taking> {
        let (id, terminal, authority) = self.target()?;
        let geometry = terminal.geometry();
        terminal
            .view()
            .zip(geometry)
            .map(|(view, geometry)| Taking {
                target: (id.clone(), authority),
                modes: view.modes(),
                alternate: matches!(view.screen(), vt::Screen::Alternate { .. }),
                area: Area {
                    cols: geometry.cols(),
                    rows: geometry.rows(),
                },
            })
    }

    /// The [`Monitor::shown`] session, its terminal and the authority it
    /// takes input with, while it may take input. `Terminal::input` alone
    /// learns of a modal or a small host only at the next frame; within one
    /// read, a key after prefix+s must already reach the selector.
    fn target(&self) -> Option<(&SessionId, &Terminal, Authority)> {
        let id = self.shown()?;
        let terminal = self.terminals.get(id)?;
        terminal.input().map(|authority| (id, terminal, authority))
    }

    /// Submits the application bytes gathered so far to the target they
    /// were encoded for, under its current authority.
    fn submit(&mut self, typed: &mut Vec<u8>) {
        if typed.is_empty() {
            return;
        }
        let bytes = mem::take(typed);
        let target = self
            .target()
            .map(|(id, _, authority)| (id.clone(), authority));
        if let Some((id, authority)) = target {
            self.terminal(&id).return_live();
            self.request(&id, Some(Request::Input(authority, bytes)));
        }
    }

    /// Prefix+x ends the monitor. Prefix+? opens the key introduction, and
    /// prefix+s the selector unless the modal key introduction is open;
    /// prefix+j jumps from anywhere. The Delete confirmation takes them its
    /// own way (#117).
    fn action(&mut self, action: Action) -> Option<Ending> {
        if self.deleting.is_some() {
            return self.confirming(action);
        }
        self.global(action)
    }

    /// A global action with no Delete confirmation open.
    fn global(&mut self, action: Action) -> Option<Ending> {
        match action {
            Action::Exit => return Some(Ending::Exit),
            Action::KeyIntroduction => self.introduction.open(),
            Action::Jump => self.jump(),
            Action::SessionList => self.session_list(),
        }
        None
    }

    /// Inside the Delete confirmation (owner decision on #117): prefix+x
    /// exits, prefix+j cancels it and jumps, and prefix+s and prefix+? do
    /// nothing.
    fn confirming(&mut self, action: Action) -> Option<Ending> {
        match action {
            Action::Exit => Some(Ending::Exit),
            Action::Jump => {
                self.deleting = None;
                self.jump();
                None
            }
            Action::KeyIntroduction | Action::SessionList => None,
        }
    }

    /// Prefix+s opens the selector, unless the modal key introduction is
    /// open.
    fn session_list(&mut self) {
        if self.introduction.is_open() {
            return;
        }
        let area = self.screen.menu();
        self.selector
            .open(&self.registry.rows(), self.selected.as_ref(), area);
    }

    /// Prefix+j (spec #73, "Attention jump target"; #114): the connected
    /// session that needs attention with the newest daemon change at the
    /// keypress. The jump closes every overlay and dismisses a toast
    /// covering it, selects it, takes control only while the monitor
    /// neither holds nor is acquiring it, shows its live view and
    /// acknowledges its attention once viewed. With none, the selection
    /// stays and `No sessions need attention` shows for a while.
    fn jump(&mut self) {
        let Some(id) = self.registry.attention() else {
            self.notice.show(Instant::now());
            return;
        };
        self.notice.clear();
        self.introduction.close();
        self.dismiss_toast(&id);
        self.choose(id.clone());
        if self.control(&id) == Control::NotControlling {
            self.acquire(&id);
        }
        self.resize();
        self.terminal(&id).return_live();
        self.defer_acknowledgement();
    }

    /// Input no application takes goes to [`Monitor::modal_input`]. When it
    /// closes the last modal, the uncovered session is painted at once, so
    /// the rest of the same read meets the screen now in force and a key
    /// typed straight after Escape reaches the session (owner decision on
    /// #109). Nothing is kept beyond the read.
    fn modal(&mut self, input: &Routed) -> io::Result<()> {
        let covered = self.covered();
        self.modal_input(input);
        if revealed(covered, self.covered()) {
            return self.show();
        }
        Ok(())
    }

    /// Whether the selector or the key introduction covers the canvas.
    fn covered(&self) -> bool {
        screen::covered(&self.selector, &self.introduction)
    }

    /// Input no application takes goes to the key introduction while it is
    /// open, otherwise to the open selector; outside them the take-control
    /// key takes control, and anything else is discarded, never kept for
    /// later. Inside a modal the take-control key does nothing.
    fn modal_input(&mut self, input: &Routed) {
        if self.introduction.is_open() {
            self.introduction_input(input);
        } else if self.selector.is_open() {
            self.selector_input(input);
        } else if self
            .take_control
            .is_some_and(|byte| *input == Routed::Key(Key::Bytes(vec![byte])))
        {
            self.take_control();
        }
    }

    /// The key introduction takes keys only: pastes and mouse reports are
    /// discarded while it is open.
    fn introduction_input(&mut self, input: &Routed) {
        if let Routed::Key(key) = input {
            self.introduction.key(key);
        }
    }

    /// Input for the open selector: the Delete confirmation takes it while
    /// open, and Delete opens it (#117).
    fn selector_input(&mut self, input: &Routed) {
        if self.deleting.is_some() {
            return self.confirmation_input(input);
        }
        if delete::requested(input) {
            return self.ask();
        }
        self.list_input(input);
    }

    /// Delete opens the confirmation on the highlighted row, unless a
    /// Delete of it is already under way: repeating it sends nothing.
    fn ask(&mut self) {
        let highlighted = self.selector.highlighted(&self.registry.rows());
        let offered = highlighted.filter(|id| self.registry.offer(id).is_some());
        self.deleting = offered.map(Confirmation::new);
    }

    /// Input for the open confirmation. Either decision returns to the
    /// selector; cancelling sends nothing and never acknowledges attention
    /// (Confirmed 2026-11-08), and confirming submits what the row offers
    /// now, its row showing the Delete under way.
    fn confirmation_input(&mut self, input: &Routed) {
        let decision = self.deleting.as_mut().and_then(|asked| asked.input(input));
        if let Some(decision) = decision {
            self.deleting = None;
            self.decide(decision);
        }
    }

    fn decide(&mut self, decision: Decision) {
        let Decision::Confirm(id) = decision else {
            return;
        };
        if let Some((metadata, offer)) = self.registry.confirm(&id) {
            let kill = offer == Offer::TerminateAndRemove;
            self.supervisor.remove(metadata, kill);
        }
    }

    /// Input for the selector's list; Enter acts on the row it chose, and
    /// cancelling back to the selected session acknowledges its attention
    /// once viewed (Confirmed 2026-11-08; #114).
    fn list_input(&mut self, input: &Routed) {
        let area = self.screen.menu();
        match self.selector.input(input, &self.registry.rows(), area) {
            Some(id) => self.enter(id),
            None if !self.selector.is_open() => self.defer_acknowledgement(),
            None => {}
        }
    }
}

/// The open Delete confirmation, its row as `rows` show it now and what
/// `registry` offers for it now.
fn asking<'a>(
    confirmation: Option<&'a Confirmation>,
    registry: &Registry,
    rows: &'a [View],
) -> Option<Asking<'a>> {
    let confirmation = confirmation?;
    let row = rows.iter().find(|row| &row.id == confirmation.target())?;
    let offer = registry.offer(&row.id)?;
    Some(Asking {
        confirmation,
        row,
        offer,
    })
}

/// The session IDs of `rows`, in order.
fn ids(rows: &[View]) -> Vec<&SessionId> {
    rows.iter().map(|row| &row.id).collect()
}

/// Whether a key uncovered the canvas: a modal was open before it and none
/// is after it.
fn revealed(covered_before: bool, covered_after: bool) -> bool {
    covered_before && !covered_after
}

/// What a row event means for the session's terminal: the view freezes at
/// the command's final result, and control ends with the client.
fn ending(event: &ClientEvent) -> Option<Presentation> {
    match event {
        ClientEvent::Report(Report::Final(_)) => Some(Presentation::Final),
        ClientEvent::Ended | ClientEvent::Unavailable(_) => Some(Presentation::Ended),
        _ => None,
    }
}

/// `id`'s terminal in `terminals`, created with `model` the first time it
/// is needed.
fn terminal<'a>(
    terminals: &'a mut BTreeMap<SessionId, Terminal>,
    model: Limits,
    id: &SessionId,
) -> &'a mut Terminal {
    terminals
        .entry(id.clone())
        .or_insert_with(|| Terminal::new(model))
}

/// The rows a mouse report scrolls read-only history `view` by: a vertical
/// wheel notch over its normal screen, as over a live one whose
/// application does not report the mouse; nothing on its alternate screen,
/// as nothing reaches the application.
fn inspected(mouse: Mouse, view: &vt::View<'_>) -> Option<isize> {
    let modes = Modes {
        mouse_reporting: MouseReporting::Off,
        ..view.modes()
    };
    let alternate = matches!(view.screen(), vt::Screen::Alternate { .. });
    match mouse::wheel(mouse, &modes, alternate)? {
        Wheel::History(rows) => Some(rows),
        Wheel::Key(_) | Wheel::Nothing => None,
    }
}

/// What `registry` says of `id`'s row beyond its view, if it has one.
fn shown<'a>(registry: &'a Registry, id: &SessionId) -> Option<Shown<'a>> {
    Some(Shown {
        kind: registry.kind(id)?,
        finished: registry.finished(id),
        recovery: registry.recovery(id),
        condition: registry.condition(id),
        refused: registry.error(id),
    })
}

/// What the registry says of a selected row beyond its view.
struct Shown<'a> {
    kind: Kind,
    /// Its final outcome is known.
    finished: bool,
    recovery: Option<&'a Recovery>,
    /// Why it waits for its client: resource-blocked, or its release
    /// loading slowly (#120).
    condition: Option<&'a str>,
    /// Why a confirmed Delete's Kill was refused or is unconfirmed (#117).
    refused: Option<&'a str>,
}

/// The selected row as the top row shows it: its label, its row status
/// (#107), its control, any error (why its Delete's Kill fell short, #117,
/// otherwise that its daemon is gone, otherwise its terminal's own,
/// otherwise why it waits for its client, #120, or its recovery's
/// diagnostic, owner decision on #116), and what its canvas shows. A gone
/// daemon outranks the terminal's own error, which its lost connection may
/// have left: a request the stream's end cancelled.
fn selected<'a>(
    row: &'a registry::View,
    shown: Shown<'a>,
    terminal: Option<&'a Terminal>,
) -> Session<'a> {
    let control = control(terminal);
    let canvas = terminal.and_then(Terminal::view).map_or_else(
        || {
            let retained = retained(shown.kind, shown.finished, terminal);
            without_view(&shown, &row.status, control, retained)
        },
        Canvas::View,
    );
    let recovery = shown.recovery.map(screen::diagnostic);
    let gone = recovery.filter(|_| shown.recovery == Some(&Recovery::Gone));
    let diagnostic = shown.condition.or(recovery);
    Session {
        id: &row.id,
        label: &row.label,
        status: &row.status,
        control: control.label(),
        error: shown
            .refused
            .or(gone)
            .or_else(|| terminal.and_then(Terminal::error))
            .or(diagnostic),
        canvas,
    }
}

/// The read-only history a selected row shows (#116): a row not connected
/// and not known final keeps its retained cache, if it ever had one. A
/// final row shows only the view frozen at its end (#108).
fn retained(kind: Kind, finished: bool, terminal: Option<&Terminal>) -> Option<vt::View<'_>> {
    let inspected = kind != Kind::Connected && !finished;
    terminal.filter(|_| inspected).and_then(Terminal::retained)
}

/// The control the monitor holds through `terminal`: a session it never
/// made a terminal for is not controlled.
fn control(terminal: Option<&Terminal>) -> Control {
    terminal.map_or(Control::NotControlling, Terminal::control)
}

/// The canvas of a selected row whose terminal has no live or frozen view:
/// the placeholder while the monitor does not control a connected session,
/// never its cache; nothing yet while it acquires a connected one; for any
/// other row its `retained` history read-only, or no terminal content and
/// its `status`, under its recovery's overlay.
fn without_view<'a>(
    shown: &Shown<'a>,
    status: &'a str,
    control: Control,
    retained: Option<vt::View<'a>>,
) -> Canvas<'a> {
    match (shown.kind, retained) {
        (Kind::Connected, _) if placeholder(shown.kind, control) => Canvas::Placeholder,
        (Kind::Connected, _) => Canvas::Nothing,
        (_, Some(view)) => Canvas::Retained(view, shown.recovery),
        (_, None) => Canvas::NoContent(status, shown.recovery),
    }
}

/// Whether a selected row shows the non-controller placeholder, on which
/// the take-control key acquires: connected, not known to be final, and
/// neither controlled nor being acquired by this monitor.
fn placeholder(kind: Kind, control: Control) -> bool {
    kind == Kind::Connected && control == Control::NotControlling
}

/// What shutdown knows when the supervisor itself never finished: nothing
/// is confirmed of the sessions it last waited for.
fn unfinished(waiting: Vec<SessionId>) -> shutdown::Report {
    let evidence = shutdown::Evidence::default;
    shutdown::Report {
        instances: waiting.into_iter().map(|id| (id, evidence())).collect(),
        unfinished: vec!["client supervisor".to_owned()],
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
        retry: limits.retry(),
        first: first_identifiers(),
        admission: limits.admission(),
        outside: Parser::BYTES as u64,
    })
}

/// Where the monitor's identifiers start: as test hooks ask (LIM-05).
#[cfg(feature = "test-hooks")]
fn first_identifiers() -> supervisor::Identifiers {
    hooks::identifiers(supervisor::Identifiers::default())
}

/// Where the monitor's identifiers start: connection generations at 1,
/// request IDs after Open's.
#[cfg(not(feature = "test-hooks"))]
fn first_identifiers() -> supervisor::Identifiers {
    supervisor::Identifiers::default()
}

/// `id`'s cache as the pressure policy sees it, its session of `kind`
/// with `selected` selected: it yields to other sessions unless its session
/// is connected, and yields last if it is the selected one.
fn cache(
    id: &SessionId,
    kind: Option<Kind>,
    selected: Option<&SessionId>,
    inspected: Option<Instant>,
) -> pressure::Cache<SessionId> {
    pressure::Cache {
        key: id.clone(),
        retained: kind != Some(Kind::Connected),
        selected: selected == Some(id),
        inspected,
    }
}

/// A byte budget as a `usize`, saturating.
fn within(budget: u64) -> usize {
    usize::try_from(budget).unwrap_or(usize::MAX)
}

/// Client work in fair turns within the `ui` quanta.
fn work(cadence: climon2_core::config::Cadence) -> cadence::Work<SessionId, ClientEvent> {
    let millis = |quantum: u32| Duration::from_millis(quantum.into());
    cadence::Work::new(
        millis(cadence.client_quantum_ms),
        millis(cadence.background_quantum_ms),
    )
}

/// The monitor's one validated loader, shared by every release it loads.
fn loader(executables: climon2_core::layout::ExecutableHome) -> Load {
    let loader = climon2_platform::library::Loader::new(executables);
    Box::new(move |version| {
        loader
            .load(version)
            .map(|library| supervisor::Loaded {
                functions: library.functions(),
                requirements: *library.requirements(),
            })
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

#[cfg(test)]
mod tests {
    use super::{
        Canvas, Control, Kind, Mouse, Presentation, Recovery, Shown, Terminal, View, also, cache,
        control, inspected, placeholder, retained, revealed, selected, terminal, without_view,
    };
    use climon2_core::SessionId;
    use std::time::Instant;

    /// EXT-03 (pure supplement, #118): a shutdown's failure is said after
    /// any earlier failure, such as a render failure, in one diagnostic.
    #[test]
    fn a_shutdown_failure_follows_any_earlier_failure() {
        let earlier = Err(std::io::Error::other("injected draw failure"));
        assert_eq!(
            also(earlier, "monitor shutdown unconfirmed: x").to_string(),
            "injected draw failure; monitor shutdown unconfirmed: x"
        );
        assert_eq!(also(Ok(0), "why").to_string(), "why");
    }

    const CONTROLS: [Control; 5] = [
        Control::Acquiring,
        Control::Restoring,
        Control::Controlling,
        Control::NotControlling,
        Control::Ended,
    ];

    /// CTL-02 and INP-01 takeover (pure supplement, #110): only a connected
    /// row the monitor neither controls, nor is acquiring, nor saw end shows
    /// the placeholder and takes the take-control key; no other kind does,
    /// and a session the monitor never made a terminal for is not
    /// controlled.
    #[test]
    fn only_an_uncontrolled_connected_row_shows_the_placeholder() {
        for kind in [Kind::Connected, Kind::Unconnected, Kind::Retained] {
            let shown: Vec<_> = CONTROLS
                .into_iter()
                .map(|control| placeholder(kind, control))
                .collect();
            let expected = [false, false, false, kind == Kind::Connected, false];
            assert_eq!(shown, expected, "{kind:?}");
        }
        assert_eq!(control(None), Control::NotControlling);
    }

    /// Spec #73 Enter and placeholders (pure supplement, #110, #116): with
    /// no live or frozen view to paint, a connected row the monitor does not
    /// control shows the placeholder, never a cache, and one it is acquiring
    /// nothing yet; any other row shows its retained history read-only, or
    /// that it has no terminal content and its status, under its recovery's
    /// overlay.
    #[test]
    fn a_selected_row_without_a_view_shows_the_placeholder_its_history_or_no_content() {
        let model = climon2_vt::Model::new(
            climon2_vt::Geometry::for_host(10, 3).unwrap(),
            climon2_vt::Limits {
                bytes: 64 << 20,
                history_rows: 200,
            },
        )
        .unwrap();
        let retrying = Recovery::Retrying("lost".into());
        let shape = |canvas: Canvas<'_>| match canvas {
            Canvas::View(_) => "view".to_owned(),
            Canvas::Placeholder => "placeholder".to_owned(),
            Canvas::Retained(_, recovery) => format!("history, {recovery:?}"),
            Canvas::NoContent(condition, recovery) => {
                format!("no content, {condition}, {recovery:?}")
            }
            Canvas::Nothing => "nothing".to_owned(),
        };
        let shown = |kind, control, cached: bool| {
            let shown = Shown {
                kind,
                finished: false,
                recovery: Some(&retrying),
                condition: None,
                refused: None,
            };
            let view = cached.then(|| model.view());
            shape(without_view(&shown, "Status", control, view))
        };
        assert_eq!(
            shown(Kind::Connected, Control::NotControlling, true),
            "placeholder"
        );
        assert_eq!(shown(Kind::Connected, Control::Acquiring, true), "nothing");
        for kind in [Kind::Unconnected, Kind::Retained] {
            assert_eq!(
                shown(kind, Control::NotControlling, true),
                "history, Some(Retrying(\"lost\"))"
            );
            assert_eq!(
                shown(kind, Control::NotControlling, false),
                "no content, Status, Some(Retrying(\"lost\"))"
            );
        }
    }

    /// REC-02 (pure supplement, #116): only a row neither connected nor
    /// known final shows a retained cache, and only once one exists; a
    /// final row keeps to the view frozen at its end (#108).
    #[test]
    fn only_a_row_neither_connected_nor_final_shows_its_retained_history() {
        let mut parser = climon2_vt::Parser::new();
        let mut terminal = Terminal::new(climon2_vt::Limits {
            bytes: 64 << 20,
            history_rows: 200,
        });
        assert!(retained(Kind::Retained, false, Some(&terminal)).is_none());
        let held = terminal::Authority {
            generation: 1,
            period: 1,
            restoration: 1,
        };
        let geometry = climon2_vt::Geometry::for_host(10, 3).unwrap();
        terminal.present(&mut parser, Presentation::Granted(held, geometry));
        terminal.present(&mut parser, Presentation::Begin(held, geometry));
        terminal.present(&mut parser, Presentation::Complete);
        terminal.present(&mut parser, Presentation::Ended);
        let cases = [
            (Kind::Retained, false, true),
            (Kind::Unconnected, false, true),
            (Kind::Connected, false, false),
            (Kind::Retained, true, false),
            (Kind::Unconnected, true, false),
        ];
        for (kind, finished, expected) in cases {
            let shown = retained(kind, finished, Some(&terminal)).is_some();
            assert_eq!(shown, expected, "{kind:?}, finished {finished}");
        }
        assert!(retained(Kind::Retained, false, None).is_none());
    }

    /// History budgets under aggregate pressure (pure supplement, #119):
    /// a terminal's cache yields to other sessions only while its session is
    /// not connected (unconnected, retained or no longer listed), and yields
    /// last only while its session is the selected one.
    #[test]
    fn only_a_disconnected_cache_yields_and_only_the_selected_one_yields_last() {
        let id: SessionId = "0d5c0000-0000-4000-8000-000000000001".parse().unwrap();
        let other: SessionId = "0d5c0000-0000-4000-8000-000000000002".parse().unwrap();
        let inspected = Some(Instant::now());
        let kinds = [
            (Some(Kind::Connected), false),
            (Some(Kind::Unconnected), true),
            (Some(Kind::Retained), true),
            (None, true),
        ];
        for (kind, retained) in kinds {
            for (selected, last) in [(Some(&id), true), (Some(&other), false), (None, false)] {
                let seen = cache(&id, kind, selected, inspected);
                assert_eq!(seen.key, id);
                assert_eq!(seen.inspected, inspected);
                assert_eq!(
                    (seen.retained, seen.selected),
                    (retained, last),
                    "{kind:?}, selected {selected:?}"
                );
            }
        }
    }

    /// REC-01 (Monitor: a gone daemon, pure supplement, #116): a request
    /// the stream's end cancelled while control was held leaves the
    /// terminal's own error, but once discovery proves the daemon gone the
    /// top row's error slot says `Session no longer exists`, as the
    /// overlay over its history does (owner decision on #116).
    #[test]
    fn a_gone_session_says_so_in_its_error_slot_over_a_cancelled_request() {
        let mut parser = climon2_vt::Parser::new();
        let mut terminal = Terminal::new(climon2_vt::Limits {
            bytes: 64 << 20,
            history_rows: 200,
        });
        let held = terminal::Authority {
            generation: 1,
            period: 1,
            restoration: 1,
        };
        let geometry = climon2_vt::Geometry::for_host(10, 3).unwrap();
        terminal.present(&mut parser, Presentation::Granted(held, geometry));
        terminal.present(&mut parser, Presentation::Begin(held, geometry));
        terminal.present(&mut parser, Presentation::Complete);
        let cancelled = "the request was accepted but never executed";
        let viewing = terminal::Request::Viewing(held, true);
        terminal.present(
            &mut parser,
            Presentation::Answered(viewing, Err(cancelled.into())),
        );
        terminal.present(&mut parser, Presentation::Ended);
        let row = View {
            id: SessionId::from_bytes([1; 16]),
            status: "Unavailable".into(),
            label: "Lost".into(),
            since: std::time::SystemTime::UNIX_EPOCH,
        };
        let error = |recovery| {
            let shown = Shown {
                kind: Kind::Unconnected,
                finished: false,
                recovery: Some(recovery),
                condition: None,
                refused: None,
            };
            selected(&row, shown, Some(&terminal)).error
        };
        // The connection's end also ended the terminal's errors, so the
        // cancelled request no longer shows either way.
        let retrying = Recovery::Retrying("transport io_error: reset".into());
        assert_eq!(error(&retrying), Some("transport io_error: reset"));
        assert_eq!(error(&Recovery::Gone), Some("Session no longer exists"));
    }

    /// Owner decision on #109 (pure supplement): only a key that closes the
    /// last modal uncovers the canvas and paints before the rest of the read;
    /// one that opens a modal, or leaves the canvas as it was, does not.
    #[test]
    fn only_closing_the_last_modal_reveals_the_canvas() {
        assert!(revealed(true, false));
        assert!(!revealed(true, true));
        assert!(!revealed(false, true));
        assert!(!revealed(false, false));
    }

    /// REC-02 and TRM-05 (pure supplement, #116): a vertical wheel notch
    /// scrolls read-only history a row, up going back, on its normal screen
    /// whatever mouse reporting its application left on; nothing on its
    /// alternate screen, even with alternate scroll; any other report
    /// nothing.
    #[test]
    fn the_wheel_scrolls_read_only_history_only_on_its_normal_screen() {
        let model = |bytes: &[u8]| {
            let mut model = climon2_vt::Model::new(
                climon2_vt::Geometry::for_host(10, 3).unwrap(),
                climon2_vt::Limits {
                    bytes: 64 << 20,
                    history_rows: 200,
                },
            )
            .unwrap();
            model.feed(&mut climon2_vt::Parser::new(), bytes).unwrap();
            model
        };
        let report = |code| Mouse {
            code,
            column: 1,
            row: 2,
            released: false,
        };
        let reporting = model(b"\x1b[?1000h");
        assert_eq!(inspected(report(64), &reporting.view()), Some(1));
        assert_eq!(inspected(report(65), &reporting.view()), Some(-1));
        assert_eq!(inspected(report(0), &reporting.view()), None);
        let alternate = model(b"\x1b[?1049h\x1b[?1007h");
        assert_eq!(inspected(report(64), &alternate.view()), None);
        let alternate = model(b"\x1b[?1049h");
        assert_eq!(inspected(report(64), &alternate.view()), None);
    }
}
