//! The stepped monitor (ADR 0011): a whole monitor with no interface of its
//! own. One consumer thread owns it and steps it: a step takes the time and
//! returns the events since the last one, an [`Action`] asks something of
//! it, and its consumer reads rows, toasts and the selected session
//! straight from it. It discovers sessions off that thread, keeps one
//! passive client per live session, and once its first discovery pass's
//! clients are ready selects and acquires the first eligible row (#266).
use crate::{
    action::{Action, Choice, Drive, Look, Paste as Part},
    attention::{self, Deferred, HostReport, Next, Notice},
    cadence,
    consumer::Consumer,
    delete,
    discovery::{Discovery, Found},
    initial::Initial,
    pressure,
    registry::{self, ClientEvent, Kind, Offer, Registry, Report, View},
    session::{Session, control, placeholder, retained, selected, shown},
    shutdown::{self, Verdict},
    supervisor::{self, Context, Supervisor},
    terminal::{Authority, Control, Paste, Presentation, Request, Terminal},
    toast::Toasts,
};
use climon2_core::{
    SessionId,
    config::{MemoryLimits, MonitorConfig},
    layout::HomeLayout,
};
use climon2_discovery::{Batch, Incomplete, Outcome, Progress};
use climon2_embed::load::Load;
use climon2_vt::{
    Limits, Parser,
    view::{self as vt, Modes},
};
use std::{
    collections::BTreeMap,
    io, mem,
    time::{Duration, Instant, SystemTime},
};

/// The time, as the monitor's consumer gives it: the library never reads a
/// clock of its own while it is stepped.
pub trait Clock {
    /// Now, on the monotonic clock.
    fn now(&self) -> Instant;

    /// Now, as a date and time.
    fn wall(&self) -> SystemTime;
}

/// The system's clocks.
pub struct System;

