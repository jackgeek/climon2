use super::{
    Authority, ConnectionId, Controlling, Effect, Grants, Issued, geometry, shutdown::Members,
};
use climon2_core::{
    protocol::TerminalSize,
    wire::{GrantReason, OperationalCode},
};
use std::mem;

/// One accepted attachment and its latest valid host size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Attachment {
    pub conn: ConnectionId,
    pub host: TerminalSize,
    /// A monitor-mode attachment: its output is canonical, serialized at
    /// the session geometry, which each of its grants keeps.
    pub canonical: bool,
}

impl Attachment {
    /// As a grant finds it: a canonical attachment's host is the session
    /// geometry, so its grant resizes nothing.
    fn granted_at(self, session: TerminalSize) -> Self {
        if self.canonical {
            return Self {
                host: session,
                ..self
            };
        }
        self
    }

    /// The size its restorations are serialized for: its host, or for a
    /// canonical attachment the session geometry its host gives.
    fn serialized(&self) -> TerminalSize {
        if self.canonical {
            return geometry(self.host);
        }
        self.host
    }
}

/// What a request needs of its connection's control beyond current
/// authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Needs {
    /// Control, even before readiness: Resize.
    Control,
    /// A complete restoration: Ready.
    Restored,
    /// Readiness: Input and Focus.
    Readiness,
}

impl Needs {
    pub(super) fn met(self, phase: Phase) -> bool {
        match (self, phase) {
            (Self::Control, _) | (_, Phase::Ready) => true,
            (Self::Restored, Phase::Restoring { complete }) => complete,
            _ => false,
        }
    }
}

/// What the controller's current restoration awaits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Phase {
    /// Its bytes are being sent; `complete` once every one of them was.
    Restoring { complete: bool },
    /// Its host changed before readiness: the next apply step replaces it
    /// within the same period.
    Replacing,
    /// Readiness confirmed: application input is admitted.
    Ready,
}

pub(super) struct Controller {
    attachment: Attachment,
    authority: Authority,
    /// The size its current restoration is serialized for.
    serialized: TerminalSize,
    phase: Phase,
    /// The desired application focus in this control period, once known.
    focus: Option<bool>,
}

impl Controller {
    /// Grants control at once under a new period and its first restoration.
    fn granted(attachment: Attachment, reason: GrantReason, grants: &mut Grants) -> (Self, Effect) {
        let attachment = attachment.granted_at(grants.sizes.session);
        let authority = grants.issued.grant();
        let serialized = attachment.serialized();
        let grant = Effect::Grant {
            conn: attachment.conn,
            authority,
            host: serialized,
            reason,
        };
        let controller = Self {
            attachment,
            authority,
            serialized,
            phase: Phase::Restoring { complete: false },
            focus: None,
        };
        (controller, grant)
    }

    fn conn(&self) -> ConnectionId {
        self.attachment.conn
    }

    fn controlling(&self) -> Controlling {
        Controlling {
            authority: self.authority,
            phase: self.phase,
        }
    }

    pub fn admits(&self, conn: ConnectionId) -> bool {
        self.conn() == conn && Needs::Readiness.met(self.phase)
    }

    /// Whether `conn` controls under `authority` and has not confirmed
    /// readiness for it.
    pub fn restoring(&self, conn: ConnectionId, authority: Authority) -> bool {
        self.conn() == conn
            && self.authority == authority
            && matches!(self.phase, Phase::Restoring { .. })
    }

    /// Every byte of the restoration of `authority` was sent.
    pub fn restored(&mut self, conn: ConnectionId, authority: Authority) {
        if self.restoring(conn, authority) {
            self.phase = Phase::Restoring { complete: true };
        }
    }

    /// Readiness needs the complete current restoration.
    pub fn ready(&mut self, conn: ConnectionId, restoration: super::Restoration) -> Vec<Effect> {
        let complete = Phase::Restoring { complete: true };
        if self.conn() != conn
            || self.authority.restoration != restoration
            || self.phase != complete
        {
            return Vec::new();
        }
        self.phase = Phase::Ready;
        vec![Effect::EnableInput { conn }]
    }

    /// Records the desired application focus; true when it changed, from
    /// unknown too.
    pub fn focus(&mut self, conn: ConnectionId, focused: bool) -> bool {
        let changed = self.conn() == conn && self.focus != Some(focused);
        if changed {
            self.focus = Some(focused);
        }
        changed
    }

    pub fn desired_focus(&self) -> Option<bool> {
        self.focus
    }

    pub fn host(&self) -> TerminalSize {
        self.attachment.host
    }

    /// Replaces a restoration whose host changed, or reports a ready
    /// controller's new host size.
    pub fn settle(&mut self, resized: bool, issued: &mut Issued) -> Vec<Effect> {
        match self.phase {
            Phase::Replacing => self.replace(issued),
            Phase::Ready if resized => self.presented(issued),
            _ => Vec::new(),
        }
    }

    /// A ready controller's new host size applied: its presentation follows
    /// it, and a canonical one at a new session geometry is restored afresh
    /// at that geometry within the same period.
    fn presented(&mut self, issued: &mut Issued) -> Vec<Effect> {
        let mut effects = vec![Effect::ResizeHost {
            conn: self.conn(),
            host: self.host(),
        }];
        if self.attachment.canonical && self.attachment.serialized() != self.serialized {
            effects.extend(self.replace(issued));
        }
        effects
    }

