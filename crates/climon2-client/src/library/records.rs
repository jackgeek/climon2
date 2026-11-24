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
    wire::{
        self, AuthorityIds, Counter, Geometry, GrantReason, NotificationSource, ProgressState,
        RevocationReason, StatusState,
    },
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
// SAFETY: every field is an integer, for which zero is valid, and zero is
// "unavailable".
const NO_STATUS: native::AvailableStatus = unsafe { mem::zeroed() };
// SAFETY: every field is an integer, for which zero is valid, and zero is
// "unavailable".
const NO_PROGRESS: native::AvailableProgress = unsafe { mem::zeroed() };

// The native status codes follow the wire's states in order, from one.
const _: () = assert!(StatusState::Idle as u32 + 1 == native::IDLE);
const _: () = assert!(StatusState::Running as u32 + 1 == native::RUNNING);
const _: () = assert!(StatusState::NeedsAttention as u32 + 1 == native::NEEDS_ATTENTION);
const _: () = assert!(StatusState::Acknowledged as u32 + 1 == native::ACKNOWLEDGED);
const _: () = assert!(StatusState::Completed as u32 + 1 == native::COMPLETED);
const _: () = assert!(StatusState::Failed as u32 + 1 == native::STATUS_FAILED);

// The native progress codes follow the wire's states in order, from one.
const _: () = assert!(ProgressState::None as u32 + 1 == native::PROGRESS_NONE);
const _: () = assert!(ProgressState::Normal as u32 + 1 == native::PROGRESS_NORMAL);
const _: () = assert!(ProgressState::Indeterminate as u32 + 1 == native::PROGRESS_INDETERMINATE);
const _: () = assert!(ProgressState::Error as u32 + 1 == native::PROGRESS_ERROR);
const _: () = assert!(ProgressState::Paused as u32 + 1 == native::PROGRESS_PAUSED);

/// What a record does to the restoration under way (#99).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Restoring {
    Begins,
    Progresses,
    Ends,
    Neither,
}

/// One record, as the library holds it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Event {
    /// One accepted request's outcome.
    Response(Response),
    /// What the session reports about itself.
    Metadata(Metadata),
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
    /// Its active result, if it carries one.
    pub(super) answer: Option<Answer>,
}

/// Session metadata records.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Metadata {
    /// The atomic Initial metadata after a successful Open.
    Initial(Initial),
    /// A Title changed, by value; never unavailable.
    Title(Title),
    /// A change of the session's status, a notification or a change of
    /// its progress.
    Report(Report),
    /// The command's final result, after an Initial metadata that did not
    /// know it (#94), and its final status when `status-v1` is selected
    /// (#95).
    Final(Command, Option<Status>),
}

/// What the session reports about its status, notifications and progress.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Report {
    /// A Status changed: an effective nonfinal status (#95).
    Status(Status),
    /// A Notification (#98): inert text and the episode it requested or
    /// joined.
    Notification(Notification),
    /// A Progress changed (#98): the retained normalized progress.
    Progress(wire::Progress),
}

impl Report {
    fn bytes(&self) -> u64 {
        match self {
            Self::Notification(notification) => notification.bytes(),
            Self::Status(_) | Self::Progress(_) => 0,
        }
    }

    fn native(&self) -> (u32, native::RecordData) {
        match self {
            Self::Status(status) => (
                native::STATUS,
                native::RecordData {
                    status: status.native(),
                },
            ),
            Self::Notification(notification) => (
                native::NOTIFICATION,
                native::RecordData {
                    notification: notification.native(),
                },
            ),
            Self::Progress(value) => (
                native::PROGRESS,
                native::RecordData {
                    progress: progress(*value),
                },
            ),
        }
    }
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
    /// What the held period shows.
    Screen(Screen),
}

/// The records of a held period: its restorations, terminal bytes and
/// geometry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Screen {
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
            answer: None,
        }
    }
}

/// A response's active result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Answer {
    Opened {
        connection: [u8; 16],
        selected: Selection,
    },
    Left {
        detached: bool,
    },
    /// What the response knows of the command's end.
    Ending(Ending),
    /// A command's result (#94).
    Done(Done),
}

/// What a response knows of the command's end.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Ending {
    /// An initial role rejection's known command result.
    KnownFinal(Command),
    /// What a Kill learned of termination: the command result, if known,
    /// and the daemon's cleanup, `CM1_CLEANUP_*` (#96).
    Termination {
        command: Option<Command>,
        cleanup: u32,
    },
}

