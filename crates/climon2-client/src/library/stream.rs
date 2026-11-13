//! One daemon connection's revision-1 protocol state, without I/O: the
//! requests the library sent and are still unanswered, what the Open
//! selected, and what the daemon may send next. Every record the daemon
//! sends maps to a native record or an explicit connection failure; nothing
//! is skipped or fabricated.

use super::{
    failure::{Failed, Fault, Retry, require},
    families,
    link::fatal,
    records::{Answer, Command, Event, Metadata, Response, Title},
};
use climon2_core::{
    native,
    wire::{
        self, Available, ControlRecord, Counter, Families, Family, Frame, FrameLimit, KnownResult,
        Mode, Outcome, Requested, ServerMessage, Success,
    },
};

/// Daemon wire revision 1.
const REVISION: u32 = 1;

const UNEXPECTED: Fault = Fault::transport(
    "invalid_record",
    "the session daemon sent a record out of order or for an unselected family",
);
const MISMATCHED: Fault = Fault::transport(
    "invalid_record",
    "a response's result does not belong to its request",
);
const UNCORRELATED: Fault = Fault::transport(
    "invalid_request_id",
    "a response names no unanswered request",
);
const OVERSIZED: Fault = Fault::library("resource_limit", "a title exceeds title_bytes");
const UNFIT: Fault = Fault::library(
    "unsupported_limit",
    "a request frame exceeds the frame ceilings",
);

/// A request the library sent and the daemon has not answered.
#[derive(Clone, Debug)]
enum Sent {
    Open {
        id: u64,
        requested: Requested,
    },
    /// Without a public ID, shutdown's own Leave.
    Leave {
        id: Option<u64>,
    },
}

/// Leave evidence: whether a departure was required and what proves it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Departure {
    leave: u32,
    detached: bool,
    fault: Option<Fault>,
}

/// Each response outcome and the Leave evidence it gives.
const EVIDENCE: [(u32, u32); 4] = [
    (native::OK, native::LEAVE_CONFIRMED),
    (native::REJECTED, native::LEAVE_REJECTED),
    (native::FAILED, native::LEAVE_FAILED),
    (native::INDETERMINATE, native::LEAVE_INDETERMINATE),
];

impl Departure {
    /// No registration remains possible.
    pub(super) const NOT_REQUIRED: Self = Self {
        leave: native::LEAVE_NOT_REQUIRED,
        detached: false,
        fault: None,
    };

    /// A Leave whose response was lost or never came: unconfirmed.
    pub(super) fn lost(fault: Fault) -> Self {
        Self {
            leave: native::LEAVE_INDETERMINATE,
            detached: false,
            fault: Some(fault),
        }
    }

    /// Only a confirmed departure can have detached an attachment.
    fn answered(outcome: u32, detached: bool, fault: Option<Fault>) -> Self {
        let leave = EVIDENCE
            .into_iter()
            .find(|(answered, _)| *answered == outcome)
            .map_or(native::LEAVE_INDETERMINATE, |(_, leave)| leave);
        Self {
            leave,
            detached: detached && leave == native::LEAVE_CONFIRMED,
            fault,
        }
    }

    pub(super) fn leave(&self) -> u32 {
        self.leave
    }

    pub(super) fn detached(&self) -> bool {
        self.detached
    }

    /// The error of a departure that was not confirmed.
    pub(super) fn fault(&self) -> Option<&Fault> {
        self.fault.as_ref()
    }

    /// Whether the registration certainly remains: a rejected Leave.
    pub(super) fn kept(&self) -> bool {
        self.leave == native::LEAVE_REJECTED
    }
}

/// What one daemon record means for the connection.
#[derive(Debug)]
pub(super) enum Effect {
    /// A record to deliver, when the event allowance has room.
    Record(Event),
    /// The Open's outcome, to deliver; then the role is open or refused.
    Opened(Response, Result<(), Failed>),
    /// A Leave's outcome: delivered when public, and its evidence.
    Left(Option<Response>, Departure),
    /// The daemon reported a connection-fatal error.
    Fatal(Fault),
}

impl Effect {
    /// Whether this record makes the command's final result known.
    pub(super) fn concludes(&self) -> bool {
        match self {
            Self::Record(Event::Metadata(metadata)) => metadata.command.is_some(),
            Self::Opened(response, _) => matches!(response.answer, Answer::KnownFinal(_)),
            _ => false,
        }
    }
}

pub(super) struct Stream {
    title_bytes: u64,
    /// The last wire request ID sent; the next is one more.
    last: u64,
    pending: Vec<(u64, Sent)>,
    /// After the Open's success: whether `title-v1` is selected.
    title: Option<bool>,
    /// Whether the Initial metadata arrived.
    synchronized: bool,
}

impl Stream {
    pub(super) fn new(title_bytes: u32) -> Self {
        Self {
            title_bytes: title_bytes.into(),
            last: 0,
            pending: Vec::new(),
            title: None,
            synchronized: false,
        }
    }