impl Clock for System {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn wall(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// What a monitor starts from. Nothing here is read from the process: a
/// test gives a monitor a home, a loader and a configuration of its own.
pub struct Inputs {
    /// The canonical climon2 home, whose sessions the monitor discovers.
    pub home: HomeLayout,
    /// Loads each session's exact-release client library.
    pub load: Load,
    pub config: MonitorConfig,
    /// The monitor's own enclosing session, which gets no row.
    pub own: Option<SessionId>,
    /// The log filter every client library is given.
    pub filter: String,
    /// Where the monitor's threads log.
    pub dispatch: tracing::Dispatch,
}

/// An effect on its consumer's device, or on the consumer's own state,
/// which the consumer delivers its own way.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    /// The host is to hear whether any connected session needs attention:
    /// the initial state, then each change.
    Attention(bool),
    /// The shown toast rings its one bell.
    Bell,
    /// An action selected a session: a consumer closes what it had open
    /// over the selection.
    Selected,
    /// A confirmed Delete removed this row: the rows before and after it
    /// went, for a consumer to move its own highlight.
    Removed {
        id: SessionId,
        before: Vec<View>,
        after: Vec<View>,
    },
}

/// One wait during shutdown, and how long shutdown waits past its deadline
/// for the discovery pass to end.
pub const TICK: Duration = Duration::from_millis(20);

/// How long shutdown waits past its deadline for the supervisor's report
/// (owner decision on #455). The supervisor's last round ends at the
/// deadline, but its thread must still be scheduled to hand the report
/// over, which a loaded host can delay well past one tick; without the
/// report, every session's evidence is lost to "client supervisor
/// unfinished". The supervisor may also have read the shutdown command up
/// to one of its own ticks late, which only shortens its shutdown, never
/// moves its end. The grace costs time only while the report is late.
const HANDOVER: Duration = Duration::from_millis(200);

/// The monitor: the rows it tracks, each session's terminal, the selection
/// and the toasts.
pub struct Monitor {
    registry: Registry,
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
    /// The one vte parser every terminal's model is fed through (#166),
    /// counted once beside their footprints at startup.
    parser: Parser,
    /// The paste under way and the target it goes to, while that target
    /// takes input; none while none is, or once its target changed.
    pasting: Option<Pasting>,
    /// The acknowledgement an explicit action deferred until its target is
    /// viewed (#114).
    acknowledging: Option<Deferred>,
    /// The attention jump's notice that no session needs attention.
    notice: Notice,
    /// The attention state the host last heard.
    host_report: HostReport,
    toasts: Toasts,
    /// Whether a toast rings the bell as it begins (`monitor.bell`).
    bell: bool,
    /// Client reports waiting to be applied, in fair turns within the work
    /// quanta (#119).
    work: cadence::Work<SessionId, ClientEvent>,
    /// The memory budgets history is kept within (#119).
    memory: MemoryLimits,
    /// When each session was last inspected: painted selected.
    inspected: BTreeMap<SessionId, Instant>,
    /// What its consumer has yet to deliver.
    events: Vec<Event>,
    /// The shutdown under way.
    leaving: Option<Leaving>,
}

/// A session under the authority it takes input with.
pub type Target = (SessionId, Authority);

/// A paste going to its target.
struct Pasting {
    target: Target,
    /// The application had bracketed paste on when the paste began.
    bracketed: bool,
}

/// The target taking application input now, its modes, whether its
/// alternate screen is active and the columns and rows of its canvas that
/// its consumer shows.
pub struct Taking {
    pub target: Target,
    pub modes: Modes,
    pub alternate: bool,
    pub cols: u16,
    pub rows: u16,
}

/// A shutdown under way.
struct Leaving {
    /// The one deadline nothing restarts.
    deadline: Instant,
    /// The sessions the supervisor last said it waits for.
    waiting: Vec<SessionId>,
    report: Option<shutdown::Report>,
}

impl Monitor {
    /// Starts discovery and the client supervisor.
    pub fn start(inputs: Inputs) -> io::Result<Self> {
        let Inputs {
            home,
            load,
            config,
            own,
            filter,
            dispatch,
        } = inputs;
        let prefix = config.escape_prefix();
        let hints = config.settings().show_key_hints;
        let jump = hints.then(|| format!("{prefix} then j: jump to attention"));
        let supervisor = Supervisor::start(context(&home, load, &config, filter)?, dispatch)?;
        let limits = &config.settings().limits;
        let discovery = Discovery::start(home, limits.discovery())?;
        let model = limits.model();
        let start = Duration::from_millis(limits.library().start_ms.into());
        Ok(Self {
            registry: Registry::new(own),
            discovery,
            supervisor,
            selected: None,
            initial: Initial::new(start),
            terminals: BTreeMap::new(),
            model: Limits {
                bytes: usize::try_from(model.bytes).unwrap_or(usize::MAX),
                history_rows: model.history_rows as usize,
            },
            parser: Parser::new(),
            pasting: None,
            acknowledging: None,
            notice: Notice::default(),
            host_report: HostReport::default(),
            toasts: Toasts::new(jump),
            bell: config.settings().bell,
            work: work(limits.cadence()),
            memory: limits.memory(),
            inspected: BTreeMap::new(),
            events: Vec::new(),
            leaving: None,
        })
    }

    /// One step: applies what discovery and the supervisor reported since
    /// the last one, keeps history within its budgets, and cuts a paste
    /// whose target changed (spec #73, "Target changes"). Returns the
    /// events since the last step.
    pub fn step(&mut self, clock: &dyn Clock, consumer: &dyn Consumer) -> Vec<Event> {
        self.collect(clock, consumer);
        self.relieve();
        let current = self.taking(consumer).map(|taking| taking.target);
        self.cut(current.as_ref());
        self.events()
    }

    /// Does what its consumer asks at `now`, and returns the events that
    /// brought.
    pub fn act(&mut self, action: Action, now: Instant, consumer: &dyn Consumer) -> Vec<Event> {
        match action {
            Action::Choose(choice) => self.choose_row(choice, now, consumer),
            Action::Drive(drive) => self.drive(drive, consumer),
            Action::Look(look) => self.look(look, now),
        }
        self.events()
    }

    /// Selects a row, jumps to attention, or deletes a row.
    fn choose_row(&mut self, choice: Choice, now: Instant, consumer: &dyn Consumer) {
        match choice {
            Choice::Select(id) => self.enter(id, now, consumer),
            Choice::Jump => self.jump(now, consumer),
            Choice::Delete(id) => self.delete(&id),
        }
    }

