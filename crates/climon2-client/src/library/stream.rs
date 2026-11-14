//! One daemon connection's revision-1 protocol state, without I/O: the
//! requests the library sent and are still unanswered, what the Open
//! selected, and what the daemon may send next. Every record the daemon
//! sends maps to a native record or an explicit connection failure; nothing
//! is skipped or fabricated. Presentation records must name the authority
//! the role holds (#94): an old period or restoration never reaches the
//! caller as current.

use super::{
    failure::{Failed, Fault, Retry, require},
    families::{self, Selection},
    link::fatal,
    records::{
        Answer, Command, Done, Event, Initial, Metadata, Outcome, Presentation, Response, Screen,
        Title,
    },
    request::{Act, Held},
};
use climon2_core::{
    native,
    wire::{
        self, AuthorityIds, Available, ControlRecord, Counter, Families, Family, Frame, FrameLimit,
        KnownResult, Mode, Requested, ServerMessage, Success,
    },
};
use std::time::Instant;

/// Daemon wire revision 1.
const REVISION: u32 = 1;

const UNEXPECTED: Fault = Fault::transport(
    "invalid_record",
    "the session daemon sent a record out of order or for an unselected family",
);
const UNHELD: Fault = Fault::transport(
    "invalid_record",
    "a record names an authority the connection does not hold",
);
const EMPTY_DISPLAY: Fault = Fault::transport("invalid_record", "a Display carries no bytes");
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
    Command {
        id: u64,
        expects: Expects,
    },
}

/// The result a command's success must carry: its own kind and, for
/// Input, every byte it carried.
#[derive(Clone, Copy, Debug)]
struct Expects {
    result: fn(Success) -> Option<Done>,
    input: Option<u32>,
}

impl Expects {
    fn answer(self, success: Success) -> Result<Answer, Fault> {
        let done = (self.result)(success).ok_or(MISMATCHED)?;
        let whole = self.input.is_none_or(|length| done == Done::Input(length));
        whole.then_some(Answer::Done(done)).ok_or(MISMATCHED)
    }
}

fn acquired(success: Success) -> Option<Done> {
    match success {
        Success::Acquired(result) => Some(Done::Authority(native::ACQUIRED, result)),
        _ => None,
    }
}

fn accepted(success: Success) -> Option<Done> {
    match success {
        Success::InputAccepted { accepted_bytes } => Some(Done::Input(accepted_bytes.get())),
        _ => None,
    }
}

fn resized(success: Success) -> Option<Done> {
    match success {
        Success::ResizeAccepted {} => Some(Done::Resized),
        _ => None,
    }
}

fn readied(success: Success) -> Option<Done> {
    match success {
        Success::Ready(result) => Some(Done::Authority(native::READY_RESULT, result)),
        _ => None,
    }
}

fn focused(success: Success) -> Option<Done> {
    match success {
        Success::Focus {
            changed,
            focused,
            accepted_bytes,
        } => Some(Done::Focus {
            changed,
            focused,
            bytes: accepted_bytes.into(),
        }),
        _ => None,
    }
}

impl Act {
    /// The result its success must carry.
    fn expects(&self) -> Expects {
        match self {
            Self::Acquire => Expects {
                result: acquired,
                input: None,
            },
            Self::Held(_, held) => held.expects(),
        }
    }

    /// Its wire request, as `request_id`.
    fn frame(self, request_id: Counter) -> Frame {
        match self {
            Self::Acquire => wire::Acquire { request_id }.to_frame(),
            Self::Held(authority, held) => held.frame(request_id, authority),
        }
    }
}

impl Held {
    fn expects(&self) -> Expects {
        let (result, input): (fn(Success) -> Option<Done>, _) = match self {
            Self::Input(payload) => (accepted, Some(payload.len() as u32)),
            Self::Resize(_) => (resized, None),
            Self::Ready => (readied, None),
            Self::Focus(_) => (focused, None),
        };
        Expects { result, input }
    }