impl Ending {
    /// Whether it makes the command's final result known.
    pub(super) fn concludes(&self) -> bool {
        self.command().is_some()
    }

    fn command(&self) -> Option<&Command> {
        match self {
            Self::KnownFinal(command) => Some(command),
            Self::Termination { command, .. } => command.as_ref(),
        }
    }

    /// Writes its result tag and active result into `response`.
    fn native(&self, response: &mut native::Response) {
        match self {
            Self::KnownFinal(command) => {
                response.result_tag = native::KNOWN_FINAL;
                response.result.known_final = command.native();
            }
            Self::Termination { command, cleanup } => {
                response.result_tag = native::TERMINATION;
                response.result.termination = native::Termination {
                    command_result: optional_command(command.as_ref()),
                    cleanup: *cleanup,
                    reserved: 0,
                };
            }
        }
    }
}

/// What a command did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Done {
    /// Input accepted, all of its bytes.
    Input(u32),
    /// A result with no active member, under its native result tag:
    /// Resize's and Break's (#326).
    Bare(u32),
    /// Acquire's or Ready's authority result, under its native result tag.
    Authority(u32, wire::AuthorityResult),
    /// Whether a state the request reported changed.
    Changed(Changed),
}

/// The results that say whether something changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Changed {
    /// Focus's or Set viewing's (#95), under its native result tag: the
    /// state now, and the bytes forwarded (none for viewing).
    Flag {
        tag: u32,
        changed: bool,
        value: bool,
        bytes: u32,
    },
    /// Acknowledge's (#95): whether pending attention became acknowledged.
    Acknowledged(bool),
}

/// An effective status, its stored change time, exact, and the episode
/// only attention carries: held inline, owning no buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Status {
    state: StatusState,
    changed_at: [u8; 24],
    episode: Option<Counter>,
}

impl From<&wire::Status> for Status {
    fn from(status: &wire::Status) -> Self {
        let changed_at = status.changed_at().as_str().as_bytes();
        Self {
            state: status.state(),
            changed_at: changed_at
                .try_into()
                .expect("a wire UTC time is 24 ASCII bytes"),
            episode: status.episode(),
        }
    }
}

impl Status {
    fn native(self) -> native::Status {
        native::Status {
            state: self.state as u32 + 1,
            episode_present: u32::from(self.episode.is_some()),
            episode_id: self.episode.map_or(0, Counter::get),
            changed_at: native::Utc {
                bytes: self.changed_at,
            },
        }
    }
}

/// An optional status as native availability: unavailable is all zero.
fn available(status: Option<Status>) -> native::AvailableStatus {
    status.map_or(NO_STATUS, |status| native::AvailableStatus {
        available: u32::from(true),
        value: status.native(),
        ..NO_STATUS
    })
}

/// Progress as native: its state code, and its percentage when present.
fn progress(progress: wire::Progress) -> native::Progress {
    let percent = progress.percent();
    native::Progress {
        state: progress.state() as u32 + 1,
        percent_present: u32::from(percent.is_some()),
        percent: percent.map_or(0, |percent| percent.get().into()),
        ..NO_PROGRESS.value
    }
}

/// Optional progress as native availability: unavailable is all zero.
fn available_progress(value: Option<wire::Progress>) -> native::AvailableProgress {
    value.map_or(NO_PROGRESS, |value| native::AvailableProgress {
        available: u32::from(true),
        value: progress(value),
        ..NO_PROGRESS
    })
}

/// A notification as the library holds it: its source's native code, the
/// episode and its owned text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Notification {
    source: u32,
    episode: Option<Counter>,
    source_id: Option<Box<str>>,
    title: Option<Box<str>>,
    body: Option<Box<str>>,
}

impl From<wire::Notification> for Notification {
    fn from(notification: wire::Notification) -> Self {
        let source = match notification.source {
            NotificationSource::Osc9 => native::OSC9,
            NotificationSource::Osc99 => native::OSC99,
            NotificationSource::Osc777 => native::OSC777,
        };
        Self {
            source,
            episode: notification.episode_id,
            source_id: notification.source_id.map(Box::from),
            title: notification.title.map(Box::from),
            body: notification.body.map(Box::from),
        }
    }
}

