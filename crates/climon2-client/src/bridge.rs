use crate::{
    attachment::{Attachment, Authority, Command, Confirm, Ending, Event, Held},
    polling::{Pending, invalid, ready, wait},
};
use climon2_core::{
    SessionId,
    keys::{ClientSettings, ControlKey},
    protocol::TerminalSize,
};
use climon2_platform::{
    terminal::{Bridged, Signals},
    waiting::Watch,
};
use std::{
    fs::File,
    io::{self, Read},
    time::{Duration, Instant},
};

const RESULT_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest one wait lasts before the bridge looks again.
const TICK: Duration = Duration::from_millis(20);
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";
/// What unexpected loss can restore without a daemon response (section 1.7.1):
/// synchronized update ended, attributes, cursor, mouse, focus, bracketed paste,
/// application cursor keys and keypad. Never the main screen.
const CONSERVATIVE_RESET: &[u8] = b"\x1b[?2026l\x1b[0m\x1b[?25h\
\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1004l\x1b[?2004l\x1b[?1l\x1b>";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Role {
    /// Not ready yet: under the authority of a grant once one came, and
    /// under none while the session is ending.
    Restoring(Option<Authority>),
    Controlling(Authority),
    Observing,
}

impl Role {
    /// The authority Input and Resize go under, while the role holds one.
    fn authority(self) -> Option<Authority> {
        match self {
            Self::Restoring(authority) => authority,
            Self::Controlling(authority) => Some(authority),
            Self::Observing => None,
        }
    }

    fn controlling(self) -> bool {
        matches!(self, Self::Controlling(_))
    }
}

enum FilterAction {
    Input(Vec<u8>),
    Detach,
    TakeControl,
}

/// The local terminal's paste stream: never reset by role changes (section 1.4).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Paste {
    #[default]
    Outside,
    Open,
    /// Began before the current role: discarded through its closing marker.
    Stale,
}

impl Paste {
    fn toggled(self) -> Self {
        match self {
            Self::Outside => Self::Open,
            _ => Self::Outside,
        }
    }

    fn forwards(self, controlling: bool) -> bool {
        controlling && self != Self::Stale
    }
}

/// Starts with no prefix pending, outside a paste, with no partial marker.
#[derive(Default)]
struct InputFilter {
    escape: Option<u8>,
    take_control: Option<u8>,
    prefix_pending: bool,
    paste: Paste,
    marker: Vec<u8>,
    /// How long held bytes wait for the next byte before they go on as keys;
    /// `None` holds them until it comes (`terminal::LONE_ESCAPE`).
    release_after: Option<Duration>,
    /// When the last read left bytes held outside a paste.
    held_since: Option<Instant>,
}

impl InputFilter {
    fn new(settings: ClientSettings) -> Self {
        Self {
            escape: settings.escape_prefix().map(|key| key.byte()),
            take_control: settings.take_control_key().map(|key| key.byte()),
            ..Self::default()
        }
    }

    /// Held bytes go on as keys once they waited `after` for the next byte.
    fn releasing_after(mut self, after: Option<Duration>) -> Self {
        self.release_after = after;
        self
    }

    /// Resets the key state; a paste still open is no longer this role's input.
    fn role_changed(&mut self) {
        self.prefix_pending = false;
        if self.paste == Paste::Open {
            self.paste = Paste::Stale;
        }
    }

    fn feed(&mut self, bytes: &[u8], controlling: bool) -> Vec<FilterAction> {
        let mut actions = Vec::new();
        for byte in bytes {
            self.marker.push(*byte);
            if self.settle(controlling, &mut actions) {
                self.marker.clear();
                break;
            }
        }
        let held = self.paste == Paste::Outside && !self.marker.is_empty();
        self.held_since = held.then(Instant::now);
        actions
    }

