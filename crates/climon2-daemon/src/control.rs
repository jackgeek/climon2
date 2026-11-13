//! The pure control state machine of one session.
//!
//! It owns the lifecycle, the attachments, the controller and its hand-back
//! recency order, the control period, the restoration and readiness, the
//! controller's desired application focus, each attachment's latest host
//! size, the applied session geometry, the end of input admission and the
//! shutdown sequence, and it authorizes requests against them. It performs
//! no I/O: each method takes one event and returns the effects the daemon
//! loop must perform, in order.
//!
//! Call protocol for one loop iteration:
//!
//! 1. Feed every event received this iteration to its method, in acceptance
//!    order, and perform the effects each returns. Every attach, takeover and
//!    hand-back is granted at once: its [`Effect::Grant`] names the new
//!    authority. [`Control::resize`] only records a pending size.
//! 2. Call [`Control::apply_pending`] exactly once, before reading the PTY and
//!    before any snapshot for a grant. It applies the controller's latest size
//!    at most once and replaces at most one restoration.
//! 3. Report each restoration whose every byte was sent with
//!    [`Control::restored`].
//! 4. Admit an attachment's application input only while
//!    [`Control::admits_input`] holds for it.

mod attachments;
mod shutdown;

pub use attachments::Needs;
use attachments::{Attachment, Attachments, Departure, Phase};
use climon2_core::{
    lifetime::Lifetime,
    protocol::TerminalSize,
    wire::{GrantReason, OperationalCode},
};
pub use shutdown::{CommandEnd, Outcome};
use shutdown::{Members, Shutdown};

/// Identifies one daemon connection: an attachment, a probe or a kill requester.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConnectionId(pub u64);

/// One uninterrupted period of control, numbered monotonically from 1.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Period(pub u64);

/// One restoration, numbered monotonically from 1 across the session: the
/// first of a grant or a replacement within its period.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Restoration(pub u64);

/// The control period and restoration that a controller's requests and
/// output carry. A restoration belongs to exactly one period.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Authority {
    pub period: Period,
    pub restoration: Restoration,
}

/// The controller's current authority and how far its restoration is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Controlling {
    authority: Authority,
    phase: Phase,
}

impl Controlling {
    /// A request's authority must be current: an earlier period is stale, as
    /// is a superseded restoration of the current period. Then the
    /// restoration must be far enough along for what the request needs.
    fn authorize(self, requested: Authority, needs: Needs) -> Result<Authorized, OperationalCode> {
        if requested.period != self.authority.period {
            return Err(OperationalCode::StalePeriod);
        }
        if requested.restoration != self.authority.restoration {
            return Err(OperationalCode::StaleRestoration);
        }
        if !needs.met(self.phase) {
            return Err(OperationalCode::NotReady);
        }
        Ok(Authorized {
            authority: self.authority,
            ready: self.phase == Phase::Ready,
        })
    }
}

/// The control a request has: the controller's current authority, and
/// whether it already confirmed readiness for that restoration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Authorized {
    pub authority: Authority,
    pub ready: bool,
}

/// An accepted Acquire: whether control changed hands, and the authority
/// the requester holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Acquisition {
    pub changed: bool,
    pub authority: Authority,
}

/// Work the daemon loop performs for the state machine, in the order given.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    /// Accept an attachment: send `Attached`.
    Admit { conn: ConnectionId },
    /// Reject an attachment because the session is terminating: send `Error`, close.
    Refuse { conn: ConnectionId },
    /// `conn` controls under `authority`, a new period and its first
    /// restoration, for `reason`: report the grant, then, once the pending
    /// size is applied, snapshot for a host of size `host` and send the
    /// restoration.
    Grant {
        conn: ConnectionId,
        authority: Authority,
        host: TerminalSize,
        reason: GrantReason,
    },
    /// The controller's host changed before readiness: its current
    /// restoration ends now, and `authority` replaces it within the same
    /// period, snapshotted for a host of size `host`.
    Restore {
        conn: ConnectionId,
        authority: Authority,
        host: TerminalSize,
    },
    /// The controller confirmed readiness for its current restoration.
    EnableInput { conn: ConnectionId },
    /// Admission ended: drop this attachment's unwritten input.
    DiscardPendingInput { conn: ConnectionId },
    /// Admission ended: queue PTY cleanup bytes for an open application paste.
    ClosePaste,
    /// End this attachment's presentation: commit its release bytes.
    Release { conn: ConnectionId },
    /// Send `Revoked`; send no further session output to this attachment.
    Revoke { conn: ConnectionId },
    /// Send `Detached` and close.
    AcceptDetach { conn: ConnectionId },
    /// Close this attachment; it has already been treated as lost.
    Disconnect { conn: ConnectionId },
    /// Resize the PTY and the virtual terminal to this session geometry.
    ResizeSession { size: TerminalSize },
    /// The ready controller's host size changed (its presentation mode may change).
    ResizeHost {
        conn: ConnectionId,
        host: TerminalSize,
    },
    /// Hang up the PTY and start the bounded shutdown.
    BeginShutdown,
    /// Remove metadata and the socket, keeping connections open; then report
    /// [`Control::cleanup_finished`] or [`Control::authoritative_failure`].
    Cleanup,
    /// Send an attachment `Exit{code}`.
    DeliverExit { conn: ConnectionId, code: u32 },
    /// Send an attachment `Error` describing the authoritative failure.
    DeliverError { conn: ConnectionId },
    /// Answer a kill requester with how the session ended.
    ConfirmKill {
        conn: ConnectionId,
        outcome: Outcome,
    },
    /// Flush connections under the shared short deadline, then exit.
    Finish,
}

/// The `lifecycle` value of the hello `state` object.
pub use climon2_core::hello::Lifecycle;

/// One consistent point-in-time view for the hello `state` object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct State {
    pub lifecycle: Lifecycle,
    pub attachments: usize,
    pub controlled: bool,
}

/// The largest session geometry; larger hosts use Derived presentation.
const MAX_COLS: u16 = 500;
const MAX_ROWS: u16 = 200;
/// Alacritty's minimum width.
const MIN_COLS: u16 = 2;

/// The most recently issued period and restoration.
#[derive(Clone, Copy, Debug, Default)]
struct Issued {
    period: Period,
    restoration: Restoration,
}

impl Issued {
    /// A new period and its first restoration.
    fn grant(&mut self) -> Authority {
        self.period.0 += 1;
        self.restore(self.period)
    }

    /// A new restoration within `period`.
    fn restore(&mut self, period: Period) -> Authority {
        self.restoration.0 += 1;
        Authority {
            period,
            restoration: self.restoration,
        }
    }
}

impl State {
    fn new(lifecycle: Lifecycle, attachments: usize) -> Self {
        Self {
            lifecycle,
            attachments,
            controlled: attachments > 0,
        }
    }
}

/// The session geometry for a host with nonzero dimensions.
pub fn geometry(host: TerminalSize) -> TerminalSize {
    TerminalSize {
        cols: host.cols.clamp(MIN_COLS, MAX_COLS),
        rows: host.rows.min(MAX_ROWS),
    }
}

/// The controller's host size as last applied, and the session geometry it gave.
struct Sizes {
    host: TerminalSize,
    session: TerminalSize,
}

impl Sizes {
    /// Returns `None` when `host` is already applied.
    fn apply(&mut self, host: TerminalSize) -> Option<Vec<Effect>> {
        if host == self.host {
            return None;
        }
        let session = geometry(host);
        let resize = (session != self.session).then_some(Effect::ResizeSession { size: session });
        *self = Self { host, session };
        Some(resize.into_iter().collect())
    }
}