    /// Its wire request under `authority`, as `request_id`.
    fn frame(self, request_id: Counter, authority: AuthorityIds) -> Frame {
        match self {
            Self::Input(payload) => wire::Input {
                request_id,
                authority,
                payload: payload.into_vec(),
            }
            .to_frame(),
            Self::Resize(geometry) => wire::Resize {
                request_id,
                authority,
                geometry,
            }
            .to_frame(),
            Self::Ready => wire::Ready {
                request_id,
                authority,
            }
            .to_frame(),
            Self::Focus(focused) => wire::Focus {
                request_id,
                authority,
                focused,
            }
            .to_frame(),
        }
    }
}

/// A request on the wire: its wire ID, what it was, and the deadline its
/// answer must meet, if one bounds it.
#[derive(Debug)]
struct Pending {
    wire: u64,
    sent: Sent,
    deadline: Option<Instant>,
}

/// Leave evidence: whether a departure was required and what proves it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Departure {
    leave: u32,
    detached: bool,
    fault: Option<Fault>,
}

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
    fn answered(outcome: &Outcome, detached: bool) -> Self {
        let leave = evidence(outcome);
        Self {
            leave,
            detached: detached && leave == native::LEAVE_CONFIRMED,
            fault: outcome.fault().cloned(),
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

/// The Leave evidence a Leave's outcome gives.
fn evidence(outcome: &Outcome) -> u32 {
    match outcome {
        Outcome::Ok => native::LEAVE_CONFIRMED,
        Outcome::Rejected(_) => native::LEAVE_REJECTED,
        Outcome::Failed(_) => native::LEAVE_FAILED,
        Outcome::Indeterminate(_) => native::LEAVE_INDETERMINATE,
    }
}

/// What one daemon record means for the connection.
#[derive(Debug)]
pub(super) enum Effect {
    /// A record to deliver, when the event allowance has room.
    Record(Event),
    /// The Open's outcome, to deliver; then the role is open or refused.
    Opened(Response, Result<(), Failed>),
    /// A command's outcome, to deliver.
    Answered(Response),
    /// A Leave's outcome: delivered when public, and its evidence.
    Left(Option<Response>, Departure),
}

impl Effect {
    /// Whether this record makes the command's final result known.
    pub(super) fn concludes(&self) -> bool {
        match self {
            Self::Record(Event::Metadata(Metadata::Initial(initial))) => initial.command.is_some(),
            Self::Opened(response, _) => matches!(response.answer, Some(Answer::KnownFinal(_))),
            _ => false,
        }
    }
}

pub(super) struct Stream {
    title_bytes: u64,
    /// The last wire request ID sent; the next is one more.
    last: u64,
    pending: Vec<Pending>,
    /// After the Open's success: what it selected.
    selected: Option<Selection>,
    /// Whether the Initial metadata arrived.
    synchronized: bool,
    /// The period and restoration the role holds, from its grant until its
    /// revocation.
    held: Option<AuthorityIds>,
}

impl Stream {
    pub(super) fn new(title_bytes: u32) -> Self {
        Self {
            title_bytes: title_bytes.into(),
            last: 0,
            pending: Vec::new(),
            selected: None,
            synchronized: false,
            held: None,
        }
    }

    /// Encodes `record` as the next request; it counts as sent, and its
    /// wire ID as used, only once it fits `limit`.
    fn send(
        &mut self,
        sent: Sent,
        record: impl FnOnce(Counter) -> Frame,
        (limit, deadline): (FrameLimit, Option<Instant>),
    ) -> Result<Vec<u8>, Fault> {
        let request_id = Counter::new(self.last + 1).expect("wire request IDs start at one");
        let mut bytes = Vec::new();
        wire::write_frame(&mut bytes, &record(request_id), limit).map_err(|_| UNFIT)?;
        self.last = request_id.get();
        self.pending.push(Pending {
            wire: self.last,
            sent,
            deadline,
        });
        Ok(bytes)
    }

    /// An Open of `mode` and the `requested` families, for public request
    /// `id`.
    pub(super) fn open(
        &mut self,
        id: u64,
        (mode, requested): (Mode, Requested),
        limit: FrameLimit,
    ) -> Result<Vec<u8>, Fault> {
        let capabilities = requested.clone();
        let record = |request_id| {
            wire::Open {
                request_id,
                revision: REVISION,
                capabilities,
                mode,
            }
            .to_frame()
        };
        self.send(Sent::Open { id, requested }, record, (limit, None))
    }

    /// A Leave, for public request `id`, or shutdown's own without one,
    /// answered before `deadline`.
    pub(super) fn leave(
        &mut self,
        id: Option<u64>,
        limit: FrameLimit,
        deadline: Instant,
    ) -> Result<Vec<u8>, Fault> {
        let record = |request_id| wire::Leave { request_id }.to_frame();
        self.send(Sent::Leave { id }, record, (limit, Some(deadline)))
    }

    /// A command for public request `id`, answered before `deadline`.
    pub(super) fn command(
        &mut self,
        id: u64,
        act: Act,
        limit: FrameLimit,
        deadline: Instant,
    ) -> Result<Vec<u8>, Fault> {
        let sent = Sent::Command {
            id,
            expects: act.expects(),
        };
        self.send(
            sent,
            |request_id| act.frame(request_id),
            (limit, Some(deadline)),
        )
    }

    /// The earliest deadline an unanswered request must meet.
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.pending
            .iter()
            .filter_map(|pending| pending.deadline)
            .min()
    }

    /// Gives up the request a deadline bounded: the one with the earliest
    /// deadline, or else, while the Open's start deadline is the only one
    /// (nothing else is sent before the Initial metadata), the unanswered
    /// Open. Its public ID, if it has one.
    pub(super) fn expire(&mut self) -> Option<u64> {
        let earliest = self
            .pending
            .iter()
            .enumerate()
            .filter_map(|(index, pending)| pending.deadline.map(|deadline| (deadline, index)))
            .min()
            .map(|(_, index)| index);
        let index = earliest.or_else(|| {
            self.pending
                .iter()
                .position(|pending| matches!(pending.sent, Sent::Open { .. }))
        })?;
        match self.pending.remove(index).sent {
            Sent::Open { id, .. } | Sent::Command { id, .. } => Some(id),
            Sent::Leave { id } => id,
        }
    }

    /// Whether a Leave is unanswered.
    pub(super) fn leaving(&self) -> bool {
        self.pending
            .iter()
            .any(|pending| matches!(pending.sent, Sent::Leave { .. }))
    }

    /// Whether the Open selected `focus-v1`.
    pub(super) fn focus(&self) -> bool {
        self.selects(Family::FocusV1)
    }

    fn selects(&self, family: Family) -> bool {
        self.selected
            .is_some_and(|selected| selected.contains(family))
    }

    /// Whether the Initial metadata arrived.
    pub(super) fn synchronized(&self) -> bool {
        self.synchronized
    }

    /// One frame from the daemon; a connection-fatal error it reports
    /// fails the connection with the daemon's error.
    pub(super) fn receive(&mut self, frame: &Frame) -> Result<Effect, Fault> {
        match ServerMessage::from_frame(frame).map_err(fatal)? {
            ServerMessage::Response(response) => self.respond(response),
            other => self.other(other),
        }
    }

    fn other(&mut self, message: ServerMessage) -> Result<Effect, Fault> {
        match message {
            ServerMessage::FatalError(wire::FatalError(error)) => Err(Fault::daemon(&error)),
            message => self.report(message).map(Effect::Record),
        }
    }

    /// Initial metadata, then title changes and presentation records.
    fn report(&mut self, message: ServerMessage) -> Result<Event, Fault> {
        match message {
            ServerMessage::InitialMetadata(metadata) => self.initial(&metadata),
            ServerMessage::TitleChanged(changed) => self.changed(changed.title.as_deref()),
            message => {
                require(self.synchronized, UNEXPECTED)?;
                self.present(message).map(Event::Presentation)
            }
        }
    }

    /// A grant starts a period when none is held; a revocation ends the
    /// one held; a Geometry names the held period; every other record
    /// shows its current restoration.
    fn present(&mut self, message: ServerMessage) -> Result<Presentation, Fault> {
        match message {
            ServerMessage::ControlGranted(granted) => self.grant(granted),
            ServerMessage::ControlRevoked(revoked) => self.revoke(revoked),
            ServerMessage::Geometry(actual) => self
                .in_period(actual.period)
                .map(|()| Presentation::Screen(Screen::Geometry(actual.period, actual.geometry))),
            message => self.screen(message).map(Presentation::Screen),
        }
    }

    fn grant(&mut self, granted: wire::ControlGranted) -> Result<Presentation, Fault> {
        require(self.held.is_none(), UNHELD)?;
        self.held = Some(granted.authority);
        Ok(Presentation::Granted(
            granted.authority,
            granted.geometry,
            granted.reason,
        ))
    }

    fn revoke(&mut self, revoked: wire::ControlRevoked) -> Result<Presentation, Fault> {
        self.holds(revoked.authority)?;
        self.held = None;
        Ok(Presentation::Revoked(revoked.authority, revoked.reason))
    }

    /// A beginning in the held period makes its restoration current; a
    /// Display or a complete names the current restoration.
    fn screen(&mut self, message: ServerMessage) -> Result<Screen, Fault> {
        match message {
            ServerMessage::RestorationBegin(begin) => self.begin(begin),
            ServerMessage::Display(display) => self.display(display),
            ServerMessage::RestorationComplete(complete) => self
                .holds(complete.authority)
                .map(|()| Screen::Complete(complete.authority)),
            _ => Err(UNEXPECTED),
        }
    }

    fn begin(&mut self, begin: wire::RestorationBegin) -> Result<Screen, Fault> {
        self.in_period(begin.authority.period)?;
        self.held = Some(begin.authority);
        Ok(Screen::Begin(begin.authority, begin.geometry))
    }

    fn display(&self, display: wire::Display) -> Result<Screen, Fault> {
        self.holds(display.authority)?;
        require(!display.payload.is_empty(), EMPTY_DISPLAY)?;
        Ok(Screen::Display(
            display.authority,
            display.payload.into_boxed_slice(),
        ))
    }

    fn holds(&self, authority: AuthorityIds) -> Result<(), Fault> {
        require(self.held == Some(authority), UNHELD)
    }

    fn in_period(&self, period: Counter) -> Result<(), Fault> {
        require(self.held.is_some_and(|held| held.period == period), UNHELD)
    }

    fn awaiting_metadata(&self) -> bool {
        self.selected.is_some() && !self.synchronized
    }

    fn respond(&mut self, response: wire::Response) -> Result<Effect, Fault> {
        let sent = self.answered(response.request_id)?;
        Verdict::from(response.outcome).and_then(|verdict| self.resolve(sent, verdict))
    }

    fn resolve(&mut self, sent: Sent, verdict: Verdict) -> Result<Effect, Fault> {
        match sent {
            Sent::Open { id, requested } => self.opened(id, &requested, verdict),
            Sent::Leave { id } => left(id, verdict),
            Sent::Command { id, expects } => answered(id, expects, verdict),
        }
    }

    fn answered(&mut self, request_id: Counter) -> Result<Sent, Fault> {
        require(!self.awaiting_metadata(), UNEXPECTED)?;
        let index = self
            .pending
            .iter()
            .position(|pending| pending.wire == request_id.get())
            .ok_or(UNCORRELATED)?;
        Ok(self.pending.remove(index).sent)
    }

    fn opened(&mut self, id: u64, sent: &Requested, verdict: Verdict) -> Result<Effect, Fault> {
        let Verdict {
            outcome,
            carried,
            retry,
        } = verdict;
        let answer = carried
            .map(|carried| self.result(carried, sent))
            .transpose()?;
        let refused = outcome
            .fault()
            .map(|fault| Failed::new(fault.clone(), retry));
        let response = Response {
            id,
            outcome,
            answer,
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
        self.selected = Some(selected);
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
                Event::Metadata(Metadata::Initial(Initial {
                    title,
                    command: metadata.command_result().map(Command::from),
                }))
            })
            .ok_or(UNEXPECTED)
    }

    fn availability(&self, title: &Available<Option<String>>) -> Result<Title, Fault> {
        match (self.selects(Family::TitleV1), title) {
            (false, Available::Unavailable) => Ok(Title::Unavailable),
            (true, Available::Available(value)) => self.value(value.as_deref()),
            _ => Err(UNEXPECTED),
        }
    }

    fn changed(&self, title: Option<&str>) -> Result<Event, Fault> {
        require(
            self.synchronized && self.selects(Family::TitleV1),
            UNEXPECTED,
        )?;
        self.value(title)
            .map(|title| Event::Metadata(Metadata::Title(title)))
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
        outcome, carried, ..
    } = verdict;
    let answer = carried.map(departed).transpose()?;
    let detached = matches!(answer, Some(Answer::Left { detached: true }));
    let departure = Departure::answered(&outcome, detached);
    let response = id.map(|id| Response {
        id,
        outcome,
        answer,
    });
    Ok(Effect::Left(response, departure))
}