    /// Once held bytes have waited `release_after` with no byte after them,
    /// passes them on as the keys they are: a lone Escape, or a paste
    /// marker's start that no marker followed.
    fn release_due(&mut self, now: Instant, controlling: bool) -> Vec<FilterAction> {
        let due = self
            .release_after
            .zip(self.held_since)
            .is_some_and(|(after, since)| now >= since + after);
        let mut actions = Vec::new();
        if due {
            self.held_since = None;
            // A paste marker's start holds no `d`, so nothing here detaches.
            for byte in std::mem::take(&mut self.marker) {
                self.pass(byte, controlling, &mut actions);
            }
        }
        actions
    }

    /// Passes on every pending byte that cannot start the next paste marker;
    /// true once a key asked to detach, which discards the rest of the read.
    fn settle(&mut self, controlling: bool, actions: &mut Vec<FilterAction>) -> bool {
        while !self.expected().starts_with(&self.marker) {
            let first = self.marker.remove(0);
            if self.pass(first, controlling, actions) {
                return true;
            }
        }
        self.complete_marker(controlling, actions);
        false
    }

    fn expected(&self) -> &'static [u8] {
        if self.paste == Paste::Outside {
            PASTE_START
        } else {
            PASTE_END
        }
    }

    fn complete_marker(&mut self, controlling: bool, actions: &mut Vec<FilterAction>) {
        let expected = self.expected();
        if self.marker.len() == expected.len() {
            self.marker.clear();
            push_input(actions, expected, self.paste.forwards(controlling));
            self.paste = self.paste.toggled();
        }
    }

    /// Paste payload is never interpreted as keys.
    fn pass(&mut self, byte: u8, controlling: bool, actions: &mut Vec<FilterAction>) -> bool {
        if self.paste != Paste::Outside {
            push_input(actions, &[byte], self.paste.forwards(controlling));
            return false;
        }
        self.key(byte, controlling, actions);
        matches!(actions.last(), Some(FilterAction::Detach))
    }

    fn key(&mut self, byte: u8, controlling: bool, actions: &mut Vec<FilterAction>) {
        let prefixed = std::mem::take(&mut self.prefix_pending);
        match self.escape {
            Some(escape) if prefixed => self.prefixed(escape, byte, controlling, actions),
            Some(escape) if byte == escape => self.prefix_pending = true,
            _ => self.plain(byte, controlling, actions),
        }
    }

    /// After the escape prefix: `d` detaches, the prefix again is itself, and
    /// anything else is the prefix followed by that byte as a plain key.
    fn prefixed(&self, escape: u8, byte: u8, controlling: bool, actions: &mut Vec<FilterAction>) {
        if byte == b'd' {
            return actions.push(FilterAction::Detach);
        }
        push_input(actions, &[escape], controlling);
        if byte != escape {
            self.plain(byte, controlling, actions);
        }
    }

    fn plain(&self, byte: u8, controlling: bool, actions: &mut Vec<FilterAction>) {
        if !controlling && Some(byte) == self.take_control {
            actions.push(FilterAction::TakeControl);
        } else {
            push_input(actions, &[byte], controlling);
        }
    }
}

/// What a non-controlling client shows (section 1.5).
fn placeholder_lines(takeover: Option<ControlKey>) -> Vec<u8> {
    let mut lines = b"This session is being viewed elsewhere.".to_vec();
    if let Some(key) = takeover {
        lines.extend(format!("\r\nPress {key} to take control").bytes());
    }
    lines
}

/// Line feeds from the bottom row scroll visible content into history.
fn placeholder_start(rows: u16, takeover: Option<ControlKey>) -> Vec<u8> {
    let mut bytes = format!("\x1b[{rows};1H").into_bytes();
    bytes.resize(bytes.len() + usize::from(rows), b'\n');
    bytes.extend(b"\x1b[H");
    bytes.extend(placeholder_lines(takeover));
    bytes
}

fn placeholder_redraw(takeover: Option<ControlKey>) -> Vec<u8> {
    let mut bytes = b"\x1b[H\x1b[2J".to_vec();
    bytes.extend(placeholder_lines(takeover));
    bytes
}

