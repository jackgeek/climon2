//! The client interface's requests and events as a session stream carries
//! them (#480): one record per request type and per event type, built from
//! the C structure in `client-v1.h` by one mechanical rule. The tag is
//! `0x40` (a request) or `0x80` (an event) plus the `CM1_` number. The body
//! starts with the request ID or the connection generation, then the
//! structure's fields in the header's order, a reserved field left out and
//! a union as its active member alone.
//!
//! Each type here is that structure with its reserved fields gone. Every
//! number is carried as the library gave it: the link code checks layouts,
//! not values, and what each value means belongs to the client interface.

use crate::layout::{Field, Reader, Tagged, layout, tagged};

/// The tag every request's `CM1_` number is added to.
const REQUEST_BASE: u8 = 0x40;
/// The tag every event's `CM1_` number is added to.
const EVENT_BASE: u8 = 0x80;

layout! {
    /// `cm1_error`: where an error came from, and its code and message.
    /// An absent error is origin 0 with both texts empty.
    pub struct Error {
        pub origin: u32,
        pub code: String,
        pub message: String,
    }
}

layout! {
    /// `cm1_geometry`.
    pub struct Geometry {
        pub columns: u32,
        pub rows: u32,
    }
}

layout! {
    /// `cm1_authority`: the generation, control period and restoration a
    /// request acts under.
    pub struct Authority {
        pub generation: u64,
        pub period: u64,
        pub restoration_id: u64,
    }
}

layout! {
    /// `cm1_open`.
    pub struct Open {
        pub mode: u32,
        pub geometry: Geometry,
        pub required: Vec<String>,
        pub optional: Vec<String>,
    }
}

layout! {
    /// `cm1_input`.
    pub struct Input {
        pub authority: Authority,
        pub bytes: Vec<u8>,
    }
}

layout! {
    /// `cm1_resize`.
    pub struct Resize {
        pub authority: Authority,
        pub geometry: Geometry,
    }
}

layout! {
    /// `cm1_flag`.
    pub struct Flag {
        pub authority: Authority,
        pub value: u8,
    }
}

tagged! {
    /// The arguments of `cm1_request`, each variant tagged with its `CM1_`
    /// number.
    pub enum Arguments: u8 {
        /// `CM1_OPEN`.
        Open(Open) = 1,
        /// `CM1_INPUT`.
        Input(Input) = 2,
        /// `CM1_RESIZE`.
        Resize(Resize) = 3,
        /// `CM1_ACQUIRE`, take control: the generation.
        TakeControl(u64) = 4,
        /// `CM1_READY`.
        Ready(Authority) = 5,
        /// `CM1_VIEWING`.
        Viewing(Flag) = 6,
        /// `CM1_ACKNOWLEDGE`.
        Acknowledge(Authority) = 7,
        /// `CM1_LEAVE`: the generation.
        Leave(u64) = 8,
        /// `CM1_KILL`: the required tokens.
        Kill(Vec<String>) = 9,
        /// `CM1_FOCUS`.
        Focus(Flag) = 10,
        /// `CM1_BREAK`.
        Break(Authority) = 11,
    }
}

/// One `cm1_request`: its request ID and its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub arguments: Arguments,
}

impl Tagged for Request {
    type Tag = u8;

    fn tag(&self) -> u8 {
        REQUEST_BASE + self.arguments.tag()
    }

    fn put_payload(&self, out: &mut Vec<u8>) {
        self.id.put(out);
        self.arguments.put_payload(out);
    }

    fn take_payload(tag: u8, reader: &mut Reader<'_>) -> Option<Self> {
        let id = u64::take(reader)?;
        let arguments = Arguments::take_payload(tag - REQUEST_BASE, reader)?;
        Some(Self { id, arguments })
    }
}

layout! {
    /// `cm1_optional_text`. The value is empty when absent.
    pub struct OptionalText {
        pub present: u32,
        pub value: String,
    }
}