    /// Takes control of the selected session, sends its application input
    /// or a paste, or keeps its size.
    fn drive(&mut self, drive: Drive, consumer: &dyn Consumer) {
        match drive {
            Drive::TakeControl => self.take_control(consumer),
            Drive::Input(bytes) => self.submit(bytes, consumer),
            Drive::Paste(part) => self.paste(part, consumer),
            Drive::Resize => self.resize(consumer),
        }
    }

    /// Scrolls the selected session's history, dismisses the shown toast at
    /// `now`, or defers the selected session's acknowledgement.
    fn look(&mut self, look: Look, now: Instant) {
        match look {
            Look::Scroll(rows) => self.scroll(rows),
            Look::Dismiss => self.toasts.dismiss(now),
            Look::Acknowledge => self.defer_acknowledgement(),
        }
    }

    /// What its consumer has yet to deliver, taken.
    fn events(&mut self) -> Vec<Event> {
        mem::take(&mut self.events)
    }

    /// Before its consumer paints a frame at `now`: the toasts advance, and
    /// each terminal learns whether this frame shows its applied
    /// restoration, so its control and input readiness are in force once
    /// this frame comes due, even when its draw waits (#419). While a
    /// consumer's modal is open, every pending attention episode that was
    /// never toasted is eligible (spec #73, "Toast eligibility").
    pub fn frame(&mut self, now: Instant, consumer: &dyn Consumer) {
        if let Some(id) = self.selected.clone() {
            self.inspected.insert(id, now);
        }
        self.menus(consumer);
        self.toasts.advance(now, |toast| consumer.fits(toast));
        self.mark_shown(consumer);
        self.synchronise(consumer);
    }

    /// The shown toast, by when it began, while the frame its consumer
    /// paints at `now` shows its full title for the first time: once that
    /// frame is out, its consumer says so ([`Monitor::titled`]).
    pub fn titling(&self, now: Instant, consumer: &dyn Consumer) -> Option<Instant> {
        let shown = self.toasts.shown();
        let placed = shown.is_some_and(|toast| consumer.fits(toast));
        self.toasts.titling(now, placed)
    }