enum Stage {
    Open(Open),
    Terminating(Shutdown),
}

/// A session that still accepts attachments.
enum Open {
    /// An ordinary session awaiting its initial attachment.
    Starting,
    Attached(Attachments),
    /// No attachments: after an accepted explicit detach or, in a headless
    /// session, from startup on and after any departure.
    Detached,
}

impl Open {
    /// Before anything attaches.
    fn unattached(lifetime: Lifetime) -> Self {
        match lifetime {
            Lifetime::Ordinary => Self::Starting,
            Lifetime::Headless => Self::Detached,
        }
    }

    fn snapshot(&self) -> State {
        match self {
            Self::Starting => State::new(Lifecycle::Starting, 0),
            Self::Attached(attachments) => State::new(Lifecycle::Attached, attachments.count()),
            Self::Detached => State::new(Lifecycle::Detached, 0),
        }
    }

    fn attach(&mut self, attachment: Attachment, issued: &mut Issued) -> Vec<Effect> {
        let mut effects = vec![Effect::Admit {
            conn: attachment.conn,
        }];
        match self {
            Self::Attached(attachments) => {
                effects.extend(attachments.promote(attachment, GrantReason::Attach, issued))
            }
            _ => {
                let (attachments, grant) = Attachments::new(attachment, issued);
                *self = Self::Attached(attachments);
                effects.push(grant);
            }
        }
        effects
    }

    fn into_members(self) -> (Members, Vec<Effect>) {
        match self {
            Self::Attached(attachments) => attachments.into_members(),
            _ => (Members::default(), Vec::new()),
        }
    }
}

fn begin_shutdown(open: Open, killers: Vec<ConnectionId>, failed: bool) -> (Stage, Vec<Effect>) {
    let (members, mut effects) = open.into_members();
    effects.push(Effect::BeginShutdown);
    let shutdown = Shutdown::begin(members, killers, failed);
    (Stage::Terminating(shutdown), effects)
}

fn command_exited(open: Open, command: CommandEnd) -> (Stage, Vec<Effect>) {
    let (members, mut effects) = open.into_members();
    effects.push(Effect::Cleanup);
    (
        Stage::Terminating(Shutdown::exited(members, command)),
        effects,
    )
}

fn initial_deadline(open: Open) -> (Stage, Vec<Effect>) {
    match open {
        Open::Starting => begin_shutdown(open, Vec::new(), false),
        _ => (Stage::Open(open), Vec::new()),
    }
}

/// The leave policy, after any hand-back: only the unexpected loss of an
/// ordinary session's last attachment ends it; a headless session survives
/// with zero clients.
fn depart(
    lifetime: Lifetime,
    open: Open,
    conn: ConnectionId,
    departure: Departure,
    issued: &mut Issued,
) -> (Stage, Vec<Effect>) {
    let Open::Attached(attachments) = open else {
        return (Stage::Open(open), Vec::new());
    };
    let (remaining, effects) = attachments.depart(conn, departure, issued);
    match (remaining, departure, lifetime) {
        (Some(attachments), _, _) => (Stage::Open(Open::Attached(attachments)), effects),
        (None, Departure::Lost, Lifetime::Ordinary) => {
            let (stage, shutdown) = begin_shutdown(Open::Detached, Vec::new(), false);
            (stage, [effects, shutdown].concat())
        }
        (None, _, _) => (Stage::Open(Open::Detached), effects),
    }
}

fn attached(stage: &mut Stage) -> Option<&mut Attachments> {
    match stage {
        Stage::Open(Open::Attached(attachments)) => Some(attachments),
        _ => None,
    }
}

pub struct Control {
    stage: Stage,
    sizes: Sizes,
    issued: Issued,
    lifetime: Lifetime,
}

impl Control {
    /// A new session whose PTY has the session geometry for `initial`.
    pub fn new(initial: TerminalSize, lifetime: Lifetime) -> Self {
        let host = initial.initial();
        Self {
            stage: Stage::Open(Open::unattached(lifetime)),
            sizes: Sizes {
                host,
                session: geometry(host),
            },
            issued: Issued::default(),
            lifetime,
        }
    }

    /// The applied session geometry: the PTY and virtual terminal size.
    pub fn session_size(&self) -> TerminalSize {
        self.sizes.session
    }

    pub fn snapshot(&self) -> State {
        match &self.stage {
            Stage::Open(open) => open.snapshot(),
            Stage::Terminating(shutdown) => State::new(Lifecycle::Terminating, shutdown.count()),
        }
    }

    /// Whether `conn` is the controller and has confirmed readiness for its
    /// current restoration.
    pub fn admits_input(&self, conn: ConnectionId) -> bool {
        match &self.stage {
            Stage::Open(Open::Attached(attachments)) => attachments.controller.admits(conn),
            _ => false,
        }
    }

    /// The control `conn` holds, or why it holds none: membership,
    /// termination and control come first.
    fn controlling(&self, conn: ConnectionId) -> Result<Controlling, OperationalCode> {
        match &self.stage {
            Stage::Open(Open::Attached(attachments)) => attachments.controlling(conn),
            Stage::Open(_) => Err(OperationalCode::NotAttached),
            Stage::Terminating(shutdown) => Err(shutdown.refusal(conn)),
        }
    }

    /// The control a request from `conn` under `requested` authority has,
    /// or why it has none: `not_attached`, `session_terminating`,
    /// `not_controller`, `stale_period`, `stale_restoration`, then
    /// `not_ready` when the current restoration does not meet `needs`.
    pub fn authorize(
        &self,
        conn: ConnectionId,
        requested: Authority,
        needs: Needs,
    ) -> Result<Authorized, OperationalCode> {
        self.controlling(conn)?.authorize(requested, needs)
    }

    /// An accepted attach request, granted at once. Each connection attaches
    /// at most once.
    pub fn attach(&mut self, conn: ConnectionId, size: TerminalSize) -> Vec<Effect> {
        let attachment = Attachment {
            conn,
            host: size.initial(),
        };
        let issued = &mut self.issued;
        match &mut self.stage {
            Stage::Open(open) => open.attach(attachment, issued),
            Stage::Terminating(_) => vec![Effect::Refuse { conn }],
        }
    }

    /// Acquire: a non-controlling attachment takes control at once, and the
    /// controller's own changes nothing. Its outcome, then its effects.
    pub fn acquire(
        &mut self,
        conn: ConnectionId,
    ) -> (Result<Acquisition, OperationalCode>, Vec<Effect>) {
        let issued = &mut self.issued;
        let taken = attached(&mut self.stage)
            .and_then(|attachments| attachments.take_control(conn, issued));
        let acquisition = self.controlling(conn).map(|controlling| Acquisition {
            changed: taken.is_some(),
            authority: controlling.authority,
        });
        (acquisition, taken.unwrap_or_default())
    }

    /// The private protocol's take-control: an Acquire's effects.
    pub fn take_control(&mut self, conn: ConnectionId) -> Vec<Effect> {
        self.acquire(conn).1
    }

    /// Every byte of the restoration of `authority` was sent to `conn`.
    pub fn restored(&mut self, conn: ConnectionId, authority: Authority) {
        if let Some(attachments) = attached(&mut self.stage) {
            attachments.controller.restored(conn, authority);
        }
    }

