//! The pure control state machine of one session.
//!
//! It owns the lifecycle, the attachments, the controller and its hand-back
//! recency order, the passive monitor connections that may inherit control,
//! the control period, the restoration and readiness, the controller's
//! desired application focus, each attachment's latest host size, the
//! applied session geometry, the end of input admission and the shutdown
//! sequence, and it authorizes requests against them. It performs no I/O:
//! each method takes one event and returns the effects the daemon loop must
//! perform, in order.
//!
//! Call protocol for one loop iteration:
//!
//! 1. Feed every event received this iteration to its method, in acceptance
//!    order, and perform the effects each returns. Every attach, takeover,
//!    hand-back and promotion is granted at once: its [`Effect::Grant`] names
//!    the new authority. [`Control::resize`] only records a pending size.
//! 2. Call [`Control::apply_pending`] exactly once, before reading the PTY and
//!    before any snapshot for a grant. It applies the controller's latest size
//!    at most once and replaces at most one restoration.
//! 3. Report each restoration whose every byte was sent with
//!    [`Control::restored`].
//! 4. Admit an attachment's application input only while
//!    [`Control::admits_input`] holds for it.

mod attachments;
mod shutdown;

use attachments::{Attachment, Attachments, Departure, Phase};
pub use attachments::{Needs, Target};
use climon2_core::{
    lifetime::Lifetime,
    protocol::TerminalSize,
    wire::{GrantReason, OperationalCode},
};
pub use shutdown::{CleanupResult, CommandEnd, SessionEnd};
use shutdown::{Members, Shutdown};

/// Identifies one daemon connection: an attachment, a passive monitor
/// connection, a probe or a kill requester.
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
    /// An attachment was admitted: answer its Open.
    Admit { conn: ConnectionId },
    /// A passive monitor connection became an attachment, by its Acquire or
    /// a promotion: its output is canonical from its grant, which follows.
    Convert { conn: ConnectionId },
    /// `conn` controls under `authority`, a new period and its first
    /// restoration, for `reason`: report the grant, then, once the pending
    /// size is applied, snapshot for a host of size `host` (a canonical
    /// attachment's is the session geometry) and send the restoration.
    Grant {
        conn: ConnectionId,
        authority: Authority,
        host: TerminalSize,
        reason: GrantReason,
    },
    /// The size the controller's restoration is serialized for changed,
    /// before readiness or, for a canonical controller, at a new session
    /// geometry: its current restoration ends now, and `authority` replaces
    /// it within the same period, snapshotted for a host of size `host`.
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
    /// A takeover ended this attachment's control: report the revocation;
    /// send no further session output to this attachment.
    Revoke { conn: ConnectionId },
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
    /// The session ended: this attachment's control ends with it.
    EndAttachment { conn: ConnectionId },
    /// Answer a kill requester with how the session ended.
    ConfirmKill { conn: ConnectionId, end: SessionEnd },
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

/// The most recently issued period and restoration. Both only increase and
/// never wrap: once either has issued its largest value, work that needs a
/// new one is refused before its effects (`resource_limit`).
#[derive(Clone, Copy, Debug, Default)]
struct Issued {
    period: Period,
    restoration: Restoration,
}

/// A new period and its first restoration, not yet issued. Only
/// [`Issued::next_grant`] makes one, so a grant never reuses a counter.
struct Fresh(Authority);

impl Issued {
    /// The authority of a new grant, unless a counter is exhausted.
    fn next_grant(self) -> Option<Fresh> {
        Some(Fresh(Authority {
            period: Period(self.period.0.checked_add(1)?),
            restoration: Restoration(self.restoration.0.checked_add(1)?),
        }))
    }

    /// Issues a new period and its first restoration.
    fn grant(&mut self, Fresh(authority): Fresh) -> Authority {
        self.period = authority.period;
        self.restoration = authority.restoration;
        authority
    }

    /// Whether a new restoration can still be issued.
    fn can_restore(self) -> bool {
        self.restoration.0.checked_add(1).is_some()
    }

    /// A new restoration within `period`. Only a resize that
    /// [`Issued::can_restore`] admitted needs one, and nothing else issues
    /// a restoration before it is applied.
    fn restore(&mut self, period: Period) -> Authority {
        self.restoration.0 = (self.restoration.0.checked_add(1))
            .expect("a resize needing a restoration is admitted only while one remains");
        Authority {
            period,
            restoration: self.restoration,
        }
    }
}

/// Session-wide state outside the stage that grants draw on: the authority
/// issued last and the applied sizes, whose session geometry every grant of
/// a canonical attachment keeps.
struct Grants {
    issued: Issued,
    sizes: Sizes,
}

/// The passive monitor connections that may inherit control, in the order
/// their Opens were accepted.
#[derive(Default)]
struct Registrations(Vec<ConnectionId>);

impl Registrations {
    fn register(&mut self, conn: ConnectionId) {
        self.0.push(conn);
    }

    /// Ends `conn`'s registration; true when it had one.
    fn remove(&mut self, conn: ConnectionId) -> bool {
        let registered = self.0.len();
        self.0.retain(|other| *other != conn);
        self.0.len() != registered
    }

