use crate::transport::{Pending, Transport, invalid, poll, ready};
use climon2_core::{
    SessionId,
    keys::{ClientSettings, ControlKey},
    protocol::{ClientMessage, ServerMessage, TerminalSize},
};
use climon2_platform::terminal::{self, Signals};
use std::{
    fs::File,
    io::{self, Read},
    os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd},
    time::{Duration, Instant},
};

const RESULT_TIMEOUT: Duration = Duration::from_secs(5);
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";
/// What unexpected loss can restore without a daemon response (section 1.7.1):
/// synchronized update ended, attributes, cursor, mouse, focus, bracketed paste,
/// application cursor keys and keypad. Never the main screen.
const CONSERVATIVE_RESET: &[u8] = b"\x1b[?2026l\x1b[0m\x1b[?25h\
\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1004l\x1b[?2004l\x1b[?1l\x1b>";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Role {
    Restoring,
    Controlling,
    Observing,
}

enum FilterAction {
    Input(Vec<u8>),
    Detach,
    TakeControl,
}

/// The local terminal's paste stream: never reset by role changes (section 1.4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Paste {
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

struct InputFilter {
    escape: Option<u8>,
    take_control: Option<u8>,
    prefix_pending: bool,
    paste: Paste,
    marker: Vec<u8>,
}

