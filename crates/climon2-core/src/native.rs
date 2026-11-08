//! Native client interface revision 1: `#[repr(C)]` declarations of the
//! published header `docs/protocol/monitor-client/client-v1.h`.
//!
//! The header is authoritative for declarations, numeric tags and field order;
//! `core-v1.md` owns their semantics. Names map mechanically: `cm1_bytes` is
//! [`Bytes`], `CM1_SUCCESS` is [`SUCCESS`], and C field names are kept (`final`
//! is spelled `r#final`). The client library, the client executable and the
//! monitor share these declarations without depending on each other. A layout
//! test compares them with the header on both supported macOS architectures.
//!
//! These are plain data declarations: nothing here dereferences a pointer or
//! validates a record.

use std::{
    ffi::CStr,
    marker::{PhantomData, PhantomPinned},
};

/// Name of the frozen bootstrap symbol every client library exports.
pub const BOOTSTRAP: &CStr = c"climon2_client_get_interface";

/// Signature of the frozen bootstrap symbol.
pub type Bootstrap = unsafe extern "C" fn(u32, *mut *const Description) -> u32;

pub const REVISION: u32 = 1;

pub const SUCCESS: u32 = 0;
pub const UNSUPPORTED_REVISION: u32 = 1;
pub const INVALID_ARGUMENT: u32 = 2;
pub const UNSUPPORTED_OPERATION: u32 = 3;
pub const INVALID_REQUEST_ID: u32 = 4;
pub const FULL: u32 = 5;
pub const CLOSED: u32 = 6;
pub const BUSY: u32 = 7;
pub const WRONG_THREAD: u32 = 8;
pub const UNSUPPORTED_LIMIT: u32 = 9;
pub const RESOURCE_LIMIT: u32 = 10;
pub const INTERNAL_ERROR: u32 = 11;
pub const CLEANUP_FAILED: u32 = 12;
pub const TIMEOUT: u32 = 13;
pub const RELEASE_MISMATCH: u32 = 14;

pub const ORIGIN_NONE: u32 = 0;
pub const ORIGIN_LIBRARY: u32 = 1;
pub const ORIGIN_DAEMON: u32 = 2;
pub const ORIGIN_TRANSPORT: u32 = 3;

pub const OK: u32 = 1;
pub const REJECTED: u32 = 2;
pub const FAILED: u32 = 3;
pub const INDETERMINATE: u32 = 4;

pub const MONITOR: u32 = 1;
pub const INTERACTIVE: u32 = 2;
pub const NATIVE_UNIX: u32 = 1;
pub const UNAVAILABLE: u32 = 0;
pub const UNSET: u32 = 1;
pub const VALUE: u32 = 2;

pub const IDLE: u32 = 1;
pub const RUNNING: u32 = 2;
pub const NEEDS_ATTENTION: u32 = 3;
pub const ACKNOWLEDGED: u32 = 4;
pub const COMPLETED: u32 = 5;
pub const STATUS_FAILED: u32 = 6;
pub const PROGRESS_NONE: u32 = 1;
pub const PROGRESS_NORMAL: u32 = 2;
pub const PROGRESS_INDETERMINATE: u32 = 3;
pub const PROGRESS_ERROR: u32 = 4;
pub const PROGRESS_PAUSED: u32 = 5;
pub const EXIT: u32 = 1;
pub const SIGNAL: u32 = 2;
pub const CLEANUP_COMPLETE: u32 = 1;
pub const CLEANUP_INCOMPLETE: u32 = 2;
pub const CLEANUP_UNKNOWN: u32 = 3;

pub const OPEN: u32 = 1;
pub const INPUT: u32 = 2;
pub const RESIZE: u32 = 3;
pub const ACQUIRE: u32 = 4;
pub const READY: u32 = 5;
pub const VIEWING: u32 = 6;
pub const ACKNOWLEDGE: u32 = 7;
pub const LEAVE: u32 = 8;
pub const KILL: u32 = 9;
pub const FOCUS: u32 = 10;