    fn replace(&mut self, issued: &mut Issued) -> Vec<Effect> {
        self.authority = issued.restore(self.authority.period);
        self.serialized = self.attachment.serialized();
        self.phase = Phase::Restoring { complete: false };
        vec![Effect::Restore {
            conn: self.conn(),
            authority: self.authority,
            host: self.serialized,
        }]
    }

    /// A restoration no longer matches the size it is serialized for after
    /// a host change before readiness.
    fn invalidate(&mut self) {
        let restoring = matches!(self.phase, Phase::Restoring { .. });
        if restoring && self.attachment.serialized() != self.serialized {
            self.phase = Phase::Replacing;
        }
    }

    pub fn end_admission(&self) -> Vec<Effect> {
        match self.phase {
            Phase::Ready => vec![
                Effect::DiscardPendingInput { conn: self.conn() },
                Effect::ClosePaste,
            ],
            _ => Vec::new(),
        }
    }

    fn revoke(&self) -> Vec<Effect> {
        let conn = self.conn();
        let mut effects = self.end_admission();
        effects.extend([Effect::Release { conn }, Effect::Revoke { conn }]);
        effects
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Departure {
    Detach,
    Lost,
}

impl Departure {
    fn of_controller(self, conn: ConnectionId) -> Vec<Effect> {
        match self {
            Self::Detach => vec![Effect::Release { conn }, Effect::AcceptDetach { conn }],
            Self::Lost => Vec::new(),
        }
    }

    /// A non-controlling attachment was released when it was revoked.
    fn of_other(self, conn: ConnectionId) -> Vec<Effect> {
        match self {
            Self::Detach => vec![Effect::AcceptDetach { conn }],
            Self::Lost => Vec::new(),
        }
    }
}

/// At least one attachment, exactly one of which controls.
pub(super) struct Attachments {
    pub controller: Controller,
    /// Non-controlling attachments, most recently controlling first.
    others: Vec<Attachment>,
}

impl Attachments {
    /// The first attachment, granted at once for `reason`.
    pub fn new(attachment: Attachment, reason: GrantReason, grants: &mut Grants) -> (Self, Effect) {
        let (controller, grant) = Controller::granted(attachment, reason, grants);
        let attachments = Self {
            controller,
            others: Vec::new(),
        };
        (attachments, grant)
    }

    pub fn count(&self) -> usize {
        1 + self.others.len()
    }

    pub fn controlling(&self, conn: ConnectionId) -> Result<Controlling, OperationalCode> {
        if self.controller.conn() == conn {
            return Ok(self.controller.controlling());
        }
        match self.position(conn) {
            Some(_) => Err(OperationalCode::NotController),
            None => Err(OperationalCode::NotAttached),
        }
    }

    /// A non-controlling attachment takes control at once; nothing changes
    /// for anyone else.
    pub fn take_control(&mut self, conn: ConnectionId, grants: &mut Grants) -> Vec<Effect> {
        let Some(index) = self.position(conn) else {
            return Vec::new();
        };
        let attachment = self.others.remove(index);
        self.promote(attachment, GrantReason::Acquire, grants)
    }

    /// Makes `attachment` the controller at once; the previous one stays,
    /// non-controlling.
    pub fn promote(
        &mut self,
        attachment: Attachment,
        reason: GrantReason,
        grants: &mut Grants,
    ) -> Vec<Effect> {
        let (controller, grant) = Controller::granted(attachment, reason, grants);
        let previous = mem::replace(&mut self.controller, controller);
        self.others.insert(0, previous.attachment);
        let mut effects = previous.revoke();
        effects.push(grant);
        effects
    }

    fn position(&self, conn: ConnectionId) -> Option<usize> {
        self.others.iter().position(|other| other.conn == conn)
    }

    pub fn resize(&mut self, conn: ConnectionId, host: TerminalSize) {
        let controller = &mut self.controller.attachment;
        for attachment in std::iter::once(controller).chain(&mut self.others) {
            if attachment.conn == conn {
                attachment.host = host;
            }
        }
        self.controller.invalidate();
    }

    /// Returns the remaining attachments, if any, after `conn` departs.
    pub fn depart(
        mut self,
        conn: ConnectionId,
        departure: Departure,
        grants: &mut Grants,
    ) -> (Option<Self>, Vec<Effect>) {
        if self.controller.conn() == conn {
            return self.hand_back(departure, grants);
        }
        let effects = match self.position(conn) {
            Some(index) => {
                self.others.remove(index);
                departure.of_other(conn)
            }
            None => Vec::new(),
        };
        (Some(self), effects)
    }

    /// The most recently controlling remaining attachment is granted at once.
    fn hand_back(self, departure: Departure, grants: &mut Grants) -> (Option<Self>, Vec<Effect>) {
        let mut effects = self.controller.end_admission();
        effects.extend(departure.of_controller(self.controller.conn()));
        let mut others = self.others.into_iter();
        let remaining = others.next().map(|recent| {
            let (controller, grant) = Controller::granted(recent, GrantReason::HandBack, grants);
            effects.push(grant);
            Self {
                controller,
                others: others.collect(),
            }
        });
        (remaining, effects)
    }

    /// Shutdown ends admission; only the controller's presentation remains to release.
    pub fn into_members(self) -> (Members, Vec<Effect>) {
        let effects = self.controller.end_admission();
        let released = self.others.iter().map(|other| other.conn).collect();
        (Members::new(self.controller.conn(), released), effects)
    }
}
