//! The monitor's terminal interface (spec #73; ADR 0011): one terminal that
//! shows every live session the monitor library tracks. It validates its
//! start before touching the host terminal, then owns the host with one
//! permanent top row, steps one [`Monitor`] on its one thread, and lists
//! the sessions in the selector on prefix+s, under the key introduction
//! when it is due, until prefix+x or a catchable termination signal. It
//! decodes the host's input into the library's actions, paints what the
//! library offers, and delivers the library's events to the host.

mod attention;
mod delete;
mod failstop;
mod frames;
mod host;
mod introduction;
mod keys;
mod mouse;
mod paint;
#[cfg(unix)]
mod pair;
#[cfg(windows)]
#[path = "pair/windows.rs"]
mod pair;
mod preference;
mod router;
mod screen;
mod selector;
mod toast;

use climon2_core::{SessionId, exit::CLIMON2_ERROR, monitor::MonitorArgs, protocol::TerminalSize};
use climon2_monitor::{
    self as library, Choice, Consumer, Drive, Inputs, Look, Monitor, Startup, System, TICK, Target,
    Verdict, View,
    toast::Toast,
    view::{self as vt, Modes, MouseReporting},
};
use climon2_platform::terminal;
use delete::{Asking, Confirmation, Decision};
use frames::Frames;
use host::{Ending, Event, Host, Output};
use introduction::KeyIntroduction;
use keys::Action;
use mouse::{Area, Gestures, Mouse, Wheel};
use preference::Preference;
use router::{Begin, Key, Pasted, Routed, Router};
use screen::{Scene, Screen};
use selector::Selector;
use std::{
    ffi::OsString,
    io, mem,
    path::Path,
    thread,
    time::{Duration, Instant},
};

/// A one-line diagnostic that the thin main prints, after the host is
/// restored, with the climon2 error code.
pub use climon2_monitor::Error;

/// The CLI execs the monitor with the arguments `c2 monitor` was given,
/// argv[0] excluded: none starts the monitor, and `pair <host:port>` pairs
/// with a relay. Arguments that are neither are a usage error.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<u32, Error> {
    let arguments =
        MonitorArgs::parse(arguments).map_err(|usage| Error::new(usage.exit_code(), usage))?;
    match arguments {
        MonitorArgs::Start => start(),
        MonitorArgs::Pair { address } => pair::run(&address),
    }
}

/// The monitor itself. Returns the exit status of an
/// orderly exit: 0 for prefix+x, 128 + signal for a termination signal,
/// never a session's own status. An exit whose cleanup cannot be confirmed
/// never returns: it fail-stops the process with the climon2 error code
/// (#118). Everything that can refuse it is checked before the host
/// terminal changes: terminals first, then what the monitor library
/// checks. The key introduction's preference is read last; it never
/// refuses the monitor.
fn start() -> Result<u32, Error> {
    terminal::require_ttys()
        .map_err(|_| Error::new(CLIMON2_ERROR, "c2 monitor needs a terminal"))?;
    let startup = Startup::read()?;
    let preference = Preference::read(&startup.home);
    let log = startup.log.path().to_owned();
    let filter = std::env::var("CLIMON2_LOG").ok();
    let dispatch = startup
        .log
        .dispatch(filter.as_deref())
        .map_err(|error| Error::from(error).logged(&log))?;
    let filter = filter.unwrap_or_else(|| DEFAULT_FILTER.to_owned());
    let started = Started {
        startup,
        preference,
        dispatch,
        filter,
    };
    let dispatch = started.dispatch.clone();
    tracing::dispatcher::with_default(&dispatch, || monitor(started))
        .map_err(|error| Error::from(error).logged(&log))
}

/// The host bell a toast rings once the frame first showing its full title
/// has been written (docs/for-humans/protocol/monitor-host/bell.md).
const BEL: &[u8] = b"\x07";
/// The library log filter when `CLIMON2_LOG` sets none.
const DEFAULT_FILTER: &str = "info";

/// A validated start, the key introduction's preference, where the
/// monitor's threads log, and the log filter every client library is given.
struct Started {
    startup: Startup,
    preference: Preference,
    dispatch: tracing::Dispatch,
    filter: String,
}