impl Notification {
    fn bytes(&self) -> u64 {
        [&self.source_id, &self.title, &self.body]
            .into_iter()
            .flatten()
            .map(|text| text_bytes(text))
            .sum()
    }

    fn native(&self) -> native::Notification {
        native::Notification {
            source: self.source,
            episode_present: u32::from(self.episode.is_some()),
            episode_id: self.episode.map_or(0, Counter::get),
            source_id: optional(self.source_id.as_deref()),
            title: optional(self.title.as_deref()),
            body: optional(self.body.as_deref()),
        }
    }
}

/// Optional text as native: absent is not present, with no bytes.
fn optional(text: Option<&str>) -> native::OptionalText {
    native::OptionalText {
        present: u32::from(text.is_some()),
        reserved: 0,
        value: bytes(text.unwrap_or_default()),
    }
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
pub(super) struct Initial {
    pub(super) title: Title,
    /// Available exactly when `status-v1` is selected (#95).
    pub(super) status: Option<Status>,
    /// Available exactly when `progress-v1` is selected (#98).
    pub(super) progress: Option<wire::Progress>,
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
            signal_name: optional(self.name.as_deref()),
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
    /// The record as records of at most `chunk` bytes each (#94): a
    /// Display's bytes in order, under its authority; any other record
    /// whole.
    pub(super) fn split(self, chunk: usize) -> Vec<Self> {
        match self {
            Self::Presentation(Presentation::Screen(Screen::Display(ids, bytes))) => bytes
                .chunks(chunk)
                .map(|part| {
                    Self::Presentation(Presentation::Screen(Screen::Display(ids, Box::from(part))))
                })
                .collect(),
            event => vec![event],
        }
    }

    /// What the record costs the event allowance: the record and every
    /// buffer it owns.
    pub(super) fn charge(&self) -> u64 {
        CHARGE + self.payload()
    }

    /// What it does to a restoration under way (#99): a beginning starts
    /// one, a Display of it is progress, its complete or a revocation ends
    /// it.
    pub(super) fn restoring(&self) -> Restoring {
        match self {
            Self::Presentation(Presentation::Screen(Screen::Begin(..))) => Restoring::Begins,
            Self::Presentation(Presentation::Screen(Screen::Display(..))) => Restoring::Progresses,
            Self::Presentation(
                Presentation::Screen(Screen::Complete(..)) | Presentation::Revoked(..),
            ) => Restoring::Ends,
            _ => Restoring::Neither,
        }
    }

    /// Whether it is ordinary work, which may not use the progress reserve
    /// (#99): a Display, Title changed, Notification, Progress changed or
    /// Geometry. Every other record is protected progress.
    pub(super) fn ordinary(&self) -> bool {
        matches!(
            self,
            Self::Metadata(
                Metadata::Title(_)
                    | Metadata::Report(Report::Notification(_) | Report::Progress(_))
            ) | Self::Presentation(Presentation::Screen(
                Screen::Display(..) | Screen::Geometry(..)
            ))
        )
    }

    /// The terminal bytes it carries: a Display's, which `snapshot` counts
    /// as delivered once drained (#99).
    fn display(&self) -> u64 {
        match self {
            Self::Presentation(presentation) => presentation.bytes(),
            _ => 0,
        }
    }

    /// The bytes of its variable data.
    pub(super) fn payload(&self) -> u64 {
        match self {
            Self::Response(response) => response.bytes(),
            Self::Metadata(metadata) => metadata.bytes(),
            Self::Presentation(presentation) => presentation.bytes(),
            Self::Diagnostic(fault) => fault.bytes(),
        }
    }
}

impl Metadata {
    fn bytes(&self) -> u64 {
        match self {
            Self::Initial(initial) => {
                initial.title.bytes() + initial.command.as_ref().map_or(0, Command::bytes)
            }
            Self::Title(title) => title.bytes(),
            Self::Report(report) => report.bytes(),
            Self::Final(command, _) => command.bytes(),
        }
    }

    /// Its tag and data, pointing into its own text.
    fn native(&self) -> (u32, native::RecordData) {
        match self {
            Self::Initial(initial) => (
                native::INITIAL_METADATA,
                native::RecordData {
                    metadata: initial.native(),
                },
            ),
            Self::Title(title) => (
                native::TITLE,
                native::RecordData {
                    title: title.native(),
                },
            ),
            Self::Report(report) => report.native(),
            Self::Final(command, status) => (
                native::FINAL,
                native::RecordData {
                    r#final: native::Final {
                        command_result: command.native(),
                        status: available(*status),
                    },
                },
            ),
        }
    }
}

