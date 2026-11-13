//! Daemon wire revision 1 on the session loop, beside the private protocol:
//! passive monitor connections, ordinary interactive attachments and
//! standalone Kills, their requests, their control and terminal records and
//! their core metadata.
use super::Session;
use crate::{
    connection::{Connection, frame_limit},
    control::{self, Authority, Authorized, ConnectionId, Needs, Period, Restoration},
};
use climon2_core::{
    protocol::TerminalSize,
    wire::{
        ActualGeometry, Advertisement, AuthorityIds, AuthorityResult, Available, Cleanup,
        CommandFinal, CommandResult, ControlGranted, ControlRecord, ControlRevoked, Counter,
        Display, Error, Failure, Families, Family, Focus, FocusBytes, Frame, FrameLimits, Geometry,
        GrantReason, Guid, InitialMetadata, Input, Kill, KnownResult, Mode, Open, OperationalCode,
        Outcome, PartialCleanup, PartialTermination, Ready, Rejection, Request, Resize, Response,
        RestorationBegin, RestorationComplete, RevocationReason, Success, TitleChanged,
    },
};
use std::{collections::BTreeSet, io, num::NonZeroU32};

const HAS_ROLE: &str = "this connection already has a role";
const NO_ROLE: &str = "this connection has no role";
const NOT_OPENED: &str = "this connection opened nothing";
const UNSUPPORTED_REVISION: &str = "this daemon supports revision 1 only";
const UNSUPPORTED_CAPABILITY: &str = "a required capability is not supported";
const FOCUS_NOT_SELECTED: &str = "focus-v1 was not selected";
const UNSUPPORTED_GEOMETRY: &str = "the geometry exceeds the supported terminal size";
const EMPTY_INPUT: &str = "Input carries no bytes";
const TERMINATING: &str = "the session is terminating";
const CLEANUP_INCOMPLETE: &str = "the session's termination or cleanup did not complete";
/// Acquire by a passive monitor connection (#89), and Set viewing and
/// Acknowledge (#91) are not served yet.
const NOT_YET_SERVED: &str = "this daemon does not serve this request yet";

/// What hello advertises: only the families this daemon implements.
pub(super) fn advertisement() -> Advertisement {
    let limits = FrameLimits {
        max_request_frame_bytes: frame_limit(),
        max_server_frame_bytes: frame_limit(),
    };
    Advertisement::new(
        [Family::MonitorV1, Family::TitleV1, Family::FocusV1],
        limits,
    )
}

/// A revision-1 connection after its first role: an Open's, with the
/// families it selected, which decide the core metadata it receives, or a
/// standalone Kill's, which selects none.
pub(super) struct Opened {
    selected: BTreeSet<Family>,
    role: Role,
}

enum Role {
    /// A passive monitor connection: registered, never an attachment.
    Monitor,
    /// An ordinary interactive attachment.
    Attachment(Attachment),
    /// A standalone Kill, answered once the session ended: never an
    /// attachment, and it receives no core metadata.
    Kill(Counter),
}

/// The connection's role, for requests that depend on it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Monitor,
    Attachment,
    Kill,
}

struct Attachment {
    /// The Open that its admission or refusal answers.
    open: Counter,
    /// What it holds while it controls.
    held: Option<Held>,
}

/// The authority an attachment's Display and control records carry, from
/// its grant until its revocation, and the session geometry it last learned.
#[derive(Clone, Copy)]
struct Held {
    authority: Authority,
    geometry: TerminalSize,
}