fn placeholder_end(rows: u16) -> Vec<u8> {
    format!("\x1b[{rows};1H\r\n").into_bytes()
}

fn push_input(actions: &mut Vec<FilterAction>, bytes: &[u8], controlling: bool) {
    if !controlling {
        return;
    }
    match actions.last_mut() {
        Some(FilterAction::Input(previous)) => previous.extend_from_slice(bytes),
        _ => actions.push(FilterAction::Input(bytes.to_vec())),
    }
}

/// The most terminal input forwarded per read.
const INPUT_CHUNK: usize = 32768;

/// While less output than this waits for the terminal, the bridge receives
/// more: the end of the session is seen even while output is blocked, and
/// what waits stays bounded by this plus one delivery.
const DRAIN_AHEAD: usize = 65_536;

struct Bridge {
    attachment: Attachment,
    /// The terminal: its input, and stdout.
    terminal: Bridged,
    signals: Signals,
    filter: InputFilter,
    takeover: Option<ControlKey>,
    /// The terminal's size, which the placeholder fills.
    size: TerminalSize,
    /// The size the session last heard of.
    told: TerminalSize,
    output: Pending,
    role: Role,
    /// A complete restoration, waiting for `Ready`.
    restored: Option<Authority>,
    /// The latest grant's authority while it was an explicit attach or
    /// takeover whose readiness has not yet acknowledged pending attention
    /// (#102).
    acknowledging: Option<Authority>,
    /// The command's status, once its final result is known.
    known: Option<u32>,
    pending_detach: bool,
    /// Set by a detach request, the end of the session's stream or its
    /// hang-up: past it, the result is unconfirmed, even while stdout is
    /// blocked.
    closing: Option<Instant>,
    /// The bridge's result, waiting for its farewell bytes to drain.
    ending: Option<io::Result<BridgeExit>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BridgeExit {
    Status(u32),
    Detached,
}

/// What the bridge runs on: the opened attachment, the events that followed
/// its Open's success, and the terminal's size at the Open.
pub struct Opened {
    pub attachment: Attachment,
    pub backlog: Vec<Event>,
    pub size: TerminalSize,
}

pub fn run(
    opened: Opened,
    terminal: File,
    signals: Signals,
    id: &SessionId,
    settings: ClientSettings,
    lone_escape: Option<Duration>,
) -> io::Result<BridgeExit> {
    let mut bridge = Bridge {
        attachment: opened.attachment,
        terminal: Bridged::new(terminal)?,
        signals,
        filter: InputFilter::new(settings).releasing_after(lone_escape),
        takeover: settings.take_control_key(),
        size: opened.size,
        told: opened.size,
        output: Pending::default(),
        role: Role::Restoring(None),
        restored: None,
        acknowledging: None,
        known: None,
        pending_detach: false,
        closing: None,
        ending: None,
    };
    bridge.events(opened.backlog);
    bridge.run().map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            io::Error::other(format!("lost connection to session {id}"))
        } else {
            error
        }
    })
}

impl Bridge {
    fn run(&mut self) -> io::Result<BridgeExit> {
        loop {
            if let Some(exit) = self.step()? {
                return Ok(exit);
            }
        }
    }

    fn step(&mut self) -> io::Result<Option<BridgeExit>> {
        if let Some(signal) = self.signals.termination_signal() {
            self.abandon(false);
            return Ok(Some(BridgeExit::Status(128 + signal)));
        }
        self.check_closing()?;
        self.tick()
    }

    fn check_closing(&mut self) -> io::Result<()> {
        if self
            .closing
            .is_some_and(|deadline| remaining(deadline).is_zero())
        {
            self.abandon(true);
            return Err(invalid(self.unconfirmed()));
        }
        Ok(())
    }

    /// Stops writing pending output and makes one non-blocking attempt at the farewell.
    fn abandon(&mut self, reset: bool) {
        self.output = Pending::new(self.farewell(reset));
        let _ = self.output.flush(&mut self.terminal);
    }