    /// Ends the newest registration, the one a promotion picks.
    fn newest(&mut self) -> Option<ConnectionId> {
        self.0.pop()
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

    /// An accepted attach request: admitted, then granted at once.
    fn attach(&mut self, attachment: Attachment, fresh: Fresh, grants: &mut Grants) -> Vec<Effect> {
        let admit = Effect::Admit {
            conn: attachment.conn,
        };
        let grant = self.grant(attachment, GrantReason::Attach, fresh, grants);
        [vec![admit], grant].concat()
    }

    /// `attachment` takes control at once for `reason`; a previous
    /// controller stays, non-controlling.
    fn grant(
        &mut self,
        attachment: Attachment,
        reason: GrantReason,
        fresh: Fresh,
        grants: &mut Grants,
    ) -> Vec<Effect> {
        match self {
            Self::Attached(attachments) => attachments.grant(attachment, reason, fresh, grants),
            _ => {
                let (attachments, grant) = Attachments::new(attachment, reason, fresh, grants);
                *self = Self::Attached(attachments);
                vec![grant]
            }
        }
    }

    /// A non-controlling attachment takes control at once.
    fn take_control(
        &mut self,
        conn: ConnectionId,
        fresh: Fresh,
        grants: &mut Grants,
    ) -> Vec<Effect> {
        match self {
            Self::Attached(attachments) => attachments.take_control(conn, fresh, grants),
            _ => Vec::new(),
        }
    }

    fn into_members(self) -> (Members, Vec<Effect>) {
        match self {
            Self::Attached(attachments) => attachments.into_members(),
            _ => (Members::default(), Vec::new()),
        }
    }
}

fn begin_shutdown(open: Open, killers: Vec<ConnectionId>) -> (Stage, Vec<Effect>) {
    let (members, mut effects) = open.into_members();
    effects.push(Effect::BeginShutdown);
    let shutdown = Shutdown::begin(members, killers);
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
        Open::Starting => begin_shutdown(open, Vec::new()),
        _ => (Stage::Open(open), Vec::new()),
    }
}

/// Acquire: a non-controlling attachment takes control at once, and so
/// does a passive monitor connection, which becomes an attachment; the
/// controller's own changes nothing.
fn acquire(
    mut open: Open,
    conn: ConnectionId,
    fresh: Fresh,
    grants: &mut Grants,
    registrations: &mut Registrations,
) -> (Stage, Vec<Effect>) {
    let effects = if registrations.remove(conn) {
        convert(&mut open, conn, GrantReason::Acquire, fresh, grants)
    } else {
        open.take_control(conn, fresh, grants)
    };
    (Stage::Open(open), effects)
}

/// With no attachment left, the newest registration becomes one, with a
/// successor's grant, unless no new period can be issued: then every
/// registration stays one.
fn promote(open: &mut Open, grants: &mut Grants, registrations: &mut Registrations) -> Vec<Effect> {
    let successor = match (&open, grants.issued.next_grant()) {
        (Open::Detached, Some(fresh)) => registrations.newest().zip(Some(fresh)),
        _ => None,
    };
    successor
        .map(|(conn, fresh)| convert(open, conn, GrantReason::Successor, fresh, grants))
        .unwrap_or_default()
}

/// The passive monitor connection `conn`, no longer registered, becomes a
/// monitor-mode attachment: canonical, at the session geometry, and in
/// control at once for `reason`.
fn convert(
    open: &mut Open,
    conn: ConnectionId,
    reason: GrantReason,
    fresh: Fresh,
    grants: &mut Grants,
) -> Vec<Effect> {
    let attachment = Attachment {
        conn,
        host: grants.sizes.session,
        target: Target::Canonical,
    };
    let grant = open.grant(attachment, reason, fresh, grants);
    [vec![Effect::Convert { conn }], grant].concat()
}