impl InputFilter {
    fn new(settings: ClientSettings) -> Self {
        Self {
            escape: settings.escape_prefix().map(|key| key.byte()),
            take_control: settings.take_control_key().map(|key| key.byte()),
            prefix_pending: false,
            paste: Paste::Outside,
            marker: Vec::new(),
        }
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

/// The single owner of the terminal's file-status flags while bridging. Both originals
/// are captured before either changes, because stdin and stdout may share one open-file
/// description, and are restored through owned duplicates, so closing the bridge's
/// descriptors can never skip or misdirect restoration.
struct Nonblocking {
    saved: [(OwnedFd, i32); 2],
}

impl Nonblocking {
    fn new(input: BorrowedFd<'_>, output: BorrowedFd<'_>) -> io::Result<Self> {
        let guard = Self {
            saved: [save(input)?, save(output)?],
        };
        guard.apply()?;
        Ok(guard)
    }

    fn apply(&self) -> io::Result<()> {
        for (fd, flags) in &self.saved {
            set_flags(fd, flags | libc::O_NONBLOCK)?;
        }
        Ok(())
    }
}

impl Drop for Nonblocking {
    fn drop(&mut self) {
        for (fd, flags) in self.saved.iter().rev() {
            let _ = set_flags(fd, *flags);
        }
    }
}

fn save(fd: BorrowedFd<'_>) -> io::Result<(OwnedFd, i32)> {
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((fd.try_clone_to_owned()?, flags))
}

fn set_flags(fd: &OwnedFd, flags: i32) -> io::Result<()> {
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct Output;

impl io::Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = unsafe { libc::write(1, bytes.as_ptr().cast(), bytes.len()) };
        if count < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(count as usize)
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Bridge {
    transport: Transport,
    terminal: File,
    signals: Signals,
    filter: InputFilter,
    takeover: Option<ControlKey>,
    size: TerminalSize,
    resize: Option<TerminalSize>,
    output: Pending,
    role: Role,
    ready_period: Option<u64>,
    pending_detach: bool,
    /// Set by a detach request, a terminal message or connection loss: past it,
    /// the result is unconfirmed, even while stdout is blocked.
    closing: Option<Instant>,
    hung_up: bool,
    /// The bridge's result, waiting for its farewell bytes to drain.
    ending: Option<io::Result<BridgeExit>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BridgeExit {
    Status(u32),
    Detached,
}

pub fn run(
    transport: Transport,
    terminal: File,
    signals: Signals,
    size: TerminalSize,
    id: &SessionId,
    settings: ClientSettings,
) -> io::Result<BridgeExit> {
    let _flags = Nonblocking::new(terminal.as_fd(), io::stdout().as_fd())?;
    let mut bridge = Bridge {
        transport,
        terminal,
        signals,
        filter: InputFilter::new(settings),
        takeover: settings.take_control_key(),
        size,
        resize: None,
        output: Pending::default(),
        role: Role::Restoring,
        ready_period: None,
        pending_detach: false,
        closing: None,
        hung_up: false,
        ending: None,
    };
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
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.abandon(true);
            return Err(invalid(self.unconfirmed()));
        }
        Ok(())
    }

    /// Stops writing pending output and makes one non-blocking attempt at the farewell.
    fn abandon(&mut self, reset: bool) {
        self.output = Pending::new(self.farewell(reset));
        let _ = self.output.flush(&mut Output);
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
        self.resize()?;
        let descriptors = self.poll()?;
        self.observe_hangup(descriptors[0].revents | self.probe_hangup()?);
        self.exchange(&descriptors)
    }

    /// While stdout is blocked the socket is not polled for input, and macOS
    /// then reports no hang-up either; one immediate probe per tick finds it.
    fn probe_hangup(&self) -> io::Result<i16> {
        if self.output.empty() {
            return Ok(0);
        }
        let mut probe = [descriptor(
            self.transport.stream.as_raw_fd(),
            libc::POLLIN,
            true,
        )];
        poll(&mut probe, 0)?;
        Ok(probe[0].revents)
    }

    fn observe_hangup(&mut self, events: i16) {
        if events & (libc::POLLHUP | libc::POLLERR) != 0 {
            self.hung_up = true;
            self.close_within_deadline();
        }
    }

    fn exchange(&mut self, descriptors: &[libc::pollfd; 3]) -> io::Result<Option<BridgeExit>> {
        self.flush_output(descriptors[2].revents)?;
        self.flush_socket(descriptors[0].revents)?;
        self.dispatch(descriptors)?.map_or_else(
            || self.ready_after_drain(descriptors).map(|()| None),
            |exit| Ok(Some(exit)),
        )
    }

    fn poll(&self) -> io::Result<[libc::pollfd; 3]> {
        let mut descriptors = self.descriptors();
        poll(&mut descriptors, self.poll_timeout())?;
        Ok(descriptors)
    }

    fn poll_timeout(&self) -> i32 {
        self.closing.map_or(20, |deadline| {
            deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .clamp(1, 20) as i32
        })
    }

    /// Once the bridge is ending, nothing more is read; it ends when stdout has drained.
    fn dispatch(&mut self, descriptors: &[libc::pollfd; 3]) -> io::Result<Option<BridgeExit>> {
        if self.ending.is_some() {
            return self.ended();
        }
        if let Some(code) = self.receive_ready(descriptors[0].revents)? {
            return Ok(Some(code));
        }
        self.input_ready(descriptors[1].revents)
    }

    fn flush_output(&mut self, events: i16) -> io::Result<()> {
        if events != 0 {
            self.output.flush(&mut Output)?;
        }
        Ok(())
    }

    fn flush_socket(&mut self, events: i16) -> io::Result<()> {
        if events & libc::POLLOUT != 0 {
            self.transport.flush()?;
        }
        Ok(())
    }

    fn receive_ready(&mut self, events: i16) -> io::Result<Option<BridgeExit>> {
        if self.output.empty() && events & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            return self.receive();
        }
        Ok(None)
    }

    fn input_ready(&mut self, events: i16) -> io::Result<Option<BridgeExit>> {
        if events != 0 && !self.input()? {
            return Ok(Some(BridgeExit::Status(129)));
        }
        Ok(None)
    }

    /// Socket, terminal input, stdout; each side waits while the other is backpressured.
    fn descriptors(&self) -> [libc::pollfd; 3] {
        let socket = self.socket_events();
        [
            // Linux reports hang-up even with events=0; omit a parked socket entirely.
            descriptor(self.transport.stream.as_raw_fd(), socket, socket != 0),
            descriptor(
                self.terminal.as_raw_fd(),
                libc::POLLIN,
                self.transport.outgoing.empty() && !self.pending_detach,
            ),
            descriptor(1, libc::POLLOUT, !self.output.empty()),
        ]
    }

    /// macOS reports POLLHUP for any requested event, so waiting to write to a
    /// hung-up session would spin while stdout is blocked.
    fn socket_events(&self) -> i16 {
        let reading = if self.output.empty() { libc::POLLIN } else { 0 };
        let writing = if self.hung_up || self.transport.outgoing.empty() {
            0
        } else {
            libc::POLLOUT
        };
        reading | writing
    }

    fn resize(&mut self) -> io::Result<()> {
        if self.pending_detach {
            return Ok(());
        }
        self.observe_resize()?;
        self.send_resize()
    }

    fn observe_resize(&mut self) -> io::Result<()> {
        if self.signals.take_resize() {
            let size = terminal::size(&self.terminal)?;
            self.resize = (size != self.size).then_some(size);
        }
        Ok(())
    }

    /// One resize at a time, only after earlier messages left.
    fn send_resize(&mut self) -> io::Result<()> {
        if !self.transport.outgoing.empty() {
            return Ok(());
        }
        if let Some(size) = self.resize.take() {
            self.transport.send(&ClientMessage::Resize(size))?;
            self.size = size;
            self.redraw();
        }
        Ok(())
    }

    fn receive(&mut self) -> io::Result<Option<BridgeExit>> {
        match self.transport.receive() {
            Ok(None) => Ok(None),
            Ok(Some(message)) => self.message(message),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                let reset = self.farewell(true);
                self.finish(Err(error), reset)
            }
            Err(error) => Err(error),
        }
    }

