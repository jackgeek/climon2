//! `monitor.limits` (docs/monitor-operational-limits.md): eleven groups of
//! budgets, ceilings, counts and durations. Each setting is read in its native
//! type; once every group is read, each setting is checked against its range,
//! and then the groups against each other. Every refusal names the settings it
//! concerns. `Limits` exists only after both checks, and nothing outside this
//! module can change it. The monitor reads its history rows, and checks that it
//! holds one session's [`MandatoryAllocation`]; its values are otherwise only
//! reported so far.
use super::{ConfigError, Range, object};
use crate::protocol::TerminalSize;
use serde::{Deserialize, Serialize};

/// A positive budget or ceiling, held in a `u64`.
const POSITIVE_64: Range = Range {
    min: 1,
    max: u64::MAX,
};
/// A positive count or single-item byte ceiling, held in a `u32`.
const POSITIVE_32: Range = Range {
    min: 1,
    max: u32::MAX as u64,
};
/// A worker count, which may be zero, held in a `u32`.
const WORKERS: Range = Range {
    min: 0,
    max: u32::MAX as u64,
};
/// Milliseconds, 1 through 3,600,000.
const MILLIS: Range = Range {
    min: 1,
    max: 3_600_000,
};
/// A processing quantum, 1 through 50 milliseconds.
const QUANTUM: Range = Range { min: 1, max: 50 };
/// A render ceiling, 1 through 120 frames per second.
const FPS: Range = Range { min: 1, max: 120 };
/// A jitter percentage, 0 through 50.
const PERCENT: Range = Range { min: 0, max: 50 };
/// The client library's own instance allocation, which its share of
/// `memory.client_bytes` holds beside its queues and one wire frame: this
/// release's qualified `instance_bytes` (#119).
const LIBRARY_INSTANCE_BYTES: u64 = 1_048_576;

/// What names the client library's share of `memory.client_bytes` in
/// refusals.
const LIBRARY_SHARE: &str = "the client library's share (monitor.limits.queues.request_bytes + monitor.limits.queues.event_bytes + monitor.limits.payload.wire_frame_bytes + its 1048576-byte instance allocation)";

/// Local history rows, 200 through 100,000.
const HISTORY_ROWS: Range = Range {
    min: 200,
    max: 100_000,
};

/// Tracked allocation budgets; every sub-budget is part of the total.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Memory {
    total_bytes: u64,
    client_bytes: u64,
    history_bytes_per_session: u64,
    history_bytes_total: u64,
    queue_bytes_total: u64,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            total_bytes: 603_979_776,
            client_bytes: 67_108_864,
            history_bytes_per_session: 16_777_216,
            history_bytes_total: 268_435_456,
            queue_bytes_total: 67_108_864,
        }
    }
}

/// The monitor's local history; never the daemon's `scrollback.max_rows`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct History {
    max_rows: u32,
}

impl Default for History {
    fn default() -> Self {
        Self { max_rows: 10_000 }
    }
}

/// Reservations for threads, descriptors and loaded releases.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Resources {
    stack_bytes: u64,
    max_workers_per_client: u32,
    descriptors_per_client: u32,
    descriptors_per_load: u32,
    descriptor_headroom: u32,
    release_bytes: u64,
}

impl Default for Resources {
    fn default() -> Self {
        Self {
            stack_bytes: 2_097_152,
            max_workers_per_client: 2,
            descriptors_per_client: 8,
            descriptors_per_load: 4,
            descriptor_headroom: 64,
            release_bytes: 67_108_864,
        }
    }
}

/// Per-client queues; each progress reserve sits inside its event allowance.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Queues {
    request_bytes: u64,
    request_records: u32,
    event_bytes: u64,
    event_records: u32,
    progress_bytes: u64,
    progress_records: u32,
    deliveries: u32,
    operations: u32,
}

impl Default for Queues {
    fn default() -> Self {
        Self {
            request_bytes: 262_144,
            request_records: 64,
            event_bytes: 1_048_576,
            event_records: 1024,
            progress_bytes: 65_536,
            progress_records: 32,
            deliveries: 8,
            operations: 16,
        }
    }
}

/// Display and input chunks, and what one delivery drains.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Transfer {
    chunk_bytes: u32,
    drain_records: u32,
    drain_payload_bytes: u64,
}

impl Default for Transfer {
    fn default() -> Self {
        Self {
            chunk_bytes: 65_536,
            drain_records: 64,
            drain_payload_bytes: 65_536,
        }
    }
}