impl<C: FnMut() -> io::Result<()>> Session<'_, C> {
    /// One revision-1 request. A stream that an earlier request in the same
    /// read ended serves nothing more.
    pub(super) fn request(&mut self, id: u64, request: Request) -> io::Result<()> {
        if self.connections.get(&id).is_some_and(Connection::receiving) {
            return self.serve(id, request);
        }
        Ok(())
    }

    fn serve(&mut self, id: u64, request: Request) -> io::Result<()> {
        match request {
            Request::Open(open) => self.open(id, open),
            Request::Leave(leave) => self.leave(id, leave.request_id),
            Request::Acquire(acquire) => self.acquire(id, acquire.request_id),
            other => self.control_request(id, other),
        }
    }

    fn control_request(&mut self, id: u64, request: Request) -> io::Result<()> {
        match request {
            Request::Ready(ready) => self.ready(id, ready),
            Request::Kill(kill) => self.wire_kill(id, kill),
            other => {
                self.attachment_request(id, other);
                Ok(())
            }
        }
    }

    fn attachment_request(&mut self, id: u64, request: Request) {
        match request {
            Request::Input(input) => self.wire_input(id, input),
            Request::Resize(resize) => self.resize(id, resize),
            Request::Focus(focus) => self.focus(id, focus),
            other => self.unserved(id, other.request_id()),
        }
    }

    /// Set viewing and Acknowledge are rejected without effects until #91
    /// serves `status-v1`; the stream goes on and any role stays.
    fn unserved(&mut self, id: u64, request_id: Counter) {
        self.reject(
            id,
            request_id,
            OperationalCode::InvalidState,
            NOT_YET_SERVED,
        );
    }

    /// A later Open never repurposes an existing role.
    fn open(&mut self, id: u64, open: Open) -> io::Result<()> {
        if self.opened.contains_key(&id) {
            self.has_role(id, open.request_id);
            return Ok(());
        }
        self.first_role(id, open)
    }

    /// An Open that would give the connection its first role.
    fn first_role(&mut self, id: u64, open: Open) -> io::Result<()> {
        let capabilities = &open.capabilities;
        let selected = select(
            open.revision,
            capabilities.required(),
            capabilities.optional(),
        );
        match (selected, open.mode) {
            (Ok(selected), Mode::Monitor) => {
                self.register(id, open.request_id, selected);
                Ok(())
            }
            (Ok(selected), Mode::Interactive(geometry)) => {
                self.attach(id, open.request_id, selected, geometry)
            }
            (Err(error), _) => {
                self.reject_role(id, open.request_id, error);
                Ok(())
            }
        }
    }

    /// A passive monitor connection: success and Initial metadata at once.
    fn register(&mut self, id: u64, request_id: Counter, selected: BTreeSet<Family>) {
        let opened = Opened {
            selected,
            role: Role::Monitor,
        };
        self.opened.insert(id, opened);
        self.report_opened(id, request_id);
    }

    /// An interactive Open attaches and is granted at once, under the leave
    /// and takeover policy every attachment shares; its admission answers it.
    fn attach(
        &mut self,
        id: u64,
        request_id: Counter,
        selected: BTreeSet<Family>,
        geometry: Geometry,
    ) -> io::Result<()> {
        let Some(host) = host_size(geometry) else {
            let error = error(OperationalCode::InvalidGeometry, UNSUPPORTED_GEOMETRY);
            self.reject_role(id, request_id, error);
            return Ok(());
        };
        let attachment = Attachment {
            open: request_id,
            held: None,
        };
        let opened = Opened {
            selected,
            role: Role::Attachment(attachment),
        };
        self.opened.insert(id, opened);
        let effects = self.control.attach(ConnectionId(id), host);
        self.effects(effects)
    }

    /// An admitted interactive Open's success and Initial metadata, before
    /// its grant.
    pub(super) fn admitted(&mut self, id: u64) {
        if let Some(request_id) = self.open_request(id) {
            self.report_opened(id, request_id);
        }
    }

    /// An interactive Open refused because the session is terminating: it
    /// has no effects, and its stream ends.
    pub(super) fn refused(&mut self, id: u64) {
        if let Some(request_id) = self.open_request(id) {
            let error = error(OperationalCode::SessionTerminating, TERMINATING);
            self.reject_with(id, request_id, error);
        }
    }

    fn open_request(&self, id: u64) -> Option<Counter> {
        match &self.opened.get(&id)?.role {
            Role::Attachment(attachment) => Some(attachment.open),
            _ => None,
        }
    }

    /// Success, then the atomic Initial metadata: the connection captures
    /// the current core metadata and receives every later change.
    fn report_opened(&mut self, id: u64, request_id: Counter) {
        let Some(opened) = self.opened.get(&id) else {
            return;
        };
        let title = if opened.selected.contains(&Family::TitleV1) {
            Available::Available(self.vt.application_title().map(str::to_owned))
        } else {
            Available::Unavailable
        };
        let success = Success::Opened {
            connection_id: fresh_connection_id(),
            selected_capabilities: tokens(&opened.selected),
        };
        self.respond(id, request_id, Outcome::Ok(success));
        self.send_wire(id, &initial_metadata(title).to_frame());
    }

    /// Leave removes the role in one step and reports whether an attachment
    /// was detached, then closes the stream. A connection that opened
    /// nothing has nothing to leave.
    fn leave(&mut self, id: u64, request_id: Counter) -> io::Result<()> {
        match self.kind(id) {
            None | Some(Kind::Kill) => {
                self.reject(id, request_id, OperationalCode::InvalidState, NOT_OPENED);
                Ok(())
            }
            Some(Kind::Monitor) => {
                self.left(id, request_id, false);
                Ok(())
            }
            Some(Kind::Attachment) => self.detach_attachment(id, request_id),
        }
    }

    /// An attachment's Leave is an accepted detach: a controller's
    /// presentation is released and control is handed back first.
    fn detach_attachment(&mut self, id: u64, request_id: Counter) -> io::Result<()> {
        let effects = self.control.leave(ConnectionId(id));
        self.effects(effects)?;
        self.left(id, request_id, true);
        Ok(())
    }

    fn left(&mut self, id: u64, request_id: Counter, attachment_detached: bool) {
        self.opened.remove(&id);
        let left = Success::Left {
            attachment_detached,
        };
        self.respond(id, request_id, Outcome::Ok(left));
        self.end_stream(id);
    }

    /// Acquire takes control for a non-controlling attachment and is an
    /// explicit no-op for the controller. A passive monitor connection
    /// cannot acquire until #89 serves it, and a connection that opened
    /// nothing has nothing to convert.
    fn acquire(&mut self, id: u64, request_id: Counter) -> io::Result<()> {
        match self.kind(id) {
            None | Some(Kind::Kill) => {
                self.reject(id, request_id, OperationalCode::InvalidState, NOT_OPENED);
                Ok(())
            }
            Some(Kind::Monitor) => {
                self.reject_with(id, request_id, not_yet());
                Ok(())
            }
            Some(Kind::Attachment) => self.take_control(id, request_id),
        }
    }

    /// The grant, its restoration's beginning and the revocation of the
    /// previous controller precede the success.
    fn take_control(&mut self, id: u64, request_id: Counter) -> io::Result<()> {
        let (acquired, effects) = self.control.acquire(ConnectionId(id));
        self.effects(effects)?;
        let acquired = acquired.map(|acquisition| {
            Success::Acquired(AuthorityResult {
                changed: acquisition.changed,
                authority: ids(acquisition.authority),
            })
        });
        self.answer(id, request_id, acquired);
        Ok(())
    }

    /// Input is accepted whole for forwarding from the ready controller
    /// under its current authority, or rejected.
    fn wire_input(&mut self, id: u64, input: Input) {
        let needs = (input.authority, Needs::Readiness);
        if self.authorized(id, input.request_id, needs).is_none() {
            return;
        }
        let Some(accepted_bytes) = payload_length(&input.payload) else {
            let error = error(OperationalCode::InvalidArgument, EMPTY_INPUT);
            return self.reject_with(id, input.request_id, error);
        };
        self.pty_writes.input(id, &input.payload);
        let accepted = Success::InputAccepted { accepted_bytes };
        self.respond(id, input.request_id, Outcome::Ok(accepted));
    }

    /// Resize records the controller's host size, even before readiness;
    /// the next apply step applies it. Acceptance is not the actual
    /// geometry, which a Geometry or a new restoration reports.
    fn resize(&mut self, id: u64, resize: Resize) {
        let needs = (resize.authority, Needs::Control);
        if self.authorized(id, resize.request_id, needs).is_none() {
            return;
        }
        let Some(host) = host_size(resize.geometry) else {
            let error = error(OperationalCode::InvalidGeometry, UNSUPPORTED_GEOMETRY);
            return self.reject_with(id, resize.request_id, error);
        };
        self.control.resize(ConnectionId(id), host);
        let accepted = Success::ResizeAccepted {};
        self.respond(id, resize.request_id, Outcome::Ok(accepted));
    }

    /// Ready confirms the complete current restoration; once ready again it
    /// is an explicit no-op.
    fn ready(&mut self, id: u64, ready: Ready) -> io::Result<()> {
        let needs = (ready.authority, Needs::Restored);
        let Some(Authorized {
            authority,
            ready: was_ready,
        }) = self.authorized(id, ready.request_id, needs)
        else {
            return Ok(());
        };
        let effects = self.control.ready(ConnectionId(id), authority.restoration);
        self.effects(effects)?;
        let confirmed = Success::Ready(AuthorityResult {
            changed: !was_ready,
            authority: ids(authority),
        });
        self.respond(id, ready.request_id, Outcome::Ok(confirmed));
        Ok(())
    }

    /// Focus needs `focus-v1`, checked first, then the ready controller's
    /// current authority. The desired focus of the control period is
    /// remembered; a change is forwarded to the application as its focus
    /// report while it has focus reporting on.
    fn focus(&mut self, id: u64, focus: Focus) {
        if !self.selects(id, Family::FocusV1) {
            let code = OperationalCode::UnsupportedCapability;
            return self.reject(id, focus.request_id, code, FOCUS_NOT_SELECTED);
        }
        let needs = (focus.authority, Needs::Readiness);
        if self.authorized(id, focus.request_id, needs).is_none() {
            return;
        }
        let changed = self.control.focus(ConnectionId(id), focus.focused);
        let accepted_bytes = self.forward_focus(id, changed, focus.focused);
        let accepted = Success::Focus {
            changed,
            focused: focus.focused,
            accepted_bytes,
        };
        self.respond(id, focus.request_id, Outcome::Ok(accepted));
    }

    /// A changed desired focus is due while the application has focus
    /// reporting on. Its report joins the controller's Input in request
    /// order, so it follows the Input accepted before it however the
    /// requests were read, and ends with that Input's admission.
    fn forward_focus(&mut self, id: u64, changed: bool, focused: bool) -> FocusBytes {
        if !(changed && self.vt.focus_reporting()) {
            return FocusBytes::None;
        }
        self.pty_writes.input(id, focus_report(focused));
        FocusBytes::Sequence
    }

    /// The application newly enabled focus reporting: it receives the
    /// controller's desired focus once, if known, with the daemon's own
    /// terminal replies; none is invented.
    pub(super) fn focus_enabled(&mut self) {
        if let Some(focused) = self.control.desired_focus() {
            self.pty_writes.daemon(focus_report(focused));
        }
    }

    /// The control a request under `(requested, needs)` has, or its
    /// rejection, already answered. A connection without a role cannot have
    /// any; the control state decides the rest.
    fn authorized(
        &mut self,
        id: u64,
        request_id: Counter,
        (requested, needs): (AuthorityIds, Needs),
    ) -> Option<Authorized> {
        let authorized = match self.kind(id) {
            None => Err(error(OperationalCode::InvalidState, NO_ROLE)),
            Some(_) => self
                .control
                .authorize(ConnectionId(id), authority(requested), needs)
                .map_err(refusal),
        };
        authorized
            .inspect_err(|error| self.reject_with(id, request_id, error.clone()))
            .ok()
    }

    fn kind(&self, id: u64) -> Option<Kind> {
        let kind = match self.opened.get(&id)?.role {
            Role::Monitor => Kind::Monitor,
            Role::Attachment(_) => Kind::Attachment,
            Role::Kill(_) => Kind::Kill,
        };
        Some(kind)
    }

    fn selects(&self, id: u64, family: Family) -> bool {
        self.opened
            .get(&id)
            .is_some_and(|opened| opened.selected.contains(&family))
    }

    fn attachment_mut(&mut self, id: u64) -> Option<&mut Attachment> {
        match &mut self.opened.get_mut(&id)?.role {
            Role::Attachment(attachment) => Some(attachment),
            _ => None,
        }
    }

    /// The authority under which an interactive attachment receives
    /// Display, or `None` for a private connection.
    pub(super) fn display_authority(&self, id: u64) -> Option<Authority> {
        match &self.opened.get(&id)?.role {
            Role::Attachment(attachment) => attachment.held.map(|held| held.authority),
            _ => None,
        }
    }

    /// A grant: Control granted, then its restoration's beginning, both
    /// at once; the restoration's Display follows from its snapshot.
    pub(super) fn report_grant(
        &mut self,
        id: u64,
        authority: Authority,
        host: TerminalSize,
        reason: GrantReason,
    ) {
        let geometry = control::geometry(host);
        if !self.hold(id, authority, geometry) {
            return;
        }
        let granted = ControlGranted {
            authority: ids(authority),
            geometry: wire_geometry(geometry),
            reason,
        };
        self.send_wire(id, &granted.to_frame());
        self.begin_restoration(id, authority, geometry);
    }

    /// A replacement restoration within the period: its beginning, at once.
    pub(super) fn report_restoration(&mut self, id: u64, authority: Authority, host: TerminalSize) {
        let geometry = control::geometry(host);
        if self.hold(id, authority, geometry) {
            self.begin_restoration(id, authority, geometry);
        }
    }

    /// Records the authority an interactive attachment now holds; false for
    /// a private connection.
    fn hold(&mut self, id: u64, authority: Authority, geometry: TerminalSize) -> bool {
        let Some(attachment) = self.attachment_mut(id) else {
            return false;
        };
        attachment.held = Some(Held {
            authority,
            geometry,
        });
        true
    }

    fn begin_restoration(&mut self, id: u64, authority: Authority, geometry: TerminalSize) {
        let begin = RestorationBegin {
            authority: ids(authority),
            geometry: wire_geometry(geometry),
        };
        self.send_wire(id, &begin.to_frame());
    }

    /// Every Display of the restoration of `authority` was sent.
    pub(super) fn report_restored(&mut self, id: u64, authority: Authority) {
        if self.display_authority(id) == Some(authority) {
            let complete = RestorationComplete {
                authority: ids(authority),
            };
            self.send_wire(id, &complete.to_frame());
        }
    }

    /// Ends what an interactive attachment holds with Control revoked under
    /// that authority; no Display follows it.
    pub(super) fn report_revocation(&mut self, id: u64, reason: RevocationReason) {
        let Some(held) = self
            .attachment_mut(id)
            .and_then(|attachment| attachment.held.take())
        else {
            return;
        };
        let revoked = ControlRevoked {
            authority: ids(held.authority),
            reason,
        };
        self.send_wire(id, &revoked.to_frame());
    }

    /// A ready controller's host changed: Geometry reports the session's
    /// actual geometry when it is not the one the controller last learned.
    pub(super) fn report_geometry(&mut self, id: u64) {
        let session = self.control.session_size();
        let Some(held) = self
            .attachment_mut(id)
            .and_then(|attachment| attachment.held.as_mut())
        else {
            return;
        };
        if held.geometry == session {
            return;
        }
        held.geometry = session;
        let actual = ActualGeometry {
            period: period(held.authority),
            geometry: wire_geometry(session),
        };
        self.send_wire(id, &actual.to_frame());
    }

    /// Terminal bytes as Display chunks under `authority`.
    pub(super) fn display(&mut self, id: u64, authority: Authority, bytes: &[u8]) {
        let display = Display {
            authority: ids(authority),
            payload: bytes.to_vec(),
        };
        self.send_wire(id, &display.to_frame());
    }

    /// Standalone Kill is validated like any first role; a later Kill never
    /// repurposes an existing role. A valid one commits the session's
    /// termination, or joins the one under way.
    fn wire_kill(&mut self, id: u64, kill: Kill) -> io::Result<()> {
        if self.opened.contains_key(&id) {
            self.has_role(id, kill.request_id);
            return Ok(());
        }
        let none = Families::default();
        if let Err(error) = select(kill.revision, &kill.required_capabilities, &none) {
            self.reject_role(id, kill.request_id, error);
            return Ok(());
        }
        let killing = Opened {
            selected: BTreeSet::new(),
            role: Role::Kill(kill.request_id),
        };
        self.opened.insert(id, killing);
        self.react(|control| control.kill(ConnectionId(id)))
    }

    /// The request a standalone Kill connection awaits the answer to.
    pub(super) fn kill_request(&self, id: u64) -> Option<Counter> {
        match self.opened.get(&id)?.role {
            Role::Kill(request_id) => Some(request_id),
            _ => None,
        }
    }

    /// A standalone Kill's answer once the session ended, then the end of
    /// its stream.
    pub(super) fn answer_kill(&mut self, id: u64, request_id: Counter, ended: &control::Outcome) {
        self.respond(id, request_id, termination(ended));
        self.end_stream(id);
    }

    /// The command's result, once the daemon knows it: every opened
    /// connection receives it as Command final, a standalone Kill's in its
    /// answer instead.
    pub(super) fn publish(&mut self, result: &CommandResult) {
        let frame = CommandFinal::new(result.clone(), None)
            .expect("no status disagrees with a result")
            .to_frame();
        for id in self.subscribed() {
            self.send_wire(id, &frame);
        }
    }

    /// The connections that opened: passive monitor connections and
    /// interactive attachments.
    fn subscribed(&self) -> Vec<u64> {
        self.opened
            .iter()
            .filter(|(_, opened)| !matches!(opened.role, Role::Kill(_)))
            .map(|(id, _)| *id)
            .collect()
    }

    /// Sends a Title changed to every connection that selected `title-v1`,
    /// in order and whatever the control state.
    pub(super) fn titles(&mut self, titles: Vec<Option<String>>) {
        for title in titles {
            let frame = TitleChanged { title }.to_frame();
            for id in self.subscribers(Family::TitleV1) {
                self.send_wire(id, &frame);
            }
        }
    }

    fn subscribers(&self, family: Family) -> Vec<u64> {
        self.opened
            .iter()
            .filter(|(_, opened)| opened.selected.contains(&family))
            .map(|(id, _)| *id)
            .collect()
    }

    /// An identifiable but impermissible request: `invalid_state`, and the
    /// stream continues.
    fn has_role(&mut self, id: u64, request_id: Counter) {
        self.reject(id, request_id, OperationalCode::InvalidState, HAS_ROLE);
    }

    fn reject(&mut self, id: u64, request_id: Counter, code: OperationalCode, message: &str) {
        self.reject_with(id, request_id, error(code, message));
    }

    /// A rejected first role has no effects and ends its stream.
    fn reject_role(&mut self, id: u64, request_id: Counter, error: Error) {
        self.reject_with(id, request_id, error);
        self.end_stream(id);
    }

    fn answer(&mut self, id: u64, request_id: Counter, result: Result<Success, OperationalCode>) {
        match result {
            Ok(success) => self.respond(id, request_id, Outcome::Ok(success)),
            Err(code) => self.reject_with(id, request_id, refusal(code)),
        }
    }

    fn reject_with(&mut self, id: u64, request_id: Counter, error: Error) {
        let rejection = Rejection {
            error,
            known_final: None,
        };
        self.respond(id, request_id, Outcome::Rejected(rejection));
    }

    fn respond(&mut self, id: u64, request_id: Counter, outcome: Outcome) {
        self.send_wire(
            id,
            &Response {
                request_id,
                outcome,
            }
            .to_frame(),
        );
    }

    fn end_stream(&mut self, id: u64) {
        if let Some(connection) = self.connections.get_mut(&id) {
            connection.close_after_flush();
        }
    }

    fn send_wire(&mut self, id: u64, frame: &Frame) {
        let Some(connection) = self.connections.get_mut(&id) else {
            return;
        };
        let queued = connection.queue_wire(frame);
        self.queued_or_lost(id, queued);
    }
}