    /// Encodes `record` as the next request; it counts as sent, and its
    /// wire ID as used, only once it fits `limit`.
    fn send(
        &mut self,
        sent: Sent,
        record: impl FnOnce(Counter) -> Frame,
        limit: FrameLimit,
    ) -> Result<Vec<u8>, Fault> {
        let request_id = Counter::new(self.last + 1).expect("wire request IDs start at one");
        let mut bytes = Vec::new();
        wire::write_frame(&mut bytes, &record(request_id), limit).map_err(|_| UNFIT)?;
        self.last = request_id.get();
        self.pending.push((self.last, sent));
        Ok(bytes)
    }

    /// A monitor-mode Open of the `requested` families, for public request
    /// `id`.
    pub(super) fn open(
        &mut self,
        id: u64,
        requested: Requested,
        limit: FrameLimit,
    ) -> Result<Vec<u8>, Fault> {
        let capabilities = requested.clone();
        let record = |request_id| {
            wire::Open {
                request_id,
                revision: REVISION,
                capabilities,
                mode: Mode::Monitor,
            }
            .to_frame()
        };
        self.send(Sent::Open { id, requested }, record, limit)
    }

    /// A Leave, for public request `id`, or shutdown's own without one.
    pub(super) fn leave(&mut self, id: Option<u64>, limit: FrameLimit) -> Result<Vec<u8>, Fault> {
        let record = |request_id| wire::Leave { request_id }.to_frame();
        self.send(Sent::Leave { id }, record, limit)
    }

    /// Gives up the request a deadline bounded, an unanswered Leave or else
    /// the unanswered Open: its public ID, if it has one.
    pub(super) fn expire(&mut self) -> Option<u64> {
        let leave = self
            .pending
            .iter()
            .position(|(_, sent)| matches!(sent, Sent::Leave { .. }));
        let index = leave.or_else(|| {
            self.pending
                .iter()
                .position(|(_, sent)| matches!(sent, Sent::Open { .. }))
        })?;
        match self.pending.remove(index).1 {
            Sent::Open { id, .. } => Some(id),
            Sent::Leave { id } => id,
        }
    }

    /// Whether a Leave is unanswered.
    pub(super) fn leaving(&self) -> bool {
        self.pending
            .iter()
            .any(|(_, sent)| matches!(sent, Sent::Leave { .. }))
    }

    /// Whether the Initial metadata arrived.
    pub(super) fn synchronized(&self) -> bool {
        self.synchronized
    }

    /// One frame from the daemon.
    pub(super) fn receive(&mut self, frame: &Frame) -> Result<Effect, Fault> {
        match ServerMessage::from_frame(frame).map_err(fatal)? {
            ServerMessage::Response(response) => self.respond(response),
            other => self.other(other),
        }
    }

    fn other(&mut self, message: ServerMessage) -> Result<Effect, Fault> {
        match message {
            ServerMessage::FatalError(wire::FatalError(error)) => {
                Ok(Effect::Fatal(Fault::daemon(&error)))
            }
            message => self.report(message).map(Effect::Record),
        }
    }

    /// Initial metadata, then title changes; nothing else is selected.
    fn report(&mut self, message: ServerMessage) -> Result<Event, Fault> {
        match message {
            ServerMessage::InitialMetadata(metadata) => self.initial(&metadata),
            ServerMessage::TitleChanged(changed) => self.changed(changed.title.as_deref()),
            _ => Err(UNEXPECTED),
        }
    }

    fn awaiting_metadata(&self) -> bool {
        self.title.is_some() && !self.synchronized
    }

    fn respond(&mut self, response: wire::Response) -> Result<Effect, Fault> {
        let sent = self.answered(response.request_id)?;
        Verdict::from(response.outcome).and_then(|verdict| self.resolve(sent, verdict))
    }

    fn resolve(&mut self, sent: Sent, verdict: Verdict) -> Result<Effect, Fault> {
        match sent {
            Sent::Open { id, requested } => self.opened(id, &requested, verdict),
            Sent::Leave { id } => left(id, verdict),
        }
    }

    fn answered(&mut self, request_id: Counter) -> Result<Sent, Fault> {
        require(!self.awaiting_metadata(), UNEXPECTED)?;
        let index = self
            .pending
            .iter()
            .position(|(wire, _)| *wire == request_id.get())
            .ok_or(UNCORRELATED)?;
        Ok(self.pending.remove(index).1)
    }

    fn opened(&mut self, id: u64, sent: &Requested, verdict: Verdict) -> Result<Effect, Fault> {
        let Verdict {
            outcome,
            fault,
            carried,
            retry,
        } = verdict;
        let answer = match carried {
            None => Answer::None,
            Some(carried) => self.result(carried, sent)?,
        };
        let refused = fault.clone().map(|fault| Failed::new(fault, retry));
        let response = Response {
            id,
            outcome,
            answer,
            fault,
        };
        Ok(Effect::Opened(response, refused.map_or(Ok(()), Err)))
    }