/// Frame, record, text and notification-assembly ceilings.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Payload {
    wire_frame_bytes: u32,
    metadata_record_bytes: u32,
    title_bytes: u32,
    notification_title_bytes: u32,
    notification_body_bytes: u32,
    assembly_records: u32,
    assembly_bytes: u64,
}

impl Default for Payload {
    fn default() -> Self {
        Self {
            wire_frame_bytes: 1_048_576,
            metadata_record_bytes: 16_384,
            title_bytes: 4096,
            notification_title_bytes: 4096,
            notification_body_bytes: 8192,
            assembly_records: 32,
            assembly_bytes: 65_536,
        }
    }
}

/// Concurrent probes, release loads and client starts.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Scheduling {
    probes: u32,
    loads: u32,
    starts: u32,
    newer_start_burst: u32,
}

impl Default for Scheduling {
    fn default() -> Self {
        Self {
            probes: 8,
            loads: 2,
            starts: 8,
            newer_start_burst: 8,
        }
    }
}

/// Discovery passes and the metadata they read.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Discovery {
    pass_ms: u32,
    stall_ms: u32,
    batch_records: u32,
    batch_bytes: u64,
    metadata_file_bytes: u64,
    metadata_depth: u32,
}

impl Default for Discovery {
    fn default() -> Self {
        Self {
            pass_ms: 30_000,
            stall_ms: 5000,
            batch_records: 128,
            batch_bytes: 1_048_576,
            metadata_file_bytes: 262_144,
            metadata_depth: 32,
        }
    }
}

/// Progress deadlines.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Deadlines {
    load_ms: u32,
    start_ms: u32,
    operation_ms: u32,
    kill_ms: u32,
    restoration_stall_ms: u32,
    restoration_total_ms: u32,
    delivery_stall_ms: u32,
    leave_ms: u32,
    cleanup_ms: u32,
    shutdown_ms: u32,
    final_host_ms: u32,
    assembly_ms: u32,
}

impl Default for Deadlines {
    fn default() -> Self {
        Self {
            load_ms: 10_000,
            start_ms: 10_000,
            operation_ms: 5000,
            kill_ms: 10_000,
            restoration_stall_ms: 5000,
            restoration_total_ms: 60_000,
            delivery_stall_ms: 5000,
            leave_ms: 5000,
            cleanup_ms: 10_000,
            shutdown_ms: 10_000,
            final_host_ms: 1000,
            assembly_ms: 5000,
        }
    }
}

/// Passive retry backoff.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Retry {
    initial_ms: u32,
    cap_ms: u32,
    jitter_percent: u32,
    admission_ms: u32,
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            initial_ms: 1000,
            cap_ms: 30_000,
            jitter_percent: 20,
            admission_ms: 1000,
        }
    }
}

/// Rendering, work quanta and search.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Ui {
    fps: u32,
    client_quantum_ms: u32,
    background_quantum_ms: u32,
    search_bytes: u64,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            fps: 30,
            client_quantum_ms: 2,
            background_quantum_ms: 8,
            search_bytes: 4096,
        }
    }
}

/// Every group as read, before its settings are checked against their ranges
/// and against each other.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Groups {
    #[serde(deserialize_with = "object")]
    memory: Memory,
    #[serde(deserialize_with = "object")]
    history: History,
    #[serde(deserialize_with = "object")]
    resources: Resources,
    #[serde(deserialize_with = "object")]
    queues: Queues,
    #[serde(deserialize_with = "object")]
    transfer: Transfer,
    #[serde(deserialize_with = "object")]
    payload: Payload,
    #[serde(deserialize_with = "object")]
    scheduling: Scheduling,
    #[serde(deserialize_with = "object")]
    discovery: Discovery,
    #[serde(deserialize_with = "object")]
    deadlines: Deadlines,
    #[serde(deserialize_with = "object")]
    retry: Retry,
    #[serde(deserialize_with = "object")]
    ui: Ui,
}

/// `monitor.limits`: the groups, once every setting is within its range and
/// consistent with the others.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "Groups")]
pub struct Limits(Groups);

/// What the monitor's own discovery passes read from `monitor.limits`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryLimits {
    /// `scheduling.probes`: the most hello probes in flight.
    pub probes: u32,
    pub pass_ms: u32,
    pub batch_records: u32,
    pub batch_bytes: u64,
    pub metadata_file_bytes: u64,
    pub metadata_depth: u32,
}