layout! {
    /// `cm1_title`.
    pub struct Title {
        pub state: u32,
        pub value: Vec<u8>,
    }
}

layout! {
    /// `cm1_status`. `changed_at` is the client interface's UTC timestamp
    /// form.
    pub struct Status {
        pub state: u32,
        pub episode_present: u32,
        pub episode_id: u64,
        pub changed_at: [u8; 24],
    }
}

layout! {
    /// `cm1_available_status`.
    pub struct AvailableStatus {
        pub available: u32,
        pub status: Status,
    }
}

layout! {
    /// `cm1_progress`.
    pub struct Progress {
        pub state: u32,
        pub percent_present: u32,
        pub percent: u32,
    }
}

layout! {
    /// `cm1_available_progress`.
    pub struct AvailableProgress {
        pub available: u32,
        pub progress: Progress,
    }
}

layout! {
    /// `cm1_command_result`.
    pub struct CommandResult {
        pub kind: u32,
        pub value: u32,
        pub signal_name: OptionalText,
    }
}

layout! {
    /// `cm1_optional_command`.
    pub struct OptionalCommand {
        pub present: u32,
        pub command: CommandResult,
    }
}

layout! {
    /// `cm1_metadata`: what Initial metadata carries.
    pub struct Metadata {
        pub title: Title,
        pub status: AvailableStatus,
        pub progress: AvailableProgress,
        pub command: OptionalCommand,
    }
}

layout! {
    /// `cm1_notification`.
    pub struct Notification {
        pub source: u32,
        pub episode_present: u32,
        pub episode_id: u64,
        pub source_id: OptionalText,
        pub title: OptionalText,
        pub body: OptionalText,
    }
}

layout! {
    /// `cm1_granted`: control granted.
    pub struct Granted {
        pub period: u64,
        pub restoration_id: u64,
        pub geometry: Geometry,
        pub reason: u32,
    }
}

layout! {
    /// `cm1_revoked`: control revoked.
    pub struct Revoked {
        pub period: u64,
        pub restoration_id: u64,
        pub reason: u32,
    }
}

layout! {
    /// `cm1_restoration`: a restoration starts.
    pub struct Restoration {
        pub period: u64,
        pub restoration_id: u64,
        pub geometry: Geometry,
    }
}

layout! {
    /// `cm1_barrier`: a control period and a restoration.
    pub struct Barrier {
        pub period: u64,
        pub restoration_id: u64,
    }
}

layout! {
    /// `cm1_display`.
    pub struct Display {
        pub authority: Barrier,
        pub bytes: Vec<u8>,
    }
}

layout! {
    /// `cm1_geometry_event`.
    pub struct GeometryChange {
        pub period: u64,
        pub geometry: Geometry,
    }
}

layout! {
    /// `cm1_final`: the session ended.
    pub struct Final {
        pub command: CommandResult,
        pub status: AvailableStatus,
    }
}

layout! {
    /// `cm1_opened`.
    pub struct Opened {
        pub connection_id: [u8; 16],
        pub selected: Vec<String>,
    }
}

layout! {
    /// `cm1_authority_result`.
    pub struct AuthorityResult {
        pub changed: u8,
        pub period: u64,
        pub restoration_id: u64,
    }
}

layout! {
    /// `cm1_flag_result`.
    pub struct FlagResult {
        pub changed: u8,
        pub value: u8,
        pub accepted_bytes: u32,
    }
}

layout! {
    /// `cm1_termination`.
    pub struct Termination {
        pub command: OptionalCommand,
        pub cleanup: u32,
    }
}

