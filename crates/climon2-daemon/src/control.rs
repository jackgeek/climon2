//! The pure control state machine of one session.
//!
//! It owns the lifecycle, the attachments, the controller and its hand-back
//! recency order, the control period and readiness, each attachment's latest
//! host size, the applied session geometry, the end of input admission and the
//! shutdown sequence. It performs no I/O: each method takes one event and
//! returns the effects the daemon loop must perform, in order.
//!
//! Call protocol for one loop iteration:
//!
//! 1. Feed every event received this iteration to its method, in acceptance
//!    order, and perform the effects each returns. [`Control::resize`] only
//!    records a pending size.
//! 2. Call [`Control::apply_pending`] exactly once, before reading the PTY and
//!    before any snapshot for a grant. It applies the controller's latest size
//!    at most once and issues at most one grant.
//! 3. Admit an attachment's application input only while
//!    [`Control::admits_input`] holds for it.

mod attachments;
mod shutdown;

use attachments::{Attachment, Attachments, Departure};
use climon2_core::{lifetime::Lifetime, protocol::TerminalSize};
use shutdown::{Members, Shutdown};

/// Identifies one daemon connection: an attachment, a probe or a kill requester.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConnectionId(pub u64);

/// One uninterrupted period of control, numbered monotonically from 1.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Period(pub u64);

/// Work the daemon loop performs for the state machine, in the order given.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    /// Accept an attachment: send `Attached`.
    Admit { conn: ConnectionId },
    /// Reject an attachment because the session is terminating: send `Error`, close.
    Refuse { conn: ConnectionId },
    /// Start `period` for `conn`: retire its previous period, snapshot for a
    /// host of size `host`, send `Granted{period}` and its restoration.
    Grant {
        conn: ConnectionId,
        period: Period,
        host: TerminalSize,
    },
    /// The controller confirmed readiness for its current period.
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
    /// Send `Exit{code}`.
    DeliverExit { conn: ConnectionId, code: u32 },
    /// Send `Error` describing the authoritative failure.
    DeliverError { conn: ConnectionId },
    /// Send `Terminated{code}` to a kill requester.
    ConfirmKill { conn: ConnectionId, code: u32 },
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