/// Records how the monitor ended: the host terminal may be gone, so a failure
/// is logged as well as returned for main to print.
fn monitor(started: Started) -> io::Result<u32> {
    tracing::info!(version = %started.startup.version, "monitor started");
    tracing::info!(effective = %started.startup.config, "monitor configuration");
    if let Some(problem) = started.preference.problem() {
        tracing::warn!(%problem, "monitor preference unusable");
    }
    let status = owned(started).inspect_err(|error| tracing::error!(%error, "monitor failed"))?;
    tracing::info!(status, "monitor stopped");
    Ok(status)
}

/// Owns the host until an ending, shuts every client down, and releases the
/// host before any diagnostic, all within `deadlines.final_host_ms` of the
/// shutdown's end (#118). A monitor that started told the host its
/// attention state, so the host hears the root clear once released (#114),
/// even after a failure.
fn owned(started: Started) -> io::Result<u32> {
    let lanes = started.startup.config.settings().limits.lanes();
    let final_host = Duration::from_millis(lanes.final_host_ms.into());
    let log = started.startup.log.path().to_owned();
    let dispatch = started.dispatch.clone();
    let (mut host, size) = Host::enter()?;
    let interface = Interface::start(size, started);
    let reported = interface.is_ok();
    let (served, verdict, output) = match interface {
        Ok(interface) => interface.serve(&mut host),
        Err(error) => (Err(error), Verdict::Clean, Output::default()),
    };
    let watchdog = failstop::Watchdog::arm(final_host, dispatch);
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

/// What the terminal interface shows on the host over the monitor, and
/// what it reports to the library: whether a modal covers the selected
/// session, whether the host has focus, the size it can show a session at,
/// and where a toast would sit.
struct Device {
    screen: Screen,
    introduction: KeyIntroduction,
    selector: Selector,
    /// The Delete confirmation, while it is open over the selector (#117).
    deleting: Option<Confirmation>,
    /// Whether the host has focus: assumed until it reports otherwise, since
    /// a host that never reports focus cannot be detected (owner decision on
    /// #111).
    host_focused: bool,
}

impl Consumer for Device {
    /// Whether the selector or the key introduction covers the canvas.
    fn covered(&self) -> bool {
        screen::covered(&self.selector, &self.introduction)
    }

    fn focused(&self) -> bool {
        self.host_focused
    }

    fn size(&self) -> Option<TerminalSize> {
        self.screen.desire()
    }

    fn fits(&self, toast: &Toast) -> bool {
        toast::place(toast, self.screen.overlay()).is_some()
    }

    /// The canvas takes the host cells of its geometry below the top row.
    fn obscures(&self, toast: &Toast, geometry: Option<(u16, u16)>) -> bool {
        let area = self.screen.overlay();
        toast::covers(toast, toast::canvas(area, geometry), area)
    }
}

/// How the user of this interface reaches each action of the monitor
/// library (ADR 0011). The match is exhaustive and only compiled, never
/// run, so an action the library adds fails this build until the
/// interface says how its user takes it.
const _: fn(&library::Action) = |action| match action {
    // Enter on a selector row.
    library::Action::Choose(Choice::Select(_))
    // Prefix+j.
    | library::Action::Choose(Choice::Jump)
    // Remove in the Delete confirmation.
    | library::Action::Choose(Choice::Delete(_))
    // The take-control key on the non-controller placeholder.
    | library::Action::Drive(Drive::TakeControl)
    // Keys typed while the selected session takes input.
    | library::Action::Drive(Drive::Input(_))
    // A bracketed paste from the host.
    | library::Action::Drive(Drive::Paste(_))
    // A host resize.
    | library::Action::Drive(Drive::Resize)
    // A wheel notch over the selected session or its read-only history.
    | library::Action::Look(Look::Scroll(_))
    // A click on the toast.
    | library::Action::Look(Look::Dismiss)
    // Leaving the selector back to the selected session.
    | library::Action::Look(Look::Acknowledge) => {}
};

/// The terminal interface: the monitor it steps, what it shows, and the
/// keys and mouse reports that act on them.
struct Interface {
    monitor: Monitor,
    device: Device,
    router: Router,
    /// The take-control key's byte, if one is bound.
    take_control: Option<u8>,
    /// The mouse gesture under way and its owner.
    gestures: Gestures<Target>,
    /// What received application input after the last read; a change
    /// before the next read is a target change.
    context: InputContext,
    /// When the next frame may be painted: at most `ui.fps` a second.
    frames: Frames,
    /// What the host has been handed and not yet taken (#285).
    output: Output,
}

/// The selected session, whether a modal covers it, and the authority its
/// control period and restoration hold, if any: any change is a target
/// change (spec #73, "Target changes").
type InputContext = (Option<SessionId>, bool, Option<library::Authority>);

impl Interface {
    fn start(size: TerminalSize, started: Started) -> io::Result<Self> {
        let Started {
            startup,
            preference,
            dispatch,
            filter,
        } = started;
        let config = &startup.config;
        let prefix = config.escape_prefix();
        let take_control = config.take_control_key();
        let hints = config.settings().show_key_hints;
        let limits = &config.settings().limits;
        let search = usize::try_from(limits.search_bytes()).unwrap_or(usize::MAX);
        let fps = limits.cadence().fps;
        let monitor = Monitor::start(Inputs {
            load: library::load::loader(startup.executable),
            config: startup.config,
            home: startup.home,
            own: library::own_session(),
            filter,
            dispatch,
        })?;
        let mut interface = Self {
            monitor,
            device: Device {
                screen: Screen::new(size, prefix, take_control, hints),
                introduction: KeyIntroduction::new(prefix, preference, hints),
                selector: Selector::new(prefix, hints, search),
                deleting: None,
                host_focused: true,
            },
            router: Router::new(prefix),
            take_control: take_control.map(climon2_core::keys::ControlKey::byte),
            gestures: Gestures::default(),
            context: InputContext::default(),
            frames: Frames::new(fps),
            output: Output::default(),
        };
        interface.show().map(|()| interface)
    }

    /// Acts on host events until an ending, or a failure of the host or
    /// rendering, then shuts down: why it ended, what the shutdown proved,
    /// and what the host has yet to take. Consuming the interface drops its
    /// screen before the host is released.
    fn serve(mut self, host: &mut Host) -> (io::Result<Ending>, Verdict, Output) {
        let ending = self.events(host);
        let hosted = self.shut_down(host, ending.is_ok());
        let verdict = self.monitor.left();
        (
            ending.and_then(|ending| hosted.map(|()| ending)),
            verdict,
            self.output,
        )
    }

    /// The coordinated shutdown (#118): application input, acquisition,
    /// discovery and retries stop at once, and the library shuts every
    /// client down concurrently, until one deadline that nothing restarts.
    /// Meanwhile the exit screen counts the sessions it waits for, and host
    /// input is read and discarded: repeated exits and signals do nothing.
    /// A host that failed is left alone. Returns whether the host still
    /// worked.
    fn shut_down(&mut self, host: &mut Host, hosting: bool) -> io::Result<()> {
        self.monitor.shut_down();
        let mut hosted = Ok(());
        while let Some(waiting) = self.monitor.waiting(Instant::now()) {
            hosted = hosted.and_then(|()| self.exiting(host, hosting, waiting));
            self.monitor.waited();
        }
        hosted
    }

    /// One round of the exit screen: painted, then one wait for host
    /// input, whose resizes alone count; without a working host, one tick.
    fn exiting(&mut self, host: &mut Host, hosting: bool, waiting: usize) -> io::Result<()> {
        if !hosting {
            thread::sleep(TICK);
            return Ok(());
        }
        self.device.screen.exiting(waiting)?;
        self.pump(host)?;
        self.resized(host)
    }

    /// One wait for host input during exit: a resize resizes the exit
    /// screen, and anything else is ignored.
    fn resized(&mut self, host: &mut Host) -> io::Result<()> {
        if let Event::Resize(size) = host.next(&self.output)? {
            self.device.screen.resize(size);
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
    /// the library collected meanwhile, and writes what the host takes of
    /// the frame.
    fn step(&mut self, host: &mut Host) -> io::Result<Option<Ending>> {
        let event = host.next(&self.output)?;
        let read = matches!(event, Event::Input(_));
        let ending = self.handle(event)?;
        if ending.is_none() {
            self.turn(host, read)?;
        }
        Ok(ending)
    }

    /// The rest of a step that did not end the monitor: one step of the
    /// library, the frame, if one is due or `read` asks for it at once, and
    /// what the host takes of it.
    fn turn(&mut self, host: &mut Host, read: bool) -> io::Result<()> {
        let events = self.monitor.step(&System, &self.device);
        self.deliver(events);
        self.collected();
        self.retarget();
        self.frame(read)?;
        self.pump(host)
    }

    /// What follows the rows the library collected: a Delete confirmation
    /// whose row went closes, and the selector keeps its highlight on the
    /// rows as they are now.
    fn collected(&mut self) {
        let monitor = &self.monitor;
        self.device.deleting = self
            .device
            .deleting
            .take()
            .filter(|confirmation| monitor.listed(confirmation.target()));
        self.device.selector.update(&self.monitor.rows());
    }

    /// Delivers the library's `events` to the host and to what the
    /// interface shows: attention reports and the bell go to the host, a
    /// selection closes every modal over it, and a removed row moves the
    /// selector's highlight.
    fn deliver(&mut self, events: Vec<library::Event>) {
        for event in events {
            self.delivered(event);
        }
    }

    fn delivered(&mut self, event: library::Event) {
        match event {
            library::Event::Attention(pending) => self.output.queue(attention::report(pending)),
            library::Event::Bell => self.output.queue(BEL),
            library::Event::Selected => {
                self.device.introduction.close();
                self.device.selector.close();
            }
            library::Event::Removed { id, before, after } => {
                self.device.selector.dismissed(&id, &before, &after);
            }
        }
    }

    /// Asks the library for `action` and delivers what that brought.
    fn ask(&mut self, action: library::Action) {
        let events = self.monitor.act(action, Instant::now(), &self.device);
        self.deliver(events);
    }

    /// A target change since the last read discards a pending prefix (spec
    /// #73, "Target changes"); one the read itself made came before any
    /// prefix it left pending. An open paste or gesture learns of a change
    /// when its next part arrives.
    fn retarget(&mut self) {
        let context = self.context();
        if context != self.context {
            self.router.retarget();
        }
        self.context = context;
    }

    fn context(&self) -> InputContext {
        let id = self.monitor.selected().cloned();
        (id, self.device.covered(), self.monitor.authority())
    }

    fn handle(&mut self, event: Event<'_>) -> io::Result<Option<Ending>> {
        match event {
            Event::End(ending) => Ok(Some(ending)),
            Event::Resize(size) => {
                self.device.screen.resize(size);
                self.ask(library::Action::Drive(Drive::Resize));
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
    /// resize) are painted when they happen: their state (readiness,
    /// routing, Viewing and Acknowledge) is in force at once, though on a
    /// backed-up host their draw waits until the host has room (owner
    /// decision on #420, option A). Every frame counts toward the cadence,
    /// so output alone never paints faster than it.
    fn frame(&mut self, read: bool) -> io::Result<()> {
        if read || self.frames.due(Instant::now()) {
            return self.show();
        }
        Ok(())
    }

    /// Paints the current scene when it differs from what the screen shows.
    /// The library first learns of the frame, so each terminal's control
    /// and input readiness are in force once this frame comes due, even
    /// when its draw waits (#419). While the host has yet to take what it
    /// was handed, the frame is drawn only once it has: one drawn meanwhile
    /// would only be replaced, and a host that takes little at a time, as a
    /// macOS terminal does, would wait for a whole frame to be drawn for
    /// each write (#419). Once it is painted, the library learns that too.
    fn show(&mut self) -> io::Result<()> {
        let now = Instant::now();
        self.frames.painted(now);
        self.monitor.frame(now, &self.device);
        let titling = self.monitor.titling(now, &self.device);
        let rows = self.monitor.rows();
        let device = &mut self.device;
        let scene = Scene {
            introduction: &device.introduction,
            rows: &rows,
            selector: &device.selector,
            asking: asking(device.deleting.as_ref(), &self.monitor, &rows),
            incomplete: self.monitor.incomplete(),
            progress: self.monitor.progress(),
            late: self.monitor.late(),
            notice: self.monitor.notice(now),
            toasts: self.monitor.toasts(),
            now,
            session: self.monitor.session(&rows),
        };
        draw(&mut device.screen, &self.output, &scene, titling)?;
        let events = self.monitor.painted(&self.device);
        self.deliver(events);
        Ok(())
    }

    /// Writes what the host takes now, never waiting for it (#285). Each
    /// time it has taken everything it was handed, a toast whose full title
    /// that showed begins its hold, and the newest frame is handed on, so
    /// frames never queue.
    fn pump(&mut self, host: &mut Host) -> io::Result<()> {
        host.write(&mut self.output)?;
        while self.hand_on()? {
            host.write(&mut self.output)?;
        }
        Ok(())
    }

    /// Once the host has written everything it was handed: hands it the
    /// newest frame, drawn now if one waited for the host, and any bell, and
    /// says whether it handed anything.
    fn hand_on(&mut self) -> io::Result<bool> {
        if !self.output.idle() {
            return Ok(false);
        }
        // The waiting frame is drawn before any bell is queued, which would
        // leave the host with something to take again, and before the toast
        // is taken, so a failed draw loses neither.
        self.redraw()?;
        if let Some(toast) = self.device.screen.written() {
            let events = self.monitor.titled(toast, Instant::now());
            self.deliver(events);
        }
        if let Some(bytes) = self.device.screen.commit() {
            self.output.queue(&bytes);
        }
        Ok(!self.output.idle())
    }

    /// The frame that waited for the host to take what it was handed,
    /// drawn now that it has (#419).
    fn redraw(&mut self) -> io::Result<()> {
        if self.device.screen.undrawn() {
            return self.show();
        }
        Ok(())
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
                self.device.host_focused = focused;
                Ok(None)
            }
            keyed => self.keyed(keyed, typed),
        }
    }

    /// A paste is data for the target that took input when it began, which
    /// the library sends with that target's markers while it still takes
    /// input (spec #73, "Paste rules", "Target changes"). A modal owns a
    /// paste: the selector's search takes it (#113), the key introduction
    /// discards it; a paste a modal covers part-way is cut, and the search
    /// ignores the rest of it, which it never saw begin. A prefix the paste
    /// discarded reaches its target as one literal prefix first.
    fn paste(&mut self, pasted: Pasted, typed: &mut Vec<u8>) -> io::Result<()> {
        let starts = pasted.begin != Begin::Continued;
        if self.device.covered() {
            let part = library::Paste {
                starts,
                bytes: pasted.bytes.clone(),
                ends: pasted.ends,
            };
            self.ask(library::Action::Drive(Drive::Paste(part)));
            return self.modal(&Routed::Paste(pasted));
        }
        if starts {
            typed.extend(pasted.begin.prefix());
        }
        self.submit(typed);
        let part = library::Paste {
            starts,
            bytes: pasted.bytes,
            ends: pasted.ends,
        };
        self.ask(library::Action::Drive(Drive::Paste(part)));
        Ok(())
    }

    /// A mouse report goes to its gesture's owner (`mouse.rs`): to the
    /// target taking input, at its cell below the top row, as its modes ask,
    /// or nowhere; chrome and overlays keep theirs, and an open modal
    /// takes them (#113); with none open, a wheel notch scrolls a selected
    /// read-only session's history (#116). Over an application that does
    /// not report the mouse, a wheel notch scrolls its history a row, or on
    /// its alternate screen is a cursor key with alternate scroll on (#112).
    fn mouse(&mut self, mouse: Mouse, typed: &mut Vec<u8>) -> io::Result<()> {
        let taking = self.monitor.taking(&self.device);
        let toasted = self.toasted(mouse);
        let target = taking.as_ref().map(|taking| {
            let area = Area {
                cols: taking.cols,
                rows: taking.rows,
            };
            (&taking.target, area)
        });
        let cell = self.gestures.route(mouse, target.filter(|_| !toasted));
        let Some((cell, taking)) = cell.zip(taking) else {
            return self.local(mouse, toasted);
        };
        match mouse::wheel(mouse, &taking.modes, taking.alternate) {
            Some(Wheel::History(rows)) => self.ask(library::Action::Look(Look::Scroll(rows))),
            Some(Wheel::Key(key)) => Key::Cursor(key).encode(&taking.modes, typed),
            Some(Wheel::Nothing) => {}
            None => mouse::encode(mouse, cell, &taking.modes, typed),
        }
        Ok(())
    }

    /// Whether the shown toast takes `mouse`, outside any gesture; a click
    /// it takes dismisses it.
    fn toasted(&mut self, mouse: Mouse) -> bool {
        let (idle, area) = (self.gestures.idle(), self.device.screen.overlay());
        let shown = self.monitor.toasts().shown();
        let taken = toast::take(shown, mouse, idle, area);
        if taken == toast::Taken::Dismissing {
            self.ask(library::Action::Look(Look::Dismiss));
        }
        taken.taken()
    }

    /// A mouse report the interface keeps: one the shown toast took is its
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
        let history = self.monitor.history(&self.device);
        if let Some(rows) = history.and_then(|view| inspected(mouse, &view)) {
            self.ask(library::Action::Look(Look::Scroll(rows)));
        }
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
        let taking = self.monitor.taking(&self.device);
        taking.map(|taking| taking.modes)
    }

    /// Submits the application bytes gathered so far to the target they
    /// were encoded for, under its current authority.
    fn submit(&mut self, typed: &mut Vec<u8>) {
        if typed.is_empty() {
            return;
        }
        self.ask(library::Action::Drive(Drive::Input(mem::take(typed))));
    }

    /// Prefix+x ends the monitor. Prefix+? opens the key introduction, and
    /// prefix+s the selector unless the modal key introduction is open;
    /// prefix+j jumps from anywhere. The Delete confirmation takes them its
    /// own way (#117).
    fn action(&mut self, action: Action) -> Option<Ending> {
        if self.device.deleting.is_some() {
            return self.confirming(action);
        }
        self.global(action)
    }

    /// A global action with no Delete confirmation open.
    fn global(&mut self, action: Action) -> Option<Ending> {
        match action {
            Action::Exit => return Some(Ending::Exit),
            Action::KeyIntroduction => self.device.introduction.open(),
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
                self.device.deleting = None;
                self.jump();
                None
            }
            Action::KeyIntroduction | Action::SessionList => None,
        }
    }

    /// Prefix+s opens the selector, unless the modal key introduction is
    /// open.
    fn session_list(&mut self) {
        if self.device.introduction.is_open() {
            return;
        }
        let area = self.device.screen.menu();
        let rows = self.monitor.rows();
        self.device
            .selector
            .open(&rows, self.monitor.selected(), area);
    }

    /// Prefix+j (spec #73, "Attention jump target"; #114): the library
    /// jumps, and a jump that selected a session closes every overlay
    /// ([`Interface::deliver`]). With none needing attention the selection
    /// and every overlay stay, under the library's notice.
    fn jump(&mut self) {
        self.ask(library::Action::Choose(Choice::Jump));
    }

    /// Input no application takes goes to [`Interface::modal_input`]. When
    /// it closes the last modal, the uncovered session is painted at once,
    /// so the rest of the same read meets the screen now in force and a key
    /// typed straight after Escape reaches the session (owner decision on
    /// #109). That frame's state is in force at once; on a backed-up host
    /// its draw waits until the host has room (owner decision on #420,
    /// option A). Nothing is kept beyond the read.
    fn modal(&mut self, input: &Routed) -> io::Result<()> {
        let covered = self.device.covered();
        self.modal_input(input);
        if revealed(covered, self.device.covered()) {
            return self.show();
        }
        Ok(())
    }

    /// Input no application takes goes to the key introduction while it is
    /// open, otherwise to the open selector; outside them the take-control
    /// key takes control, and anything else is discarded, never kept for
    /// later. Inside a modal the take-control key does nothing.
    fn modal_input(&mut self, input: &Routed) {
        if self.device.introduction.is_open() {
            self.introduction_input(input);
        } else if self.device.selector.is_open() {
            self.selector_input(input);
        } else if self
            .take_control
            .is_some_and(|byte| *input == Routed::Key(Key::Bytes(vec![byte])))
        {
            self.ask(library::Action::Drive(Drive::TakeControl));
        }
    }

    /// The key introduction takes keys only: pastes and mouse reports are
    /// discarded while it is open.
    fn introduction_input(&mut self, input: &Routed) {
        if let Routed::Key(key) = input {
            self.device.introduction.key(key);
        }
    }

    /// Input for the open selector: the Delete confirmation takes it while
    /// open, and Delete opens it (#117).
    fn selector_input(&mut self, input: &Routed) {
        if self.device.deleting.is_some() {
            return self.confirmation_input(input);
        }
        if delete::requested(input) {
            return self.ask_delete();
        }
        self.list_input(input);
    }

    /// Delete opens the confirmation on the highlighted row, unless a
    /// Delete of it is already under way: repeating it sends nothing.
    fn ask_delete(&mut self) {
        let rows = self.monitor.rows();
        let highlighted = self.device.selector.highlighted(&rows);
        let offered = highlighted.filter(|id| self.monitor.offer(id).is_some());
        self.device.deleting = offered.map(Confirmation::new);
    }

    /// Input for the open confirmation. Either decision returns to the
    /// selector; cancelling sends nothing and never acknowledges attention
    /// (Confirmed 2026-11-08), and confirming asks the library to Delete
    /// the row.
    fn confirmation_input(&mut self, input: &Routed) {
        let deleting = self.device.deleting.as_mut();
        let decision = deleting.and_then(|asked| asked.input(input));
        if let Some(decision) = decision {
            self.device.deleting = None;
            if let Decision::Confirm(id) = decision {
                self.ask(library::Action::Choose(Choice::Delete(id)));
            }
        }
    }

    /// Input for the selector's list; Enter acts on the row it chose, and
    /// cancelling back to the selected session acknowledges its attention
    /// once viewed (Confirmed 2026-11-08; #114).
    fn list_input(&mut self, input: &Routed) {
        let area = self.device.screen.menu();
        let rows = self.monitor.rows();
        match self.device.selector.input(input, &rows, area) {
            Some(id) => self.ask(library::Action::Choose(Choice::Select(id))),
            None if !self.device.selector.is_open() => {
                self.ask(library::Action::Look(Look::Acknowledge))
            }
            None => {}
        }
    }
}

/// The open Delete confirmation, its row as `rows` show it now and what
/// `monitor` offers for it now.
fn asking<'a>(
    confirmation: Option<&'a Confirmation>,
    monitor: &Monitor,
    rows: &'a [View],
) -> Option<Asking<'a>> {
    let confirmation = confirmation?;
    let row = rows.iter().find(|row| &row.id == confirmation.target())?;
    let offer = monitor.offer(&row.id)?;
    Some(Asking {
        confirmation,
        row,
        offer,
    })
}