/// What bounds each session's terminal model in the monitor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelLimits {
    /// Allocation capacity for the model and its staged restoration.
    pub bytes: u64,
    /// `history.max_rows`.
    pub history_rows: u32,
}

/// The client supervisor's bounded lanes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Lanes {
    /// `scheduling.loads`: release loads at once.
    pub loads: u32,
    /// `scheduling.starts`: client starts at once.
    pub starts: u32,
    /// `scheduling.newer_start_burst`: newer first starts before the oldest
    /// waiting one gets a turn (#120).
    pub newer_start_burst: u32,
    /// `deadlines.load_ms`: past it a load still running is reported as
    /// loading slowly, never aborted (owner decision on #120).
    pub load_ms: u32,
    /// `deadlines.shutdown_ms`: how long clients may take to shut down.
    pub shutdown_ms: u32,
    /// `deadlines.final_host_ms`: how long the final host restoration and
    /// diagnostics may take after it.
    pub final_host_ms: u32,
}

/// What admission reserves before each release load and each instance's
/// create, and against what (#120): the configured reservations
/// (`resources`), which the greater qualified requirement of a loaded
/// release raises, within `memory.total_bytes`, and the reconsideration
/// interval `retry.admission_ms`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Admission {
    /// `memory.total_bytes`: every reservation consumes it.
    pub total_bytes: u64,
    /// `resources.stack_bytes`: each owner, worker and load thread's stack.
    pub stack_bytes: u64,
    /// `resources.max_workers_per_client`: never raised for a release.
    pub max_workers_per_client: u32,
    /// `resources.descriptors_per_client`.
    pub descriptors_per_client: u32,
    /// `resources.descriptors_per_load`.
    pub descriptors_per_load: u32,
    /// `resources.descriptor_headroom`: descriptors left to the rest of the
    /// monitor below the operating system's limit.
    pub descriptor_headroom: u32,
    /// `resources.release_bytes`.
    pub release_bytes: u64,
    /// `retry.admission_ms`: the most often a blocked row or release is
    /// reconsidered.
    pub admission_ms: u32,
}

/// The monitor's passive retry backoff (`retry`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Backoff {
    /// `retry.initial_ms`: the first delay.
    pub initial_ms: u32,
    /// `retry.cap_ms`: no delay is longer.
    pub cap_ms: u32,
    /// `retry.jitter_percent`: each delay varies by up to this much.
    pub jitter_percent: u32,
}

/// The monitor's own memory budgets (`memory`) its pressure policy keeps
/// to (#119).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryLimits {
    /// `memory.total_bytes`: everything the monitor tracks.
    pub total_bytes: u64,
    /// `memory.history_bytes_per_session`: one session's history.
    pub history_bytes_per_session: u64,
    /// `memory.history_bytes_total`: every session's history together.
    pub history_bytes_total: u64,
}

/// The monitor UI's cadence (`ui`): frames a second and the work quanta.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cadence {
    /// `ui.fps`: the most frames painted a second.
    pub fps: u32,
    /// `ui.client_quantum_ms`: one client's work at a turn.
    pub client_quantum_ms: u32,
    /// `ui.background_quantum_ms`: all clients' work at one UI turn.
    pub background_quantum_ms: u32,
}

impl Limits {
    /// The effective `cm1_limits` of every library instance, each field from
    /// the setting the native contract names. `client_bytes` is the
    /// library's share of `memory.client_bytes`: its request and event
    /// queues, one wire frame and its instance allocation; the session's
    /// terminal model takes the rest ([`Limits::model`]), never a second
    /// allowance (#119).
    pub fn library(&self) -> crate::native::Limits {
        let Groups {
            resources,
            queues,
            transfer,
            payload,
            deadlines,
            ..
        } = self.0;
        crate::native::Limits {
            client_bytes: self.0.library_share(),
            request_bytes: queues.request_bytes,
            event_bytes: queues.event_bytes,
            progress_bytes: queues.progress_bytes,
            drain_payload_bytes: transfer.drain_payload_bytes,
            request_records: queues.request_records,
            event_records: queues.event_records,
            progress_records: queues.progress_records,
            deliveries: queues.deliveries,
            operations: queues.operations,
            chunk_bytes: transfer.chunk_bytes,
            drain_records: transfer.drain_records,
            wire_frame_bytes: payload.wire_frame_bytes,
            metadata_record_bytes: payload.metadata_record_bytes,
            title_bytes: payload.title_bytes,
            notification_title_bytes: payload.notification_title_bytes,
            notification_body_bytes: payload.notification_body_bytes,
            max_workers: resources.max_workers_per_client,
            operation_ms: deadlines.operation_ms,
            kill_ms: deadlines.kill_ms,
            restoration_stall_ms: deadlines.restoration_stall_ms,
            restoration_total_ms: deadlines.restoration_total_ms,
            delivery_stall_ms: deadlines.delivery_stall_ms,
            leave_ms: deadlines.leave_ms,
            cleanup_ms: deadlines.cleanup_ms,
            start_ms: deadlines.start_ms,
            reserved: 0,
        }
    }