    /// An Open's result: what it opened, or the command result a rejection
    /// knew.
    fn result(&mut self, carried: Carried, sent: &Requested) -> Result<Answer, Fault> {
        match carried {
            Carried::Success(Success::Opened {
                connection_id,
                selected_capabilities,
            }) => self.selected(&connection_id, &selected_capabilities, sent),
            Carried::KnownFinal(result) => Ok(Answer::KnownFinal(Command::from(&result))),
            Carried::Success(_) => Err(MISMATCHED),
        }
    }

    fn selected(
        &mut self,
        connection: &wire::Guid,
        selection: &Families,
        sent: &Requested,
    ) -> Result<Answer, Fault> {
        let selected = families::selected(selection, sent).ok_or(MISMATCHED)?;
        self.title = Some(selected.contains(Family::TitleV1));
        Ok(Answer::Opened {
            connection: connection.bytes(),
            selected,
        })
    }

    fn initial(&mut self, metadata: &wire::InitialMetadata) -> Result<Event, Fault> {
        require(self.awaiting_metadata(), UNEXPECTED)?;
        let event = self.metadata(metadata)?;
        self.synchronized = true;
        Ok(event)
    }

    /// Status and progress are never selected yet, so both must be
    /// unavailable; the title is available exactly when selected.
    fn metadata(&self, metadata: &wire::InitialMetadata) -> Result<Event, Fault> {
        let title = self.availability(metadata.title())?;
        let unselected = matches!(metadata.status(), Available::Unavailable)
            && matches!(metadata.progress(), Available::Unavailable);
        unselected
            .then(|| {
                Event::Metadata(Metadata {
                    title,
                    command: metadata.command_result().map(Command::from),
                })
            })
            .ok_or(UNEXPECTED)
    }

    fn availability(&self, title: &Available<Option<String>>) -> Result<Title, Fault> {
        match (self.title == Some(true), title) {
            (false, Available::Unavailable) => Ok(Title::Unavailable),
            (true, Available::Available(value)) => self.value(value.as_deref()),
            _ => Err(UNEXPECTED),
        }
    }

    fn changed(&self, title: Option<&str>) -> Result<Event, Fault> {
        require(self.synchronized && self.title == Some(true), UNEXPECTED)?;
        self.value(title).map(Event::Title)
    }

    /// A title by value, never truncated.
    fn value(&self, title: Option<&str>) -> Result<Title, Fault> {
        title.map_or(Ok(Title::Unset), |title| {
            (title.len() as u64 <= self.title_bytes)
                .then(|| Title::Value(Box::from(title)))
                .ok_or(OVERSIZED)
        })
    }
}

/// A Leave's answer: its response when public, and its evidence.
fn left(id: Option<u64>, verdict: Verdict) -> Result<Effect, Fault> {
    let Verdict {
        outcome,
        fault,
        carried,
        ..
    } = verdict;
    let answer = match carried {
        None => Answer::None,
        Some(Carried::Success(Success::Left {
            attachment_detached,
        })) => Answer::Left {
            detached: attachment_detached,
        },
        Some(_) => return Err(MISMATCHED),
    };
    let detached = matches!(answer, Answer::Left { detached: true });
    let departure = Departure::answered(outcome, detached, fault.clone());
    let response = id.map(|id| Response {
        id,
        outcome,
        answer,
        fault,
    });
    Ok(Effect::Left(response, departure))
}

/// What a response may carry: an operation's result, or a rejection's
/// known final command result.
#[derive(Debug)]
enum Carried {
    Success(Success),
    KnownFinal(wire::CommandResult),
}

/// A response's native outcome, its error, the result it carries and, for
/// a non-success, the retry its error calls for.
struct Verdict {
    outcome: u32,
    fault: Option<Fault>,
    carried: Option<Carried>,
    retry: Retry,
}

impl Verdict {
    fn from(outcome: Outcome) -> Result<Self, Fault> {
        match outcome {
            Outcome::Ok(success) => Ok(Self {
                outcome: native::OK,
                fault: None,
                carried: Some(Carried::Success(success)),
                retry: Retry::None,
            }),
            Outcome::Rejected(rejection) => Ok(Self::of(
                native::REJECTED,
                &rejection.error,
                rejection.known_final.map(Carried::KnownFinal),
            )),
            Outcome::Failed(failure) => Self::failure(native::FAILED, &failure),
            Outcome::Indeterminate(failure) => Self::failure(native::INDETERMINATE, &failure),
        }
    }

    /// A non-success with the daemon's `error`.
    fn of(outcome: u32, error: &wire::Error, carried: Option<Carried>) -> Self {
        Self {
            outcome,
            fault: Some(Fault::daemon(error)),
            carried,
            retry: Retry::refused(&error.code),
        }
    }

    /// A failed or indeterminate outcome keeps a known result of its own
    /// operation; a Kill's partial termination belongs to no request here.
    fn failure(outcome: u32, failure: &wire::Failure) -> Result<Self, Fault> {
        let carried = match failure.known() {
            None => None,
            Some(KnownResult::Operation(success)) => Some(Carried::Success(success.clone())),
            Some(KnownResult::Termination(_)) => return Err(MISMATCHED),
        };
        Ok(Self::of(outcome, failure.error(), carried))
    }
}