/// Whether a key uncovered the canvas: a modal was open before it and none
/// is after it.
fn revealed(covered_before: bool, covered_after: bool) -> bool {
    covered_before && !covered_after
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

/// Draws `scene` for the host, unless the host has yet to take `output`:
/// then the frame waits until it has, since one drawn meanwhile would only
/// be replaced (#419).
fn draw(
    screen: &mut Screen,
    output: &Output,
    scene: &Scene<'_>,
    titling: Option<Instant>,
) -> io::Result<()> {
    if output.idle() {
        return screen.show(scene, titling);
    }
    screen.defer();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Mouse, also, inspected, revealed};
    use climon2_monitor::NO_ATTENTION;
    use std::time::Instant;

    /// A terminal interface on an empty home in `directory`, as `c2
    /// monitor` starts it at 60x12, without a host.
    fn started(directory: &std::path::Path) -> super::Interface {
        let layout = climon2_core::layout::HomeLayout::at(directory).unwrap();
        let log = climon2_platform::logging::LogFile::monitor(directory).unwrap();
        let dispatch = log.dispatch(Some("info")).unwrap();
        let startup = super::Startup {
            log,
            version: "0.1.0".parse().unwrap(),
            home: layout.clone(),
            executable: climon2_core::layout::ExecutableHome::new(directory).unwrap(),
            config: climon2_core::config::MonitorConfig::parse(b"{}").unwrap(),
        };
        let started = super::Started {
            startup,
            preference: crate::preference::Preference::read(&layout),
            dispatch,
            filter: "info".to_owned(),
        };
        let size = climon2_core::protocol::TerminalSize { cols: 60, rows: 12 };
        super::Interface::start(size, started).unwrap()
    }

    /// The runtime directory prepared for a home, removed when dropped
    /// unless kept.
    struct Runtime {
        path: std::path::PathBuf,
        kept: bool,
    }

    impl Runtime {
        /// Prepares the runtime directory for `home`, as a monitor does.
        fn prepared(home: &std::path::Path) -> Self {
            let directory = climon2_platform::runtime::RuntimeDirectory::prepare(home).unwrap();
            let path = directory.path().to_owned();
            Self { path, kept: false }
        }
    }

    impl Drop for Runtime {
        /// Removes the directory unless it is kept; a failure to remove it
        /// names the path and the error, after no earlier failure.
        fn drop(&mut self) {
            if self.kept {
                return;
            }
            let removed = std::fs::remove_dir(&self.path);
            if let (Err(error), false) = (removed, std::thread::panicking()) {
                panic!("removing {}: {error}", self.path.display());
            }
        }
    }

    /// A started interface whose monitor shuts down when it is dropped, a
    /// failed assertion included, and its home's runtime directory,
    /// removed once the shutdown proved everything stopped.
    struct Stopping(super::Interface, Runtime);

    impl Drop for Stopping {
        /// Shuts the monitor down and waits for it; the runtime directory
        /// is kept when it did not stop cleanly.
        fn drop(&mut self) {
            let monitor = &mut self.0.monitor;
            monitor.shut_down();
            while monitor.waiting(Instant::now()).is_some() {
                std::thread::sleep(std::time::Duration::from_millis(10));
                monitor.waited();
            }
            let verdict = monitor.left();
            let stopped = verdict == super::Verdict::Clean;
            self.1.kept = !stopped;
            if !std::thread::panicking() {
                let runtime = self.1.path.display();
                assert!(stopped, "{verdict:?}; {runtime} is left");
            }
        }
    }

    /// #419 (Monitor supplement): while the host has yet to take what it
    /// was handed, a due frame is not drawn; once the host has taken it,
    /// handing on draws the scene as it is then and hands that frame to the
    /// host.
    #[test]
    fn a_frame_due_while_the_host_is_backed_up_is_drawn_once_it_has_room() {
        let directory = tempfile::tempdir().unwrap();
        let home = std::fs::canonicalize(directory.path()).unwrap();
        let runtime = Runtime::prepared(&home);
        let path = runtime.path.clone();
        let mut stopping = Stopping(started(&home), runtime);
        let interface = &mut stopping.0;
        // The host takes the first attention report, and the key
        // introduction closes.
        interface.output = super::Output::default();
        interface.device.introduction.close();
        interface.show().unwrap();
        assert!(interface.hand_on().unwrap(), "the first frame");
        let first = String::from_utf8_lossy(interface.output.pending()).into_owned();
        assert!(!first.contains(NO_ATTENTION), "{first}");
        // The host has not taken the first frame when prefix+j finds no
        // session needing attention.
        interface.jump();
        interface.show().unwrap();
        assert!(interface.device.screen.undrawn());
        assert_eq!(interface.device.screen.commit(), None);
        assert!(!interface.hand_on().unwrap());
        assert_eq!(interface.output.pending(), first.as_bytes());
        // The host takes everything.
        interface.output = super::Output::default();
        assert!(interface.hand_on().unwrap());
        assert!(!interface.device.screen.undrawn());
        let newest = String::from_utf8_lossy(interface.output.pending()).into_owned();
        assert!(newest.contains(NO_ATTENTION), "{newest}");
        drop(stopping);
        assert!(!path.exists(), "{}", path.display());
    }

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
