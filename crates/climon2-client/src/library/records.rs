//! The records the library delivers (native revision 1's events), owned by
//! the instance until drained and then by their delivery until it is
//! released: every buffer a delivered record points to lives in that
//! delivery.

use super::{
    failure::{Fault, bytes},
    families::Selection,
};
use climon2_core::{
    native,
    wire::{self, AuthorityIds, Counter, Geometry, GrantReason, RevocationReason},
};
use std::mem;

/// Bytes every record is charged before its variable data: a delivered
/// record is held twice, as its `cm1_record` and as the library's own copy,
/// each charged `sizeof(cm1_record)`. Every buffer either points to is the
/// library copy's, charged once as its payload.
pub(super) const CHARGE: u64 = 2 * mem::size_of::<native::Record>() as u64;

// The library's own copy never outgrows the `cm1_record` charged for it.
const _: () = assert!(mem::size_of::<Event>() <= mem::size_of::<native::Record>());
const TOKEN: u64 = mem::size_of::<native::Bytes>() as u64;

// SAFETY (all three): every field is an integer, a raw pointer or a union of
// them, for which zero (null) is a valid value.
const NO_RECORD: native::Record = unsafe { mem::zeroed() };
const NO_ERROR: native::Error = unsafe { mem::zeroed() };
const NO_COMMAND: native::OptionalCommand = unsafe { mem::zeroed() };

/// One record, as the library holds it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Event {
    /// One accepted request's outcome.
    Response(Response),
    /// The atomic Initial metadata after a successful Open.
    Metadata(Metadata),
    /// A Title changed, by value; never unavailable.
    Title(Title),
    /// A control, restoration or display record (#94).
    Presentation(Presentation),
    /// An explicit error the stream reports, such as a connection failure.
    Diagnostic(Fault),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Response {
    /// The caller's public request ID.
    pub(super) id: u64,
    pub(super) outcome: Outcome,
    pub(super) answer: Answer,
}

/// What the daemon presents to the role it controls, under immutable
/// period and restoration identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Presentation {
    /// Control granted: a new period and its restoration, at the session's
    /// effective geometry.
    Granted(AuthorityIds, Geometry, GrantReason),
    /// Control revoked under the authority it ends.
    Revoked(AuthorityIds, RevocationReason),
    /// A restoration's beginning, at the geometry its bytes use.
    Begin(AuthorityIds, Geometry),
    /// Nonempty exact terminal bytes, of a restoration or live.
    Display(AuthorityIds, Box<[u8]>),
    /// Every Display of the restoration was sent.
    Complete(AuthorityIds),
    /// The session's effective geometry in a period.
    Geometry(Counter, Geometry),
}

/// A response's outcome: success, or a non-success with the error that
/// explains it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Outcome {
    Ok,
    /// The request never executed.
    Rejected(Fault),
    Failed(Fault),
    /// Its effect is unknown.
    Indeterminate(Fault),
}

impl Outcome {
    /// `CM1_OK`, `CM1_REJECTED`, `CM1_FAILED` or `CM1_INDETERMINATE`.
    fn code(&self) -> u32 {
        match self {
            Self::Ok => native::OK,
            Self::Rejected(_) => native::REJECTED,
            Self::Failed(_) => native::FAILED,
            Self::Indeterminate(_) => native::INDETERMINATE,
        }
    }

    pub(super) fn fault(&self) -> Option<&Fault> {
        match self {
            Self::Ok => None,
            Self::Rejected(fault) | Self::Failed(fault) | Self::Indeterminate(fault) => Some(fault),
        }
    }
}

impl Response {
    /// A request that never executed.
    pub(super) fn rejected(id: u64, fault: Fault) -> Self {
        Self::unanswered(id, Outcome::Rejected(fault))
    }

    /// A request whose effect is unknown.
    pub(super) fn uncertain(id: u64, fault: Fault) -> Self {
        Self::unanswered(id, Outcome::Indeterminate(fault))
    }

