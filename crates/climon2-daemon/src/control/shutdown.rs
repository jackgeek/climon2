use super::{ConnectionId, Effect};
use climon2_core::wire::{CommandResult, OperationalCode};

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

    /// The session ended: the presentation the controller still has is
    /// released, then every attachment's control ends with the session.
    fn end(self) -> impl Iterator<Item = Effect> {
        let presenting = self
            .presenting
            .into_iter()
            .flat_map(|conn| [Effect::Release { conn }, Effect::EndAttachment { conn }]);
        let released = self
            .released
            .into_iter()
            .map(|conn| Effect::EndAttachment { conn });
        presenting.chain(released)
    }
}

/// How the daemon learned that the command ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandEnd {
    /// The command ended with this result, and nothing of its terminal
    /// session remains.
    Confirmed(CommandResult),
    /// The daemon could not confirm the end: its hang-up, or the kill of
    /// what remained of the command's terminal session, failed. The result
    /// it knew already, from reaping the command earlier, if any.
    Unconfirmed(Option<CommandResult>),
}

impl CommandEnd {
    /// The command's result, whether or not its end was confirmed.
    pub fn known(&self) -> Option<&CommandResult> {
        match self {
            Self::Confirmed(result) => Some(result),
            Self::Unconfirmed(result) => result.as_ref(),
        }
    }
}

/// How the daemon's cleanup of the session went: removing its session
/// metadata and socket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupResult {
    Complete,
    Failed,
}

/// How a session ended, as a Kill's answer reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionEnd {
    pub command: CommandEnd,
    pub cleanup: CleanupResult,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Ending {
    /// The PTY is hung up; the command has not exited yet.
    AwaitingCommand,
    /// The command ended; the daemon is cleaning up.
    CleaningUp(CommandEnd),
    Finished(SessionEnd),
}

/// A session that refuses attachments and input until it finishes.
#[derive(Debug)]
pub(super) struct Shutdown {
    members: Members,
    killers: Vec<ConnectionId>,
    ending: Ending,
}

impl Shutdown {
    pub fn begin(members: Members, killers: Vec<ConnectionId>) -> Self {
        Self {
            members,
            killers,
            ending: Ending::AwaitingCommand,
        }
    }

    pub fn exited(members: Members, command: CommandEnd) -> Self {
        Self {
            members,
            killers: Vec::new(),
            ending: Ending::CleaningUp(command),
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
        if let Ending::Finished(end) = &self.ending {
            let end = end.clone();
            return vec![Effect::ConfirmKill { conn, end }];
        }
        self.killers.push(conn);
        Vec::new()
    }

    /// The command ended, as the daemon learned it.
    pub fn command_exited(&mut self, command: CommandEnd) -> Vec<Effect> {
        if self.ending != Ending::AwaitingCommand {
            return Vec::new();
        }
        self.ending = Ending::CleaningUp(command);
        vec![Effect::Cleanup]
    }

    pub fn cleanup_finished(&mut self) -> Vec<Effect> {
        self.finish(CleanupResult::Complete)
    }

    /// A failure while cleaning up leaves the cleanup incomplete; any other
    /// changes nothing, since the session already terminates.
    pub fn authoritative_failure(&mut self) -> Vec<Effect> {
        self.finish(CleanupResult::Failed)
    }

    /// Releases before every result; a kill is confirmed only after cleanup.
    fn finish(&mut self, cleanup: CleanupResult) -> Vec<Effect> {
        let Ending::CleaningUp(command) = &self.ending else {
            return Vec::new();
        };
        let end = SessionEnd {
            command: command.clone(),
            cleanup,
        };
        self.ending = Ending::Finished(end.clone());
        let killers = std::mem::take(&mut self.killers).into_iter();
        std::mem::take(&mut self.members)
            .end()
            .chain(killers.map(|conn| Effect::ConfirmKill {
                conn,
                end: end.clone(),
            }))
            .chain([Effect::Finish])
            .collect()
    }
}
