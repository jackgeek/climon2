//! `monitor.limits` (docs/monitor-operational-limits.md): eleven groups of
//! budgets, ceilings, counts and durations. Each setting is read in its native
//! type; once every group is read, each setting is checked against its range,
//! and then the groups against each other. Every refusal names the settings it
//! concerns. `Limits` exists only after both checks, and nothing outside this
//! module can change it; its values are only reported so far.
use super::{Range, object};
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
            total_bytes: 536_870_912,
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

/// Rendering, work quanta, toasts and search.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Ui {
    fps: u32,
    client_quantum_ms: u32,
    background_quantum_ms: u32,
    toast_records: u32,
    toast_bytes: u64,
    search_bytes: u64,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            fps: 30,
            client_quantum_ms: 2,
            background_quantum_ms: 8,
            toast_records: 128,
            toast_bytes: 524_288,
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

impl TryFrom<Groups> for Limits {
    type Error = String;

    fn try_from(groups: Groups) -> Result<Self, String> {
        groups.violation().map_or(Ok(Self(groups)), Err)
    }
}

/// A setting's full name and its value, for its errors.
#[derive(Clone, Copy)]
struct Named {
    name: &'static str,
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

impl Groups {
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
            (named!(ui.toast_records), POSITIVE_32),
            (named!(ui.toast_bytes), POSITIVE_64),
            (named!(ui.search_bytes), POSITIVE_64),
        ]
        .into_iter()
        .find_map(|(setting, range)| range.refusal(setting.name, setting.value))
    }

    /// The first rule between settings these limits break, in this order:
    /// sub-budgets, reservations and every other monitor byte budget fit the
    /// totals that include them (each client's queues fit both the aggregate
    /// queue budget and the client's own, and toasts, a UI queue, fit the
    /// aggregate one; the daemon's notification assembly limits are not
    /// monitor budgets); progress reserves leave room for ordinary work; the
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
        [
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
            at_most(named!(ui.toast_bytes), queued),
            at_most(request, client),
            at_most(events, client),
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
        ]
        .into_iter()
        .flatten()
        .next()
    }
}