    /// Readiness for `restoration`, which must be the controller's current,
    /// complete restoration.
    pub fn ready(&mut self, conn: ConnectionId, restoration: Restoration) -> Vec<Effect> {
        attached(&mut self.stage)
            .map(|attachments| attachments.controller.ready(conn, restoration))
            .unwrap_or_default()
    }

    /// Records the controller's desired application focus for its period;
    /// true when it changed, from unknown too.
    pub fn focus(&mut self, conn: ConnectionId, focused: bool) -> bool {
        attached(&mut self.stage)
            .is_some_and(|attachments| attachments.controller.focus(conn, focused))
    }

    /// The current controller's desired application focus, once known in
    /// its period.
    pub fn desired_focus(&self) -> Option<bool> {
        match &self.stage {
            Stage::Open(Open::Attached(attachments)) => attachments.controller.desired_focus(),
            _ => None,
        }
    }

    /// Records a pending host size; [`Control::apply_pending`] applies it.
    pub fn resize(&mut self, conn: ConnectionId, size: TerminalSize) {
        if let (Some(host), Some(attachments)) = (size.resize(), attached(&mut self.stage)) {
            attachments.resize(conn, host);
        }
    }

    /// An accepted explicit detach request.
    pub fn detach(&mut self, conn: ConnectionId) -> Vec<Effect> {
        let lifetime = self.lifetime;
        self.open(|open, issued| depart(lifetime, open, conn, Departure::Detach, issued))
    }

    /// An accepted departure: a detach, which a terminating session accepts
    /// too, releasing a presentation the attachment still has.
    pub fn leave(&mut self, conn: ConnectionId) -> Vec<Effect> {
        match &mut self.stage {
            Stage::Terminating(shutdown) => shutdown.leave(conn),
            Stage::Open(_) => self.detach(conn),
        }
    }

    /// The connection closed or failed without an accepted detach.
    pub fn lost(&mut self, conn: ConnectionId) -> Vec<Effect> {
        if let Stage::Terminating(shutdown) = &mut self.stage {
            shutdown.lost(conn);
        }
        let lifetime = self.lifetime;
        self.open(|open, issued| depart(lifetime, open, conn, Departure::Lost, issued))
    }

    pub fn kill(&mut self, conn: ConnectionId) -> Vec<Effect> {
        match &mut self.stage {
            Stage::Terminating(shutdown) => shutdown.kill(conn),
            Stage::Open(_) => self.open(|open, _| begin_shutdown(open, vec![conn], false)),
        }
    }

    pub fn initial_deadline(&mut self) -> Vec<Effect> {
        self.open(|open, _| initial_deadline(open))
    }

    /// The restoration deadline of `authority` expired.
    pub fn restore_deadline(&mut self, conn: ConnectionId, authority: Authority) -> Vec<Effect> {
        let restoring = attached(&mut self.stage)
            .is_some_and(|attachments| attachments.controller.restoring(conn, authority));
        if !restoring {
            return Vec::new();
        }
        [vec![Effect::Disconnect { conn }], self.lost(conn)].concat()
    }

    /// The command ended, as the daemon learned it. After a natural exit,
    /// report it once the output drained.
    pub fn command_exited(&mut self, command: CommandEnd) -> Vec<Effect> {
        match &mut self.stage {
            Stage::Terminating(shutdown) => shutdown.command_exited(command),
            Stage::Open(_) => self.open(|open, _| command_exited(open, command)),
        }
    }

    pub fn cleanup_finished(&mut self) -> Vec<Effect> {
        match &mut self.stage {
            Stage::Terminating(shutdown) => shutdown.cleanup_finished(),
            Stage::Open(_) => Vec::new(),
        }
    }

    /// A PTY, command or cleanup failure (section 1.18).
    pub fn authoritative_failure(&mut self) -> Vec<Effect> {
        match &mut self.stage {
            Stage::Terminating(shutdown) => shutdown.authoritative_failure(),
            Stage::Open(_) => self.open(|open, _| begin_shutdown(open, Vec::new(), true)),
        }
    }

    /// Applies the controller's latest pending size once, then replaces a
    /// restoration that size invalidated.
    pub fn apply_pending(&mut self) -> Vec<Effect> {
        let Some(attachments) = attached(&mut self.stage) else {
            return Vec::new();
        };
        let applied = self.sizes.apply(attachments.controller.host());
        let resized = applied.is_some();
        let mut effects = applied.unwrap_or_default();
        effects.extend(attachments.controller.settle(resized, &mut self.issued));
        effects
    }

