use super::{ConnectionId, Effect};
use climon2_core::wire::OperationalCode;

/// Attachments still present while the session terminates.
#[derive(Debug, Default)]
pub(super) struct Members {
    /// The controller at shutdown; its presentation still needs a release.
    presenting: Option<ConnectionId>,
    /// Non-controlling attachments, already released at revocation.
    released: Vec<ConnectionId>,
}

impl Members {
    pub fn new(presenting: ConnectionId, released: Vec<ConnectionId>) -> Self {
        Self {
            presenting: Some(presenting),
            released,
        }
    }

    fn count(&self) -> usize {
        usize::from(self.presenting.is_some()) + self.released.len()
    }

    fn lost(&mut self, conn: ConnectionId) {
        self.presenting = self.presenting.filter(|presenting| *presenting != conn);
        self.released.retain(|released| *released != conn);
    }

    fn contains(&self, conn: ConnectionId) -> bool {
        self.presenting == Some(conn) || self.released.contains(&conn)
    }

    /// An accepted departure: the presentation a member still has is released
    /// before the departure is confirmed.
    fn leave(&mut self, conn: ConnectionId) -> Vec<Effect> {
        let effects = match (self.presenting == Some(conn), self.released.contains(&conn)) {
            (true, _) => vec![Effect::Release { conn }, Effect::AcceptDetach { conn }],
            (false, true) => vec![Effect::AcceptDetach { conn }],
            (false, false) => Vec::new(),
        };
        self.lost(conn);
        effects
    }

    fn deliver(self, outcome: Outcome) -> impl Iterator<Item = Effect> {
        let presenting = self
            .presenting
            .into_iter()
            .flat_map(move |conn| [Effect::Release { conn }, outcome.to_attachment(conn)]);
        let released = self
            .released
            .into_iter()
            .map(move |conn| outcome.to_attachment(conn));
        presenting.chain(released)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Outcome {
    Exited(u32),
    Failed,
}

impl Outcome {
    fn to_attachment(self, conn: ConnectionId) -> Effect {
        match self {
            Self::Exited(code) => Effect::DeliverExit { conn, code },
            Self::Failed => Effect::DeliverError { conn },
        }
    }

    fn to_killer(self, conn: ConnectionId) -> Effect {
        match self {
            Self::Exited(code) => Effect::ConfirmKill { conn, code },
            Self::Failed => Effect::DeliverError { conn },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Ending {
    /// The PTY is hung up; the command has not exited yet.
    AwaitingCommand {
        failed: bool,
    },
    CleaningUp(Outcome),
    Finished(Outcome),
}

/// A session that refuses attachments and input until it finishes.
#[derive(Debug)]
pub(super) struct Shutdown {
    members: Members,
    killers: Vec<ConnectionId>,
    ending: Ending,
}

impl Shutdown {
    pub fn begin(members: Members, killers: Vec<ConnectionId>, failed: bool) -> Self {
        Self {
            members,
            killers,
            ending: Ending::AwaitingCommand { failed },
        }
    }

    pub fn exited(members: Members, code: u32) -> Self {
        Self {
            members,
            killers: Vec::new(),
            ending: Ending::CleaningUp(Outcome::Exited(code)),
        }
    }

    pub fn count(&self) -> usize {
        self.members.count()
    }

    pub fn lost(&mut self, conn: ConnectionId) {
        self.members.lost(conn);
        self.killers.retain(|killer| *killer != conn);
    }

    /// Why a request needing control is refused: a remaining attachment
    /// can still leave, but takes on no new work.
    pub fn refusal(&self, conn: ConnectionId) -> OperationalCode {
        if self.members.contains(conn) {
            OperationalCode::SessionTerminating
        } else {
            OperationalCode::NotAttached
        }
    }

    pub fn leave(&mut self, conn: ConnectionId) -> Vec<Effect> {
        self.members.leave(conn)
    }

    /// Repeated kills join the shutdown already under way.
    pub fn kill(&mut self, conn: ConnectionId) -> Vec<Effect> {
        match self.ending {
            Ending::Finished(outcome) => vec![outcome.to_killer(conn)],
            _ => {
                self.killers.push(conn);
                Vec::new()
            }
        }
    }

    pub fn command_exited(&mut self, code: u32) -> Vec<Effect> {
        match self.ending {
            Ending::AwaitingCommand { failed } => {
                self.ending = Ending::CleaningUp(outcome(failed, code));
                vec![Effect::Cleanup]
            }
            _ => Vec::new(),
        }
    }

    pub fn cleanup_finished(&mut self) -> Vec<Effect> {
        match self.ending {
            Ending::CleaningUp(outcome) => self.finish(outcome),
            _ => Vec::new(),
        }
    }

    pub fn authoritative_failure(&mut self) -> Vec<Effect> {
        match self.ending {
            Ending::AwaitingCommand { .. } => {
                self.ending = Ending::AwaitingCommand { failed: true };
                Vec::new()
            }
            Ending::CleaningUp(_) => self.finish(Outcome::Failed),
            Ending::Finished(_) => Vec::new(),
        }
    }

    /// Releases before every result; a kill is confirmed only after cleanup.
    fn finish(&mut self, outcome: Outcome) -> Vec<Effect> {
        self.ending = Ending::Finished(outcome);
        let killers = std::mem::take(&mut self.killers).into_iter();
        std::mem::take(&mut self.members)
            .deliver(outcome)
            .chain(killers.map(|conn| outcome.to_killer(conn)))
            .chain([Effect::Finish])
            .collect()
    }
}

fn outcome(failed: bool, code: u32) -> Outcome {
    if failed {
        Outcome::Failed
    } else {
        Outcome::Exited(code)
    }
}