/// A Kill succeeds only with the command's confirmed result after complete
/// cleanup. Otherwise it failed: its termination or cleanup did not
/// complete, and the result is reported if it was confirmed.
fn termination(ended: &control::Outcome) -> Outcome {
    match (&ended.command, ended.cleaned) {
        (Some(command_result), true) => Outcome::Ok(Success::Termination {
            command_result: command_result.clone(),
            cleanup: Cleanup::Complete,
        }),
        (command_result, _) => Outcome::Failed(incomplete(command_result.clone())),
    }
}

fn incomplete(command_result: Option<CommandResult>) -> Failure {
    let known = KnownResult::Termination(PartialTermination {
        command_result,
        cleanup: PartialCleanup::Incomplete,
    });
    let error = error(OperationalCode::CleanupFailed, CLEANUP_INCOMPLETE);
    Failure::new(error, Some(known)).expect("a partial termination confirms nothing")
}

/// The application's focus report: `ESC [ I` for focus in, `ESC [ O` out.
fn focus_report(focused: bool) -> &'static [u8] {
    if focused { b"\x1b[I" } else { b"\x1b[O" }
}

/// The payload's length, positive and within the frame limit.
fn payload_length(payload: &[u8]) -> Option<NonZeroU32> {
    u32::try_from(payload.len()).ok().and_then(NonZeroU32::new)
}