    pub fn discovery(&self) -> DiscoveryLimits {
        let Groups {
            scheduling,
            discovery,
            ..
        } = self.0;
        DiscoveryLimits {
            probes: scheduling.probes,
            pass_ms: discovery.pass_ms,
            batch_records: discovery.batch_records,
            batch_bytes: discovery.batch_bytes,
            metadata_file_bytes: discovery.metadata_file_bytes,
            metadata_depth: discovery.metadata_depth,
        }
    }

    /// Each session's terminal model: `memory.client_bytes` less the
    /// client library's share ([`Limits::library`]), with its staged
    /// restoration, and `history.max_rows` rows.
    pub fn model(&self) -> ModelLimits {
        ModelLimits {
            bytes: self.0.memory.client_bytes - self.0.library_share(),
            history_rows: self.0.history.max_rows,
        }
    }

    pub fn retry(&self) -> Backoff {
        let Retry {
            initial_ms,
            cap_ms,
            jitter_percent,
            ..
        } = self.0.retry;
        Backoff {
            initial_ms,
            cap_ms,
            jitter_percent,
        }
    }

    pub fn memory(&self) -> MemoryLimits {
        let Memory {
            total_bytes,
            history_bytes_per_session,
            history_bytes_total,
            ..
        } = self.0.memory;
        MemoryLimits {
            total_bytes,
            history_bytes_per_session,
            history_bytes_total,
        }
    }

    pub fn cadence(&self) -> Cadence {
        let Ui {
            fps,
            client_quantum_ms,
            background_quantum_ms,
            ..
        } = self.0.ui;
        Cadence {
            fps,
            client_quantum_ms,
            background_quantum_ms,
        }
    }

    /// `ui.search_bytes`: the most UTF-8 bytes the selector's search holds.
    pub fn search_bytes(&self) -> u64 {
        self.0.ui.search_bytes
    }

    pub fn lanes(&self) -> Lanes {
        Lanes {
            loads: self.0.scheduling.loads,
            starts: self.0.scheduling.starts,
            newer_start_burst: self.0.scheduling.newer_start_burst,
            load_ms: self.0.deadlines.load_ms,
            shutdown_ms: self.0.deadlines.shutdown_ms,
            final_host_ms: self.0.deadlines.final_host_ms,
        }
    }

    pub fn admission(&self) -> Admission {
        let Groups {
            memory,
            resources,
            retry,
            ..
        } = self.0;
        Admission {
            total_bytes: memory.total_bytes,
            stack_bytes: resources.stack_bytes,
            max_workers_per_client: resources.max_workers_per_client,
            descriptors_per_client: resources.descriptors_per_client,
            descriptors_per_load: resources.descriptors_per_load,
            descriptor_headroom: resources.descriptor_headroom,
            release_bytes: resources.release_bytes,
            admission_ms: retry.admission_ms,
        }
    }
}

impl TryFrom<Groups> for Limits {
    type Error = String;

    fn try_from(groups: Groups) -> Result<Self, String> {
        groups.violation().map_or(Ok(Self(groups)), Err)
    }
}

/// A setting's full name, or what else a rule compares, and its value, for
/// its errors.
#[derive(Clone, Copy)]
struct Named<'a> {
    name: &'a str,
    value: u64,
}

/// The setting at `group.field` of the groups in scope, named by that path, so
/// that no message can name a setting other than the one it reads.
macro_rules! named {
    ($group:ident . $field:ident) => {
        Named {
            name: concat!(
                "monitor.limits.",
                stringify!($group),
                ".",
                stringify!($field)
            ),
            value: u64::from($group.$field),
        }
    };
}