    /// The conservative reset when no daemon response restored the terminal, then
    /// the end of a shown placeholder (section 1.7.1).
    fn farewell(&self, reset: bool) -> Vec<u8> {
        let mut bytes = if reset {
            CONSERVATIVE_RESET.to_vec()
        } else {
            Vec::new()
        };
        if self.role == Role::Observing {
            bytes.extend(placeholder_end(self.size.rows));
        }
        bytes
    }

    fn unconfirmed(&self) -> &'static str {
        if self.pending_detach {
            "detach not confirmed"
        } else {
            "session result not confirmed"
        }
    }

    fn close_within_deadline(&mut self) {
        self.closing
            .get_or_insert_with(|| Instant::now() + RESULT_TIMEOUT);
    }

    fn tick(&mut self) -> io::Result<Option<BridgeExit>> {
        self.release_held();
        self.resize()?;
        self.pass_break();
        self.send()?;
        let reported = self.poll()?;
        self.exchange(reported)
    }

    /// Offers the waiting commands. A stream that no longer accepts them
    /// has hung up, even while blocked output keeps the bridge from
    /// receiving its end; the command it holds keeps keys unread.
    fn send(&mut self) -> io::Result<()> {
        if self.attachment.send()? {
            self.close_within_deadline();
        }
        Ok(())
    }

    fn exchange(&mut self, reported: Reported) -> io::Result<Option<BridgeExit>> {
        self.flush_output(reported.output)?;
        self.dispatch(reported)?.map_or_else(
            || {
                self.ready_after_drain(reported);
                Ok(None)
            },
            |exit| Ok(Some(exit)),
        )
    }

    fn poll(&self) -> io::Result<Reported> {
        let mut watches = self.watches();
        wait(&mut watches, self.poll_timeout())?;
        Ok(Reported {
            input: watches[1].ready(),
            typed: watches[1].readable(),
            output: watches[2].ready(),
        })
    }

    /// The wait rounds this up, so the deadline's last fraction of a
    /// millisecond is waited, not spun.
    fn poll_timeout(&self) -> Duration {
        self.closing
            .map_or(TICK, |deadline| remaining(deadline).min(TICK))
    }

    /// Once the bridge is ending, nothing more is received or read; it ends
    /// when stdout has drained.
    fn dispatch(&mut self, reported: Reported) -> io::Result<Option<BridgeExit>> {
        if self.ending.is_none() {
            self.receive()?;
        }
        if self.ending.is_some() {
            return self.ended();
        }
        self.input_ready(reported.input)
    }

    fn flush_output(&mut self, ready: bool) -> io::Result<()> {
        if ready {
            self.output.flush(&mut self.terminal)?;
        }
        Ok(())
    }

    /// Receives one delivery while little output waits.
    fn receive(&mut self) -> io::Result<()> {
        if self.draining() {
            let events = self.attachment.receive()?;
            self.events(events);
        }
        Ok(())
    }

    fn draining(&self) -> bool {
        self.output.backlog() < DRAIN_AHEAD
    }

    fn input_ready(&mut self, ready: bool) -> io::Result<Option<BridgeExit>> {
        if ready && !self.input()? {
            return Ok(Some(BridgeExit::Status(129)));
        }
        Ok(None)
    }

    /// The library, terminal input, stdout; each side waits while the other is
    /// backpressured.
    fn watches(&self) -> [Watch<'_>; 3] {
        [
            watch(self.attachment.watch(), self.listening()),
            watch(self.terminal.watch_input(), self.reading()),
            watch(self.terminal.watch_output(), !self.output.empty()),
        ]
    }

    /// The library's descriptor is readable from the end of its stream on,
    /// so it is polled only until then, and while the bridge would receive.
    fn listening(&self) -> bool {
        self.ending.is_none() && self.draining()
    }

    /// Keys are read only once earlier commands were accepted, and never
    /// after a detach or the end of the stream.
    fn reading(&self) -> bool {
        self.attachment.idle() && self.staying()
    }

    /// Whether the bridge is neither detaching nor ending, so what it reads
    /// or passes on still goes to the session.
    fn staying(&self) -> bool {
        !self.pending_detach && self.ending.is_none()
    }

    /// Ctrl+Break, which only a Windows console reports, goes to the
    /// session from the ready controller that stays attached (#326);
    /// otherwise it is dropped, never held for a later controller.
    fn pass_break(&mut self) {
        if self.signals.take_break()
            && self.staying()
            && let Role::Controlling(authority) = self.role
        {
            self.held(authority, Held::Confirm(Confirm::Break));
        }
    }

    fn resize(&mut self) -> io::Result<()> {
        if self.pending_detach {
            return Ok(());
        }
        self.observe_resize()?;
        self.send_resize();
        Ok(())
    }

    fn observe_resize(&mut self) -> io::Result<()> {
        if self.signals.take_resize() {
            let size = self.terminal.size()?;
            self.resized(size);
        }
        Ok(())
    }

    /// Only a changed size redraws a shown placeholder.
    fn resized(&mut self, size: TerminalSize) {
        if size != self.size {
            self.size = size;
            self.redraw();
        }
    }

    /// One resize at a time, only after earlier commands were accepted and
    /// only under an authority: the session hears an observer's size once
    /// it holds control again.
    fn send_resize(&mut self) {
        let unheard = self.told != self.size && self.attachment.idle();
        if let Some(authority) = self.role.authority().filter(|_| unheard) {
            self.held(authority, Held::Resize(self.size));
            self.told = self.size;
        }
    }

    fn redraw(&mut self) {
        if self.role == Role::Observing {
            self.output.append(placeholder_redraw(self.takeover));
        }
    }

    fn events(&mut self, events: Vec<Event>) {
        for event in events {
            self.event(event);
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Display(bytes) => self.output.append(bytes),
            Event::Known(status) => self.known = Some(status),
            Event::Ended(ending) => self.end(ending),
            other => self.presentation(other),
        }
    }

    fn presentation(&mut self, event: Event) {
        match event {
            Event::Granted {
                authority,
                explicit,
            } => {
                self.acknowledging = explicit.then_some(authority);
                self.enter(Role::Restoring(Some(authority)));
            }
            Event::Restoring(authority) => self.enter(Role::Restoring(Some(authority))),
            Event::Restored(authority) => self.restored = Some(authority),
            other => self.revoked(other),
        }
    }

    /// A takeover leaves this client observing; a session that is ending
    /// takes control away without a placeholder. The Open's answer came
    /// before the bridge, and never comes again.
    fn revoked(&mut self, event: Event) {
        if let Event::Revoked { ending } = event {
            let role = if ending {
                Role::Restoring(None)
            } else {
                Role::Observing
            };
            self.enter(role);
        }
    }

    /// The farewell is written before the terminal is restored, within the
    /// closing deadline that the stream's end starts (section 1.7.1). Only a
    /// lost connection resets conservatively.
    fn end(&mut self, ending: Ending) {
        let ending = result(ending, self.known);
        let lost = ending
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::UnexpectedEof);
        let farewell = self.farewell(lost);
        self.output.append(farewell);
        self.ending = Some(ending);
        self.close_within_deadline();
    }

    fn ended(&mut self) -> io::Result<Option<BridgeExit>> {
        if !self.output.empty() {
            return Ok(None);
        }
        self.ending.take().transpose()
    }

    /// A new grant restores before input is ready; a revoked controller observes.
    fn enter(&mut self, role: Role) {
        self.role = role;
        self.restored = None;
        self.filter.role_changed();
        if role == Role::Observing {
            self.output
                .append(placeholder_start(self.size.rows, self.takeover));
        }
    }

    fn input(&mut self) -> io::Result<bool> {
        let mut bytes = [u8::default(); INPUT_CHUNK];
        match self.read_terminal(&mut bytes)? {
            Some(count) => Ok(self.forward(&bytes[..count])),
            // Another reader consumed what poll reported.
            None => Ok(true),
        }
    }

    /// An empty read is the terminal hanging up.
    /// Keys typed while restoring are drained and dropped.
    fn forward(&mut self, read: &[u8]) -> bool {
        if read.is_empty() {
            return false;
        }
        let actions = self.filter.feed(read, self.role.controlling());
        self.apply(actions);
        true
    }

    /// Held bytes that waited long enough go on as keys (`terminal::LONE_ESCAPE`).
    fn release_held(&mut self) {
        let actions = self
            .filter
            .release_due(Instant::now(), self.role.controlling());
        self.apply(actions);
    }

    fn apply(&mut self, actions: Vec<FilterAction>) {
        if !matches!(self.role, Role::Restoring(_)) {
            self.perform(actions);
        }
    }

    fn perform(&mut self, actions: Vec<FilterAction>) {
        for action in actions {
            self.act(action);
        }
    }

    fn act(&mut self, action: FilterAction) {
        match action {
            FilterAction::Input(bytes) => self.send_input(bytes),
            FilterAction::Detach => self.detach(),
            FilterAction::TakeControl => self.attachment.queue(Command::Acquire),
        }
    }

    /// Only a controller's keys become input, under its authority.
    fn send_input(&mut self, bytes: Vec<u8>) {
        if let Some(authority) = self.role.authority() {
            self.held(authority, Held::Input(bytes));
        }
    }

    fn held(&mut self, authority: Authority, held: Held) {
        self.attachment.queue(Command::Held(authority, held));
    }

    /// A client holding an authority stops viewing before it leaves (#102).
    fn detach(&mut self) {
        if let Some(authority) = self.role.authority() {
            self.held(authority, Held::Viewing(false));
        }
        self.attachment.queue(Command::Leave);
        self.pending_detach = true;
        self.close_within_deadline();
    }

    /// `Ready` waits until typed-ahead keys are drained and the restoration
    /// has been written.
    fn ready_after_drain(&mut self, reported: Reported) {
        if reported.typed || !self.output.empty() {
            return;
        }
        if let Some(authority) = self.restored.take() {
            self.ready(authority);
        }
    }

    /// The ready controller is viewing, focus assumed (#102); an explicit
    /// attach or takeover then acknowledges pending attention, once.
    fn ready(&mut self, authority: Authority) {
        self.filter.role_changed();
        self.held(authority, Held::Confirm(Confirm::Ready));
        self.held(authority, Held::Viewing(true));
        if self.acknowledging.take().is_some() {
            self.held(authority, Held::Confirm(Confirm::Acknowledge));
        }
        self.role = Role::Controlling(authority);
    }

    fn read_terminal(&mut self, bytes: &mut [u8]) -> io::Result<Option<usize>> {
        ready(self.terminal.read(bytes))
    }
}