pub const RESULT_NONE: u32 = 0;
pub const OPENED: u32 = 1;
pub const INPUT_ACCEPTED: u32 = 2;
pub const RESIZE_ACCEPTED: u32 = 3;
pub const ACQUIRED: u32 = 4;
pub const READY_RESULT: u32 = 5;
pub const VIEWING_RESULT: u32 = 6;
pub const ACKNOWLEDGEMENT: u32 = 7;
pub const LEFT: u32 = 8;
pub const TERMINATION: u32 = 9;
pub const FOCUS_RESULT: u32 = 10;
pub const KNOWN_FINAL: u32 = 11;

pub const RESPONSE: u32 = 1;
pub const INITIAL_METADATA: u32 = 2;
pub const TITLE: u32 = 3;
pub const STATUS: u32 = 4;
pub const NOTIFICATION: u32 = 5;
pub const PROGRESS: u32 = 6;
pub const GRANTED: u32 = 7;
pub const REVOKED: u32 = 8;
pub const RESTORATION_BEGIN: u32 = 9;
pub const DISPLAY: u32 = 10;
pub const RESTORATION_COMPLETE: u32 = 11;
pub const FINAL: u32 = 12;
pub const GEOMETRY: u32 = 13;
pub const DIAGNOSTIC: u32 = 14;

pub const ATTACH: u32 = 1;
pub const TAKE_CONTROL: u32 = 2;
pub const HAND_BACK: u32 = 3;
pub const SUCCESSOR: u32 = 4;
pub const TAKEOVER: u32 = 1;
pub const SESSION_ENDING: u32 = 2;
pub const OSC9: u32 = 1;
pub const OSC99: u32 = 2;
pub const OSC777: u32 = 3;

pub const DELIVERY: u32 = 1;
pub const EMPTY: u32 = 2;
pub const TERMINAL: u32 = 3;
pub const CLIENT_STOPPED: u32 = 1;
pub const CLIENT_FAILED: u32 = 2;
pub const RETRY_NONE: u32 = 1;
pub const RETRY_PASSIVE: u32 = 2;
pub const RETRY_PERMANENT: u32 = 3;
pub const LEAVE_NOT_REQUIRED: u32 = 1;
pub const LEAVE_CONFIRMED: u32 = 2;
pub const LEAVE_REJECTED: u32 = 3;
pub const LEAVE_FAILED: u32 = 4;
pub const LEAVE_INDETERMINATE: u32 = 5;