    /// Runs a transition of an open session; a terminating session ignores it.
    fn open(
        &mut self,
        transition: impl FnOnce(Open, &mut Issued) -> (Stage, Vec<Effect>),
    ) -> Vec<Effect> {
        let (stage, effects) = match std::mem::replace(&mut self.stage, Stage::Open(Open::Starting))
        {
            Stage::Open(open) => transition(open, &mut self.issued),
            terminating => (terminating, Vec::new()),
        };
        self.stage = stage;
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::Effect::*;
    use super::GrantReason::{Acquire, Attach, HandBack};
    use super::Lifecycle::*;
    use super::*;
    use climon2_core::wire::CommandResult;

    const A: ConnectionId = ConnectionId(1);
    const B: ConnectionId = ConnectionId(2);
    const C: ConnectionId = ConnectionId(3);
    const K: ConnectionId = ConnectionId(10);
    const K2: ConnectionId = ConnectionId(11);
    const NONE: [Effect; 0] = [];

    fn size(cols: u16, rows: u16) -> TerminalSize {
        TerminalSize { cols, rows }
    }

    fn ordinary(initial: TerminalSize) -> Control {
        Control::new(initial, Lifetime::Ordinary)
    }

    fn host() -> TerminalSize {
        size(80, 24)
    }

    fn state(lifecycle: Lifecycle, attachments: usize, controlled: bool) -> State {
        State {
            lifecycle,
            attachments,
            controlled,
        }
    }

    fn authority(period: u64, restoration: u64) -> Authority {
        Authority {
            period: Period(period),
            restoration: Restoration(restoration),
        }
    }

    fn grant(
        conn: ConnectionId,
        (period, restoration): (u64, u64),
        host: TerminalSize,
        reason: GrantReason,
    ) -> Effect {
        Grant {
            conn,
            authority: authority(period, restoration),
            host,
            reason,
        }
    }

    fn controlling(
        period: u64,
        restoration: u64,
        phase: Phase,
    ) -> Result<Controlling, OperationalCode> {
        Ok(Controlling {
            authority: authority(period, restoration),
            phase,
        })
    }

    const RESTORING: Phase = Phase::Restoring { complete: false };
    const RESTORED: Phase = Phase::Restoring { complete: true };

    fn exit(code: u32) -> CommandEnd {
        CommandEnd::Confirmed(CommandResult::Exit { code })
    }

    fn signal(number: u32) -> CommandEnd {
        let number = std::num::NonZeroU32::new(number).unwrap();
        CommandEnd::Confirmed(CommandResult::Signal { number, name: None })
    }

    /// A session that ended with `command` and cleaned up, without failing.
    fn ended(command: CommandEnd) -> Outcome {
        Outcome {
            command,
            failed: false,
            cleaned: true,
        }
    }

    fn confirmed(conn: ConnectionId, outcome: Outcome) -> Effect {
        ConfirmKill { conn, outcome }
    }

    fn granted(effects: Vec<Effect>) -> Authority {
        match effects.last() {
            Some(Grant { authority, .. }) => *authority,
            other => panic!("expected a grant, got {other:?}"),
        }
    }

    /// Completes the restoration of `authority` and confirms readiness.
    fn restore(control: &mut Control, conn: ConnectionId, authority: Authority) {
        control.restored(conn, authority);
        assert_eq!(
            control.ready(conn, authority.restoration),
            [EnableInput { conn }]
        );
    }

    /// Attaches each connection in turn at 80x24 and lets it confirm readiness.
    fn session(conns: &[ConnectionId]) -> Control {
        with_attachments(ordinary(host()), conns)
    }

    fn with_attachments(mut control: Control, conns: &[ConnectionId]) -> Control {
        for conn in conns {
            let authority = granted(control.attach(*conn, host()));
            control.apply_pending();
            restore(&mut control, *conn, authority);
        }
        control
    }

    #[test]
    fn a_new_session_is_starting_without_attachments() {
        let control = ordinary(size(100, 30));
        assert_eq!(control.snapshot(), state(Starting, 0, false));
        assert_eq!(control.session_size(), size(100, 30));
        assert_eq!(control.controlling(A), Err(OperationalCode::NotAttached));
    }

    #[test]
    fn a_zero_initial_size_falls_back_to_80x24() {
        assert_eq!(ordinary(size(0, 30)).session_size(), host());
    }

    #[test]
    fn session_geometry_is_clamped_to_the_budget() {
        assert_eq!(ordinary(size(501, 201)).session_size(), size(500, 200));
        assert_eq!(ordinary(size(500, 200)).session_size(), size(500, 200));
        assert_eq!(ordinary(size(1, 1)).session_size(), size(2, 1));
    }

    #[test]
    fn the_first_attach_is_admitted_and_granted_at_once() {
        let mut control = ordinary(host());
        assert_eq!(
            control.attach(A, host()),
            [Admit { conn: A }, grant(A, (1, 1), host(), Attach)]
        );
        assert_eq!(control.snapshot(), state(Attached, 1, true));
        assert_eq!(control.controlling(A), controlling(1, 1, RESTORING));
        assert_eq!(control.apply_pending(), NONE);
    }

    #[test]
    fn a_grant_applies_the_attaching_host_size_before_its_snapshot() {
        let mut control = ordinary(host());
        assert_eq!(
            control.attach(A, size(100, 30)),
            [Admit { conn: A }, grant(A, (1, 1), size(100, 30), Attach)]
        );
        assert_eq!(
            control.apply_pending(),
            [ResizeSession {
                size: size(100, 30)
            }]
        );
        assert_eq!(control.session_size(), size(100, 30));
    }

    #[test]
    fn an_attach_without_a_size_uses_80x24() {
        let mut control = ordinary(size(100, 30));
        assert_eq!(
            control.attach(A, size(0, 0)),
            [Admit { conn: A }, grant(A, (1, 1), host(), Attach)]
        );
        assert_eq!(control.apply_pending(), [ResizeSession { size: host() }]);
    }

    #[test]
    fn input_is_admitted_only_after_ready_for_the_complete_current_restoration() {
        let mut control = ordinary(host());
        let authority = granted(control.attach(A, host()));
        assert_eq!(control.ready(A, authority.restoration), NONE);
        control.restored(A, authority);
        assert_eq!(control.controlling(A), controlling(1, 1, RESTORED));
        assert!(!control.admits_input(A));
        assert_eq!(control.ready(A, Restoration(2)), NONE);
        assert_eq!(control.ready(B, authority.restoration), NONE);
        assert_eq!(
            control.ready(A, authority.restoration),
            [EnableInput { conn: A }]
        );
        assert!(control.admits_input(A));
        assert!(!control.admits_input(B));
        assert_eq!(control.ready(A, authority.restoration), NONE);
        assert_eq!(control.controlling(A), controlling(1, 1, Phase::Ready));
    }

    #[test]
    fn only_the_current_restoration_of_the_controller_completes() {
        let mut control = ordinary(host());
        let current = granted(control.attach(A, host()));
        control.restored(B, current);
        control.restored(A, authority(1, 2));
        control.restored(A, authority(2, 1));
        assert_eq!(control.controlling(A), controlling(1, 1, RESTORING));
        control.restored(A, current);
        assert_eq!(control.controlling(A), controlling(1, 1, RESTORED));
    }

    #[test]
    fn a_stale_or_foreign_ready_is_ignored() {
        let mut control = session(&[A]);
        let authority = granted(control.attach(B, host()));
        control.restored(B, authority);
        assert_eq!(control.ready(B, Restoration(1)), NONE);
        assert_eq!(control.ready(A, Restoration(2)), NONE);
        assert!(!control.admits_input(A));
        assert!(!control.admits_input(B));
        assert_eq!(control.ready(B, Restoration(2)), [EnableInput { conn: B }]);
    }

    #[test]
    fn control_tells_absent_other_and_controlling_attachments_apart() {
        let mut control = session(&[A, B]);
        assert_eq!(control.controlling(A), Err(OperationalCode::NotController));
        assert_eq!(control.controlling(B), controlling(2, 2, Phase::Ready));
        assert_eq!(control.controlling(C), Err(OperationalCode::NotAttached));
        control.detach(A);
        control.detach(B);
        assert_eq!(control.controlling(A), Err(OperationalCode::NotAttached));
        assert_eq!(control.controlling(B), Err(OperationalCode::NotAttached));
    }

    #[test]
    fn an_attach_takes_control_from_a_ready_controller() {
        let mut control = session(&[A]);
        assert_eq!(
            control.attach(B, host()),
            [
                Admit { conn: B },
                DiscardPendingInput { conn: A },
                ClosePaste,
                Release { conn: A },
                Revoke { conn: A },
                grant(B, (2, 2), host(), Attach)
            ]
        );
        assert!(!control.admits_input(A));
        assert_eq!(control.snapshot(), state(Attached, 2, true));
        assert_eq!(control.controlling(A), Err(OperationalCode::NotController));
        assert_eq!(control.apply_pending(), NONE);
    }

    #[test]
    fn revoking_a_controller_that_was_never_ready_closes_no_paste() {
        let mut control = ordinary(host());
        control.attach(A, host());
        control.apply_pending();
        assert_eq!(
            control.attach(B, host()),
            [
                Admit { conn: B },
                Release { conn: A },
                Revoke { conn: A },
                grant(B, (2, 2), host(), Attach)
            ]
        );
    }

    #[test]
    fn attaches_in_one_iteration_are_each_granted_and_the_last_wins() {
        let mut control = ordinary(host());
        assert_eq!(
            control.attach(A, host()),
            [Admit { conn: A }, grant(A, (1, 1), host(), Attach)]
        );
        assert_eq!(
            control.attach(B, host()),
            [
                Admit { conn: B },
                Release { conn: A },
                Revoke { conn: A },
                grant(B, (2, 2), host(), Attach)
            ]
        );
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.controlling(B), controlling(2, 2, RESTORING));
    }

    #[test]
    fn take_control_moves_control_to_the_requester_in_a_new_period() {
        let mut control = session(&[A, B]);
        assert_eq!(
            control.take_control(A),
            [
                DiscardPendingInput { conn: B },
                ClosePaste,
                Release { conn: B },
                Revoke { conn: B },
                grant(A, (3, 3), host(), Acquire)
            ]
        );
        assert_eq!(control.apply_pending(), NONE);
        control.restored(A, authority(3, 3));
        assert_eq!(control.ready(A, Restoration(1)), NONE);
        assert_eq!(control.ready(A, Restoration(3)), [EnableInput { conn: A }]);
    }