fn at_most(lesser: Named, greater: Named) -> Option<String> {
    (lesser.value > greater.value).then(|| {
        format!(
            "{} ({}) may not exceed {} ({})",
            lesser.name, lesser.value, greater.name, greater.value
        )
    })
}

fn below(lesser: Named, greater: Named) -> Option<String> {
    (lesser.value >= greater.value).then(|| {
        format!(
            "{} ({}) must be less than {} ({})",
            lesser.name, lesser.value, greater.name, greater.value
        )
    })
}

/// The refusal of the first of `rules` broken, in their order.
fn first_broken<const N: usize>(rules: [Option<String>; N]) -> Option<String> {
    rules.into_iter().flatten().next()
}

impl Groups {
    /// The client library's share of `memory.client_bytes`: its request and
    /// event queues, one wire frame and its instance allocation, what #99's
    /// create requires; saturating, so an oversized share is refused.
    fn library_share(&self) -> u64 {
        [
            self.queues.request_bytes,
            self.queues.event_bytes,
            self.payload.wire_frame_bytes.into(),
            LIBRARY_INSTANCE_BYTES,
        ]
        .into_iter()
        .fold(0, u64::saturating_add)
    }

    /// The first rule these limits break: a setting outside its range, or
    /// else settings that disagree.
    fn violation(&self) -> Option<String> {
        self.out_of_range().or_else(|| self.disagreement())
    }

    /// The first setting outside its range, in the order of the groups.
    fn out_of_range(&self) -> Option<String> {
        let Self {
            memory,
            history,
            resources,
            queues,
            transfer,
            payload,
            scheduling,
            discovery,
            deadlines,
            retry,
            ui,
        } = self;
        [
            (named!(memory.total_bytes), POSITIVE_64),
            (named!(memory.client_bytes), POSITIVE_64),
            (named!(memory.history_bytes_per_session), POSITIVE_64),
            (named!(memory.history_bytes_total), POSITIVE_64),
            (named!(memory.queue_bytes_total), POSITIVE_64),
            (named!(history.max_rows), HISTORY_ROWS),
            (named!(resources.stack_bytes), POSITIVE_64),
            (named!(resources.max_workers_per_client), WORKERS),
            (named!(resources.descriptors_per_client), POSITIVE_32),
            (named!(resources.descriptors_per_load), POSITIVE_32),
            (named!(resources.descriptor_headroom), POSITIVE_32),
            (named!(resources.release_bytes), POSITIVE_64),
            (named!(queues.request_bytes), POSITIVE_64),
            (named!(queues.request_records), POSITIVE_32),
            (named!(queues.event_bytes), POSITIVE_64),
            (named!(queues.event_records), POSITIVE_32),
            (named!(queues.progress_bytes), POSITIVE_64),
            (named!(queues.progress_records), POSITIVE_32),
            (named!(queues.deliveries), POSITIVE_32),
            (named!(queues.operations), POSITIVE_32),
            (named!(transfer.chunk_bytes), POSITIVE_32),
            (named!(transfer.drain_records), POSITIVE_32),
            (named!(transfer.drain_payload_bytes), POSITIVE_64),
            (named!(payload.wire_frame_bytes), POSITIVE_32),
            (named!(payload.metadata_record_bytes), POSITIVE_32),
            (named!(payload.title_bytes), POSITIVE_32),
            (named!(payload.notification_title_bytes), POSITIVE_32),
            (named!(payload.notification_body_bytes), POSITIVE_32),
            (named!(payload.assembly_records), POSITIVE_32),
            (named!(payload.assembly_bytes), POSITIVE_64),
            (named!(scheduling.probes), POSITIVE_32),
            (named!(scheduling.loads), POSITIVE_32),
            (named!(scheduling.starts), POSITIVE_32),
            (named!(scheduling.newer_start_burst), POSITIVE_32),
            (named!(discovery.pass_ms), MILLIS),
            (named!(discovery.stall_ms), MILLIS),
            (named!(discovery.batch_records), POSITIVE_32),
            (named!(discovery.batch_bytes), POSITIVE_64),
            (named!(discovery.metadata_file_bytes), POSITIVE_64),
            (named!(discovery.metadata_depth), POSITIVE_32),
            (named!(deadlines.load_ms), MILLIS),
            (named!(deadlines.start_ms), MILLIS),
            (named!(deadlines.operation_ms), MILLIS),
            (named!(deadlines.kill_ms), MILLIS),
            (named!(deadlines.restoration_stall_ms), MILLIS),
            (named!(deadlines.restoration_total_ms), MILLIS),
            (named!(deadlines.delivery_stall_ms), MILLIS),
            (named!(deadlines.leave_ms), MILLIS),
            (named!(deadlines.cleanup_ms), MILLIS),
            (named!(deadlines.shutdown_ms), MILLIS),
            (named!(deadlines.final_host_ms), MILLIS),
            (named!(deadlines.assembly_ms), MILLIS),
            (named!(retry.initial_ms), MILLIS),
            (named!(retry.cap_ms), MILLIS),
            (named!(retry.jitter_percent), PERCENT),
            (named!(retry.admission_ms), MILLIS),
            (named!(ui.fps), FPS),
            (named!(ui.client_quantum_ms), QUANTUM),
            (named!(ui.background_quantum_ms), QUANTUM),
            (named!(ui.search_bytes), POSITIVE_64),
        ]
        .into_iter()
        .find_map(|(setting, range)| range.refusal(setting.name, setting.value))
    }