tagged! {
    /// The result of `cm1_response`, each variant tagged with the result
    /// tag that selects it.
    pub enum Answer: u32 {
        /// `CM1_RESULT_NONE`.
        None = 0,
        /// `CM1_OPENED`.
        Opened(Opened) = 1,
        /// `CM1_INPUT_ACCEPTED`: the accepted bytes.
        InputAccepted(u32) = 2,
        /// `CM1_RESIZE_ACCEPTED`.
        ResizeAccepted = 3,
        /// `CM1_ACQUIRED`.
        Acquired(AuthorityResult) = 4,
        /// `CM1_READY_RESULT`.
        Ready(AuthorityResult) = 5,
        /// `CM1_VIEWING_RESULT`.
        Viewing(FlagResult) = 6,
        /// `CM1_ACKNOWLEDGEMENT`: whether it changed.
        Acknowledgement(u8) = 7,
        /// `CM1_LEFT`: whether an attachment was detached.
        Left(u8) = 8,
        /// `CM1_TERMINATION`.
        Termination(Termination) = 9,
        /// `CM1_FOCUS_RESULT`.
        Focus(FlagResult) = 10,
        /// `CM1_KNOWN_FINAL`.
        KnownFinal(CommandResult) = 11,
        /// `CM1_BREAK_SENT`.
        BreakSent = 12,
    }
}

/// `cm1_response`: one accepted request's outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub request_id: u64,
    pub outcome: u32,
    pub error: Error,
    pub answer: Answer,
}

layout! {
    /// A response's fields up to its result, which the result tag selects.
    struct ResponseHead {
        request_id: u64,
        outcome: u32,
        result_tag: u32,
        error: Error,
    }
}

impl Field for Response {
    fn put(&self, out: &mut Vec<u8>) {
        self.request_id.put(out);
        self.outcome.put(out);
        self.answer.tag().put(out);
        self.error.put(out);
        self.answer.put_payload(out);
    }

    fn take(reader: &mut Reader<'_>) -> Option<Self> {
        let head = ResponseHead::take(reader)?;
        let answer = Answer::take_payload(head.result_tag, reader)?;
        Some(Self {
            request_id: head.request_id,
            outcome: head.outcome,
            error: head.error,
            answer,
        })
    }
}

tagged! {
    /// The data of `cm1_record`, each variant tagged with its `CM1_`
    /// number.
    pub enum Data: u8 {
        /// `CM1_RESPONSE`.
        Response(Response) = 1,
        /// `CM1_INITIAL_METADATA`.
        InitialMetadata(Metadata) = 2,
        /// `CM1_TITLE`.
        Title(Title) = 3,
        /// `CM1_STATUS`: the session's status.
        Status(Status) = 4,
        /// `CM1_NOTIFICATION`.
        Notification(Notification) = 5,
        /// `CM1_PROGRESS`.
        Progress(Progress) = 6,
        /// `CM1_GRANTED`: control granted.
        Granted(Granted) = 7,
        /// `CM1_REVOKED`: control revoked.
        Revoked(Revoked) = 8,
        /// `CM1_RESTORATION_BEGIN`: a restoration starts.
        RestorationStart(Restoration) = 9,
        /// `CM1_DISPLAY`.
        Display(Display) = 10,
        /// `CM1_RESTORATION_COMPLETE`.
        RestorationComplete(Barrier) = 11,
        /// `CM1_FINAL`: the session ended.
        Final(Final) = 12,
        /// `CM1_GEOMETRY`.
        Geometry(GeometryChange) = 13,
        /// `CM1_DIAGNOSTIC`.
        Diagnostic(Error) = 14,
    }
}

/// One `cm1_record`: its connection generation and its data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub generation: u64,
    pub data: Data,
}

impl Tagged for Event {
    type Tag = u8;

    fn tag(&self) -> u8 {
        EVENT_BASE + self.data.tag()
    }

    fn put_payload(&self, out: &mut Vec<u8>) {
        self.generation.put(out);
        self.data.put_payload(out);
    }

    fn take_payload(tag: u8, reader: &mut Reader<'_>) -> Option<Self> {
        let generation = u64::take(reader)?;
        let data = Data::take_payload(tag - EVENT_BASE, reader)?;
        Some(Self { generation, data })
    }
}