    /// While a menu is open, every pending attention episode that was
    /// never toasted is eligible (spec #73, "Toast eligibility"): one first
    /// seen while viewed toasts as a menu opens, and the toasts' ledger
    /// keeps any from toasting twice.
    fn menus(&mut self, consumer: &dyn Consumer) {
        if consumer.covered() {
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
    /// `toast` has been written to its consumer's device, at `now`, the
    /// toast's hold starts and, while `monitor.bell` is on, its one bell
    /// rings, unless it was dismissed first (owner decision on #115,
    /// 2026-11-17; docs/for-humans/protocol/monitor-host/bell.md).
    pub fn titled(&mut self, toast: Instant, now: Instant) -> Vec<Event> {
        if self.toasts.titled(toast, now) && self.bell {
            self.events.push(Event::Bell);
        }
        self.events()
    }

    /// Once a frame is painted (spec #73, "Viewing, focus, acknowledgement
    /// and host signals"; #114): each daemon hears whether the monitor
    /// views its session, a deferred acknowledgement is submitted once its
    /// target is viewed, after that viewing was submitted (Set viewing is
    /// coalesced, so it can lag the frame: owner decision on #114), and the
    /// host hears whether any connected session needs attention.
    pub fn painted(&mut self, consumer: &dyn Consumer) -> Vec<Event> {
        self.reacknowledge();
        let viewed = self.viewed(consumer).cloned();
        for (id, terminal) in &mut self.terminals {
            if let Some(request) = terminal.viewing(viewed.as_ref() == Some(id)) {
                self.supervisor.request(id.clone(), request);
            }
        }
        let reported = viewed.and_then(|id| self.terminals.get(&id));
        let viewing = reported.is_some_and(Terminal::viewing_reported);
        self.acknowledge(viewing, consumer);
        let pending = self.registry.attention().is_some();
        if let Some(pending) = self.host_report.report(pending) {
            self.events.push(Event::Attention(pending));
        }
        self.events()
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
    /// its view is live, its consumer's window has focus (assumed until it
    /// reports otherwise) and no toast covers its canvas.
    fn viewed(&self, consumer: &dyn Consumer) -> Option<&SessionId> {
        let (id, terminal, _) = self.target(consumer)?;
        let live = terminal.view().is_some_and(|view| view.scrolled() == 0);
        let covered = self.obscured(id, consumer);
        (live && consumer.focused() && !covered).then_some(id)
    }

    /// Whether the shown toast covers `id`'s canvas, as its consumer
    /// places both.
    fn obscured(&self, id: &SessionId, consumer: &dyn Consumer) -> bool {
        let terminal = self.terminals.get(id);
        let geometry = terminal.and_then(Terminal::geometry);
        let geometry = geometry.map(|geometry| (geometry.cols(), geometry.rows()));
        let shown = self.toasts.shown();
        shown.is_some_and(|toast| consumer.obscures(toast, geometry))
    }

    /// The attention jump and Enter dismiss a toast covering their
    /// target's canvas (owner decision on #114).
    fn dismiss_toast(&mut self, id: &SessionId, now: Instant, consumer: &dyn Consumer) {
        if self.obscured(id, consumer) {
            self.toasts.dismiss(now);
        }
    }

    /// Advances a deferred acknowledgement against the selected target as
    /// this frame left it: submitted once `viewing`, kept while not, dropped
    /// once the target context changed.
    fn acknowledge(&mut self, viewing: bool, consumer: &dyn Consumer) {
        let Some(deferred) = self.acknowledging.take() else {
            return;
        };
        match deferred.next(&self.attended(viewing, consumer)) {
            Next::Wait(deferred) => self.acknowledging = Some(deferred),
            Next::Submit(id, authority) => {
                self.supervisor.request(id, Request::Acknowledge(authority));
            }
            Next::Cancel => {}
        }
    }

    /// The selected target as the frame just painted left it.
    fn attended(&self, viewing: bool, consumer: &dyn Consumer) -> attention::Target<'_> {
        let id = self.selected.as_ref();
        let terminal = id.and_then(|id| self.terminals.get(id));
        attention::Target {
            id,
            covered: consumer.covered(),
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
    /// session is focused while no modal covers it and its consumer's
    /// window has focus; every other is not. Each ready controller with
    /// `focus-v1` is told of each change once, a newly ready one of the
    /// current desire.
    fn synchronise(&mut self, consumer: &dyn Consumer) {
        let focused = consumer.focused() && !consumer.covered();
        for (id, terminal) in &mut self.terminals {
            let desired = focused && self.selected.as_ref() == Some(id);
            let owed = terminal.paste_end();
            for request in owed.into_iter().chain(terminal.focus(desired)) {
                self.supervisor.request(id.clone(), request);
            }
        }
    }

    /// Only the [`Monitor::shown`] terminal is shown.
    fn mark_shown(&mut self, consumer: &dyn Consumer) {
        let shown = self.shown(consumer).cloned();
        for (id, terminal) in &mut self.terminals {
            terminal.painted(shown.as_ref() == Some(id));
        }
    }

    /// The selected session while its consumer shows its canvas: nothing
    /// covers it and the consumer has room for it.
    fn shown(&self, consumer: &dyn Consumer) -> Option<&SessionId> {
        let shows = !consumer.covered() && consumer.size().is_some();
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

    /// Resize goes only to the selected controller, and only while its
    /// consumer has room for a session.
    fn resize(&mut self, consumer: &dyn Consumer) {
        let Some(desire) = consumer.size() else {
            return;
        };
        let Some(id) = self.selected.clone() else {
            return;
        };
        let request = self.terminal(&id).resize(desire);
        self.request(&id, request);
    }

    /// Scrolls the selected session's history by `rows`.
    fn scroll(&mut self, rows: isize) {
        if let Some(id) = self.selected.clone() {
            self.terminal(&id).scroll(rows);
        }
    }

    /// Enter acts on the row's condition when pressed (spec #73, "Enter"):
    /// a connected row not known to be final is selected and acquired; a
    /// Starting or Unavailable row with no connected client, and a
    /// disconnected or final row, are selected without acquiring, read-only
    /// (#116). This monitor's own session has no row (owner decision on
    /// #110).
    /// Enter on a connected row also dismisses a toast covering it and
    /// acknowledges its attention once viewed (#114).
    fn enter(&mut self, id: SessionId, now: Instant, consumer: &dyn Consumer) {
        match self.registry.kind(&id) {
            Some(Kind::Connected) => {
                self.dismiss_toast(&id, now, consumer);
                self.choose(id.clone());
                self.acquire(&id, consumer);
                self.defer_acknowledgement();
            }
            Some(Kind::Unconnected | Kind::Retained) => self.choose(id),
            None => {}
        }
    }

    /// Selects `id`, which its consumer hears. A session selected before
    /// keeps any control in the background, and nothing is selected
    /// automatically any more.
    fn choose(&mut self, id: SessionId) {
        self.events.push(Event::Selected);
        self.initial.selected();
        self.selected = Some(id);
    }

    /// Initial selection (spec #73): once collection has ended, the first
    /// eligible row is selected and acquired, at most once and never over
    /// an existing selection. Any open modal stays open over it.
    fn initialise(&mut self, now: Instant, consumer: &dyn Consumer) {
        if let Some(id) = self.initial.choose(&self.registry, now) {
            self.selected = Some(id.clone());
            self.acquire(&id, consumer);
        }
    }

    /// Takes control of `id` explicitly, then keeps the selected
    /// controller's size.
    fn acquire(&mut self, id: &SessionId, consumer: &dyn Consumer) {
        let request = self.terminal(id).acquire();
        self.request(id, Some(request));
        self.resize(consumer);
    }

    /// Taking control acquires the selected session while it shows the
    /// non-controller placeholder; the monitor never takes control back by
    /// itself.
    fn take_control(&mut self, consumer: &dyn Consumer) {
        let revoked = self.selected.clone().filter(|id| {
            self.registry
                .kind(id)
                .is_some_and(|kind| placeholder(kind, self.control(id)))
        });
        if let Some(id) = revoked {
            self.acquire(&id, consumer);
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
    fn present(&mut self, id: &SessionId, presentation: Presentation, consumer: &dyn Consumer) {
        let terminal = terminal(&mut self.terminals, self.model, id);
        let ready = terminal.present(&mut self.parser, presentation);
        if let Some(why) = terminal.take_rejection() {
            self.supervisor.drop_client(id.clone(), why);
        }
        self.request(id, ready);
        self.resize(consumer);
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

    /// Applies what discovery reported since the last step, asking for a
    /// client for each session that now needs one, then client reports in
    /// fair turns: each client's at most `ui.client_quantum_ms` at a time,
    /// all at most `ui.background_quantum_ms` this step (#119). The rest
    /// waits for the next step, still charged to its client, which is not
    /// drained meanwhile once its reports fill its allowance.
    fn collect(&mut self, clock: &dyn Clock, consumer: &dyn Consumer) {
        let now = clock.wall();
        self.discover(now, clock);
        while let Ok((id, event)) = self.supervisor.reports.try_recv() {
            self.work.push(id, event);
        }
        self.work.begin(clock.now());
        while let Some((id, event)) = self.work.next(clock.now()) {
            let bytes = supervisor::footprint(&id, &event);
            self.report(&id, event, now, consumer);
            self.supervisor.processed(&id, bytes);
        }
        self.initialise(clock.now(), consumer);
    }

    /// What discovery found since the last step: batches of sessions, and
    /// the end of each pass.
    fn discover(&mut self, now: SystemTime, clock: &dyn Clock) {
        while let Ok(found) = self.discovery.found.try_recv() {
            match found {
                Found::Batch(batch) => self.batch(batch, now, clock),
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
    fn report(
        &mut self,
        id: &SessionId,
        event: ClientEvent,
        now: SystemTime,
        consumer: &dyn Consumer,
    ) {
        match event {
            ClientEvent::Presentation(presentation) => self.present(id, presentation, consumer),
            ClientEvent::Removed => self.dismiss(id),
            event => self.client(id, event, now, consumer),
        }
    }

    /// Any other client event goes to the session's row, and the command's
    /// final result and the end of the client to its terminal too. What
    /// the row announces goes to the toasts, with whether the monitor
    /// viewed the session before the event changed it.
    fn client(
        &mut self,
        id: &SessionId,
        event: ClientEvent,
        now: SystemTime,
        consumer: &dyn Consumer,
    ) {
        let viewed = self.viewed(consumer) == Some(id);
        if let Some(presentation) = ending(&event) {
            self.present(id, presentation, consumer);
        }
        if let Some(news) = self.registry.client(id, event, now) {
            self.announce(id, news, viewed);
        }
    }

    /// A confirmed Delete is done (#117): the row is dismissed for the run
    /// and its retained cache discarded. A selected row's selection falls
    /// back to the row now at its place, else the one before it, else none,
    /// and its consumer hears of the removal to move its own highlight;
    /// neither acquires anything.
    fn dismiss(&mut self, id: &SessionId) {
        let before = self.registry.rows();
        self.registry.dismiss(id);
        self.terminals.remove(id);
        let after = self.registry.rows();
        if self.selected.as_ref() == Some(id) {
            self.selected = delete::fallback(id, &ids(&before), &ids(&after));
        }
        self.events.push(Event::Removed {
            id: id.clone(),
            before,
            after,
        });
    }

    fn batch(&mut self, batch: Batch, now: SystemTime, clock: &dyn Clock) {
        for invalid in &batch.invalid {
            tracing::warn!(path = ?invalid.path, reason = ?invalid.reason, "invalid session metadata");
        }
        for outcome in batch.sessions {
            self.discovered(outcome, now, clock);
        }
    }

    fn discovered(&mut self, outcome: Outcome, now: SystemTime, clock: &dyn Clock) {
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
                .started(metadata.session_id.clone(), clock.now());
            self.supervisor.start_client(metadata);
        }
    }

    /// A paste is data for the target that took input when it began, sent
    /// with that target's markers while it still takes input; once it does
    /// not, the paste is [cut](Monitor::cut) (spec #73, "Paste rules",
    /// "Target changes"). A consumer's modal owns a paste it covers: one
    /// covered part-way is cut.
    fn paste(&mut self, part: Part, consumer: &dyn Consumer) {
        if consumer.covered() {
            return self.cut(None);
        }
        let taking = self.taking(consumer);
        if part.starts {
            self.pasting = taking.as_ref().map(|taking| Pasting {
                target: taking.target.clone(),
                bracketed: taking.modes.bracketed_paste,
            });
        }
        let current = taking.map(|taking| taking.target);
        self.cut(current.as_ref());
        let ends = part.ends;
        if let Some(Pasting { target, bracketed }) = &self.pasting {
            terminal(&mut self.terminals, self.model, &target.0).return_live();
            let part = Paste {
                bracketed: *bracketed,
                starts: part.starts,
                bytes: part.bytes,
                ends,
            };
            self.supervisor
                .request(target.0.clone(), Request::Paste(target.1, part));
        }
        self.pasting = self.pasting.take().filter(|_| !ends);
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
            bracketed,
        }) = cut
            && bracketed
        {
            self.terminal(&id).cut_paste(authority);
        }
    }

    /// The target taking application input, its modes and its canvas's
    /// area: its geometry, which its consumer shows as far as it has room,
    /// so no cell beyond it is reported. There is one only while no modal
    /// is open over the selected session, its consumer has room for it,
    /// and its control period's restoration is applied, ready and painted.
    pub fn taking(&self, consumer: &dyn Consumer) -> Option<Taking> {
        let (id, terminal, authority) = self.target(consumer)?;
        let geometry = terminal.geometry();
        terminal
            .view()
            .zip(geometry)
            .map(|(view, geometry)| Taking {
                target: (id.clone(), authority),
                modes: view.modes(),
                alternate: matches!(view.screen(), vt::Screen::Alternate { .. }),
                cols: geometry.cols(),
                rows: geometry.rows(),
            })
    }

    /// The [`Monitor::shown`] session, its terminal and the authority it
    /// takes input with, while it may take input. `Terminal::input` alone
    /// learns of a modal or a small consumer only at the next frame; within
    /// one read of its device, input after a modal opened must already
    /// reach the modal.
    fn target(&self, consumer: &dyn Consumer) -> Option<(&SessionId, &Terminal, Authority)> {
        let id = self.shown(consumer)?;
        let terminal = self.terminals.get(id)?;
        terminal.input().map(|authority| (id, terminal, authority))
    }

    /// Submits application `bytes` to the target taking input now, under
    /// its current authority; without one they are discarded.
    fn submit(&mut self, bytes: Vec<u8>, consumer: &dyn Consumer) {
        if bytes.is_empty() {
            return;
        }
        let target = self
            .target(consumer)
            .map(|(id, _, authority)| (id.clone(), authority));
        if let Some((id, authority)) = target {
            self.terminal(&id).return_live();
            self.request(&id, Some(Request::Input(authority, bytes)));
        }
    }

    /// The attention jump (spec #73, "Attention jump target"; #114): the
    /// connected session that needs attention with the newest daemon
    /// change at the action. The jump dismisses a toast covering it,
    /// selects it, takes control only while the monitor neither holds nor
    /// is acquiring it, shows its live view and acknowledges its attention
    /// once viewed. With none, the selection stays and `No sessions need
    /// attention` shows for a while.
    fn jump(&mut self, now: Instant, consumer: &dyn Consumer) {
        let Some(id) = self.registry.attention() else {
            self.notice.show(now);
            return;
        };
        self.notice.clear();
        self.dismiss_toast(&id, now, consumer);
        self.choose(id.clone());
        if self.control(&id) == Control::NotControlling {
            self.acquire(&id, consumer);
        }
        self.resize(consumer);
        self.terminal(&id).return_live();
        self.defer_acknowledgement();
    }

    /// A confirmed Delete submits what the row offers now, its row showing
    /// the Delete under way; a row that offers nothing sends nothing.
    fn delete(&mut self, id: &SessionId) {
        if let Some((metadata, offer)) = self.registry.confirm(id) {
            let kill = offer == Offer::TerminateAndRemove;
            self.supervisor.remove(metadata, kill);
        }
    }

    /// The rows, in the order a selector lists them.
    pub fn rows(&self) -> Vec<View> {
        self.registry.rows()
    }

    /// The selected session.
    pub fn selected(&self) -> Option<&SessionId> {
        self.selected.as_ref()
    }

    /// The authority the selected session's control period and restoration
    /// hold, if any: a change is a target change.
    pub fn authority(&self) -> Option<Authority> {
        let id = self.selected.as_ref();
        id.and_then(|id| self.terminals.get(id))
            .and_then(Terminal::authority)
    }

    /// The selected session as `rows` list it now, with what its canvas
    /// shows.
    pub fn session<'a>(&'a self, rows: &'a [View]) -> Option<Session<'a>> {
        let row = self
            .selected
            .as_ref()
            .and_then(|id| rows.iter().find(|row| &row.id == id))?;
        let shown = shown(&self.registry, &row.id)?;
        Some(selected(row, shown, self.terminals.get(&row.id)))
    }

    /// The selected session's read-only history, while no modal of its
    /// consumer covers it (#116).
    pub fn history(&self, consumer: &dyn Consumer) -> Option<vt::View<'_>> {
        let id = self.selected.as_ref().filter(|_| !consumer.covered())?;
        let shown = shown(&self.registry, id)?;
        retained(shown.kind, shown.finished, self.terminals.get(id))
    }

    /// Whether `id` has a row.
    pub fn listed(&self, id: &SessionId) -> bool {
        self.registry.kind(id).is_some()
    }

    /// What a Delete of `id`'s row would do now, unless one is already
    /// under way or it has no row.
    pub fn offer(&self, id: &SessionId) -> Option<Offer> {
        self.registry.offer(id)
    }

    /// Why the latest discovery pass was incomplete, if it was.
    pub fn incomplete(&self) -> Option<&str> {
        self.registry.incomplete()
    }

    /// The startup progress while the initial collection lasts (#266).
    pub fn progress(&self) -> Option<String> {
        let progress = self.initial.progress(&self.registry);
        progress.map(|progress| progress.to_string())
    }

    /// Whether the initial collection ended with sessions still starting.
    pub fn late(&self) -> bool {
        self.initial.late(&self.registry)
    }

    /// The attention jump's notice that no session needs attention, while
    /// it shows at `now`.
    pub fn notice(&self, now: Instant) -> Option<&'static str> {
        self.notice.text(now)
    }

    /// The toasts, for a consumer to place and draw the shown one.
    pub fn toasts(&self) -> &Toasts {
        &self.toasts
    }

    /// Begins the coordinated shutdown (#118): acquisition, discovery and
    /// retries stop at once, and the supervisor shuts every client down
    /// concurrently, until one deadline that nothing restarts.
    pub fn shut_down(&mut self) {
        let deadline = self.supervisor.shut_down();
        self.discovery.stop();
        self.leaving = Some(Leaving {
            deadline,
            waiting: Vec::new(),
            report: None,
        });
    }

    /// One round of a shutdown under way, at `now`: how many sessions it
    /// still waits for, or none once it is over. Its consumer waits a
    /// moment its own way, then says so ([`Monitor::waited`]).
    pub fn waiting(&mut self, now: Instant) -> Option<usize> {
        let discovered = self.discovery.join();
        let leaving = self.leaving.as_mut()?;
        if !waits(leaving.deadline, leaving.report.is_some(), discovered, now) {
            return None;
        }
        if let Some(waiting) = self.supervisor.waiting() {
            leaving.waiting = waiting;
        }
        Some(leaving.waiting.len())
    }

    /// Its consumer waited one round of a shutdown: the supervisor's
    /// report is taken once it is there.
    pub fn waited(&mut self) {
        if let Some(leaving) = self.leaving.as_mut()
            && leaving.report.is_none()
        {
            leaving.report = self.supervisor.report();
        }
    }

    /// What the shutdown proved, discovery's end included, logged: a
    /// monitor that never began one proved nothing of its clients.
    pub fn left(&mut self) -> Verdict {
        let leaving = self.leaving.take();
        let (waiting, report) = leaving.map_or_else(Default::default, |leaving| {
            (leaving.waiting, leaving.report)
        });
        let mut report = report.unwrap_or_else(|| unfinished(waiting));
        if !self.discovery.join() {
            report.unfinished.push("discovery pass".to_owned());
        }
        report.log();
        report.verdict()
    }
}

/// The session IDs of `rows`, in order.
fn ids(rows: &[View]) -> Vec<&SessionId> {
    rows.iter().map(|row| &row.id).collect()
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

/// Whether shutdown, past `deadline` or not, still waits at `now`: for the
/// supervisor's report until the handover grace has passed, and for the
/// discovery pass until a tick has, so a pass that never ends costs no
/// grace when the report came in time.
fn waits(deadline: Instant, reported: bool, discovered: bool, now: Instant) -> bool {
    let until = |grace: Duration| (deadline + grace).checked_duration_since(now).is_some();
    (!reported && until(HANDOVER)) || (!discovered && until(TICK))
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
fn context(
    home: &HomeLayout,
    load: Load,
    config: &MonitorConfig,
    filter: String,
) -> io::Result<Context> {
    let limits = &config.settings().limits;
    let runtime = climon2_platform::runtime::RuntimeDirectory::prepare(home.root())?;
    let lanes = limits.lanes();
    Ok(Context {
        load,
        home: home.clone(),
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
    crate::hooks::identifiers(supervisor::Identifiers::default())
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
    use super::*;

    /// EXT-02 (pure supplement, #455): past the shutdown deadline, shutdown
    /// waits the 200 ms handover grace only while the supervisor's report
    /// is missing; a discovery pass that never ends is waited for only a
    /// tick, as before, so with the report in time it costs no grace.
    #[test]
    fn only_a_missing_report_is_waited_for_past_a_tick() {
        let now = Instant::now();
        let waits = |deadline, reported, discovered| waits(deadline, reported, discovered, now);
        let ticked = now - std::time::Duration::from_millis(60);
        assert!(waits(ticked, false, true));
        assert!(waits(ticked, false, false));
        assert!(!waits(ticked, true, false));
        assert!(!waits(ticked, true, true));
        let later = now + std::time::Duration::from_secs(60);
        assert!(waits(later, true, false));
        assert!(!waits(later, true, true));
        let graced = now - std::time::Duration::from_secs(1);
        assert!(!waits(graced, false, false));
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
}
