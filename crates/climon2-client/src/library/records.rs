//! The records the library delivers (native revision 1's events), owned by
//! the instance until drained and then by their delivery until it is
//! released: every buffer a delivered record points to lives in that
//! delivery.

use super::failure::{Fault, bytes};
use climon2_core::{native, wire};
use std::mem;

/// Bytes every record is charged before its variable data.
pub(super) const RECORD: u64 = mem::size_of::<native::Record>() as u64;
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
    /// An explicit error the stream reports, such as a connection failure.
    Diagnostic(Fault),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Response {
    /// The caller's public request ID.
    pub(super) id: u64,
    /// `CM1_OK`, `CM1_REJECTED`, `CM1_FAILED` or `CM1_INDETERMINATE`.
    pub(super) outcome: u32,
    pub(super) answer: Answer,
    /// Every non-success has one; success none.
    pub(super) fault: Option<Fault>,
}

impl Response {
    /// A request that never executed.
    pub(super) fn rejected(id: u64, fault: Fault) -> Self {
        Self::failed(id, native::REJECTED, fault)
    }

    /// A request whose effect is unknown.
    pub(super) fn uncertain(id: u64, fault: Fault) -> Self {
        Self::failed(id, native::INDETERMINATE, fault)
    }

    fn failed(id: u64, outcome: u32, fault: Fault) -> Self {
        Self {
            id,
            outcome,
            answer: Answer::None,
            fault: Some(fault),
        }
    }
}

/// A response's active result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Answer {
    None,
    Opened {
        connection: [u8; 16],
        selected: Vec<&'static str>,
    },
    Left {
        detached: bool,
    },
    /// An initial role rejection's known command result.
    KnownFinal(Command),
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
        RECORD + self.payload()
    }

    /// The bytes of its variable data.
    pub(super) fn payload(&self) -> u64 {
        match self {
            Self::Response(response) => response.bytes(),
            Self::Metadata(metadata) => {
                metadata.title.bytes() + metadata.command.as_ref().map_or(0, Command::bytes)
            }
            Self::Title(title) => title.bytes(),
            Self::Diagnostic(fault) => fault.bytes(),
        }
    }
}

impl Response {
    fn bytes(&self) -> u64 {
        let answer = match &self.answer {
            Answer::Opened { selected, .. } => TOKEN * selected.len() as u64,
            Answer::KnownFinal(command) => command.bytes(),
            _ => 0,
        };
        answer + self.fault.as_ref().map_or(0, Fault::bytes)
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
    }
    record
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
        response.outcome = self.outcome;
        response.error = self.fault.as_ref().map_or(NO_ERROR, Fault::error);
        match &self.answer {
            Answer::None => {}
            Answer::Opened {
                connection,
                selected,
            } => {
                response.result_tag = native::OPENED;
                response.result.opened = opened(*connection, selected, tokens);
            }
            Answer::Left { detached } => {
                response.result_tag = native::LEFT;
                response.result.attachment_detached = u8::from(*detached);
            }
            Answer::KnownFinal(command) => {
                response.result_tag = native::KNOWN_FINAL;
                response.result.known_final = command.native();
            }
        }
        response
    }
}

fn opened(
    connection: [u8; 16],
    selected: &[&'static str],
    tokens: &mut Vec<Box<[native::Bytes]>>,
) -> native::Opened {
    let list: Box<[native::Bytes]> = selected.iter().map(|token| bytes(token)).collect();
    let selected = native::Tokens {
        data: list.as_ptr(),
        count: list.len() as u64,
    };
    tokens.push(list);
    native::Opened {
        connection_id: native::Guid { bytes: connection },
        selected,
    }
}
