use super::{ConnectionId, Effect, Period, shutdown::Members};
use climon2_core::protocol::TerminalSize;
use std::mem;

/// One accepted attachment and its latest valid host size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Attachment {
    pub conn: ConnectionId,
    pub host: TerminalSize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    /// Control decided; the next apply step grants a new period.
    Due,
    Restoring(Period),
    /// Readiness confirmed: application input is admitted.
    Ready(Period),
}

pub(super) struct Controller {
    attachment: Attachment,
    phase: Phase,
}

impl Controller {
    fn due(attachment: Attachment) -> Self {
        Self {
            attachment,
            phase: Phase::Due,
        }
    }

    fn conn(&self) -> ConnectionId {
        self.attachment.conn
    }

    pub fn admits(&self, conn: ConnectionId) -> bool {
        self.conn() == conn && matches!(self.phase, Phase::Ready(_))
    }

    pub fn restoring(&self, conn: ConnectionId, period: Period) -> bool {
        self.conn() == conn && self.phase == Phase::Restoring(period)
    }

    pub fn ready(&mut self, conn: ConnectionId, period: Period) -> Vec<Effect> {
        if !self.restoring(conn, period) {
            return Vec::new();
        }
        self.phase = Phase::Ready(period);
        vec![Effect::EnableInput { conn }]
    }

    pub fn host(&self) -> TerminalSize {
        self.attachment.host
    }

    /// Issues the due grant, or reports a ready controller's new host size.
    pub fn settle(&mut self, resized: bool, period: &mut Period) -> Vec<Effect> {
        let (conn, host) = (self.conn(), self.host());
        match self.phase {
            Phase::Due => {
                let period = period.next();
                self.phase = Phase::Restoring(period);
                vec![Effect::Grant { conn, period, host }]
            }
            Phase::Ready(_) if resized => vec![Effect::ResizeHost { conn, host }],
            _ => Vec::new(),
        }
    }

    /// A restoration no longer matches a host that changed before readiness.
    fn invalidate(&mut self, applied: TerminalSize) {
        if matches!(self.phase, Phase::Restoring(_)) && self.host() != applied {
            self.phase = Phase::Due;
        }
    }

    pub fn end_admission(&self) -> Vec<Effect> {
        match self.phase {
            Phase::Ready(_) => vec![
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
    pub fn new(attachment: Attachment) -> Self {
        Self {
            controller: Controller::due(attachment),
            others: Vec::new(),
        }
    }

    pub fn count(&self) -> usize {
        1 + self.others.len()
    }

    pub fn take_control(&mut self, conn: ConnectionId) -> Vec<Effect> {
        match self.position(conn) {
            Some(index) => {
                let attachment = self.others.remove(index);
                self.promote(attachment)
            }
            None => Vec::new(),
        }
    }

    /// Makes `attachment` the controller; the previous one stays, non-controlling.
    pub fn promote(&mut self, attachment: Attachment) -> Vec<Effect> {
        let previous = mem::replace(&mut self.controller, Controller::due(attachment));
        self.others.insert(0, previous.attachment);
        previous.revoke()
    }

    fn position(&self, conn: ConnectionId) -> Option<usize> {
        self.others.iter().position(|other| other.conn == conn)
    }

    pub fn resize(&mut self, conn: ConnectionId, host: TerminalSize, applied: TerminalSize) {
        let controller = &mut self.controller.attachment;
        for attachment in std::iter::once(controller).chain(&mut self.others) {
            if attachment.conn == conn {
                attachment.host = host;
            }
        }
        self.controller.invalidate(applied);
    }

    /// Returns the remaining attachments, if any, after `conn` departs.
    pub fn depart(
        mut self,
        conn: ConnectionId,
        departure: Departure,
    ) -> (Option<Self>, Vec<Effect>) {
        if self.controller.conn() == conn {
            return self.hand_back(departure);
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

    fn hand_back(self, departure: Departure) -> (Option<Self>, Vec<Effect>) {
        let mut effects = self.controller.end_admission();
        effects.extend(departure.of_controller(self.controller.conn()));
        let mut others = self.others.into_iter();
        let remaining = others.next().map(|recent| Self {
            controller: Controller::due(recent),
            others: others.collect(),
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