/// The leave policy, after succession: a hand-back, or else the promotion
/// of the newest passive monitor connection. Only the unexpected loss of an
/// ordinary session's last attachment then ends it; a headless session
/// survives with zero clients.
fn depart(
    lifetime: Lifetime,
    open: Open,
    conn: ConnectionId,
    departure: Departure,
    grants: &mut Grants,
    registrations: &mut Registrations,
) -> (Stage, Vec<Effect>) {
    let Open::Attached(attachments) = open else {
        return (Stage::Open(open), Vec::new());
    };
    let (mut open, mut effects) = attachments.depart(conn, departure, grants);
    effects.extend(promote(&mut open, grants, registrations));
    match (open, departure, lifetime) {
        (Open::Detached, Departure::Lost, Lifetime::Ordinary) => {
            let (stage, shutdown) = begin_shutdown(Open::Detached, Vec::new());
            (stage, [effects, shutdown].concat())
        }
        (open, _, _) => (Stage::Open(open), effects),
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
    grants: Grants,
    registrations: Registrations,
    lifetime: Lifetime,
}

impl Control {
    /// A new session whose PTY has the session geometry for `initial`.
    pub fn new(initial: TerminalSize, lifetime: Lifetime) -> Self {
        let host = initial.initial();
        let sizes = Sizes {
            host,
            session: geometry(host),
        };
        Self {
            stage: Stage::Open(Open::unattached(lifetime)),
            grants: Grants {
                issued: Issued::default(),
                sizes,
            },
            registrations: Registrations::default(),
            lifetime,
        }
    }

    /// Test hooks only: the session starts as if `last` were the control
    /// period and restoration issued last, to bring them near exhaustion.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn issued_after(mut self, last: Authority) -> Self {
        self.grants.issued = Issued {
            period: last.period,
            restoration: last.restoration,
        };
        self
    }

    /// The applied session geometry: the PTY and virtual terminal size.
    pub fn session_size(&self) -> TerminalSize {
        self.grants.sizes.session
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

    /// An attach request: admitted and granted at once, or refused without
    /// effects because the session is terminating (`session_terminating`)
    /// or no new control period and restoration can be issued
    /// (`resource_limit`). Each connection attaches at most once.
    pub fn attach(
        &mut self,
        conn: ConnectionId,
        size: TerminalSize,
    ) -> Result<Vec<Effect>, OperationalCode> {
        let attachment = Attachment {
            conn,
            host: size.initial(),
            target: Target::Host,
        };
        let Stage::Open(open) = &mut self.stage else {
            return Err(OperationalCode::SessionTerminating);
        };
        let fresh = (self.grants.issued.next_grant()).ok_or(OperationalCode::ResourceLimit)?;
        Ok(open.attach(attachment, fresh, &mut self.grants))
    }

    /// A passive monitor connection registered: never an attachment, nor
    /// in control, until its Acquire or a promotion.
    pub fn register(&mut self, conn: ConnectionId) {
        self.registrations.register(conn);
    }

    /// Acquire: a non-controlling attachment takes control at once, and so
    /// does a passive monitor connection, which becomes an attachment; the
    /// controller's own changes nothing. Committed termination refuses it,
    /// and so does the lack of a new period for anyone but the controller.
    /// Its outcome, then its effects.
    pub fn acquire(
        &mut self,
        conn: ConnectionId,
    ) -> (Result<Acquisition, OperationalCode>, Vec<Effect>) {
        if let Stage::Terminating(_) = self.stage {
            return (Err(OperationalCode::SessionTerminating), Vec::new());
        }
        match self.take(conn) {
            Ok(effects) => (self.acquisition(conn, &effects), effects),
            Err(code) => (Err(code), Vec::new()),
        }
    }

    /// The effects of `conn` taking control: none for the controller, and
    /// otherwise a grant under a fresh authority, unless none can be issued.
    fn take(&mut self, conn: ConnectionId) -> Result<Vec<Effect>, OperationalCode> {
        if self.controlling(conn).is_ok() {
            return Ok(Vec::new());
        }
        let fresh = (self.grants.issued.next_grant()).ok_or(OperationalCode::ResourceLimit)?;
        Ok(self
            .open(|open, grants, registrations| acquire(open, conn, fresh, grants, registrations)))
    }

    /// The authority `conn` holds after its Acquire, and whether it changed.
    fn acquisition(
        &self,
        conn: ConnectionId,
        effects: &[Effect],
    ) -> Result<Acquisition, OperationalCode> {
        self.controlling(conn).map(|controlling| Acquisition {
            changed: !effects.is_empty(),
            authority: controlling.authority,
        })
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

    /// Records the controller's latest reported viewing for its period;
    /// true when it changed.
    pub fn set_viewing(&mut self, conn: ConnectionId, viewing: bool) -> bool {
        attached(&mut self.stage)
            .is_some_and(|attachments| attachments.controller.set_viewing(conn, viewing))
    }

    /// Whether the current controller's latest reported viewing is true.
    pub fn viewing(&self) -> bool {
        matches!(
            &self.stage,
            Stage::Open(Open::Attached(attachments)) if attachments.controller.viewing()
        )
    }

    /// Records a pending host size; [`Control::apply_pending`] applies it.
    /// A size that would need a new restoration when none can be issued is
    /// refused (`resource_limit`) and not recorded.
    pub fn resize(
        &mut self,
        conn: ConnectionId,
        size: TerminalSize,
    ) -> Result<(), OperationalCode> {
        let issued = self.grants.issued;
        if let (Some(host), Some(attachments)) = (size.resize(), attached(&mut self.stage)) {
            attachments.resize(conn, host, issued)?;
        }
        Ok(())
    }

    /// An attachment's accepted Leave while the session is open.
    fn detach(&mut self, conn: ConnectionId) -> Vec<Effect> {
        let lifetime = self.lifetime;
        self.open(|open, grants, registrations| {
            depart(
                lifetime,
                open,
                conn,
                Departure::Detach,
                grants,
                registrations,
            )
        })
    }

    /// An accepted departure: a passive monitor connection's registration
    /// ends, and an attachment's departure is a detach, which a terminating
    /// session accepts too, releasing a presentation the attachment still
    /// has.
    pub fn leave(&mut self, conn: ConnectionId) -> Vec<Effect> {
        self.unregister(conn);
        match &mut self.stage {
            Stage::Terminating(shutdown) => shutdown.leave(conn),
            Stage::Open(_) => self.detach(conn),
        }
    }

    /// The connection closed or failed without an accepted detach.
    pub fn lost(&mut self, conn: ConnectionId) -> Vec<Effect> {
        self.unregister(conn);
        if let Stage::Terminating(shutdown) = &mut self.stage {
            shutdown.lost(conn);
        }
        let lifetime = self.lifetime;
        self.open(|open, grants, registrations| {
            depart(lifetime, open, conn, Departure::Lost, grants, registrations)
        })
    }

    /// A departing connection is removed before any succession. A
    /// connection lost inside output routing is removed when it is marked,
    /// before its loss is applied, so no departure in between promotes it.
    pub fn unregister(&mut self, conn: ConnectionId) {
        self.registrations.remove(conn);
    }

    pub fn kill(&mut self, conn: ConnectionId) -> Vec<Effect> {
        match &mut self.stage {
            Stage::Terminating(shutdown) => shutdown.kill(conn),
            Stage::Open(_) => self.open(|open, _, _| begin_shutdown(open, vec![conn])),
        }
    }

    pub fn initial_deadline(&mut self) -> Vec<Effect> {
        self.open(|open, _, _| initial_deadline(open))
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
            Stage::Open(_) => self.open(|open, _, _| command_exited(open, command)),
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
            Stage::Open(_) => self.open(|open, _, _| begin_shutdown(open, Vec::new())),
        }
    }

    /// Applies the controller's latest pending size once, then replaces a
    /// restoration that size invalidated.
    pub fn apply_pending(&mut self) -> Vec<Effect> {
        let Some(attachments) = attached(&mut self.stage) else {
            return Vec::new();
        };
        let applied = self.grants.sizes.apply(attachments.controller.host());
        let resized = applied.is_some();
        let mut effects = applied.unwrap_or_default();
        effects.extend(
            attachments
                .controller
                .settle(resized, &mut self.grants.issued),
        );
        effects
    }

    /// Runs a transition of an open session; a terminating session ignores it.
    fn open(
        &mut self,
        transition: impl FnOnce(Open, &mut Grants, &mut Registrations) -> (Stage, Vec<Effect>),
    ) -> Vec<Effect> {
        let (stage, effects) = match std::mem::replace(&mut self.stage, Stage::Open(Open::Starting))
        {
            Stage::Open(open) => transition(open, &mut self.grants, &mut self.registrations),
            terminating => (terminating, Vec::new()),
        };
        self.stage = stage;
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::Effect::*;
    use super::GrantReason::{Acquire, Attach, HandBack, Successor};
    use super::Lifecycle::*;
    use super::*;
    use climon2_core::wire::CommandResult;

    const A: ConnectionId = ConnectionId(1);
    const B: ConnectionId = ConnectionId(2);
    const C: ConnectionId = ConnectionId(3);
    const K: ConnectionId = ConnectionId(10);
    const K2: ConnectionId = ConnectionId(11);
    /// Passive monitor connections.
    const M: ConnectionId = ConnectionId(20);
    const N: ConnectionId = ConnectionId(21);
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

    /// A session that ended with `command` and cleaned up.
    fn ended(command: CommandEnd) -> SessionEnd {
        SessionEnd {
            command,
            cleanup: CleanupResult::Complete,
        }
    }

    /// The largest counter value; none follows it.
    const LAST: u64 = u64::MAX;

    /// An ordinary 80x24 session whose last issued control period and
    /// restoration are `period` and `restoration`, as the test hook makes it.
    fn issued(period: u64, restoration: u64) -> Control {
        ordinary(host()).issued_after(authority(period, restoration))
    }

    /// A ready controller's departure: its admission ends.
    fn ended_admission(conn: ConnectionId) -> [Effect; 2] {
        [DiscardPendingInput { conn }, ClosePaste]
    }

    fn confirmed(conn: ConnectionId, end: SessionEnd) -> Effect {
        ConfirmKill { conn, end }
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
            let authority = granted(control.attach(*conn, host()).unwrap());
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
            control.attach(A, host()).unwrap(),
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
            control.attach(A, size(100, 30)).unwrap(),
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
            control.attach(A, size(0, 0)).unwrap(),
            [Admit { conn: A }, grant(A, (1, 1), host(), Attach)]
        );
        assert_eq!(control.apply_pending(), [ResizeSession { size: host() }]);
    }

    #[test]
    fn input_is_admitted_only_after_ready_for_the_complete_current_restoration() {
        let mut control = ordinary(host());
        let authority = granted(control.attach(A, host()).unwrap());
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
        let current = granted(control.attach(A, host()).unwrap());
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
        let authority = granted(control.attach(B, host()).unwrap());
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
            control.attach(B, host()).unwrap(),
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
        control.attach(A, host()).unwrap();
        control.apply_pending();
        assert_eq!(
            control.attach(B, host()).unwrap(),
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
            control.attach(A, host()).unwrap(),
            [Admit { conn: A }, grant(A, (1, 1), host(), Attach)]
        );
        assert_eq!(
            control.attach(B, host()).unwrap(),
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
    fn an_acquire_moves_control_to_the_requester_in_a_new_period() {
        let mut control = session(&[A, B]);
        assert_eq!(
            control.acquire(A).1,
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
    fn an_acquire_by_the_controller_is_a_no_op() {
        let mut control = session(&[A]);
        assert_eq!(control.acquire(A).1, NONE);
        assert_eq!(control.apply_pending(), NONE);
        assert!(control.admits_input(A));
        assert_eq!(control.controlling(A), controlling(1, 1, Phase::Ready));
    }

    #[test]
    fn an_acquire_from_an_unknown_connection_changes_nothing() {
        assert_eq!(ordinary(host()).acquire(C).1, NONE);
        let mut control = session(&[A]);
        assert_eq!(control.acquire(C).1, NONE);
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

        let restoring = granted(control.attach(C, host()).unwrap());
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
        let authority = granted(control.acquire(A).1);
        restore(&mut control, A, authority);
        assert_eq!(
            control.detach(A),
            [
                DiscardPendingInput { conn: A },
                ClosePaste,
                Release { conn: A },
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
        control.resize(A, size(120, 40)).unwrap();
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
        assert_eq!(control.detach(A), NONE);
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
        let authority = granted(control.attach(A, size(100, 30)).unwrap());
        control.apply_pending();
        restore(&mut control, A, authority);
        assert_eq!(
            control.detach(A),
            [
                DiscardPendingInput { conn: A },
                ClosePaste,
                Release { conn: A },
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
            control.attach(B, host()).unwrap(),
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
        assert_eq!(control.leave(A), NONE);
        assert_eq!(
            control.leave(B),
            [
                DiscardPendingInput { conn: B },
                ClosePaste,
                Release { conn: B },
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
        assert_eq!(control.leave(B), [Release { conn: B }]);
        assert_eq!(control.leave(A), NONE);
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
        control.resize(A, size(100, 30)).unwrap();
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
        control.resize(A, size(90, 30)).unwrap();
        control.resize(A, size(100, 40)).unwrap();
        control.resize(A, size(110, 50)).unwrap();
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
        control.resize(A, host()).unwrap();
        assert_eq!(control.apply_pending(), NONE);
        control.resize(A, size(0, 30)).unwrap();
        control.resize(A, size(100, 0)).unwrap();
        assert_eq!(control.apply_pending(), NONE);
    }

    #[test]
    fn a_non_controller_resize_is_only_remembered() {
        let mut control = session(&[A, B]);
        control.resize(A, size(100, 30)).unwrap();
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(
            control.acquire(A).1,
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
        control.resize(A, size(600, 300)).unwrap();
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
        control.resize(A, size(700, 250)).unwrap();
        assert_eq!(
            control.apply_pending(),
            [ResizeHost {
                conn: A,
                host: size(700, 250)
            }]
        );
        control.resize(A, size(1, 1)).unwrap();
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
        let first = granted(control.attach(A, host()).unwrap());
        control.apply_pending();
        control.restored(A, first);
        control.resize(A, size(100, 30)).unwrap();
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
            control.attach(A, size(600, 300)).unwrap(),
            [Admit { conn: A }, grant(A, (1, 1), size(600, 300), Attach)]
        );
        assert_eq!(control.apply_pending(), NONE);
        control.resize(A, size(700, 300)).unwrap();
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
        let authority = granted(control.attach(A, host()).unwrap());
        control.apply_pending();
        control.resize(A, host()).unwrap();
        assert_eq!(control.apply_pending(), NONE);
        restore(&mut control, A, authority);
    }

    #[test]
    fn a_non_controller_resize_keeps_the_restoring_controllers_restoration() {
        let mut control = session(&[A]);
        let authority = granted(control.attach(B, host()).unwrap());
        control.apply_pending();
        control.resize(A, size(100, 30)).unwrap();
        assert_eq!(control.apply_pending(), NONE);
        restore(&mut control, B, authority);
    }

    #[test]
    fn resizes_outside_an_attached_session_are_ignored() {
        let mut control = ordinary(host());
        control.resize(A, size(100, 30)).unwrap();
        assert_eq!(control.apply_pending(), NONE);
        let mut control = session(&[A]);
        control.kill(K);
        control.resize(A, size(100, 30)).unwrap();
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
        control.resize(B, size(100, 30)).unwrap();
        control.apply_pending();
        assert_eq!(control.desired_focus(), Some(false));
        let authority = granted(control.acquire(A).1);
        assert_eq!(control.desired_focus(), None);
        assert!(control.focus(A, false));
        control.resize(A, size(90, 30)).unwrap();
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
        let authority = granted(control.attach(B, host()).unwrap());
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
        let authority = granted(control.attach(A, host()).unwrap());
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
        let current = granted(control.attach(B, host()).unwrap());
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
        assert_eq!(
            control.attach(C, host()),
            Err(OperationalCode::SessionTerminating)
        );
        assert_eq!(control.detach(B), NONE);
        assert_eq!(control.detach(A), NONE);
        assert_eq!(control.acquire(A).1, NONE);
        assert_eq!(control.ready(B, Restoration(2)), NONE);
        assert_eq!(control.restore_deadline(B, authority(2, 2)), NONE);
        assert_eq!(control.initial_deadline(), NONE);
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.kill(K2), NONE);
        assert!(!control.focus(B, false));
        assert_eq!(control.desired_focus(), None);
        assert_eq!(control.snapshot(), state(Terminating, 2, true));
    }
    /// Every kill gets the command's signal itself, and every attachment's
    /// control ends with the session.
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
                EndAttachment { conn: B },
                EndAttachment { conn: A },
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
                EndAttachment { conn: C },
                EndAttachment { conn: A },
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
                EndAttachment { conn: A },
                confirmed(K, ended(exit(0))),
                Finish
            ]
        );
    }

    #[test]
    fn a_command_exit_releases_and_ends_every_attachment() {
        let mut control = session(&[A, B]);
        assert_eq!(
            control.command_exited(exit(3)),
            [DiscardPendingInput { conn: B }, ClosePaste, Cleanup]
        );
        assert_eq!(
            control.attach(C, host()),
            Err(OperationalCode::SessionTerminating)
        );
        assert_eq!(control.snapshot(), state(Terminating, 2, true));
        assert_eq!(control.command_exited(exit(4)), NONE);
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: B },
                EndAttachment { conn: B },
                EndAttachment { conn: A },
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
                EndAttachment { conn: A },
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
        assert_eq!(
            control.attach(C, host()),
            Err(OperationalCode::SessionTerminating)
        );
    }

    #[test]
    fn an_authoritative_failure_ends_every_attachment_and_confirms_kills() {
        let mut control = session(&[A, B]);
        assert_eq!(
            control.authoritative_failure(),
            [DiscardPendingInput { conn: B }, ClosePaste, BeginShutdown]
        );
        assert_eq!(
            control.attach(C, host()),
            Err(OperationalCode::SessionTerminating)
        );
        assert_eq!(control.kill(K), NONE);
        assert_eq!(control.command_exited(signal(1)), [Cleanup]);
        let failed = ended(signal(1));
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: B },
                EndAttachment { conn: B },
                EndAttachment { conn: A },
                confirmed(K, failed.clone()),
                Finish
            ]
        );
        assert_eq!(control.kill(K2), [confirmed(K2, failed)]);
        assert_eq!(control.authoritative_failure(), NONE);
    }

    /// A failure while the session awaits its command changes nothing: it
    /// already terminates, and its kill is confirmed once it cleaned up.
    #[test]
    fn an_authoritative_failure_awaiting_the_command_changes_nothing() {
        let mut control = session(&[A]);
        control.kill(K);
        assert_eq!(control.authoritative_failure(), NONE);
        control.command_exited(exit(0));
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: A },
                EndAttachment { conn: A },
                confirmed(K, ended(exit(0))),
                Finish
            ]
        );
    }

    /// A failure while cleaning up leaves the cleanup incomplete, which no
    /// kill confirms.
    #[test]
    fn a_failed_cleanup_never_confirms_a_kill() {
        let mut control = session(&[A]);
        control.kill(K);
        control.command_exited(exit(0));
        let uncleaned = SessionEnd {
            cleanup: CleanupResult::Failed,
            ..ended(exit(0))
        };
        assert_eq!(
            control.authoritative_failure(),
            [
                Release { conn: A },
                EndAttachment { conn: A },
                confirmed(K, uncleaned),
                Finish
            ]
        );
        assert_eq!(control.cleanup_finished(), NONE);
        assert_eq!(control.snapshot(), state(Terminating, 0, false));
    }

    /// A command end the daemon could not confirm: a kill learns what was
    /// known.
    #[test]
    fn an_unconfirmed_command_end_reaches_the_kill() {
        let mut control = session(&[A]);
        control.kill(K);
        let unconfirmed = CommandEnd::Unconfirmed(None);
        assert_eq!(control.command_exited(unconfirmed.clone()), [Cleanup]);
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: A },
                EndAttachment { conn: A },
                confirmed(K, ended(unconfirmed)),
                Finish
            ]
        );
    }

    /// The result a command end makes known, whether or not the daemon
    /// confirmed it.
    #[test]
    fn a_command_end_knows_its_result_whether_or_not_confirmed() {
        assert_eq!(exit(3).known(), Some(&CommandResult::Exit { code: 3 }));
        let known = Some(CommandResult::Exit { code: 7 });
        let unconfirmed = CommandEnd::Unconfirmed(known.clone());
        assert_eq!(unconfirmed.known(), known.as_ref());
        assert_eq!(CommandEnd::Unconfirmed(None).known(), None);
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

    /// The effects of a ready controller's deliberate detach, before
    /// succession.
    fn detached(conn: ConnectionId) -> [Effect; 3] {
        [DiscardPendingInput { conn }, ClosePaste, Release { conn }]
    }

    #[test]
    fn a_registration_acquires_only_by_its_own_acquire_at_the_session_geometry() {
        let mut control = ordinary(host());
        let first = granted(control.attach(A, size(100, 30)).unwrap());
        control.apply_pending();
        restore(&mut control, A, first);
        control.detach(A);
        control.register(M);
        assert_eq!(control.snapshot(), state(Detached, 0, false));
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(
            control.acquire(B),
            (Err(OperationalCode::NotAttached), Vec::new())
        );

        let (acquired, effects) = control.acquire(M);
        assert_eq!(
            effects,
            [
                Convert { conn: M },
                grant(M, (2, 2), size(100, 30), Acquire)
            ]
        );
        let changed = Acquisition {
            changed: true,
            authority: authority(2, 2),
        };
        assert_eq!(acquired, Ok(changed));
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.snapshot(), state(Attached, 1, true));
        let kept = Acquisition {
            changed: false,
            authority: authority(2, 2),
        };
        assert_eq!(control.acquire(M), (Ok(kept), Vec::new()));
    }

    #[test]
    fn a_departure_hands_back_first_then_promotes_the_newest_registration() {
        let mut control = session(&[A, B]);
        control.register(M);
        control.register(N);
        assert_eq!(
            control.lost(B),
            [
                DiscardPendingInput { conn: B },
                ClosePaste,
                grant(A, (3, 3), host(), HandBack)
            ]
        );
        restore(&mut control, A, authority(3, 3));
        let promoted = [Convert { conn: N }, grant(N, (4, 4), host(), Successor)];
        assert_eq!(control.detach(A), [&detached(A)[..], &promoted].concat());
        assert_eq!(control.snapshot(), state(Attached, 1, true));
        assert_eq!(
            control.lost(N),
            [Convert { conn: M }, grant(M, (5, 5), host(), Successor)]
        );
        assert_eq!(control.lost(M), [BeginShutdown]);
    }

    #[test]
    fn leave_and_loss_remove_a_registration_before_any_succession() {
        let mut control = session(&[A]);
        control.register(M);
        control.register(N);
        assert_eq!(control.leave(N), NONE);
        assert_eq!(control.lost(M), NONE);
        assert_eq!(control.detach(A), detached(A));
        assert_eq!(control.snapshot(), state(Detached, 0, false));
    }

    #[test]
    fn committed_termination_refuses_a_registrations_acquire_and_promotes_nobody() {
        let mut control = session(&[A]);
        control.register(M);
        control.kill(K);
        assert_eq!(
            control.acquire(M),
            (Err(OperationalCode::SessionTerminating), Vec::new())
        );
        assert_eq!(control.leave(A), [Release { conn: A }]);
        assert_eq!(control.snapshot(), state(Terminating, 0, false));
    }

    #[test]
    fn a_canonical_grant_keeps_the_session_geometry_whoever_sized_it_last() {
        let mut control = session(&[A]);
        control.register(M);
        let promoted = granted(control.acquire(M).1);
        restore(&mut control, M, promoted);
        control.resize(M, size(120, 40)).unwrap();
        control.apply_pending();
        restore(&mut control, M, authority(2, 3));
        let (_, effects) = control.acquire(A);
        assert_eq!(effects.last(), Some(&grant(A, (3, 4), host(), Acquire)));
        assert_eq!(control.apply_pending(), [ResizeSession { size: host() }]);
        restore(&mut control, A, authority(3, 4));
        let handed_back = grant(M, (4, 5), host(), HandBack);
        assert_eq!(
            control.detach(A),
            [&detached(A)[..], &[handed_back]].concat()
        );
        assert_eq!(control.apply_pending(), NONE);
    }

    #[test]
    fn a_canonical_controller_is_restored_afresh_only_at_a_new_session_geometry() {
        let mut control = ordinary(host());
        control.register(M);
        assert_eq!(granted(control.acquire(M).1), authority(1, 1));
        control.resize(M, size(600, 250)).unwrap();
        let replaced = Restore {
            conn: M,
            authority: authority(1, 2),
            host: size(500, 200),
        };
        let resized = ResizeSession {
            size: size(500, 200),
        };
        assert_eq!(control.apply_pending(), [resized, replaced]);
        control.resize(M, size(610, 260)).unwrap();
        assert_eq!(control.apply_pending(), NONE);
        restore(&mut control, M, authority(1, 2));

        control.resize(M, size(620, 270)).unwrap();
        let host_only = ResizeHost {
            conn: M,
            host: size(620, 270),
        };
        assert_eq!(control.apply_pending(), [host_only]);
        assert!(control.admits_input(M));
        control.resize(M, size(100, 30)).unwrap();
        let effects = [
            ResizeSession {
                size: size(100, 30),
            },
            ResizeHost {
                conn: M,
                host: size(100, 30),
            },
            Restore {
                conn: M,
                authority: authority(1, 3),
                host: size(100, 30),
            },
        ];
        assert_eq!(control.apply_pending(), effects);
        assert!(!control.admits_input(M));
    }

    /// LIM-05 (Contract and Fault, pure decision): the last control period
    /// and restoration ID are issued, and then an attach, which needs a new
    /// pair, is refused with `resource_limit` without effects: no counter
    /// wraps or is reused, and the controller keeps its authority.
    #[test]
    fn the_last_counters_are_issued_then_an_attach_is_refused_without_effects() {
        let mut control = issued(LAST - 1, LAST - 1);
        assert_eq!(
            control.attach(A, host()).unwrap(),
            [Admit { conn: A }, grant(A, (LAST, LAST), host(), Attach)]
        );
        let refused = Err(OperationalCode::ResourceLimit);
        assert_eq!(control.attach(B, host()), refused);
        assert_eq!(control.snapshot(), state(Attached, 1, true));
        assert_eq!(control.controlling(A), controlling(LAST, LAST, RESTORING));
        assert_eq!(control.controlling(B), Err(OperationalCode::NotAttached));
    }

    /// LIM-05 (pure decision): a grant needs a new restoration ID as well as
    /// a new period, so an exhausted restoration counter refuses it even
    /// with periods left.
    #[test]
    fn an_exhausted_restoration_counter_refuses_a_grant_with_periods_left() {
        let mut control = issued(7, LAST);
        let refused = Err(OperationalCode::ResourceLimit);
        assert_eq!(control.attach(A, host()), refused);
        assert_eq!(control.snapshot(), state(Starting, 0, false));
    }

    /// LIM-05 (pure decision): once exhausted, an Acquire that needs a new
    /// period, by a non-controlling attachment or a registration, is
    /// refused with `resource_limit` without effects, while the
    /// controller's own Acquire still keeps its authority.
    #[test]
    fn an_acquire_that_needs_new_counters_is_refused_once_they_are_exhausted() {
        let mut control = with_attachments(issued(LAST - 2, LAST - 2), &[A, B]);
        control.register(M);
        let refused = (Err(OperationalCode::ResourceLimit), Vec::new());
        assert_eq!(control.acquire(A), refused);
        assert_eq!(control.acquire(M), refused);
        assert_eq!(control.acquire(A).1, NONE);
        let kept = Acquisition {
            changed: false,
            authority: authority(LAST, LAST),
        };
        assert_eq!(control.acquire(B), (Ok(kept), Vec::new()));
        assert_eq!(
            control.controlling(B),
            controlling(LAST, LAST, Phase::Ready)
        );
    }

    /// LIM-05 (pure decision, at the limit and one past): a resize before
    /// readiness that needs a replacement restoration takes the last
    /// restoration ID; the next such resize is refused with
    /// `resource_limit` and not recorded, while one that needs no new
    /// restoration is still accepted.
    #[test]
    fn a_resize_that_needs_a_new_restoration_is_refused_once_none_remains() {
        let mut control = issued(5, LAST - 2);
        assert_eq!(
            granted(control.attach(A, host()).unwrap()),
            authority(6, LAST - 1)
        );
        control.apply_pending();
        assert_eq!(control.resize(A, size(100, 30)), Ok(()));
        let replaced = Restore {
            conn: A,
            authority: authority(6, LAST),
            host: size(100, 30),
        };
        let resized = ResizeSession {
            size: size(100, 30),
        };
        assert_eq!(control.apply_pending(), [resized, replaced]);
        assert_eq!(
            control.resize(A, size(90, 30)),
            Err(OperationalCode::ResourceLimit)
        );
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.controlling(A), controlling(6, LAST, RESTORING));
        assert_eq!(control.resize(A, size(100, 30)), Ok(()));
        restore(&mut control, A, authority(6, LAST));
        assert_eq!(control.resize(A, size(90, 30)), Ok(()));
        let host_only = [
            ResizeSession { size: size(90, 30) },
            ResizeHost {
                conn: A,
                host: size(90, 30),
            },
        ];
        assert_eq!(control.apply_pending(), host_only);
    }

    /// LIM-05 (pure decision): only the controller's own restoration can
    /// need a new restoration ID, so once none remains a non-controlling
    /// attachment's host size is still recorded.
    #[test]
    fn once_exhausted_only_the_controllers_resize_is_refused() {
        let mut control = issued(LAST - 2, LAST - 2);
        control.attach(A, host()).unwrap();
        assert_eq!(
            granted(control.attach(B, host()).unwrap()),
            authority(LAST, LAST)
        );
        control.apply_pending();
        assert_eq!(control.resize(A, size(100, 30)), Ok(()));
        assert_eq!(
            control.resize(B, size(100, 30)),
            Err(OperationalCode::ResourceLimit)
        );
        assert_eq!(control.apply_pending(), NONE);
    }

    /// LIM-05 (pure decision): a ready canonical controller's resize to a
    /// new geometry needs a new restoration, so it is refused once none
    /// remains; one that keeps the geometry is accepted.
    #[test]
    fn a_canonical_resize_to_a_new_geometry_is_refused_once_no_restoration_remains() {
        let mut control = issued(LAST - 1, LAST - 1);
        control.register(M);
        assert_eq!(granted(control.acquire(M).1), authority(LAST, LAST));
        control.apply_pending();
        restore(&mut control, M, authority(LAST, LAST));
        assert_eq!(
            control.resize(M, size(100, 30)),
            Err(OperationalCode::ResourceLimit)
        );
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.resize(M, host()), Ok(()));
        assert_eq!(control.apply_pending(), NONE);
        assert!(control.admits_input(M));
    }

    /// LIM-05 (pure decision): once exhausted, a departing controller hands
    /// control back to nobody, since that needs a new period. No remaining
    /// attachment can ever control again, so each is explicitly failed: its
    /// connection closes, most recently controlling first. The leave policy
    /// then applies as to the last attachment's departure.
    #[test]
    fn once_exhausted_a_departing_controller_hands_back_nothing_and_the_rest_fail() {
        let mut control = with_attachments(issued(LAST - 2, LAST - 2), &[A, B]);
        let detached = [Release { conn: B }];
        let failed = [Disconnect { conn: A }];
        let effects = [ended_admission(B).as_slice(), &detached, &failed].concat();
        assert_eq!(control.detach(B), effects);
        assert_eq!(control.snapshot(), state(Detached, 0, false));
        assert_eq!(control.controlling(A), Err(OperationalCode::NotAttached));
        let refused = (Err(OperationalCode::ResourceLimit), Vec::new());
        assert_eq!(control.acquire(A), refused);

        let mut control = with_attachments(issued(LAST - 3, LAST - 3), &[A, B, C]);
        let failed = [Disconnect { conn: B }, Disconnect { conn: A }];
        let effects = [ended_admission(C).as_slice(), &failed, &[BeginShutdown]].concat();
        assert_eq!(control.lost(C), effects);
        assert_eq!(control.snapshot(), state(Terminating, 0, false));
    }

    /// LIM-05 (pure decision): once exhausted, no registration is promoted,
    /// since that needs a new period; it stays registered without control,
    /// and the leave policy applies as if there were none.
    #[test]
    fn once_exhausted_no_registration_is_promoted_and_the_leave_policy_applies() {
        let mut control = with_attachments(issued(LAST - 1, LAST - 1), &[A]);
        control.register(M);
        let detached = [ended_admission(A).to_vec(), vec![Release { conn: A }]].concat();
        assert_eq!(control.detach(A), detached);
        assert_eq!(control.snapshot(), state(Detached, 0, false));
        let refused = (Err(OperationalCode::ResourceLimit), Vec::new());
        assert_eq!(control.acquire(M), refused);

        let mut control = with_attachments(issued(LAST - 1, LAST - 1), &[A]);
        control.register(M);
        let ended = [ended_admission(A).to_vec(), vec![BeginShutdown]].concat();
        assert_eq!(control.lost(A), ended);
    }
}