    fn unanswered(id: u64, outcome: Outcome) -> Self {
        Self {
            id,
            outcome,
            answer: Answer::None,
        }
    }
}

/// A response's active result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Answer {
    None,
    Opened {
        connection: [u8; 16],
        selected: Selection,
    },
    Left {
        detached: bool,
    },
    /// An initial role rejection's known command result.
    KnownFinal(Command),
    /// Input accepted, all of its bytes.
    Input(u32),
    Resized,
    Acquired(wire::AuthorityResult),
    Ready(wire::AuthorityResult),
    /// Focus: whether it changed, the state, and the bytes forwarded.
    Focus {
        changed: bool,
        focused: bool,
        bytes: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Title {
    /// `title-v1` is not selected.
    Unavailable,
    Unset,
    /// A value, possibly empty.
    Value(Box<str>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Metadata {
    pub(super) title: Title,
    pub(super) command: Option<Command>,
}

/// A raw normal exit or a source-host signal, with its optional name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Command {
    kind: u32,
    value: u32,
    name: Option<Box<str>>,
}

impl From<&wire::CommandResult> for Command {
    fn from(result: &wire::CommandResult) -> Self {
        match result {
            wire::CommandResult::Exit { code } => Self {
                kind: native::EXIT,
                value: *code,
                name: None,
            },
            wire::CommandResult::Signal { number, name } => Self {
                kind: native::SIGNAL,
                value: number.get(),
                name: name.as_deref().map(Box::from),
            },
        }
    }
}

impl Command {
    fn bytes(&self) -> u64 {
        self.name.as_deref().map_or(0, text_bytes)
    }

    fn native(&self) -> native::CommandResult {
        native::CommandResult {
            kind: self.kind,
            value: self.value,
            signal_name: native::OptionalText {
                present: u32::from(self.name.is_some()),
                reserved: 0,
                value: bytes(self.name.as_deref().unwrap_or_default()),
            },
        }
    }
}

fn text_bytes(text: &str) -> u64 {
    text.len() as u64
}

impl Title {
    fn bytes(&self) -> u64 {
        match self {
            Self::Value(value) => text_bytes(value),
            _ => 0,
        }
    }

    fn native(&self) -> native::Title {
        let (state, value) = match self {
            Self::Unavailable => (native::UNAVAILABLE, ""),
            Self::Unset => (native::UNSET, ""),
            Self::Value(value) => (native::VALUE, &**value),
        };
        native::Title {
            state,
            reserved: 0,
            value: bytes(value),
        }
    }
}

impl Event {
    /// What the record costs the event allowance: the record and every
    /// buffer it owns.
    pub(super) fn charge(&self) -> u64 {
        CHARGE + self.payload()
    }

    /// The bytes of its variable data.
    pub(super) fn payload(&self) -> u64 {
        match self {
            Self::Response(response) => response.bytes(),
            Self::Metadata(metadata) => {
                metadata.title.bytes() + metadata.command.as_ref().map_or(0, Command::bytes)
            }
            Self::Title(title) => title.bytes(),
            Self::Presentation(Presentation::Display(_, bytes)) => bytes.len() as u64,
            Self::Presentation(_) => 0,
            Self::Diagnostic(fault) => fault.bytes(),
        }
    }
}

impl Response {
    fn bytes(&self) -> u64 {
        let answer = match &self.answer {
            Answer::Opened { selected, .. } => TOKEN * selected.families().count() as u64,
            Answer::KnownFinal(command) => command.bytes(),
            _ => 0,
        };
        answer + self.outcome.fault().map_or(0, Fault::bytes)
    }
}

/// One delivery's records and everything they point to, immutable once
/// built.
pub(super) struct Batch {
    events: Vec<Event>,
    /// The selected tokens of each Opened response.
    _tokens: Vec<Box<[native::Bytes]>>,
    records: Vec<native::Record>,
}

impl Batch {
    pub(super) fn new(generation: u64, events: Vec<Event>) -> Self {
        let mut tokens = Vec::new();
        let records = events
            .iter()
            .map(|event| record(generation, event, &mut tokens))
            .collect();
        Self {
            events,
            _tokens: tokens,
            records,
        }
    }

    pub(super) fn records(&self) -> (*const native::Record, u64) {
        (self.records.as_ptr(), self.records.len() as u64)
    }

    /// What the batch costs the event allowance, record by record.
    pub(super) fn charge(&self) -> u64 {
        self.events.iter().map(Event::charge).sum()
    }

    pub(super) fn len(&self) -> u32 {
        self.records.len() as u32
    }
}

/// The native record of `event`, pointing into `event` and `tokens`.
fn record(
    generation: u64,
    event: &Event,
    tokens: &mut Vec<Box<[native::Bytes]>>,
) -> native::Record {
    let mut record = native::Record {
        generation,
        ..NO_RECORD
    };
    match event {
        Event::Response(response) => {
            record.tag = native::RESPONSE;
            record.data.response = response.native(tokens);
        }
        Event::Metadata(metadata) => {
            record.tag = native::INITIAL_METADATA;
            record.data.metadata = metadata.native();
        }
        Event::Title(title) => {
            record.tag = native::TITLE;
            record.data.title = title.native();
        }
        Event::Diagnostic(fault) => {
            record.tag = native::DIAGNOSTIC;
            record.data.diagnostic = fault.error();
        }
        Event::Presentation(presentation) => {
            (record.tag, record.data) = presentation.native();
        }
    }
    record
}

fn geometry(geometry: Geometry) -> native::Geometry {
    native::Geometry {
        cols: geometry.cols.get(),
        rows: geometry.rows.get(),
    }
}

fn barrier(ids: AuthorityIds) -> native::Barrier {
    native::Barrier {
        period: ids.period.get(),
        restoration_id: ids.restoration_id.get(),
    }
}

fn granted(reason: GrantReason) -> u32 {
    match reason {
        GrantReason::Attach => native::ATTACH,
        GrantReason::Acquire => native::TAKE_CONTROL,
        GrantReason::HandBack => native::HAND_BACK,
        GrantReason::Successor => native::SUCCESSOR,
    }
}

fn revoked(reason: RevocationReason) -> u32 {
    match reason {
        RevocationReason::Takeover => native::TAKEOVER,
        RevocationReason::SessionEnding => native::SESSION_ENDING,
    }
}

impl Presentation {
    /// Its tag and data, pointing into its own bytes.
    fn native(&self) -> (u32, native::RecordData) {
        match self {
            Self::Granted(ids, size, reason) => (
                native::GRANTED,
                native::RecordData {
                    granted: native::Granted {
                        period: ids.period.get(),
                        restoration_id: ids.restoration_id.get(),
                        geometry: geometry(*size),
                        reason: granted(*reason),
                        reserved: 0,
                    },
                },
            ),
            Self::Revoked(ids, reason) => (
                native::REVOKED,
                native::RecordData {
                    revoked: native::Revoked {
                        period: ids.period.get(),
                        restoration_id: ids.restoration_id.get(),
                        reason: revoked(*reason),
                        reserved: 0,
                    },
                },
            ),
            Self::Begin(ids, size) => (
                native::RESTORATION_BEGIN,
                native::RecordData {
                    restoration: native::Restoration {
                        period: ids.period.get(),
                        restoration_id: ids.restoration_id.get(),
                        geometry: geometry(*size),
                    },
                },
            ),
            Self::Display(ids, bytes) => (
                native::DISPLAY,
                native::RecordData {
                    display: native::Display {
                        authority: barrier(*ids),
                        bytes: native::Bytes {
                            data: bytes.as_ptr(),
                            length: bytes.len() as u64,
                        },
                    },
                },
            ),
            Self::Complete(ids) => (
                native::RESTORATION_COMPLETE,
                native::RecordData {
                    barrier: barrier(*ids),
                },
            ),
            Self::Geometry(period, size) => (
                native::GEOMETRY,
                native::RecordData {
                    geometry: native::GeometryEvent {
                        period: period.get(),
                        geometry: geometry(*size),
                    },
                },
            ),
        }
    }
}

impl Metadata {
    /// Status and progress stay explicitly unavailable: `status-v1` and
    /// `progress-v1` are never selected yet.
    fn native(&self) -> native::Metadata {
        // SAFETY: every field is an integer or a raw pointer, for which zero
        // (null) is valid, and zero is "unavailable".
        let mut metadata: native::Metadata = unsafe { mem::zeroed() };
        metadata.title = self.title.native();
        metadata.command_result =
            self.command
                .as_ref()
                .map_or(NO_COMMAND, |command| native::OptionalCommand {
                    present: u32::from(true),
                    reserved: 0,
                    value: command.native(),
                });
        metadata
    }
}

impl Response {
    fn native(&self, tokens: &mut Vec<Box<[native::Bytes]>>) -> native::Response {
        // SAFETY: every field is an integer, a raw pointer or a union of
        // them, for which zero (null) is valid.
        let mut response: native::Response = unsafe { mem::zeroed() };
        response.request_id = self.id;
        response.outcome = self.outcome.code();
        response.error = self.outcome.fault().map_or(NO_ERROR, Fault::error);
        match &self.answer {
            Answer::None => {}
            Answer::Opened {
                connection,
                selected,
            } => {
                response.result_tag = native::OPENED;
                response.result.opened = opened(*connection, *selected, tokens);
            }
            Answer::Left { detached } => {
                response.result_tag = native::LEFT;
                response.result.attachment_detached = u8::from(*detached);
            }
            Answer::KnownFinal(command) => {
                response.result_tag = native::KNOWN_FINAL;
                response.result.known_final = command.native();
            }
            Answer::Input(bytes) => {
                response.result_tag = native::INPUT_ACCEPTED;
                response.result.accepted_bytes = *bytes;
            }
            Answer::Resized => response.result_tag = native::RESIZE_ACCEPTED,
            Answer::Acquired(result) => {
                response.result_tag = native::ACQUIRED;
                response.result.authority = authority(result);
            }
            Answer::Ready(result) => {
                response.result_tag = native::READY_RESULT;
                response.result.authority = authority(result);
            }
            Answer::Focus {
                changed,
                focused,
                bytes,
            } => {
                response.result_tag = native::FOCUS_RESULT;
                response.result.flag = native::FlagResult {
                    changed: u8::from(*changed),
                    value: u8::from(*focused),
                    reserved: [0; 2],
                    accepted_bytes: *bytes,
                };
            }
        }
        response
    }
}

fn authority(result: &wire::AuthorityResult) -> native::AuthorityResult {
    native::AuthorityResult {
        changed: u8::from(result.changed),
        reserved: [0; 7],
        period: result.authority.period.get(),
        restoration_id: result.authority.restoration_id.get(),
    }
}

/// A list's native view: null and zero when empty, as every empty native
/// list is.
fn listed(list: &[native::Bytes]) -> native::Tokens {
    if list.is_empty() {
        return native::Tokens {
            data: std::ptr::null(),
            count: 0,
        };
    }
    native::Tokens {
        data: list.as_ptr(),
        count: list.len() as u64,
    }
}

fn opened(
    connection: [u8; 16],
    selected: Selection,
    tokens: &mut Vec<Box<[native::Bytes]>>,
) -> native::Opened {
    let list: Box<[native::Bytes]> = selected
        .families()
        .map(|family| bytes(family.token()))
        .collect();
    let selected = listed(&list);
    tokens.push(list);
    native::Opened {
        connection_id: native::Guid { bytes: connection },
        selected,
    }
}