    fn redraw(&mut self) {
        if self.role == Role::Observing {
            self.output.append(placeholder_redraw(self.takeover));
        }
    }

    fn message(&mut self, message: ServerMessage) -> io::Result<Option<BridgeExit>> {
        if let Some(ending) = result(&message) {
            let farewell = self.farewell(false);
            return self.finish(ending, farewell);
        }
        self.session_message(message).map(|()| None)
    }

    /// The farewell is written before the terminal is restored, within the closing
    /// deadline that a terminal message or connection loss starts (section 1.7.1).
    fn finish(
        &mut self,
        ending: io::Result<BridgeExit>,
        farewell: Vec<u8>,
    ) -> io::Result<Option<BridgeExit>> {
        self.output.append(farewell);
        self.ending = Some(ending);
        self.close_within_deadline();
        self.ended()
    }

    fn ended(&mut self) -> io::Result<Option<BridgeExit>> {
        if !self.output.empty() {
            return Ok(None);
        }
        self.ending.take().transpose()
    }

    fn session_message(&mut self, message: ServerMessage) -> io::Result<()> {
        match message {
            ServerMessage::Output(bytes) => {
                self.output = Pending::new(bytes);
                Ok(())
            }
            ServerMessage::RestoreEnd { period } => {
                self.ready_period = Some(period);
                Ok(())
            }
            other => role_after(other).map(|role| self.enter(role)),
        }
    }

    /// A new grant restores before input is ready; a revoked controller observes.
    fn enter(&mut self, role: Role) {
        self.role = role;
        self.ready_period = None;
        self.filter.role_changed();
        if role == Role::Observing {
            self.output
                .append(placeholder_start(self.size.rows, self.takeover));
        }
    }

    fn input(&mut self) -> io::Result<bool> {
        let mut bytes = [0; 32768];
        match self.read_terminal(&mut bytes)? {
            Some(count) => self.forward(&bytes[..count]),
            // Another reader consumed what poll reported.
            None => Ok(true),
        }
    }

    /// An empty read is the terminal hanging up (EOF, or EIO via `hangup_as_eof`).
    /// Keys typed while restoring are drained and dropped.
    fn forward(&mut self, read: &[u8]) -> io::Result<bool> {
        if read.is_empty() {
            return Ok(false);
        }
        let actions = self.filter.feed(read, self.role == Role::Controlling);
        if self.role != Role::Restoring {
            self.perform(actions)?;
        }
        Ok(true)
    }

    fn perform(&mut self, actions: Vec<FilterAction>) -> io::Result<()> {
        for action in actions {
            self.act(action)?;
        }
        Ok(())
    }