/// Opaque per-instance handle (`cm1_instance`); only the library knows its layout.
#[repr(C)]
pub struct Instance {
    _private: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

/// Opaque thread-safe communication handle (`cm1_communication`).
#[repr(C)]
pub struct Communication {
    _private: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

/// Opaque thread-safe shutdown handle (`cm1_control`).
#[repr(C)]
pub struct Control {
    _private: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

/// Opaque immutable delivery (`cm1_delivery`), released through its own library.
#[repr(C)]
pub struct Delivery {
    _private: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Guid {
    pub bytes: [u8; 16],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Utc {
    pub bytes: [u8; 24],
}

/// A counted buffer; empty buffers use a null `data` and zero `length`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Bytes {
    pub data: *const u8,
    pub length: u64,
}

/// A counted token list; empty lists use a null `data` and zero `count`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Tokens {
    pub data: *const Bytes,
    pub count: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptionalText {
    pub present: u32,
    pub reserved: u32,
    pub value: Bytes,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Path {
    pub encoding: u32,
    pub reserved: u32,
    pub bytes: Bytes,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptionalPath {
    pub present: u32,
    pub reserved: u32,
    pub value: Path,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Geometry {
    pub cols: u32,
    pub rows: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Authority {
    pub generation: u64,
    pub period: u64,
    pub restoration_id: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Error {
    pub origin: u32,
    pub reserved: u32,
    pub code: Bytes,
    pub message: Bytes,
}

/// The result of every function-table call.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Call {
    pub code: u32,
    pub reserved: u32,
    pub error: Error,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Title {
    pub state: u32,
    pub reserved: u32,
    pub value: Bytes,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Status {
    pub state: u32,
    pub episode_present: u32,
    pub episode_id: u64,
    pub changed_at: Utc,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AvailableStatus {
    pub available: u32,
    pub reserved: u32,
    pub value: Status,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Progress {
    pub state: u32,
    pub percent_present: u32,
    pub percent: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AvailableProgress {
    pub available: u32,
    pub reserved: u32,
    pub value: Progress,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CommandResult {
    pub kind: u32,
    pub value: u32,
    pub signal_name: OptionalText,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptionalCommand {
    pub present: u32,
    pub reserved: u32,
    pub value: CommandResult,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Metadata {
    pub title: Title,
    pub status: AvailableStatus,
    pub progress: AvailableProgress,
    pub command_result: OptionalCommand,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Settings {
    pub escape_enabled: u8,
    pub escape_byte: u8,
    pub take_control_enabled: u8,
    pub take_control_byte: u8,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Diagnostics {
    pub log_path: OptionalPath,
    pub filter: Bytes,
}

/// Effective per-instance limits; every field is required, never zero-as-default.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Limits {
    pub client_bytes: u64,
    pub request_bytes: u64,
    pub event_bytes: u64,
    pub progress_bytes: u64,
    pub drain_payload_bytes: u64,
    pub request_records: u32,
    pub event_records: u32,
    pub progress_records: u32,
    pub deliveries: u32,
    pub operations: u32,
    pub chunk_bytes: u32,
    pub drain_records: u32,
    pub wire_frame_bytes: u32,
    pub metadata_record_bytes: u32,
    pub title_bytes: u32,
    pub notification_title_bytes: u32,
    pub notification_body_bytes: u32,
    pub max_workers: u32,
    pub operation_ms: u32,
    pub kill_ms: u32,
    pub restoration_stall_ms: u32,
    pub restoration_total_ms: u32,
    pub delivery_stall_ms: u32,
    pub leave_ms: u32,
    pub cleanup_ms: u32,
    pub start_ms: u32,
    pub reserved: u32,
}

/// Bounded resource requirements a release embeds in its description.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Requirements {
    pub release_bytes: u64,
    pub instance_bytes: u64,
    pub owner_stack_bytes: u64,
    pub worker_stack_bytes: u64,
    pub workers: u32,
    pub instance_descriptors: u32,
    pub load_descriptors: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Startup {
    pub session_id: Guid,
    pub generation: u64,
    pub expected_release: Bytes,
    pub metadata_path: Path,
    pub home_path: Path,
    pub runtime_path: Path,
    pub settings: Settings,
    pub diagnostics: Diagnostics,
    pub limits: Limits,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Created {
    pub instance: *mut Instance,
    pub communication: *mut Communication,
    pub control: *mut Control,
    pub effective_limits: Limits,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Open {
    pub mode: u32,
    pub reserved: u32,
    pub geometry: Geometry,
    pub required: Tokens,
    pub optional: Tokens,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Input {
    pub authority: Authority,
    pub bytes: Bytes,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Resize {
    pub authority: Authority,
    pub geometry: Geometry,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Flag {
    pub authority: Authority,
    pub value: u8,
    pub reserved: [u8; 7],
}

/// The active argument member named by [`Request::tag`].
#[repr(C)]
#[derive(Clone, Copy)]
pub union RequestArgs {
    pub open: Open,
    pub input: Input,
    pub resize: Resize,
    pub generation: u64,
    pub authority: Authority,
    pub flag: Flag,
    pub required: Tokens,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Request {
    pub tag: u32,
    pub reserved: u32,
    pub request_id: u64,
    pub args: RequestArgs,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Opened {
    pub connection_id: Guid,
    pub selected: Tokens,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AuthorityResult {
    pub changed: u8,
    pub reserved: [u8; 7],
    pub period: u64,
    pub restoration_id: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FlagResult {
    pub changed: u8,
    pub value: u8,
    pub reserved: [u8; 2],
    pub accepted_bytes: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Termination {
    pub command_result: OptionalCommand,
    pub cleanup: u32,
    pub reserved: u32,
}

/// The active result member named by [`Response::result_tag`].
#[repr(C)]
#[derive(Clone, Copy)]
pub union ResponseResult {
    pub opened: Opened,
    pub accepted_bytes: u32,
    pub authority: AuthorityResult,
    pub flag: FlagResult,
    pub changed: u8,
    pub attachment_detached: u8,
    pub termination: Termination,
    pub known_final: CommandResult,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Response {
    pub request_id: u64,
    pub outcome: u32,
    pub result_tag: u32,
    pub error: Error,
    pub result: ResponseResult,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Granted {
    pub period: u64,
    pub restoration_id: u64,
    pub geometry: Geometry,
    pub reason: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Revoked {
    pub period: u64,
    pub restoration_id: u64,
    pub reason: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Restoration {
    pub period: u64,
    pub restoration_id: u64,
    pub geometry: Geometry,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Barrier {
    pub period: u64,
    pub restoration_id: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Display {
    pub authority: Barrier,
    pub bytes: Bytes,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GeometryEvent {
    pub period: u64,
    pub geometry: Geometry,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Notification {
    pub source: u32,
    pub episode_present: u32,
    pub episode_id: u64,
    pub source_id: OptionalText,
    pub title: OptionalText,
    pub body: OptionalText,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Final {
    pub command_result: CommandResult,
    pub status: AvailableStatus,
}

/// The active data member named by [`Record::tag`].
#[repr(C)]
#[derive(Clone, Copy)]
pub union RecordData {
    pub response: Response,
    pub metadata: Metadata,
    pub title: Title,
    pub status: Status,
    pub notification: Notification,
    pub progress: Progress,
    pub granted: Granted,
    pub revoked: Revoked,
    pub restoration: Restoration,
    pub display: Display,
    pub barrier: Barrier,
    pub geometry: GeometryEvent,
    pub r#final: Final,
    pub diagnostic: Error,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Record {
    pub tag: u32,
    pub reserved: u32,
    pub generation: u64,
    pub data: RecordData,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Terminal {
    pub generation: u64,
    pub client_outcome: u32,
    pub retry: u32,
    pub leave: u32,
    pub attachment_detached: u8,
    pub connection_closed: u8,
    pub reserved: [u8; 2],
    pub error: Error,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RunReport {
    pub workers_cleanup: u32,
    pub reserved: u32,
    pub terminal: Terminal,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Drained {
    pub kind: u32,
    pub reserved: u32,
    pub delivery: *mut Delivery,
    pub records: *const Record,
    pub count: u64,
    pub terminal: *const Terminal,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Metrics {
    pub allocation_bytes: u64,
    pub peak_allocation_bytes: u64,
    pub release_allocation_bytes: u64,
    pub peak_release_allocation_bytes: u64,
    pub request_bytes: u64,
    pub peak_request_bytes: u64,
    pub event_bytes: u64,
    pub peak_event_bytes: u64,
    pub input_bytes_accepted: u64,
    pub display_bytes_delivered: u64,
    pub records_drained: u64,
    pub deliveries_released: u64,
    pub request_records: u32,
    pub peak_request_records: u32,
    pub event_records: u32,
    pub peak_event_records: u32,
    pub deliveries: u32,
    pub peak_deliveries: u32,
    pub operations: u32,
    pub peak_operations: u32,
    pub workers: u32,
    pub peak_workers: u32,
    pub descriptors: u32,
    pub peak_descriptors: u32,
}

/// The eleven-call function table. A valid description has no null entry, so
/// `None` only appears when a host reads a malformed description.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Interface {
    pub create: Option<unsafe extern "C" fn(*const Startup, *mut Created) -> Call>,
    pub run: Option<unsafe extern "C" fn(*mut Instance, *mut RunReport) -> Call>,
    pub destroy: Option<unsafe extern "C" fn(*mut Instance) -> Call>,
    pub submit: Option<unsafe extern "C" fn(*mut Communication, *const Request) -> Call>,
    pub drain: Option<unsafe extern "C" fn(*mut Communication, *mut Drained) -> Call>,
    pub release_delivery: Option<unsafe extern "C" fn(*mut Delivery) -> Call>,
    pub shutdown: Option<unsafe extern "C" fn(*mut Control) -> Call>,
    pub release_communication: Option<unsafe extern "C" fn(*mut Communication) -> Call>,
    pub release_control: Option<unsafe extern "C" fn(*mut Control) -> Call>,
    pub notification: Option<unsafe extern "C" fn(*mut Communication, *mut i32) -> Call>,
    pub snapshot: Option<unsafe extern "C" fn(*mut Communication, *mut Metrics) -> Call>,
}

/// The immutable description the bootstrap returns; it lives until process exit.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Description {
    pub revision: u32,
    pub reserved: u32,
    pub release: Bytes,
    pub capabilities: Tokens,
    pub requirements: Requirements,
    pub functions: Interface,
}