/// A host size the daemon supports: each dimension fits a terminal size.
fn host_size(geometry: Geometry) -> Option<TerminalSize> {
    let cols = u16::try_from(geometry.cols.get()).ok()?;
    let rows = u16::try_from(geometry.rows.get()).ok()?;
    Some(TerminalSize { cols, rows })
}

/// A session geometry on the wire; its dimensions are never zero.
fn wire_geometry(size: TerminalSize) -> Geometry {
    let dimension =
        |value: u16| NonZeroU32::new(u32::from(value)).expect("a session geometry is nonzero");
    Geometry {
        cols: dimension(size.cols),
        rows: dimension(size.rows),
    }
}

fn period(authority: Authority) -> Counter {
    counter(authority.period.0)
}

/// The authority IDs a record carries.
fn ids(authority: Authority) -> AuthorityIds {
    AuthorityIds {
        period: period(authority),
        restoration_id: counter(authority.restoration.0),
    }
}

/// The authority a request's IDs name.
fn authority(ids: AuthorityIds) -> Authority {
    Authority {
        period: Period(ids.period.get()),
        restoration: Restoration(ids.restoration_id.get()),
    }
}

/// Periods and restorations are issued from 1.
fn counter(value: u64) -> Counter {
    Counter::new(value).expect("issued counters start at 1")
}