    fn act(&mut self, action: FilterAction) -> io::Result<()> {
        match action {
            FilterAction::Input(bytes) => self.transport.send(&ClientMessage::Input(bytes)),
            FilterAction::Detach => self.detach(),
            FilterAction::TakeControl => self.transport.send(&ClientMessage::TakeControl),
        }
    }

    fn detach(&mut self) -> io::Result<()> {
        self.transport.send(&ClientMessage::Detach)?;
        self.pending_detach = true;
        self.close_within_deadline();
        Ok(())
    }

    /// `Ready` waits until restoration is written and typed-ahead keys are drained.
    fn ready_after_drain(&mut self, descriptors: &[libc::pollfd; 3]) -> io::Result<()> {
        if !self.drained(descriptors) {
            return Ok(());
        }
        if let Some(period) = self.ready_period.take() {
            self.ready(period)?;
        }
        Ok(())
    }

    fn drained(&self, descriptors: &[libc::pollfd; 3]) -> bool {
        self.output.empty() && descriptors[1].revents & libc::POLLIN == 0
    }

    fn ready(&mut self, period: u64) -> io::Result<()> {
        self.filter.role_changed();
        self.transport.send(&ClientMessage::Ready { period })?;
        self.role = Role::Controlling;
        Ok(())
    }

    fn read_terminal(&mut self, bytes: &mut [u8]) -> io::Result<Option<usize>> {
        ready(self.terminal.read(bytes).or_else(hangup_as_eof))
    }
}

/// The session's final answers end the bridge.
fn result(message: &ServerMessage) -> Option<io::Result<BridgeExit>> {
    match message {
        ServerMessage::Exit { code } => Some(Ok(BridgeExit::Status(*code))),
        ServerMessage::Detached => Some(Ok(BridgeExit::Detached)),
        ServerMessage::Error { message } => Some(Err(invalid(message.clone()))),
        _ => None,
    }
}

fn role_after(message: ServerMessage) -> io::Result<Role> {
    match message {
        ServerMessage::Granted { .. } => Ok(Role::Restoring),
        ServerMessage::Revoked => Ok(Role::Observing),
        _ => Err(invalid("unexpected session message")),
    }
}

/// A lost terminal reads as EIO; like EOF it is a silent HUP.
fn hangup_as_eof(error: io::Error) -> io::Result<usize> {
    if error.raw_os_error() == Some(libc::EIO) {
        return Ok(0);
    }
    Err(error)
}

/// An inactive descriptor is -1, which poll ignores. `revents` is output-only.
fn descriptor(fd: i32, events: i16, active: bool) -> libc::pollfd {
    libc::pollfd {
        fd: if active { fd } else { -1 },
        events,
        revents: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BridgeExit, FilterAction, InputFilter, Role, placeholder_end, placeholder_lines,
        placeholder_redraw, placeholder_start, result, role_after,
    };
    use climon2_core::{
        keys::{ClientSettings, ControlKey},
        protocol::ServerMessage,
    };

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

    #[test]
    fn only_exit_detached_and_error_end_the_bridge() {
        let ended = |message| result(&message).map(|ending| ending.map_err(|e| e.to_string()));
        assert_eq!(
            ended(ServerMessage::Exit { code: 3 }),
            Some(Ok(BridgeExit::Status(3)))
        );
        assert_eq!(
            ended(ServerMessage::Detached),
            Some(Ok(BridgeExit::Detached))
        );
        let error = ServerMessage::Error {
            message: "refused".into(),
        };
        assert_eq!(ended(error), Some(Err("refused".into())));
        assert_eq!(ended(ServerMessage::Revoked), None);
    }

    #[test]
    fn grants_restore_revocations_observe_and_anything_else_fails() {
        let granted = ServerMessage::Granted { period: 1 };
        assert_eq!(role_after(granted).unwrap(), Role::Restoring);
        assert_eq!(role_after(ServerMessage::Revoked).unwrap(), Role::Observing);
        let other = role_after(ServerMessage::Attached).unwrap_err();
        assert_eq!(other.to_string(), "unexpected session message");
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