impl Response {
    fn bytes(&self) -> u64 {
        let answer = match &self.answer {
            Some(Answer::Opened { selected, .. }) => TOKEN * selected.families().count() as u64,
            Some(Answer::Ending(ending)) => ending.command().into_iter().map(Command::bytes).sum(),
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

    /// The Display bytes its records carry.
    pub(super) fn display(&self) -> u64 {
        self.events.iter().map(Event::display).sum()
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
            (record.tag, record.data) = metadata.native();
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
    /// The bytes it owns: a Display's.
    fn bytes(&self) -> u64 {
        match self {
            Self::Screen(Screen::Display(_, bytes)) => bytes.len() as u64,
            _ => 0,
        }
    }

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
            Self::Screen(screen) => screen.native(),
        }
    }
}

impl Screen {
    fn native(&self) -> (u32, native::RecordData) {
        match self {
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

impl Initial {
    fn native(&self) -> native::Metadata {
        // SAFETY: every field is an integer or a raw pointer, for which zero
        // (null) is valid, and zero is "unavailable".
        let mut metadata: native::Metadata = unsafe { mem::zeroed() };
        metadata.title = self.title.native();
        metadata.status = available(self.status);
        metadata.progress = available_progress(self.progress);
        metadata.command_result = optional_command(self.command.as_ref());
        metadata
    }
}

/// An optional command result's native view: absent is all zero.
fn optional_command(command: Option<&Command>) -> native::OptionalCommand {
    command.map_or(NO_COMMAND, |command| native::OptionalCommand {
        present: u32::from(true),
        value: command.native(),
        ..NO_COMMAND
    })
}

impl Response {
    fn native(&self, tokens: &mut Vec<Box<[native::Bytes]>>) -> native::Response {
        // SAFETY: every field is an integer, a raw pointer or a union of
        // them, for which zero (null) is valid.
        let mut response: native::Response = unsafe { mem::zeroed() };
        response.request_id = self.id;
        response.outcome = self.outcome.code();
        response.error = self.outcome.fault().map_or(NO_ERROR, Fault::error);
        if let Some(answer) = &self.answer {
            answer.native(&mut response, tokens);
        }
        response
    }
}

impl Answer {
    /// Writes its result tag and active result into `response`.
    fn native(&self, response: &mut native::Response, tokens: &mut Vec<Box<[native::Bytes]>>) {
        match self {
            Self::Opened {
                connection,
                selected,
            } => {
                response.result_tag = native::OPENED;
                response.result.opened = opened(*connection, *selected, tokens);
            }
            Self::Left { detached } => {
                response.result_tag = native::LEFT;
                response.result.attachment_detached = u8::from(*detached);
            }
            Self::Ending(ending) => ending.native(response),
            Self::Done(done) => done.native(response),
        }
    }
}

impl Done {
    /// Writes its result tag and active result into `response`, whose
    /// other bytes stay zero.
    fn native(self, response: &mut native::Response) {
        match self {
            Self::Input(accepted_bytes) => {
                response.result_tag = native::INPUT_ACCEPTED;
                response.result.accepted_bytes = accepted_bytes;
            }
            Self::Bare(tag) => response.result_tag = tag,
            Self::Authority(tag, result) => {
                response.result_tag = tag;
                response.result.authority = authority(&result);
            }
            Self::Changed(changed) => changed.native(response),
        }
    }
}

impl Changed {
    /// Writes its result tag and active result into `response`, whose
    /// other bytes stay zero.
    fn native(self, response: &mut native::Response) {
        match self {
            Self::Flag {
                tag,
                changed,
                value,
                bytes,
            } => {
                response.result_tag = tag;
                response.result.flag = native::FlagResult {
                    changed: u8::from(changed),
                    value: u8::from(value),
                    reserved: [0; 2],
                    accepted_bytes: bytes,
                };
            }
            Self::Acknowledged(changed) => {
                response.result_tag = native::ACKNOWLEDGEMENT;
                response.result.changed = u8::from(changed);
            }
        }
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