/// A Leave's result: whether it detached an attachment.
fn departed(carried: Carried) -> Result<Answer, Fault> {
    match carried {
        Carried::Success(Success::Left {
            attachment_detached,
        }) => Ok(Answer::Left {
            detached: attachment_detached,
        }),
        _ => Err(MISMATCHED),
    }
}

/// A command's answer: its result when it succeeded, or a failure's known
/// partial result of its own operation.
fn answered(id: u64, expects: Expects, verdict: Verdict) -> Result<Effect, Fault> {
    let Verdict {
        outcome, carried, ..
    } = verdict;
    let answer = carried
        .map(|carried| match carried {
            Carried::Success(success) => expects.answer(success),
            Carried::KnownFinal(_) => Err(MISMATCHED),
        })
        .transpose()?;
    Ok(Effect::Answered(Response {
        id,
        outcome,
        answer,
    }))
}

/// What a response may carry: an operation's result, or a rejection's
/// known final command result.
#[derive(Debug)]
enum Carried {
    Success(Success),
    KnownFinal(wire::CommandResult),
}

/// A response's outcome, the result it carries and, for a non-success, the
/// retry its error calls for.
struct Verdict {
    outcome: Outcome,
    carried: Option<Carried>,
    retry: Retry,
}

impl Verdict {
    fn from(outcome: wire::Outcome) -> Result<Self, Fault> {
        match outcome {
            wire::Outcome::Ok(success) => Ok(Self {
                outcome: Outcome::Ok,
                carried: Some(Carried::Success(success)),
                retry: Retry::None,
            }),
            wire::Outcome::Rejected(rejection) => Ok(Self::of(
                Outcome::Rejected,
                &rejection.error,
                rejection.known_final.map(Carried::KnownFinal),
            )),
            wire::Outcome::Failed(failure) => Self::failure(Outcome::Failed, &failure),
            wire::Outcome::Indeterminate(failure) => {
                Self::failure(Outcome::Indeterminate, &failure)
            }
        }
    }

    /// A non-success with the daemon's `error`.
    fn of(outcome: fn(Fault) -> Outcome, error: &wire::Error, carried: Option<Carried>) -> Self {
        Self {
            outcome: outcome(Fault::daemon(error)),
            carried,
            retry: Retry::refused(&error.code),
        }
    }

    /// A failed or indeterminate outcome keeps a known result of its own
    /// operation; a Kill's partial termination belongs to no request here.
    fn failure(outcome: fn(Fault) -> Outcome, failure: &wire::Failure) -> Result<Self, Fault> {
        let carried = match failure.known() {
            None => None,
            Some(KnownResult::Operation(success)) => Some(Carried::Success(success.clone())),
            Some(KnownResult::Termination(_)) => return Err(MISMATCHED),
        };
        Ok(Self::of(outcome, failure.error(), carried))
    }
}