/// How the bridge ends with the stream: a detach, then the command's known
/// status, then why the stream ended.
fn result(ending: Ending, known: Option<u32>) -> io::Result<BridgeExit> {
    match (ending, known) {
        (Ending::Detached, _) => Ok(BridgeExit::Detached),
        (_, Some(status)) => Ok(BridgeExit::Status(status)),
        (ending, None) => Err(ending.error()),
    }
}

/// Time left before `deadline`; zero once it has passed.
fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

/// What one tick's wait reported for terminal input (anything at all, and
/// readable input) and for stdout.
#[derive(Clone, Copy)]
struct Reported {
    input: bool,
    typed: bool,
    output: bool,
}

/// An inactive slot is idle, which the wait ignores.
fn watch(watch: Watch<'_>, active: bool) -> Watch<'_> {
    if active { watch } else { Watch::idle() }
}

#[cfg(test)]
mod tests {
    use super::{
        BridgeExit, FilterAction, InputFilter, placeholder_end, placeholder_lines,
        placeholder_redraw, placeholder_start, result,
    };
    use crate::attachment::Ending;
    use climon2_core::keys::{ClientSettings, ControlKey};
    use std::io;
    use std::time::{Duration, Instant};

    fn actions(actions: Vec<FilterAction>) -> Vec<String> {
        actions
            .into_iter()
            .map(|action| match action {
                FilterAction::Input(bytes) => format!("input:{bytes:?}"),
                FilterAction::Detach => "detach".into(),
                FilterAction::TakeControl => "take-control".into(),
            })
            .collect()
    }

    #[test]
    fn escape_prefix_detaches_and_double_prefix_is_literal() {
        let mut filter = InputFilter::new(ClientSettings::default());
        assert_eq!(
            actions(filter.feed(b"\x1d\x1d\x1dX\x1dd", true)),
            ["input:[29, 29, 88]", "detach"]
        );
    }

    #[test]
    fn paste_markers_survive_role_change_without_interpreting_payload() {
        let mut filter = InputFilter::new(ClientSettings::default());
        assert!(filter.feed(b"\x1b[20", true).is_empty());
        assert_eq!(
            actions(filter.feed(b"0~hello", true)),
            ["input:[27, 91, 50, 48, 48, 126, 104, 101, 108, 108, 111]"]
        );
        assert!(filter.feed(b"\x1d", false).is_empty());
        assert!(filter.feed(b"dworld\x1b[201", false).is_empty());
        assert!(filter.feed(b"~", false).is_empty());
        assert_eq!(actions(filter.feed(b"x", true)), ["input:[120]"]);
    }

    #[test]
    fn a_paste_begun_before_a_role_change_is_discarded_through_its_closing_marker() {
        let mut filter = InputFilter::new(ClientSettings::default());
        assert_eq!(
            actions(filter.feed(b"\x1b[200~a", true)),
            ["input:[27, 91, 50, 48, 48, 126, 97]"]
        );
        filter.role_changed();
        assert!(filter.feed(b"\x14\x1dd", true).is_empty());
        assert!(filter.feed(b"b\x1b[201", true).is_empty());
        assert_eq!(actions(filter.feed(b"~c", true)), ["input:[99]"]);

        assert!(filter.feed(b"\x1b[200~x", false).is_empty());
        filter.role_changed();
        filter.role_changed();
        assert!(filter.feed(b"y\x1b[201~", true).is_empty());
        assert_eq!(
            actions(filter.feed(b"\x1b[200~z\x1b[201~", true)),
            ["input:[27, 91, 50, 48, 48, 126, 122, 27, 91, 50, 48, 49, 126]"]
        );
    }

    const AFTER: Duration = Duration::from_millis(50);

    #[test]
    fn a_held_escape_goes_on_alone_once_it_waited_with_no_byte_after_it() {
        let mut filter = InputFilter::new(ClientSettings::default()).releasing_after(Some(AFTER));
        assert!(filter.feed(b"x\x1b", true).len() == 1);
        let held = filter.held_since.expect("the Escape is held");
        let early = held + AFTER - Duration::from_nanos(1);
        assert!(filter.release_due(early, true).is_empty());
        // Exactly the delay after the read is late enough.
        assert_eq!(
            actions(filter.release_due(held + AFTER, true)),
            ["input:[27]"]
        );
        // Released once.
        assert!(filter.release_due(held + AFTER * 2, true).is_empty());
    }

    #[test]
    fn a_held_marker_start_is_released_as_keys_and_a_whole_marker_is_not_held() {
        let mut filter = InputFilter::new(ClientSettings::default()).releasing_after(Some(AFTER));
        assert!(filter.feed(b"\x1b[20", true).is_empty());
        let later = Instant::now() + AFTER;
        assert_eq!(
            actions(filter.release_due(later, false)),
            Vec::<String>::new(),
            "an observer's held keys are not input"
        );
        assert_eq!(
            actions(filter.feed(b"\x1b[200~p", true)),
            ["input:[27, 91, 50, 48, 48, 126, 112]"]
        );
        // Inside a paste, a held end marker's start is payload in waiting.
        assert!(filter.feed(b"\x1b[20", true).is_empty());
        assert!(filter.release_due(Instant::now() + AFTER, true).is_empty());
    }

    #[test]
    fn without_a_delay_a_held_escape_waits_for_the_next_byte() {
        let mut filter = InputFilter::new(ClientSettings::default());
        assert!(filter.feed(b"\x1b", true).is_empty());
        assert!(
            filter
                .release_due(Instant::now() + AFTER * 100, true)
                .is_empty()
        );
        assert_eq!(actions(filter.feed(b":", true)), ["input:[27, 58]"]);
    }

    #[test]
    fn a_role_change_outside_a_paste_only_forgets_a_pending_prefix() {
        let mut filter = InputFilter::new(ClientSettings::default());
        assert!(filter.feed(b"\x1d", true).is_empty());
        filter.role_changed();
        assert_eq!(actions(filter.feed(b"d", true)), ["input:[100]"]);
    }

    #[test]
    fn an_observer_takes_control_after_the_escape_prefix_too() {
        let mut filter = InputFilter::new(ClientSettings::default());
        assert_eq!(actions(filter.feed(b"\x1d\x14", false)), ["take-control"]);
        assert_eq!(actions(filter.feed(b"\x1d\x14", true)), ["input:[29, 20]"]);
    }

    #[test]
    fn an_observer_takes_control_but_sends_no_keys_and_detach_drops_the_rest() {
        let mut filter = InputFilter::new(ClientSettings::default());
        assert_eq!(actions(filter.feed(b"a\x14b", false)), ["take-control"]);
        assert_eq!(
            actions(filter.feed(b"\x14a\x1dd\x14", true)),
            ["input:[20, 97]", "detach"]
        );
    }

    /// The end of the stream decides the result: a confirmed detach, then
    /// the command's known status, then why the stream ended; a lost
    /// connection stays unexpected EOF, which is named and resets the
    /// terminal conservatively.
    #[test]
    fn a_detach_then_a_known_status_then_the_failure_decide_the_result() {
        let lost = || Ending::Failed(io::Error::new(io::ErrorKind::UnexpectedEof, "gone"));
        assert_eq!(
            result(Ending::Detached, Some(3)).unwrap(),
            BridgeExit::Detached
        );
        assert_eq!(result(lost(), Some(3)).unwrap(), BridgeExit::Status(3));
        assert_eq!(
            result(Ending::Stopped, Some(0)).unwrap(),
            BridgeExit::Status(0)
        );
        let error = result(lost(), None).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        let error = result(Ending::Stopped, None).unwrap_err();
        assert_eq!(error.to_string(), "session result not confirmed");
    }

    #[test]
    fn the_placeholder_names_the_configured_takeover_key_or_omits_the_line() {
        assert_eq!(
            placeholder_lines(ControlKey::from_byte(0x0B)),
            b"This session is being viewed elsewhere.\r\nPress Ctrl+K to take control"
        );
        assert_eq!(
            placeholder_lines(None),
            b"This session is being viewed elsewhere."
        );
    }

    #[test]
    fn the_placeholder_scrolls_visible_content_away_redraws_in_place_and_ends_below() {
        let key = ControlKey::from_byte(0x14);
        let lines = placeholder_lines(key);
        let mut start = b"\x1b[3;1H\n\n\n\x1b[H".to_vec();
        start.extend(&lines);
        assert_eq!(placeholder_start(3, key), start);
        let mut redraw = b"\x1b[H\x1b[2J".to_vec();
        redraw.extend(&lines);
        assert_eq!(placeholder_redraw(key), redraw);
        assert_eq!(placeholder_end(42), b"\x1b[42;1H\r\n");
    }
}