    /// The first rule between settings these limits break, in this order:
    /// sub-budgets, reservations and every other monitor byte budget fit the
    /// totals that include them (each client's queues fit both the aggregate
    /// queue budget and the client's own, and the client library's whole
    /// share fits the client's budget; the daemon's notification assembly
    /// limits are not monitor budgets); progress reserves leave room for
    /// ordinary work; the
    /// largest chunk fits one request, the ordinary event capacity and one
    /// delivery, and the largest metadata record fits the reserve, that
    /// capacity and one delivery; the per-client quantum fits the aggregate
    /// one; the initial retry delay fits its cap.
    fn disagreement(&self) -> Option<String> {
        let Self {
            memory,
            resources,
            queues,
            transfer,
            payload,
            discovery,
            retry,
            ui,
            ..
        } = self;
        let total = named!(memory.total_bytes);
        let client = named!(memory.client_bytes);
        let per_session = named!(memory.history_bytes_per_session);
        let history = named!(memory.history_bytes_total);
        let queued = named!(memory.queue_bytes_total);
        let request = named!(queues.request_bytes);
        let events = named!(queues.event_bytes);
        let reserve = named!(queues.progress_bytes);
        let ordinary = Named {
            name: "monitor.limits.queues.event_bytes - monitor.limits.queues.progress_bytes",
            value: events.value.saturating_sub(reserve.value),
        };
        let chunk = named!(transfer.chunk_bytes);
        let delivery = named!(transfer.drain_payload_bytes);
        let record = named!(payload.metadata_record_bytes);
        first_broken([
            at_most(client, total),
            at_most(history, total),
            at_most(queued, total),
            at_most(named!(resources.stack_bytes), total),
            at_most(named!(resources.release_bytes), total),
            at_most(delivery, total),
            at_most(named!(discovery.batch_bytes), total),
            at_most(named!(discovery.metadata_file_bytes), total),
            at_most(named!(ui.search_bytes), total),
            at_most(per_session, history),
            at_most(per_session, client),
            at_most(request, queued),
            at_most(events, queued),
            at_most(request, client),
            at_most(events, client),
            at_most(
                Named {
                    name: LIBRARY_SHARE,
                    value: self.library_share(),
                },
                client,
            ),
            below(reserve, events),
            below(
                named!(queues.progress_records),
                named!(queues.event_records),
            ),
            at_most(chunk, request),
            at_most(chunk, ordinary),
            at_most(chunk, delivery),
            at_most(record, reserve),
            at_most(record, ordinary),
            at_most(record, delivery),
            at_most(
                named!(ui.client_quantum_ms),
                named!(ui.background_quantum_ms),
            ),
            at_most(named!(retry.initial_ms), named!(retry.cap_ms)),
        ])
    }
}

/// The session daemon's notification assembly limits:
/// `payload.assembly_records` incomplete assemblies, holding at most
/// `payload.assembly_bytes` together, each living `deadlines.assembly_ms`
/// from its creation. `exec` passes the values it validated to the daemon
/// it starts, which keeps them for the session's lifetime; no monitor
/// reconfigures a running daemon.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Assembly {
    records: u32,
    bytes: u64,
    lifetime_ms: u32,
}

