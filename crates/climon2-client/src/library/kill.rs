//! A Kill's auxiliary connection (#96): its own hello-validated
//! connection, whose first and only role is one standalone Kill, never the
//! instance's opened stream. It carries one request, wire ID 1, and one
//! answer, after which the library closes it. Whatever ends it gives the
//! Kill exactly one outcome: the daemon's answer, or indeterminate when the
//! answer was lost, malformed or late. Nothing here sends it again.

use super::{
    failure::{Fault, Retry, require},
    handshake::Peer,
    instance::Descriptor,
    link::{self, Link, TIMEOUT, fatal},
    records::{Answer, Command, Response},
    stream::{Carried, MISMATCHED, UNCORRELATED, Verdict},
};
use climon2_core::{
    native,
    wire::{self, ControlRecord, Counter, Families, Frame, PartialCleanup, ServerMessage, Success},
};
use std::time::Instant;

/// The only request on the connection.
const WIRE_ID: u64 = 1;
/// Daemon wire revision 1.
const REVISION: u32 = 1;

const UNEXPECTED: Fault = Fault::transport(
    "invalid_record",
    "the session daemon sent a Kill's connection a record other than its answer",
);
const UNFIT: Fault = Fault::library(
    "unsupported_limit",
    "a request frame exceeds the frame ceilings",
);

/// A Kill sent and not answered yet.
pub(super) struct Errand<'a> {
    /// The caller's public request ID.
    id: u64,
    link: Link,
    peer: Peer,
    /// `kill_ms` after the Kill started.
    deadline: Instant,
    _counted: Descriptor<'a>,
}

/// A Kill's outcome, and the retry it calls for when it ends a Kill-only
/// instance.
pub(super) struct Settled {
    pub(super) response: Response,
    pub(super) retry: Retry,
}

impl Settled {
    /// The answer never came, or broke the wire's rules: the Kill's effect
    /// is unknown, and a fresh passive instance may learn it.
    fn uncertain(id: u64, fault: Fault) -> Self {
        Self {
            response: Response::uncertain(id, fault),
            retry: Retry::Passive,
        }
    }
}

impl<'a> Errand<'a> {
    /// Sends the Kill for public request `id` on its hello-validated
    /// `link`; a frame that does not fit is refused unsent.
    pub(super) fn send(
        id: u64,
        required: Families,
        (mut link, peer): (Link, Peer),
        deadline: Instant,
        counted: Descriptor<'a>,
    ) -> Result<Self, Fault> {
        let record = wire::Kill {
            request_id: Counter::new(WIRE_ID).expect("one is a counter"),
            revision: REVISION,
            required_capabilities: required,
        };
        let mut bytes = Vec::new();
        wire::write_frame(&mut bytes, &record.to_frame(), peer.send).map_err(|_| UNFIT)?;
        link.send(&bytes);
        Ok(Self {
            id,
            link,
            peer,
            deadline,
            _counted: counted,
        })
    }

    pub(super) fn link(&self) -> &Link {
        &self.link
    }

    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Waits for the Kill's outcome alone, bounded by `kill_ms`.
    pub(super) fn outcome(mut self) -> Settled {
        loop {
            if let Some(settled) = self.service() {
                return settled;
            }
            link::wait(None, &[&self.link], Some(self.deadline));
        }
    }

    /// Writes what waits, reads what arrived, then checks `kill_ms`: the
    /// Kill's outcome once it has one.
    pub(super) fn service(&mut self) -> Option<Settled> {
        let id = self.id;
        let received = self
            .link
            .flush()
            .and_then(|()| self.link.receive(self.peer.receive));
        match received {
            Ok(Some(frame)) => {
                Some(answer(id, &frame).unwrap_or_else(|fault| Settled::uncertain(id, fault)))
            }
            Ok(None) => link::remaining(self.deadline)
                .is_none()
                .then(|| Settled::uncertain(id, TIMEOUT)),
            Err(fault) => Some(Settled::uncertain(id, fault)),
        }
    }
}

/// The daemon's one frame on the Kill's connection: its answer, or a
/// Fatal error after which the Kill's effect is unknown.
fn answer(id: u64, frame: &Frame) -> Result<Settled, Fault> {
    match ServerMessage::from_frame(frame).map_err(fatal)? {
        ServerMessage::Response(response) => {
            require(response.request_id.get() == WIRE_ID, UNCORRELATED)?;
            resolve(id, Verdict::from(response.outcome))
        }
        ServerMessage::FatalError(wire::FatalError(error)) => {
            Ok(Settled::uncertain(id, Fault::daemon(&error)))
        }
        _ => Err(UNEXPECTED),
    }
}

/// A Kill's outcome with what it knows: a confirmed termination, a
/// rejection's known final result, or a partial termination.
fn resolve(id: u64, verdict: Verdict) -> Result<Settled, Fault> {
    let Verdict {
        outcome,
        carried,
        retry,
    } = verdict;
    let answer = carried.map(termination).transpose()?;
    Ok(Settled {
        response: Response {
            id,
            outcome,
            answer,
        },
        retry,
    })
}

fn termination(carried: Carried) -> Result<Answer, Fault> {
    match carried {
        Carried::Success(Success::Termination { command_result, .. }) => Ok(Answer::Termination {
            command: Some(Command::from(&command_result)),
            cleanup: native::CLEANUP_COMPLETE,
        }),
        Carried::KnownFinal(result) => Ok(Answer::KnownFinal(Command::from(&result))),
        Carried::Termination(partial) => Ok(Answer::Termination {
            command: partial.command_result.as_ref().map(Command::from),
            cleanup: cleanup(partial.cleanup),
        }),
        Carried::Success(_) => Err(MISMATCHED),
    }
}

/// `CM1_CLEANUP_INCOMPLETE` or `CM1_CLEANUP_UNKNOWN`.
fn cleanup(cleanup: PartialCleanup) -> u32 {
    match cleanup {
        PartialCleanup::Incomplete => native::CLEANUP_INCOMPLETE,
        PartialCleanup::Unknown => native::CLEANUP_UNKNOWN,
    }
}