/// The families a first role request selects: every required family, which
/// this daemon must support, and the supported optional ones. Unknown or
/// unsupported optional families are not selected.
fn select(
    revision: u32,
    required: &Families,
    optional: &Families,
) -> Result<BTreeSet<Family>, Error> {
    if revision != 1 {
        return Err(error(
            OperationalCode::UnsupportedRevision,
            UNSUPPORTED_REVISION,
        ));
    }
    let required: Option<Vec<Family>> = required
        .tokens()
        .iter()
        .map(|token| supported(token))
        .collect();
    let required = required.ok_or_else(|| {
        error(
            OperationalCode::UnsupportedCapability,
            UNSUPPORTED_CAPABILITY,
        )
    })?;
    let optional = optional
        .tokens()
        .iter()
        .filter_map(|token| supported(token));
    Ok(required.into_iter().chain(optional).collect())
}

fn supported(token: &str) -> Option<Family> {
    Family::from_token(token).filter(|family| advertisement().supports(*family))
}

fn tokens(selected: &BTreeSet<Family>) -> Families {
    let tokens = selected.iter().map(|family| family.token().to_owned());
    Families::new(tokens.collect()).expect("a set holds each token once")
}

/// The title by value when `title-v1` is selected (an available `None` is
/// unset), with status and progress explicitly unavailable. No command result
/// is reported: the exit-or-signal result it needs arrives with #88.
fn initial_metadata(title: Available<Option<String>>) -> InitialMetadata {
    InitialMetadata::new(title, Available::Unavailable, Available::Unavailable, None)
        .expect("unavailable status agrees with any command result")
}

/// A fresh logical connection identity, never reused.
fn fresh_connection_id() -> Guid {
    uuid::Uuid::new_v4()
        .to_string()
        .parse()
        .expect("a v4 UUID is a canonical lowercase GUID")
}

fn error(code: OperationalCode, message: &str) -> Error {
    Error {
        code: code.into(),
        message: message.into(),
    }
}

/// An operational refusal decided by the control state.
fn refusal(code: OperationalCode) -> Error {
    Error {
        code: code.into(),
        message: format!("request refused: {}", code.as_str()),
    }
}

fn not_yet() -> Error {
    error(OperationalCode::InternalError, NOT_YET_SERVED)
}
