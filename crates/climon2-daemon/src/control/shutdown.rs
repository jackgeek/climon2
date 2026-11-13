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

    fn deliver(self, outcome: &Outcome) -> impl Iterator<Item = Effect> {
        let code = outcome.private_code();
        let presenting = self
            .presenting
            .into_iter()
            .flat_map(move |conn| [Effect::Release { conn }, to_attachment(conn, code)]);
        let released = self
            .released
            .into_iter()
            .map(move |conn| to_attachment(conn, code));
        presenting.chain(released)
    }
}

/// An attachment's result in the private protocol: `Exit`, or `Error`.
fn to_attachment(conn: ConnectionId, code: Option<u32>) -> Effect {
    match code {
        Some(code) => Effect::DeliverExit { conn, code },
        None => Effect::DeliverError { conn },
    }
}

/// How the session ended, as every result it delivers reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Outcome {
    /// The command's result, once the daemon confirmed how the command
    /// ended; `None` when it could not.
    pub command: Option<CommandResult>,
    /// An authoritative failure ended the session.
    pub failed: bool,
    /// The daemon's cleanup of the session completed.
    pub cleaned: bool,
}

impl Outcome {
    /// The private protocol's status for this ending: the command's, with a
    /// signal folded into 128 plus its number, or climon2's own when the
    /// command's end was not confirmed. `None`, reported as an error, after
    /// a failure of the session or of its cleanup.
    pub fn private_code(&self) -> Option<u32> {
        if self.failed || !self.cleaned {
            return None;
        }
        Some(self.command.as_ref().map_or(CLIMON2_ERROR, private_status))
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
        command: Option<CommandResult>,
        failed: bool,
    },
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

    pub fn exited(members: Members, command: Option<CommandResult>) -> Self {
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
        if let Ending::Finished(outcome) = &self.ending {
            let outcome = outcome.clone();
            return vec![Effect::ConfirmKill { conn, outcome }];
        }
        self.killers.push(conn);
        Vec::new()
    }

    /// The command ended: its result, or `None` when the daemon could not
    /// confirm how.
    pub fn command_exited(&mut self, command: Option<CommandResult>) -> Vec<Effect> {
        let Ending::AwaitingCommand { failed } = self.ending else {
            return Vec::new();
        };
        self.ending = Ending::CleaningUp { command, failed };
        vec![Effect::Cleanup]
    }

    pub fn cleanup_finished(&mut self) -> Vec<Effect> {
        self.finish(true)
    }

    /// A failure while cleaning up leaves the cleanup incomplete.
    pub fn authoritative_failure(&mut self) -> Vec<Effect> {
        match self.ending {
            Ending::AwaitingCommand { .. } => {
                self.ending = Ending::AwaitingCommand { failed: true };
                Vec::new()
            }
            Ending::CleaningUp { .. } => self.finish(false),
            Ending::Finished(_) => Vec::new(),
        }
    }

    /// Releases before every result; a kill is confirmed only after cleanup.
    fn finish(&mut self, cleaned: bool) -> Vec<Effect> {
        let Ending::CleaningUp { command, failed } = &self.ending else {
            return Vec::new();
        };
        let outcome = Outcome {
            command: command.clone(),
            failed: *failed,
            cleaned,
        };
        self.ending = Ending::Finished(outcome.clone());
        let killers = std::mem::take(&mut self.killers).into_iter();
        std::mem::take(&mut self.members)
            .deliver(&outcome)
            .chain(killers.map(|conn| Effect::ConfirmKill {
                conn,
                outcome: outcome.clone(),
            }))
            .chain([Effect::Finish])
            .collect()
    }
}
