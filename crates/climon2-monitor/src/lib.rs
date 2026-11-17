//! The interactive monitor (spec #73): one terminal that tracks every live
//! session. It validates its start before touching the host terminal, then
//! owns the host with one permanent top row, discovers sessions off its UI
//! thread, keeps one passive client per live session, and lists them in the
//! selector on prefix+s, under the key introduction when it is due, until
//! prefix+x or a catchable termination signal. Once its first discovery
//! pass's clients are ready, it selects and acquires the first eligible row
//! (#266).

mod discovery;
#[cfg(feature = "test-hooks")]
mod hooks;
mod host;
mod initial;
mod introduction;
mod keys;
mod mouse;
mod paint;
mod preference;
mod registry;
mod router;
mod screen;
mod search;
mod selector;
mod startup;
mod supervisor;
mod terminal;

use climon2_core::{SessionId, protocol::TerminalSize};
use climon2_discovery::{Batch, Incomplete, Outcome, Progress};
use climon2_vt::{
    Limits, Parser,
    view::{self as vt, Modes, MouseReporting},
};
use discovery::{Discovery, Found};
use host::{Ending, Event, Host};
use initial::Initial;
use introduction::KeyIntroduction;
use keys::Action;
use mouse::{Area, Gestures, Mouse, Wheel};
use registry::{ClientEvent, Kind, Recovery, Registry, Report};
use router::{Begin, Key, Pasted, Routed, Router};
use screen::{Canvas, Scene, Screen, Session};
use selector::Selector;
use startup::Startup;
use std::{
    collections::BTreeMap,
    io, mem,
    time::{Duration, Instant, SystemTime},
};
use supervisor::{Context, Load, Refusal, Supervisor};
use terminal::{Authority, Control, Paste, Presentation, Request, Terminal};

/// A one-line diagnostic that the thin main prints, after the host is
/// restored, with the climon2 error code.
pub use climon2_core::failure::Failure as Error;

/// The CLI execs the monitor with no arguments. Returns the exit status of an
/// orderly exit: 0 for prefix+x, 128 + signal for a termination signal.
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
    router: Router,
    introduction: KeyIntroduction,
    registry: Registry,
    selector: Selector,
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
        };
        monitor.show().map(|()| monitor)
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
            self.retarget();
            self.show()?;
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

    /// Paints the current scene when it differs from what the screen shows.
    /// Each terminal first learns whether this frame shows its applied
    /// restoration, so its control and input readiness describe the frame
    /// being painted.
    fn show(&mut self) -> io::Result<()> {
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
            incomplete: self.registry.incomplete(),
            progress: self
                .initial
                .progress(&self.registry)
                .map(|progress| progress.to_string()),
            late: self.initial.late(&self.registry),
            session,
        };
        self.screen.show(&scene)
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
    fn enter(&mut self, id: SessionId) {
        match self.registry.kind(&id) {
            Some(Kind::Connected) => {
                self.choose(id.clone());
                self.acquire(&id);
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
    /// keeps the selected controller's size.
    fn present(&mut self, id: &SessionId, presentation: Presentation) {
        let ready =
            terminal(&mut self.terminals, self.model, id).present(&mut self.parser, presentation);
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
                Found::Pass(progress) => {
                    self.registry.pass(incomplete(&progress));
                    self.initial.passed();
                }
            }
        }
        while let Ok((id, event)) = self.supervisor.reports.try_recv() {
            self.report(&id, event, now);
        }
        self.selector.update(&self.registry.rows());
        self.initialise();
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
    /// takes them (#113). Over an application that does not report the
    /// mouse, a wheel notch scrolls its history a row, or on its alternate
    /// screen is a cursor key with alternate scroll on (#112).
    fn mouse(&mut self, mouse: Mouse, typed: &mut Vec<u8>) -> io::Result<()> {
        let taking = self.taking();
        let target = taking.as_ref().map(|taking| (&taking.target, taking.area));
        let cell = self.gestures.route(mouse, target);
        let Some((cell, taking)) = cell.zip(taking) else {
            self.inspect(mouse);
            return self.modal(&Routed::Mouse(mouse));
        };
        match mouse::wheel(mouse, &taking.modes, taking.alternate) {
            Some(Wheel::History(rows)) => self.terminal(&taking.target.0).scroll(rows),
            Some(Wheel::Key(key)) => Key::Cursor(key).encode(&taking.modes, typed),
            Some(Wheel::Nothing) => {}
            None => mouse::encode(mouse, cell, &taking.modes, typed),
        }
        Ok(())
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
    /// prefix+j's jump arrives with #114.
    fn action(&mut self, action: Action) -> Option<Ending> {
        match action {
            Action::Exit => return Some(Ending::Exit),
            Action::KeyIntroduction => self.introduction.open(),
            Action::SessionList if !self.introduction.is_open() => {
                let area = self.screen.overlay();
                self.selector
                    .open(&self.registry.rows(), self.selected.as_ref(), area);
            }
            _ => {}
        }
        None
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

    /// Input for the open selector; Enter acts on the row it chose.
    fn selector_input(&mut self, input: &Routed) {
        let area = self.screen.overlay();
        if let Some(id) = self.selector.input(input, &self.registry.rows(), area) {
            self.enter(id);
        }
    }
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
    })
}

/// What the registry says of a selected row beyond its view.
struct Shown<'a> {
    kind: Kind,
    /// Its final outcome is known.
    finished: bool,
    recovery: Option<&'a Recovery>,
}

/// The selected row as the top row shows it: its label, its row status
/// (#107), its control, any error (its terminal's own, otherwise its
/// recovery's diagnostic, owner decision on #116), and what its canvas
/// shows.
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
    let diagnostic = shown.recovery.map(screen::diagnostic);
    Session {
        id: &row.id,
        label: &row.label,
        status: &row.status,
        control: control.label(),
        error: terminal.and_then(Terminal::error).or(diagnostic),
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

#[cfg(test)]
mod tests {
    use super::{
        Canvas, Control, Kind, Mouse, Presentation, Recovery, Shown, Terminal, control, inspected,
        placeholder, retained, revealed, terminal, without_view,
    };

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