impl Assembly {
    /// Limits within their settings' ranges: a positive count and byte
    /// budget, and a lifetime of 1 through 3,600,000 ms.
    pub fn new(records: u32, bytes: u64, lifetime_ms: u32) -> Option<Self> {
        let within = POSITIVE_32.contains(records.into())
            && POSITIVE_64.contains(bytes)
            && MILLIS.contains(lifetime_ms.into());
        within.then_some(Self {
            records,
            bytes,
            lifetime_ms,
        })
    }

    pub fn records(self) -> u32 {
        self.records
    }

    pub fn bytes(self) -> u64 {
        self.bytes
    }

    pub fn lifetime_ms(self) -> u32 {
        self.lifetime_ms
    }
}

/// The documented defaults.
impl Default for Assembly {
    fn default() -> Self {
        Limits::default().assembly()
    }
}

/// One session's mandatory allocation, as the monitor's terminal model of it
/// is charged: the current model, and the history holding its recent page,
/// beside the parser the monitor shares among all its models. The monitor
/// cannot admit a session its limits cannot hold this for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MandatoryAllocation {
    /// The session geometry the bytes are charged at.
    pub size: TerminalSize,
    /// Bytes of the monitor's one parser, which every session's model shares
    /// and the aggregate budget holds once.
    pub parser: u64,
    /// Bytes of the current model alone.
    pub current_model: u64,
    /// Bytes of the history that holds the recent page.
    pub recent_page: u64,
}

impl Limits {
    /// The daemon's notification assembly limits these settings give.
    pub fn assembly(&self) -> Assembly {
        let Groups {
            payload, deadlines, ..
        } = &self.0;
        Assembly {
            records: payload.assembly_records,
            bytes: payload.assembly_bytes,
            lifetime_ms: deadlines.assembly_ms,
        }
    }

    /// The most rows of local history a session's terminal model keeps.
    pub fn history_rows(&self) -> u32 {
        self.0.history.max_rows
    }

    /// Refuses limits that cannot hold one session's `mandatory` allocation:
    /// the total must hold the parser and the session's current model,
    /// recent page and client library share together, then the client
    /// budget the model, page and library share, the history total, then
    /// the per-session history budget, the page. Each total comes before the
    /// budgets it contains, so a refusal names the limit to raise.
    pub fn hold(&self, mandatory: MandatoryAllocation) -> Result<(), ConfigError> {
        let memory = &self.0.memory;
        let TerminalSize { cols, rows } = mandatory.size;
        let session = format!("a {cols}x{rows} session");
        let model_and_page = format!(
            "the current model and recent page of {session} and its client library's share"
        );
        let with_parser = format!("the parser and {model_and_page}");
        let recent_page = format!("the recent page of {session}");
        let model_and_page = Named {
            name: &model_and_page,
            value: mandatory.current_model + mandatory.recent_page + self.0.library_share(),
        };
        let with_parser = Named {
            name: &with_parser,
            value: mandatory.parser + model_and_page.value,
        };
        let recent_page = Named {
            name: &recent_page,
            value: mandatory.recent_page,
        };
        first_broken([
            at_most(with_parser, named!(memory.total_bytes)),
            at_most(model_and_page, named!(memory.client_bytes)),
            at_most(recent_page, named!(memory.history_bytes_total)),
            at_most(recent_page, named!(memory.history_bytes_per_session)),
        ])
        .map_or(Ok(()), |detail| Err(ConfigError::Invalid(detail)))
    }
}

#[cfg(test)]
mod tests {
    use super::Assembly;

    /// LIM-01 (pure, exec-provided assembly limits): the documented
    /// defaults are 32 assemblies, 65,536 bytes and 5,000 ms; each limit
    /// is accepted at its bounds and refused one past them.
    #[test]
    fn assembly_limits_hold_their_settings_ranges() {
        let defaults = Assembly::default();
        assert_eq!(
            (defaults.records(), defaults.bytes(), defaults.lifetime_ms()),
            (32, 65_536, 5_000)
        );
        let lowest = Assembly::new(1, 1, 1).unwrap();
        assert_eq!(
            (lowest.records(), lowest.bytes(), lowest.lifetime_ms()),
            (1, 1, 1)
        );
        assert!(Assembly::new(u32::MAX, u64::MAX, 3_600_000).is_some());
        for (records, bytes, lifetime_ms) in [(0, 1, 1), (1, 0, 1), (1, 1, 0), (1, 1, 3_600_001)] {
            assert_eq!(Assembly::new(records, bytes, lifetime_ms), None);
        }
    }
}