    #[test]
    fn take_control_by_the_controller_is_a_no_op() {
        let mut control = session(&[A]);
        assert_eq!(control.take_control(A), NONE);
        assert_eq!(control.apply_pending(), NONE);
        assert!(control.admits_input(A));
        assert_eq!(control.controlling(A), controlling(1, 1, Phase::Ready));
    }

    #[test]
    fn take_control_from_an_unknown_connection_changes_nothing() {
        assert_eq!(ordinary(host()).take_control(C), NONE);
        let mut control = session(&[A]);
        assert_eq!(control.take_control(C), NONE);
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.snapshot(), state(Attached, 1, true));
    }

    fn authorized(
        period: u64,
        restoration: u64,
        ready: bool,
    ) -> Result<Authorized, OperationalCode> {
        Ok(Authorized {
            authority: authority(period, restoration),
            ready,
        })
    }

    #[test]
    fn authorize_checks_membership_control_period_restoration_then_needs() {
        let mut control = session(&[A, B]);
        let current = authority(2, 2);
        let refused = |code| Err::<Authorized, _>(code);
        assert_eq!(
            control.authorize(C, current, Needs::Control),
            refused(OperationalCode::NotAttached)
        );
        assert_eq!(
            control.authorize(A, current, Needs::Control),
            refused(OperationalCode::NotController)
        );
        for stale in [authority(1, 1), authority(1, 2)] {
            assert_eq!(
                control.authorize(B, stale, Needs::Control),
                refused(OperationalCode::StalePeriod)
            );
        }
        assert_eq!(
            control.authorize(B, authority(2, 1), Needs::Control),
            refused(OperationalCode::StaleRestoration)
        );
        assert_eq!(
            control.authorize(B, current, Needs::Readiness),
            authorized(2, 2, true)
        );

        let restoring = granted(control.attach(C, host()));
        assert_eq!(
            control.authorize(C, restoring, Needs::Control),
            authorized(3, 3, false)
        );
        for needs in [Needs::Restored, Needs::Readiness] {
            assert_eq!(
                control.authorize(C, restoring, needs),
                refused(OperationalCode::NotReady)
            );
        }
        control.restored(C, restoring);
        assert_eq!(
            control.authorize(C, restoring, Needs::Restored),
            authorized(3, 3, false)
        );
        assert_eq!(
            control.authorize(C, restoring, Needs::Readiness),
            refused(OperationalCode::NotReady)
        );
        assert_eq!(
            control.ready(C, restoring.restoration),
            [EnableInput { conn: C }]
        );
        assert_eq!(
            control.authorize(C, restoring, Needs::Restored),
            authorized(3, 3, true)
        );

        control.kill(K);
        assert_eq!(
            control.authorize(C, restoring, Needs::Control),
            refused(OperationalCode::SessionTerminating)
        );
        assert_eq!(
            control.authorize(K, restoring, Needs::Control),
            refused(OperationalCode::NotAttached)
        );
    }

    #[test]
    fn acquire_takes_control_or_keeps_it_and_says_whether_it_changed_hands() {
        let mut control = session(&[A, B]);
        let (acquired, effects) = control.acquire(A);
        let changed = Acquisition {
            changed: true,
            authority: authority(3, 3),
        };
        assert_eq!(acquired, Ok(changed));
        assert_eq!(
            effects,
            [
                DiscardPendingInput { conn: B },
                ClosePaste,
                Release { conn: B },
                Revoke { conn: B },
                grant(A, (3, 3), host(), Acquire)
            ]
        );
        let kept = Acquisition {
            changed: false,
            authority: authority(3, 3),
        };
        assert_eq!(control.acquire(A), (Ok(kept), Vec::new()));
        assert_eq!(
            control.acquire(C),
            (Err(OperationalCode::NotAttached), Vec::new())
        );
        control.kill(K);
        assert_eq!(
            control.acquire(B),
            (Err(OperationalCode::SessionTerminating), Vec::new())
        );
    }

    #[test]
    fn controller_departure_hands_back_to_the_most_recent_controller() {
        let mut control = session(&[A, B, C]);
        let authority = granted(control.take_control(A));
        restore(&mut control, A, authority);
        assert_eq!(
            control.detach(A),
            [
                DiscardPendingInput { conn: A },
                ClosePaste,
                Release { conn: A },
                AcceptDetach { conn: A },
                grant(C, (5, 5), host(), HandBack)
            ]
        );
        assert_eq!(control.lost(C), [grant(B, (6, 6), host(), HandBack)]);
        assert_eq!(control.snapshot(), state(Attached, 1, true));
    }

    #[test]
    fn losing_a_ready_controller_ends_its_admission_and_hands_back() {
        let mut control = session(&[A, B]);
        assert_eq!(
            control.lost(B),
            [
                DiscardPendingInput { conn: B },
                ClosePaste,
                grant(A, (3, 3), host(), HandBack)
            ]
        );
        assert!(!control.admits_input(B));
        assert_eq!(control.apply_pending(), NONE);
    }

    #[test]
    fn a_hand_back_applies_the_recipients_latest_size() {
        let mut control = session(&[A, B]);
        control.resize(A, size(120, 40));
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(
            control.lost(B),
            [
                DiscardPendingInput { conn: B },
                ClosePaste,
                grant(A, (3, 3), size(120, 40), HandBack)
            ]
        );
        assert_eq!(
            control.apply_pending(),
            [ResizeSession {
                size: size(120, 40)
            }]
        );
    }

    #[test]
    fn a_non_controlling_detach_needs_no_second_release() {
        let mut control = session(&[A, B]);
        assert_eq!(control.detach(A), [AcceptDetach { conn: A }]);
        assert_eq!(control.apply_pending(), NONE);
        assert!(control.admits_input(B));
        assert_eq!(control.snapshot(), state(Attached, 1, true));
    }

    #[test]
    fn losing_a_non_controlling_attachment_keeps_control() {
        let mut control = session(&[A, B]);
        assert_eq!(control.lost(A), NONE);
        assert_eq!(control.apply_pending(), NONE);
        assert!(control.admits_input(B));
        assert_eq!(control.snapshot(), state(Attached, 1, true));
    }

    #[test]
    fn the_final_explicit_detach_keeps_the_session_detached_at_its_last_size() {
        let mut control = ordinary(host());
        let authority = granted(control.attach(A, size(100, 30)));
        control.apply_pending();
        restore(&mut control, A, authority);
        assert_eq!(
            control.detach(A),
            [
                DiscardPendingInput { conn: A },
                ClosePaste,
                Release { conn: A },
                AcceptDetach { conn: A }
            ]
        );
        assert_eq!(control.snapshot(), state(Detached, 0, false));
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.initial_deadline(), NONE);
        assert_eq!(control.session_size(), size(100, 30));
    }

    #[test]
    fn a_detach_is_not_sticky_after_reattachment() {
        let mut control = session(&[A]);
        control.detach(A);
        assert_eq!(
            control.attach(B, host()),
            [Admit { conn: B }, grant(B, (2, 2), host(), Attach)]
        );
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.lost(B), [BeginShutdown]);
        assert_eq!(control.snapshot(), state(Terminating, 0, false));
    }

    #[test]
    fn the_final_unexpected_loss_begins_shutdown() {
        let mut control = session(&[A]);
        assert_eq!(
            control.lost(A),
            [DiscardPendingInput { conn: A }, ClosePaste, BeginShutdown]
        );
        assert_eq!(control.snapshot(), state(Terminating, 0, false));
    }

    #[test]
    fn leaving_an_open_session_is_a_detach() {
        let mut control = session(&[A, B]);
        assert_eq!(control.leave(A), [AcceptDetach { conn: A }]);
        assert_eq!(
            control.leave(B),
            [
                DiscardPendingInput { conn: B },
                ClosePaste,
                Release { conn: B },
                AcceptDetach { conn: B }
            ]
        );
        assert_eq!(control.snapshot(), state(Detached, 0, false));
    }

    #[test]
    fn leaving_a_terminating_session_releases_a_presentation_then_confirms() {
        let mut control = session(&[A, B]);
        control.kill(K);
        assert_eq!(
            control.controlling(A),
            Err(OperationalCode::SessionTerminating)
        );
        assert_eq!(
            control.controlling(B),
            Err(OperationalCode::SessionTerminating)
        );
        assert_eq!(control.controlling(C), Err(OperationalCode::NotAttached));
        assert_eq!(control.leave(C), NONE);
        assert_eq!(
            control.leave(B),
            [Release { conn: B }, AcceptDetach { conn: B }]
        );
        assert_eq!(control.leave(A), [AcceptDetach { conn: A }]);
        assert_eq!(control.leave(A), NONE);
        assert_eq!(control.controlling(A), Err(OperationalCode::NotAttached));
        assert_eq!(control.snapshot(), state(Terminating, 0, false));
        control.command_exited(exit(0));
        assert_eq!(
            control.cleanup_finished(),
            [confirmed(K, ended(exit(0))), Finish]
        );
    }

    /// HDL-01 (pure decision; developer decision of 2026-11-08): a headless
    /// session is `detached` from startup, and its initial deadline ends nothing.
    #[test]
    fn a_new_headless_session_is_detached_and_outlives_its_initial_deadline() {
        let mut control = Control::new(host(), Lifetime::Headless);
        assert_eq!(control.snapshot(), state(Detached, 0, false));
        assert_eq!(control.initial_deadline(), NONE);
        assert_eq!(control.snapshot(), state(Detached, 0, false));
    }

    /// HDL-01 (pure decision): a headless session stays `detached` after its
    /// last attachment detaches and after its last attachment is lost; Kill
    /// still ends it.
    #[test]
    fn a_headless_session_survives_the_departure_of_its_last_attachment() {
        let mut control = with_attachments(Control::new(host(), Lifetime::Headless), &[A]);
        assert_eq!(
            control.detach(A),
            [
                DiscardPendingInput { conn: A },
                ClosePaste,
                Release { conn: A },
                AcceptDetach { conn: A }
            ]
        );
        assert_eq!(control.snapshot(), state(Detached, 0, false));
        let mut control = with_attachments(control, &[B]);
        assert_eq!(
            control.lost(B),
            [DiscardPendingInput { conn: B }, ClosePaste]
        );
        assert_eq!(control.snapshot(), state(Detached, 0, false));
        assert_eq!(control.kill(K), [BeginShutdown]);
    }

    #[test]
    fn probes_and_unknown_connections_never_trigger_the_leave_policy() {
        let mut control = ordinary(host());
        assert_eq!(control.lost(C), NONE);
        assert_eq!(control.detach(C), NONE);
        assert_eq!(control.snapshot(), state(Starting, 0, false));
        let mut control = session(&[A]);
        assert_eq!(control.lost(C), NONE);
        assert_eq!(control.detach(C), NONE);
        assert_eq!(control.snapshot(), state(Attached, 1, true));
        control.detach(A);
        assert_eq!(control.lost(C), NONE);
        assert_eq!(control.snapshot(), state(Detached, 0, false));
    }

    #[test]
    fn a_resize_is_only_recorded_until_applied() {
        let mut control = session(&[A]);
        control.resize(A, size(100, 30));
        assert_eq!(control.session_size(), host());
        assert_eq!(
            control.apply_pending(),
            [
                ResizeSession {
                    size: size(100, 30)
                },
                ResizeHost {
                    conn: A,
                    host: size(100, 30)
                }
            ]
        );
        assert_eq!(control.session_size(), size(100, 30));
        assert!(control.admits_input(A));
        assert_eq!(control.controlling(A), controlling(1, 1, Phase::Ready));
    }

    #[test]
    fn a_burst_of_resizes_applies_only_the_latest_once() {
        let mut control = session(&[A]);
        control.resize(A, size(90, 30));
        control.resize(A, size(100, 40));
        control.resize(A, size(110, 50));
        assert_eq!(
            control.apply_pending(),
            [
                ResizeSession {
                    size: size(110, 50)
                },
                ResizeHost {
                    conn: A,
                    host: size(110, 50)
                }
            ]
        );
        assert_eq!(control.apply_pending(), NONE);
    }

    #[test]
    fn equal_and_zero_sized_resizes_apply_nothing() {
        let mut control = session(&[A]);
        control.resize(A, host());
        assert_eq!(control.apply_pending(), NONE);
        control.resize(A, size(0, 30));
        control.resize(A, size(100, 0));
        assert_eq!(control.apply_pending(), NONE);
    }

    #[test]
    fn a_non_controller_resize_is_only_remembered() {
        let mut control = session(&[A, B]);
        control.resize(A, size(100, 30));
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(
            control.take_control(A),
            [
                DiscardPendingInput { conn: B },
                ClosePaste,
                Release { conn: B },
                Revoke { conn: B },
                grant(A, (3, 3), size(100, 30), Acquire)
            ]
        );
        assert_eq!(
            control.apply_pending(),
            [ResizeSession {
                size: size(100, 30)
            }]
        );
    }

    #[test]
    fn hosts_beyond_the_budget_resize_the_host_but_clamp_the_session() {
        let mut control = session(&[A]);
        control.resize(A, size(600, 300));
        assert_eq!(
            control.apply_pending(),
            [
                ResizeSession {
                    size: size(500, 200)
                },
                ResizeHost {
                    conn: A,
                    host: size(600, 300)
                }
            ]
        );
        control.resize(A, size(700, 250));
        assert_eq!(
            control.apply_pending(),
            [ResizeHost {
                conn: A,
                host: size(700, 250)
            }]
        );
        control.resize(A, size(1, 1));
        assert_eq!(
            control.apply_pending(),
            [
                ResizeSession { size: size(2, 1) },
                ResizeHost {
                    conn: A,
                    host: size(1, 1)
                }
            ]
        );
    }

    #[test]
    fn a_resize_before_readiness_replaces_the_restoration_within_the_period() {
        let mut control = ordinary(host());
        let first = granted(control.attach(A, host()));
        control.apply_pending();
        control.restored(A, first);
        control.resize(A, size(100, 30));
        assert_eq!(control.controlling(A), controlling(1, 1, Phase::Replacing));
        control.restored(A, first);
        assert_eq!(control.controlling(A), controlling(1, 1, Phase::Replacing));
        assert_eq!(control.ready(A, first.restoration), NONE);
        assert_eq!(
            control.apply_pending(),
            [
                ResizeSession {
                    size: size(100, 30)
                },
                Restore {
                    conn: A,
                    authority: authority(1, 2),
                    host: size(100, 30)
                }
            ]
        );
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.restore_deadline(A, first), NONE);
        assert_eq!(control.ready(A, Restoration(2)), NONE);
        restore(&mut control, A, authority(1, 2));
    }

    #[test]
    fn a_host_only_resize_before_readiness_replaces_the_restoration() {
        let mut control = ordinary(size(600, 300));
        assert_eq!(
            control.attach(A, size(600, 300)),
            [Admit { conn: A }, grant(A, (1, 1), size(600, 300), Attach)]
        );
        assert_eq!(control.apply_pending(), NONE);
        control.resize(A, size(700, 300));
        assert_eq!(
            control.apply_pending(),
            [Restore {
                conn: A,
                authority: authority(1, 2),
                host: size(700, 300)
            }]
        );
    }

    #[test]
    fn a_resize_to_the_restored_size_before_readiness_keeps_the_restoration() {
        let mut control = ordinary(host());
        let authority = granted(control.attach(A, host()));
        control.apply_pending();
        control.resize(A, host());
        assert_eq!(control.apply_pending(), NONE);
        restore(&mut control, A, authority);
    }

    #[test]
    fn a_non_controller_resize_keeps_the_restoring_controllers_restoration() {
        let mut control = session(&[A]);
        let authority = granted(control.attach(B, host()));
        control.apply_pending();
        control.resize(A, size(100, 30));
        assert_eq!(control.apply_pending(), NONE);
        restore(&mut control, B, authority);
    }

    #[test]
    fn resizes_outside_an_attached_session_are_ignored() {
        let mut control = ordinary(host());
        control.resize(A, size(100, 30));
        assert_eq!(control.apply_pending(), NONE);
        let mut control = session(&[A]);
        control.kill(K);
        control.resize(A, size(100, 30));
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.session_size(), host());
    }

    #[test]
    fn desired_focus_is_known_for_the_control_period_and_reported_when_it_changes() {
        let mut control = session(&[A, B]);
        assert_eq!(control.desired_focus(), None);
        assert!(!control.focus(A, true));
        assert_eq!(control.desired_focus(), None);
        assert!(control.focus(B, true));
        assert!(!control.focus(B, true));
        assert!(control.focus(B, false));
        assert_eq!(control.desired_focus(), Some(false));
        control.resize(B, size(100, 30));
        control.apply_pending();
        assert_eq!(control.desired_focus(), Some(false));
        let authority = granted(control.take_control(A));
        assert_eq!(control.desired_focus(), None);
        assert!(control.focus(A, false));
        control.resize(A, size(90, 30));
        assert!(matches!(
            control.apply_pending()[..],
            [ResizeSession { .. }, Restore { .. }]
        ));
        assert_eq!(control.desired_focus(), Some(false));
        assert_eq!(
            control.controlling(A),
            controlling(authority.period.0, 4, RESTORING)
        );
    }

    #[test]
    fn a_restoration_deadline_loses_the_restoring_controller() {
        let mut control = session(&[A]);
        let authority = granted(control.attach(B, host()));
        control.restored(B, authority);
        assert_eq!(
            control.restore_deadline(B, authority),
            [Disconnect { conn: B }, grant(A, (3, 3), host(), HandBack)]
        );
        assert_eq!(control.snapshot(), state(Attached, 1, true));
    }

    #[test]
    fn a_restoration_deadline_of_the_only_attachment_begins_shutdown() {
        let mut control = ordinary(host());
        let authority = granted(control.attach(A, host()));
        assert_eq!(
            control.restore_deadline(A, authority),
            [Disconnect { conn: A }, BeginShutdown]
        );
        assert_eq!(control.snapshot(), state(Terminating, 0, false));
    }

    #[test]
    fn a_stale_restoration_deadline_is_ignored() {
        assert_eq!(ordinary(host()).restore_deadline(A, authority(1, 1)), NONE);
        let mut control = session(&[A]);
        assert_eq!(control.restore_deadline(A, authority(1, 1)), NONE);
        let current = granted(control.attach(B, host()));
        assert_eq!(control.restore_deadline(A, current), NONE);
        assert_eq!(control.restore_deadline(B, authority(1, 1)), NONE);
        assert_eq!(control.restore_deadline(B, authority(2, 1)), NONE);
        assert_eq!(control.snapshot(), state(Attached, 2, true));
    }

    #[test]
    fn a_kill_ends_admission_and_begins_shutdown() {
        let mut control = session(&[A, B]);
        assert_eq!(
            control.kill(K),
            [DiscardPendingInput { conn: B }, ClosePaste, BeginShutdown]
        );
        assert!(!control.admits_input(B));
        assert_eq!(control.snapshot(), state(Terminating, 2, true));
    }

    #[test]
    fn a_kill_without_attachments_begins_shutdown() {
        assert_eq!(ordinary(host()).kill(K), [BeginShutdown]);
        let mut control = session(&[A]);
        control.detach(A);
        assert_eq!(control.kill(K), [BeginShutdown]);
        assert_eq!(control.snapshot(), state(Terminating, 0, false));
    }

    #[test]
    fn shutdown_refuses_attachments_and_ignores_control_requests() {
        let mut control = session(&[A, B]);
        control.focus(B, true);
        control.kill(K);
        assert_eq!(control.attach(C, host()), [Refuse { conn: C }]);
        assert_eq!(control.detach(B), NONE);
        assert_eq!(control.detach(A), NONE);
        assert_eq!(control.take_control(A), NONE);
        assert_eq!(control.ready(B, Restoration(2)), NONE);
        assert_eq!(control.restore_deadline(B, authority(2, 2)), NONE);
        assert_eq!(control.initial_deadline(), NONE);
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.kill(K2), NONE);
        assert!(!control.focus(B, false));
        assert_eq!(control.desired_focus(), None);
        assert_eq!(control.snapshot(), state(Terminating, 2, true));
    }
    /// Every kill gets the command's signal itself; attachments get the
    /// private protocol's status, 128 plus the signal's number.
    #[test]
    fn shutdown_releases_the_presenting_attachment_and_confirms_every_kill() {
        let mut control = session(&[A, B]);
        control.kill(K);
        control.kill(K2);
        assert_eq!(control.command_exited(signal(9)), [Cleanup]);
        assert_eq!(control.snapshot(), state(Terminating, 2, true));
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: B },
                DeliverExit { conn: B, code: 137 },
                DeliverExit { conn: A, code: 137 },
                confirmed(K, ended(signal(9))),
                confirmed(K2, ended(signal(9))),
                Finish
            ]
        );
        assert_eq!(control.snapshot(), state(Terminating, 0, false));
    }

    #[test]
    fn departures_during_shutdown_get_no_result_and_no_hand_back() {
        let mut control = session(&[A, B, C]);
        control.kill(K);
        control.kill(K2);
        assert_eq!(control.lost(B), NONE);
        assert_eq!(control.lost(K), NONE);
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.snapshot(), state(Terminating, 2, true));
        control.command_exited(exit(0));
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: C },
                DeliverExit { conn: C, code: 0 },
                DeliverExit { conn: A, code: 0 },
                confirmed(K2, ended(exit(0))),
                Finish
            ]
        );
    }

    #[test]
    fn losing_the_presenting_attachment_during_shutdown_hands_nothing_back() {
        let mut control = session(&[A, B]);
        control.kill(K);
        assert_eq!(control.lost(B), NONE);
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.snapshot(), state(Terminating, 1, true));
        control.command_exited(exit(0));
        assert_eq!(
            control.cleanup_finished(),
            [
                DeliverExit { conn: A, code: 0 },
                confirmed(K, ended(exit(0))),
                Finish
            ]
        );
    }

    #[test]
    fn a_command_exit_releases_and_delivers_exit_to_every_attachment() {
        let mut control = session(&[A, B]);
        assert_eq!(
            control.command_exited(exit(3)),
            [DiscardPendingInput { conn: B }, ClosePaste, Cleanup]
        );
        assert_eq!(control.attach(C, host()), [Refuse { conn: C }]);
        assert_eq!(control.snapshot(), state(Terminating, 2, true));
        assert_eq!(control.command_exited(exit(4)), NONE);
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: B },
                DeliverExit { conn: B, code: 3 },
                DeliverExit { conn: A, code: 3 },
                Finish
            ]
        );
        assert_eq!(control.cleanup_finished(), NONE);
    }

    #[test]
    fn a_command_exit_before_any_attach_finishes_after_cleanup() {
        let mut control = ordinary(host());
        assert_eq!(control.cleanup_finished(), NONE);
        assert_eq!(control.command_exited(exit(0)), [Cleanup]);
        assert_eq!(control.initial_deadline(), NONE);
        assert_eq!(control.cleanup_finished(), [Finish]);
    }

    #[test]
    fn cleanup_waits_for_the_command_to_exit() {
        let mut control = session(&[A]);
        control.kill(K);
        assert_eq!(control.cleanup_finished(), NONE);
        assert_eq!(control.command_exited(signal(15)), [Cleanup]);
    }

    #[test]
    fn a_kill_during_cleanup_is_confirmed_after_it() {
        let mut control = session(&[A]);
        control.command_exited(exit(5));
        assert_eq!(control.kill(K), NONE);
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: A },
                DeliverExit { conn: A, code: 5 },
                confirmed(K, ended(exit(5))),
                Finish
            ]
        );
    }

    #[test]
    fn a_kill_after_the_session_finished_is_confirmed_immediately() {
        let mut control = session(&[A]);
        control.command_exited(exit(5));
        control.cleanup_finished();
        assert_eq!(control.kill(K), [confirmed(K, ended(exit(5)))]);
        assert_eq!(control.attach(C, host()), [Refuse { conn: C }]);
    }

    #[test]
    fn an_authoritative_failure_ends_with_errors_for_everyone() {
        let mut control = session(&[A, B]);
        assert_eq!(
            control.authoritative_failure(),
            [DiscardPendingInput { conn: B }, ClosePaste, BeginShutdown]
        );
        assert_eq!(control.attach(C, host()), [Refuse { conn: C }]);
        assert_eq!(control.kill(K), NONE);
        assert_eq!(control.command_exited(signal(1)), [Cleanup]);
        let failed = Outcome {
            failed: true,
            ..ended(signal(1))
        };
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: B },
                DeliverError { conn: B },
                DeliverError { conn: A },
                confirmed(K, failed.clone()),
                Finish
            ]
        );
        assert_eq!(control.kill(K2), [confirmed(K2, failed)]);
        assert_eq!(control.authoritative_failure(), NONE);
    }

    #[test]
    fn an_authoritative_failure_during_shutdown_turns_the_result_into_errors() {
        let mut control = session(&[A]);
        control.kill(K);
        assert_eq!(control.authoritative_failure(), NONE);
        control.command_exited(exit(0));
        let failed = Outcome {
            failed: true,
            ..ended(exit(0))
        };
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: A },
                DeliverError { conn: A },
                confirmed(K, failed),
                Finish
            ]
        );
    }

    /// A failure while cleaning up leaves the cleanup incomplete, distinct
    /// from a failure of the session, though the private protocol reports
    /// both as `Error`.
    #[test]
    fn a_failed_cleanup_never_confirms_a_kill() {
        let mut control = session(&[A]);
        control.kill(K);
        control.command_exited(exit(0));
        let uncleaned = Outcome {
            cleaned: false,
            ..ended(exit(0))
        };
        assert_eq!(
            control.authoritative_failure(),
            [
                Release { conn: A },
                DeliverError { conn: A },
                confirmed(K, uncleaned),
                Finish
            ]
        );
        assert_eq!(control.cleanup_finished(), NONE);
        assert_eq!(control.snapshot(), state(Terminating, 0, false));
    }

    /// A command end the daemon could not confirm: the private protocol
    /// reports climon2's own status, and a kill learns what was known.
    #[test]
    fn an_unconfirmed_command_end_reports_climon2s_own_status() {
        let mut control = session(&[A]);
        control.kill(K);
        let unconfirmed = CommandEnd::Unconfirmed(None);
        assert_eq!(control.command_exited(unconfirmed.clone()), [Cleanup]);
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: A },
                DeliverExit { conn: A, code: 125 },
                confirmed(K, ended(unconfirmed)),
                Finish
            ]
        );
    }

    /// The private protocol keeps its exit mapping: a normal exit's raw
    /// status, 128 plus a signal's number, 125 when nothing was confirmed,
    /// and an error after a failure of the session or of its cleanup.
    #[test]
    fn the_private_protocol_keeps_its_exit_mapping() {
        assert_eq!(ended(exit(0)).private_code(), Some(0));
        assert_eq!(ended(exit(143)).private_code(), Some(143));
        assert_eq!(ended(signal(15)).private_code(), Some(143));
        let known = Some(CommandResult::Exit { code: 7 });
        let unconfirmed = CommandEnd::Unconfirmed(known.clone());
        assert_eq!(unconfirmed.known(), known.as_ref());
        assert_eq!(ended(unconfirmed).private_code(), Some(125));
        assert_eq!(
            ended(CommandEnd::Unconfirmed(None)).private_code(),
            Some(125)
        );
        assert_eq!(exit(3).known(), Some(&CommandResult::Exit { code: 3 }));
        let failed = Outcome {
            failed: true,
            ..ended(exit(0))
        };
        assert_eq!(failed.private_code(), None);
        let uncleaned = Outcome {
            cleaned: false,
            ..ended(exit(0))
        };
        assert_eq!(uncleaned.private_code(), None);
    }

    #[test]
    fn an_authoritative_failure_before_any_attach_begins_shutdown() {
        let mut control = ordinary(host());
        assert_eq!(control.authoritative_failure(), [BeginShutdown]);
        assert_eq!(control.snapshot(), state(Terminating, 0, false));
    }

    #[test]
    fn the_initial_deadline_terminates_a_session_nobody_attached_to() {
        let mut control = ordinary(host());
        assert_eq!(control.initial_deadline(), [BeginShutdown]);
        assert_eq!(control.snapshot(), state(Terminating, 0, false));
        assert_eq!(control.initial_deadline(), NONE);
    }

    #[test]
    fn a_kill_joining_an_initial_deadline_shutdown_is_confirmed() {
        let mut control = ordinary(host());
        control.initial_deadline();
        assert_eq!(control.kill(K), NONE);
        control.command_exited(signal(1));
        assert_eq!(
            control.cleanup_finished(),
            [confirmed(K, ended(signal(1))), Finish]
        );
    }

    #[test]
    fn a_kill_joining_a_final_loss_shutdown_is_confirmed() {
        let mut control = session(&[A]);
        control.lost(A);
        assert_eq!(control.kill(K), NONE);
        control.command_exited(signal(1));
        assert_eq!(
            control.cleanup_finished(),
            [confirmed(K, ended(signal(1))), Finish]
        );
    }

    #[test]
    fn the_initial_deadline_is_void_once_attached() {
        let mut control = session(&[A]);
        assert_eq!(control.initial_deadline(), NONE);
        assert_eq!(control.snapshot(), state(Attached, 1, true));
    }
}