impl Period {
    fn next(&mut self) -> Self {
        self.0 += 1;
        *self
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
fn geometry(host: TerminalSize) -> TerminalSize {
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

    fn attach(&mut self, attachment: Attachment) -> Vec<Effect> {
        let mut effects = vec![Effect::Admit {
            conn: attachment.conn,
        }];
        match self {
            Self::Attached(attachments) => effects.extend(attachments.promote(attachment)),
            _ => *self = Self::Attached(Attachments::new(attachment)),
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

fn command_exited(open: Open, code: u32) -> (Stage, Vec<Effect>) {
    let (members, mut effects) = open.into_members();
    effects.push(Effect::Cleanup);
    (Stage::Terminating(Shutdown::exited(members, code)), effects)
}

fn initial_deadline(open: Open) -> (Stage, Vec<Effect>) {
    match open {
        Open::Starting => begin_shutdown(open, Vec::new(), false),
        _ => (Stage::Open(open), Vec::new()),
    }
}

/// The leave policy: only the unexpected loss of an ordinary session's last
/// attachment ends it; a headless session survives with zero clients.
fn depart(
    lifetime: Lifetime,
    open: Open,
    conn: ConnectionId,
    departure: Departure,
) -> (Stage, Vec<Effect>) {
    let Open::Attached(attachments) = open else {
        return (Stage::Open(open), Vec::new());
    };
    let (remaining, effects) = attachments.depart(conn, departure);
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
    /// The most recently issued period.
    period: Period,
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
            period: Period(0),
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

    /// Whether `conn` is the controller and has confirmed readiness for its period.
    pub fn admits_input(&self, conn: ConnectionId) -> bool {
        match &self.stage {
            Stage::Open(Open::Attached(attachments)) => attachments.controller.admits(conn),
            _ => false,
        }
    }

    /// An accepted attach request. Each connection attaches at most once.
    pub fn attach(&mut self, conn: ConnectionId, size: TerminalSize) -> Vec<Effect> {
        let attachment = Attachment {
            conn,
            host: size.initial(),
        };
        match &mut self.stage {
            Stage::Open(open) => open.attach(attachment),
            Stage::Terminating(_) => vec![Effect::Refuse { conn }],
        }
    }

    pub fn take_control(&mut self, conn: ConnectionId) -> Vec<Effect> {
        attached(&mut self.stage)
            .map(|attachments| attachments.take_control(conn))
            .unwrap_or_default()
    }

    pub fn ready(&mut self, conn: ConnectionId, period: Period) -> Vec<Effect> {
        attached(&mut self.stage)
            .map(|attachments| attachments.controller.ready(conn, period))
            .unwrap_or_default()
    }

    /// Records a pending host size; [`Control::apply_pending`] applies it.
    pub fn resize(&mut self, conn: ConnectionId, size: TerminalSize) {
        let applied = self.sizes.host;
        if let (Some(host), Some(attachments)) = (size.resize(), attached(&mut self.stage)) {
            attachments.resize(conn, host, applied);
        }
    }

    /// An accepted explicit detach request.
    pub fn detach(&mut self, conn: ConnectionId) -> Vec<Effect> {
        let lifetime = self.lifetime;
        self.open(|open| depart(lifetime, open, conn, Departure::Detach))
    }

    /// The connection closed or failed without an accepted detach.
    pub fn lost(&mut self, conn: ConnectionId) -> Vec<Effect> {
        if let Stage::Terminating(shutdown) = &mut self.stage {
            shutdown.lost(conn);
        }
        let lifetime = self.lifetime;
        self.open(|open| depart(lifetime, open, conn, Departure::Lost))
    }

    pub fn kill(&mut self, conn: ConnectionId) -> Vec<Effect> {
        match &mut self.stage {
            Stage::Terminating(shutdown) => shutdown.kill(conn),
            Stage::Open(_) => self.open(|open| begin_shutdown(open, vec![conn], false)),
        }
    }

    pub fn initial_deadline(&mut self) -> Vec<Effect> {
        self.open(initial_deadline)
    }

    /// The restoration deadline of `period` expired.
    pub fn restore_deadline(&mut self, conn: ConnectionId, period: Period) -> Vec<Effect> {
        let restoring = attached(&mut self.stage)
            .is_some_and(|attachments| attachments.controller.restoring(conn, period));
        if !restoring {
            return Vec::new();
        }
        [vec![Effect::Disconnect { conn }], self.lost(conn)].concat()
    }

    /// The command exited; for a normal exit, report it after the output drain.
    pub fn command_exited(&mut self, code: u32) -> Vec<Effect> {
        match &mut self.stage {
            Stage::Terminating(shutdown) => shutdown.command_exited(code),
            Stage::Open(_) => self.open(|open| command_exited(open, code)),
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
            Stage::Open(_) => self.open(|open| begin_shutdown(open, Vec::new(), true)),
        }
    }

    /// Applies the controller's latest pending size once, then issues a due grant.
    pub fn apply_pending(&mut self) -> Vec<Effect> {
        let Some(attachments) = attached(&mut self.stage) else {
            return Vec::new();
        };
        let applied = self.sizes.apply(attachments.controller.host());
        let resized = applied.is_some();
        let mut effects = applied.unwrap_or_default();
        effects.extend(attachments.controller.settle(resized, &mut self.period));
        effects
    }

    /// Runs a transition of an open session; a terminating session ignores it.
    fn open(&mut self, transition: impl FnOnce(Open) -> (Stage, Vec<Effect>)) -> Vec<Effect> {
        let (stage, effects) = match std::mem::replace(&mut self.stage, Stage::Open(Open::Starting))
        {
            Stage::Open(open) => transition(open),
            terminating => (terminating, Vec::new()),
        };
        self.stage = stage;
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::Effect::*;
    use super::Lifecycle::*;
    use super::*;

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

    fn grant(conn: ConnectionId, period: u64, host: TerminalSize) -> Effect {
        Grant {
            conn,
            period: Period(period),
            host,
        }
    }

    fn granted(effects: Vec<Effect>) -> Period {
        match effects.last() {
            Some(Grant { period, .. }) => *period,
            other => panic!("expected a grant, got {other:?}"),
        }
    }

    /// Attaches each connection in turn at 80x24 and lets it confirm readiness.
    fn session(conns: &[ConnectionId]) -> Control {
        let mut control = ordinary(host());
        for conn in conns {
            control.attach(*conn, host());
            let period = granted(control.apply_pending());
            assert_eq!(control.ready(*conn, period), [EnableInput { conn: *conn }]);
        }
        control
    }

    #[test]
    fn a_new_session_is_starting_without_attachments() {
        let control = ordinary(size(100, 30));
        assert_eq!(control.snapshot(), state(Starting, 0, false));
        assert_eq!(control.session_size(), size(100, 30));
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
    fn the_first_attach_is_admitted_and_granted_by_the_next_apply() {
        let mut control = ordinary(host());
        assert_eq!(control.attach(A, host()), [Admit { conn: A }]);
        assert_eq!(control.snapshot(), state(Attached, 1, true));
        assert_eq!(control.apply_pending(), [grant(A, 1, host())]);
        assert_eq!(control.apply_pending(), NONE);
    }

    #[test]
    fn a_grant_applies_the_attaching_host_size_first() {
        let mut control = ordinary(host());
        control.attach(A, size(100, 30));
        assert_eq!(
            control.apply_pending(),
            [
                ResizeSession {
                    size: size(100, 30)
                },
                grant(A, 1, size(100, 30))
            ]
        );
        assert_eq!(control.session_size(), size(100, 30));
    }

    #[test]
    fn an_attach_without_a_size_uses_80x24() {
        let mut control = ordinary(size(100, 30));
        control.attach(A, size(0, 0));
        assert_eq!(
            control.apply_pending(),
            [ResizeSession { size: host() }, grant(A, 1, host())]
        );
    }

    #[test]
    fn input_is_admitted_only_after_ready_for_the_current_period() {
        let mut control = ordinary(host());
        control.attach(A, host());
        assert_eq!(control.ready(A, Period(1)), NONE);
        control.apply_pending();
        assert!(!control.admits_input(A));
        assert_eq!(control.ready(A, Period(1)), [EnableInput { conn: A }]);
        assert!(control.admits_input(A));
        assert!(!control.admits_input(B));
        assert_eq!(control.ready(A, Period(1)), NONE);
    }

    #[test]
    fn a_stale_or_foreign_ready_is_ignored() {
        let mut control = session(&[A]);
        control.attach(B, host());
        control.apply_pending();
        assert_eq!(control.ready(B, Period(1)), NONE);
        assert_eq!(control.ready(A, Period(2)), NONE);
        assert!(!control.admits_input(A));
        assert!(!control.admits_input(B));
        assert_eq!(control.ready(B, Period(2)), [EnableInput { conn: B }]);
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
                Revoke { conn: A }
            ]
        );
        assert!(!control.admits_input(A));
        assert_eq!(control.snapshot(), state(Attached, 2, true));
        assert_eq!(control.apply_pending(), [grant(B, 2, host())]);
    }

    #[test]
    fn revoking_a_controller_that_was_never_admitted_closes_no_paste() {
        let mut control = ordinary(host());
        control.attach(A, host());
        control.apply_pending();
        assert_eq!(
            control.attach(B, host()),
            [Admit { conn: B }, Release { conn: A }, Revoke { conn: A }]
        );
    }

    #[test]
    fn attaches_in_one_iteration_grant_only_the_last() {
        let mut control = ordinary(host());
        control.attach(A, host());
        assert_eq!(
            control.attach(B, host()),
            [Admit { conn: B }, Release { conn: A }, Revoke { conn: A }]
        );
        assert_eq!(control.apply_pending(), [grant(B, 1, host())]);
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
                Revoke { conn: B }
            ]
        );
        assert_eq!(control.apply_pending(), [grant(A, 3, host())]);
        assert_eq!(control.ready(A, Period(1)), NONE);
        assert_eq!(control.ready(A, Period(3)), [EnableInput { conn: A }]);
    }

    #[test]
    fn take_control_by_the_controller_is_a_no_op() {
        let mut control = session(&[A]);
        assert_eq!(control.take_control(A), NONE);
        assert_eq!(control.apply_pending(), NONE);
        assert!(control.admits_input(A));
    }

    #[test]
    fn take_control_from_an_unknown_connection_changes_nothing() {
        assert_eq!(ordinary(host()).take_control(C), NONE);
        let mut control = session(&[A]);
        assert_eq!(control.take_control(C), NONE);
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.snapshot(), state(Attached, 1, true));
    }

    #[test]
    fn controller_departure_hands_back_to_the_most_recent_controller() {
        let mut control = session(&[A, B, C]);
        control.take_control(A);
        let period = granted(control.apply_pending());
        control.ready(A, period);
        assert_eq!(
            control.detach(A),
            [
                DiscardPendingInput { conn: A },
                ClosePaste,
                Release { conn: A },
                AcceptDetach { conn: A }
            ]
        );
        assert_eq!(control.apply_pending(), [grant(C, 5, host())]);
        assert_eq!(control.lost(C), NONE);
        assert_eq!(control.apply_pending(), [grant(B, 6, host())]);
        assert_eq!(control.snapshot(), state(Attached, 1, true));
    }

    #[test]
    fn losing_a_ready_controller_ends_its_admission_and_hands_back() {
        let mut control = session(&[A, B]);
        assert_eq!(
            control.lost(B),
            [DiscardPendingInput { conn: B }, ClosePaste]
        );
        assert!(!control.admits_input(B));
        assert_eq!(control.apply_pending(), [grant(A, 3, host())]);
    }

    #[test]
    fn a_hand_back_applies_the_recipients_latest_size() {
        let mut control = session(&[A, B]);
        control.resize(A, size(120, 40));
        assert_eq!(control.apply_pending(), NONE);
        control.lost(B);
        assert_eq!(
            control.apply_pending(),
            [
                ResizeSession {
                    size: size(120, 40)
                },
                grant(A, 3, size(120, 40))
            ]
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
        control.attach(A, size(100, 30));
        let period = granted(control.apply_pending());
        control.ready(A, period);
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
        assert_eq!(control.attach(B, host()), [Admit { conn: B }]);
        assert_eq!(control.apply_pending(), [grant(B, 2, host())]);
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
        control.take_control(A);
        assert_eq!(
            control.apply_pending(),
            [
                ResizeSession {
                    size: size(100, 30)
                },
                grant(A, 3, size(100, 30))
            ]
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
    fn a_resize_before_readiness_regrants_in_a_new_period() {
        let mut control = ordinary(host());
        control.attach(A, host());
        control.apply_pending();
        control.resize(A, size(100, 30));
        assert_eq!(control.ready(A, Period(1)), NONE);
        assert_eq!(
            control.apply_pending(),
            [
                ResizeSession {
                    size: size(100, 30)
                },
                grant(A, 2, size(100, 30))
            ]
        );
        assert_eq!(control.restore_deadline(A, Period(1)), NONE);
        assert_eq!(control.ready(A, Period(2)), [EnableInput { conn: A }]);
    }

    #[test]
    fn a_host_only_resize_before_readiness_regrants() {
        let mut control = ordinary(size(600, 300));
        control.attach(A, size(600, 300));
        assert_eq!(control.apply_pending(), [grant(A, 1, size(600, 300))]);
        control.resize(A, size(700, 300));
        assert_eq!(control.apply_pending(), [grant(A, 2, size(700, 300))]);
    }

    #[test]
    fn a_resize_to_the_applied_size_before_readiness_keeps_the_period() {
        let mut control = ordinary(host());
        control.attach(A, host());
        control.apply_pending();
        control.resize(A, host());
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.ready(A, Period(1)), [EnableInput { conn: A }]);
    }

    #[test]
    fn a_non_controller_resize_keeps_the_restoring_controllers_period() {
        let mut control = session(&[A]);
        control.attach(B, host());
        control.apply_pending();
        control.resize(A, size(100, 30));
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.ready(B, Period(2)), [EnableInput { conn: B }]);
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
    fn a_restoration_deadline_loses_the_restoring_controller() {
        let mut control = session(&[A]);
        control.attach(B, host());
        control.apply_pending();
        assert_eq!(
            control.restore_deadline(B, Period(2)),
            [Disconnect { conn: B }]
        );
        assert_eq!(control.snapshot(), state(Attached, 1, true));
        assert_eq!(control.apply_pending(), [grant(A, 3, host())]);
    }

    #[test]
    fn a_restoration_deadline_of_the_only_attachment_begins_shutdown() {
        let mut control = ordinary(host());
        control.attach(A, host());
        control.apply_pending();
        assert_eq!(
            control.restore_deadline(A, Period(1)),
            [Disconnect { conn: A }, BeginShutdown]
        );
        assert_eq!(control.snapshot(), state(Terminating, 0, false));
    }

    #[test]
    fn a_stale_restoration_deadline_is_ignored() {
        assert_eq!(ordinary(host()).restore_deadline(A, Period(1)), NONE);
        let mut control = session(&[A]);
        assert_eq!(control.restore_deadline(A, Period(1)), NONE);
        control.attach(B, host());
        control.apply_pending();
        assert_eq!(control.restore_deadline(A, Period(2)), NONE);
        assert_eq!(control.restore_deadline(B, Period(1)), NONE);
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
        control.kill(K);
        assert_eq!(control.attach(C, host()), [Refuse { conn: C }]);
        assert_eq!(control.detach(B), NONE);
        assert_eq!(control.detach(A), NONE);
        assert_eq!(control.take_control(A), NONE);
        assert_eq!(control.ready(B, Period(2)), NONE);
        assert_eq!(control.restore_deadline(B, Period(2)), NONE);
        assert_eq!(control.initial_deadline(), NONE);
        assert_eq!(control.apply_pending(), NONE);
        assert_eq!(control.kill(K2), NONE);
        assert_eq!(control.snapshot(), state(Terminating, 2, true));
    }

    #[test]
    fn shutdown_releases_the_presenting_attachment_and_confirms_every_kill() {
        let mut control = session(&[A, B]);
        control.kill(K);
        control.kill(K2);
        assert_eq!(control.command_exited(137), [Cleanup]);
        assert_eq!(control.snapshot(), state(Terminating, 2, true));
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: B },
                DeliverExit { conn: B, code: 137 },
                DeliverExit { conn: A, code: 137 },
                ConfirmKill { conn: K, code: 137 },
                ConfirmKill {
                    conn: K2,
                    code: 137
                },
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
        control.command_exited(0);
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: C },
                DeliverExit { conn: C, code: 0 },
                DeliverExit { conn: A, code: 0 },
                ConfirmKill { conn: K2, code: 0 },
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
        control.command_exited(0);
        assert_eq!(
            control.cleanup_finished(),
            [
                DeliverExit { conn: A, code: 0 },
                ConfirmKill { conn: K, code: 0 },
                Finish
            ]
        );
    }

    #[test]
    fn a_command_exit_releases_and_delivers_exit_to_every_attachment() {
        let mut control = session(&[A, B]);
        assert_eq!(
            control.command_exited(3),
            [DiscardPendingInput { conn: B }, ClosePaste, Cleanup]
        );
        assert_eq!(control.attach(C, host()), [Refuse { conn: C }]);
        assert_eq!(control.snapshot(), state(Terminating, 2, true));
        assert_eq!(control.command_exited(4), NONE);
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
        assert_eq!(control.command_exited(0), [Cleanup]);
        assert_eq!(control.initial_deadline(), NONE);
        assert_eq!(control.cleanup_finished(), [Finish]);
    }

    #[test]
    fn cleanup_waits_for_the_command_to_exit() {
        let mut control = session(&[A]);
        control.kill(K);
        assert_eq!(control.cleanup_finished(), NONE);
        assert_eq!(control.command_exited(143), [Cleanup]);
    }

    #[test]
    fn a_kill_during_cleanup_is_confirmed_after_it() {
        let mut control = session(&[A]);
        control.command_exited(5);
        assert_eq!(control.kill(K), NONE);
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: A },
                DeliverExit { conn: A, code: 5 },
                ConfirmKill { conn: K, code: 5 },
                Finish
            ]
        );
    }

    #[test]
    fn a_kill_after_the_session_finished_is_confirmed_immediately() {
        let mut control = session(&[A]);
        control.command_exited(5);
        control.cleanup_finished();
        assert_eq!(control.kill(K), [ConfirmKill { conn: K, code: 5 }]);
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
        assert_eq!(control.command_exited(1), [Cleanup]);
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: B },
                DeliverError { conn: B },
                DeliverError { conn: A },
                DeliverError { conn: K },
                Finish
            ]
        );
        assert_eq!(control.kill(K2), [DeliverError { conn: K2 }]);
        assert_eq!(control.authoritative_failure(), NONE);
    }

    #[test]
    fn an_authoritative_failure_during_shutdown_turns_the_result_into_errors() {
        let mut control = session(&[A]);
        control.kill(K);
        assert_eq!(control.authoritative_failure(), NONE);
        control.command_exited(0);
        assert_eq!(
            control.cleanup_finished(),
            [
                Release { conn: A },
                DeliverError { conn: A },
                DeliverError { conn: K },
                Finish
            ]
        );
    }

    #[test]
    fn a_failed_cleanup_never_confirms_a_kill() {
        let mut control = session(&[A]);
        control.kill(K);
        control.command_exited(0);
        assert_eq!(
            control.authoritative_failure(),
            [
                Release { conn: A },
                DeliverError { conn: A },
                DeliverError { conn: K },
                Finish
            ]
        );
        assert_eq!(control.cleanup_finished(), NONE);
        assert_eq!(control.snapshot(), state(Terminating, 0, false));
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
        control.command_exited(129);
        assert_eq!(
            control.cleanup_finished(),
            [ConfirmKill { conn: K, code: 129 }, Finish]
        );
    }

    #[test]
    fn a_kill_joining_a_final_loss_shutdown_is_confirmed() {
        let mut control = session(&[A]);
        control.lost(A);
        assert_eq!(control.kill(K), NONE);
        control.command_exited(129);
        assert_eq!(
            control.cleanup_finished(),
            [ConfirmKill { conn: K, code: 129 }, Finish]
        );
    }

    #[test]
    fn the_initial_deadline_is_void_once_attached() {
        let mut control = session(&[A]);
        assert_eq!(control.initial_deadline(), NONE);
        assert_eq!(control.snapshot(), state(Attached, 1, true));
    }
}
