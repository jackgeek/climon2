use super::{ConnectionId, Effect};
use climon2_core::{
    exit::{CLIMON2_ERROR, ExitStatus},
    wire::{CommandResult, OperationalCode},
};

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

    fn deliver(self, end: &SessionEnd) -> impl Iterator<Item = Effect> {
        let result = end.private_result();
        let presenting = self
            .presenting
            .into_iter()
            .flat_map(move |conn| [Effect::Release { conn }, to_attachment(conn, result)]);
        let released = self
            .released
            .into_iter()
            .map(move |conn| to_attachment(conn, result));
        presenting.chain(released)
    }
}

/// An attachment's result in the private protocol: `Exit`, or `Error`.
fn to_attachment(conn: ConnectionId, result: PrivateResult) -> Effect {
    match result {
        PrivateResult::Status(code) => Effect::DeliverExit { conn, code },
        PrivateResult::Error => Effect::DeliverError { conn },
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

    /// The private protocol reports climon2's own status for an end it
    /// could not confirm, whatever it knew.
    fn private_status(&self) -> u32 {
        match self {
            Self::Confirmed(result) => private_status(result),
            Self::Unconfirmed(_) => CLIMON2_ERROR,
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

/// What the private protocol reports when the session ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivateResult {
    /// `Exit` to an attachment, `Terminated` to a kill requester.
    Status(u32),
    /// `Error`, describing the failure.
    Error,
}

/// How a session ended, as every result it delivers reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionEnd {
    pub command: CommandEnd,
    /// An authoritative failure ended the session.
    pub failed: bool,
    pub cleanup: CleanupResult,
}

impl SessionEnd {
    /// The private protocol's report: the command's status, with a signal
    /// folded into 128 plus its number, or climon2's own when the command's
    /// end was not confirmed; an error after a failure of the session or of
    /// its cleanup.
    pub fn private_result(&self) -> PrivateResult {
        if self.failed || self.cleanup == CleanupResult::Failed {
            return PrivateResult::Error;
        }
        PrivateResult::Status(self.command.private_status())
    }
}

/// Core's exit mapping, which the private protocol keeps.
fn private_status(result: &CommandResult) -> u32 {
    let status = match result {
        CommandResult::Exit { code } => ExitStatus::Command(*code),
        CommandResult::Signal { number, .. } => ExitStatus::Signal(*number),
    };
    status.code().unwrap_or(CLIMON2_ERROR)
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Ending {
    /// The PTY is hung up; the command has not exited yet.
    AwaitingCommand {
        failed: bool,
    },
    /// The command ended; the daemon is cleaning up.
    CleaningUp {
        command: CommandEnd,
        failed: bool,
    },
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
    pub fn begin(members: Members, killers: Vec<ConnectionId>, failed: bool) -> Self {
        Self {
            members,
            killers,
            ending: Ending::AwaitingCommand { failed },
        }
    }

    pub fn exited(members: Members, command: CommandEnd) -> Self {
        Self {
            members,
            killers: Vec::new(),
            ending: Ending::CleaningUp {
                command,
                failed: false,
            },
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
        let Ending::AwaitingCommand { failed } = self.ending else {
            return Vec::new();
        };
        self.ending = Ending::CleaningUp { command, failed };
        vec![Effect::Cleanup]
    }

    pub fn cleanup_finished(&mut self) -> Vec<Effect> {
        self.finish(CleanupResult::Complete)
    }

    /// A failure while cleaning up leaves the cleanup incomplete.
    pub fn authoritative_failure(&mut self) -> Vec<Effect> {
        match self.ending {
            Ending::AwaitingCommand { .. } => {
                self.ending = Ending::AwaitingCommand { failed: true };
                Vec::new()
            }
            Ending::CleaningUp { .. } => self.finish(CleanupResult::Failed),
            Ending::Finished(_) => Vec::new(),
        }
    }

    /// Releases before every result; a kill is confirmed only after cleanup.
    fn finish(&mut self, cleanup: CleanupResult) -> Vec<Effect> {
        let Ending::CleaningUp { command, failed } = &self.ending else {
            return Vec::new();
        };
        let end = SessionEnd {
            command: command.clone(),
            failed: *failed,
            cleanup,
        };
        self.ending = Ending::Finished(end.clone());
        let killers = std::mem::take(&mut self.killers).into_iter();
        std::mem::take(&mut self.members)
            .deliver(&end)
            .chain(killers.map(|conn| Effect::ConfirmKill {
                conn,
                end: end.clone(),
            }))
            .chain([Effect::Finish])
            .collect()
    }
}
