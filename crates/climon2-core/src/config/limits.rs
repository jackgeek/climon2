//! `monitor.limits` (docs/monitor-operational-limits.md): eleven groups of
//! budgets, ceilings, counts and durations. Each setting is range-checked as it
//! is read, in its native type, and the groups are then checked against each
//! other once. `Limits` exists only after both checks, and nothing outside
//! this module can change it; its values are only reported so far.
use super::object;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{Error as _, Expected, Unexpected},
};
use std::fmt;

/// An integer setting from `MIN` through `MAX`, held in its native type `T`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
struct Limit<T, const MIN: u64, const MAX: u64>(T);

/// A positive budget or ceiling, held in a `u64`.
type Positive64 = Limit<u64, 1, { u64::MAX }>;
/// A positive count or single-item byte ceiling, held in a `u32`.
type Positive32 = Limit<u32, 1, { u32::MAX as u64 }>;
/// A worker count, which may be zero, held in a `u32`.
type Workers = Limit<u32, 0, { u32::MAX as u64 }>;
/// Milliseconds, 1 through 3,600,000.
type Millis = Limit<u32, 1, 3_600_000>;
/// A processing quantum, 1 through 50 milliseconds.
type Quantum = Limit<u32, 1, 50>;
/// A render ceiling, 1 through 120 frames per second.
type Fps = Limit<u32, 1, 120>;
/// A jitter percentage, 0 through 50.
type Percent = Limit<u32, 0, 50>;
/// Local history rows, 200 through 100,000.
type HistoryRows = Limit<u32, 200, 100_000>;

/// Reading `T` first rejects null, other types and integers that overflow it.
impl<'de, T, const MIN: u64, const MAX: u64> Deserialize<'de> for Limit<T, MIN, MAX>
where
    T: Deserialize<'de> + Copy + Into<u64>,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = T::deserialize(deserializer)?;
        let wide = value.into();
        if (MIN..=MAX).contains(&wide) {
            return Ok(Self(value));
        }
        Err(D::Error::invalid_value(
            Unexpected::Unsigned(wide),
            &Range::<MIN, MAX>,
        ))
    }
}

/// What a setting accepts, for its error.
struct Range<const MIN: u64, const MAX: u64>;

impl<const MIN: u64, const MAX: u64> Expected for Range<MIN, MAX> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "an integer from {MIN} to {MAX}")
    }
}

/// Tracked allocation budgets; every sub-budget is part of the total.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Memory {
    total_bytes: Positive64,
    client_bytes: Positive64,
    history_bytes_per_session: Positive64,
    history_bytes_total: Positive64,
    queue_bytes_total: Positive64,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            total_bytes: Limit(536_870_912),
            client_bytes: Limit(67_108_864),
            history_bytes_per_session: Limit(16_777_216),
            history_bytes_total: Limit(268_435_456),
            queue_bytes_total: Limit(67_108_864),
        }
    }
}

/// The monitor's local history; never the daemon's `scrollback.max_rows`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct History {
    max_rows: HistoryRows,
}

impl Default for History {
    fn default() -> Self {
        Self {
            max_rows: Limit(10_000),
        }
    }
}

/// Reservations for threads, descriptors and loaded releases.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Resources {
    stack_bytes: Positive64,
    max_workers_per_client: Workers,
    descriptors_per_client: Positive32,
    descriptors_per_load: Positive32,
    descriptor_headroom: Positive32,
    release_bytes: Positive64,
}

impl Default for Resources {
    fn default() -> Self {
        Self {
            stack_bytes: Limit(2_097_152),
            max_workers_per_client: Limit(2),
            descriptors_per_client: Limit(8),
            descriptors_per_load: Limit(4),
            descriptor_headroom: Limit(64),
            release_bytes: Limit(67_108_864),
        }
    }
}

/// Per-client queues; each progress reserve sits inside its event allowance.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Queues {
    request_bytes: Positive64,
    request_records: Positive32,
    event_bytes: Positive64,
    event_records: Positive32,
    progress_bytes: Positive64,
    progress_records: Positive32,
    deliveries: Positive32,
    operations: Positive32,
}

impl Default for Queues {
    fn default() -> Self {
        Self {
            request_bytes: Limit(262_144),
            request_records: Limit(64),
            event_bytes: Limit(1_048_576),
            event_records: Limit(1024),
            progress_bytes: Limit(65_536),
            progress_records: Limit(32),
            deliveries: Limit(8),
            operations: Limit(16),
        }
    }
}

/// Display and input chunks, and what one delivery drains.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Transfer {
    chunk_bytes: Positive32,
    drain_records: Positive32,
    drain_payload_bytes: Positive64,
}

impl Default for Transfer {
    fn default() -> Self {
        Self {
            chunk_bytes: Limit(65_536),
            drain_records: Limit(64),
            drain_payload_bytes: Limit(65_536),
        }
    }
}

/// Frame, record, text and notification-assembly ceilings.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Payload {
    wire_frame_bytes: Positive32,
    metadata_record_bytes: Positive32,
    title_bytes: Positive32,
    notification_title_bytes: Positive32,
    notification_body_bytes: Positive32,
    assembly_records: Positive32,
    assembly_bytes: Positive64,
}

impl Default for Payload {
    fn default() -> Self {
        Self {
            wire_frame_bytes: Limit(1_048_576),
            metadata_record_bytes: Limit(16_384),
            title_bytes: Limit(4096),
            notification_title_bytes: Limit(4096),
            notification_body_bytes: Limit(8192),
            assembly_records: Limit(32),
            assembly_bytes: Limit(65_536),
        }
    }
}

/// Concurrent probes, release loads and client starts.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Scheduling {
    probes: Positive32,
    loads: Positive32,
    starts: Positive32,
    newer_start_burst: Positive32,
}

impl Default for Scheduling {
    fn default() -> Self {
        Self {
            probes: Limit(8),
            loads: Limit(2),
            starts: Limit(8),
            newer_start_burst: Limit(8),
        }
    }
}

/// Discovery passes and the metadata they read.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Discovery {
    pass_ms: Millis,
    stall_ms: Millis,
    batch_records: Positive32,
    batch_bytes: Positive64,
    metadata_file_bytes: Positive64,
    metadata_depth: Positive32,
}

impl Default for Discovery {
    fn default() -> Self {
        Self {
            pass_ms: Limit(30_000),
            stall_ms: Limit(5000),
            batch_records: Limit(128),
            batch_bytes: Limit(1_048_576),
            metadata_file_bytes: Limit(262_144),
            metadata_depth: Limit(32),
        }
    }
}

/// Progress deadlines.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Deadlines {
    load_ms: Millis,
    start_ms: Millis,
    operation_ms: Millis,
    kill_ms: Millis,
    restoration_stall_ms: Millis,
    restoration_total_ms: Millis,
    delivery_stall_ms: Millis,
    leave_ms: Millis,
    cleanup_ms: Millis,
    shutdown_ms: Millis,
    final_host_ms: Millis,
    assembly_ms: Millis,
}

impl Default for Deadlines {
    fn default() -> Self {
        Self {
            load_ms: Limit(10_000),
            start_ms: Limit(10_000),
            operation_ms: Limit(5000),
            kill_ms: Limit(10_000),
            restoration_stall_ms: Limit(5000),
            restoration_total_ms: Limit(60_000),
            delivery_stall_ms: Limit(5000),
            leave_ms: Limit(5000),
            cleanup_ms: Limit(10_000),
            shutdown_ms: Limit(10_000),
            final_host_ms: Limit(1000),
            assembly_ms: Limit(5000),
        }
    }
}

/// Passive retry backoff.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Retry {
    initial_ms: Millis,
    cap_ms: Millis,
    jitter_percent: Percent,
    admission_ms: Millis,
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            initial_ms: Limit(1000),
            cap_ms: Limit(30_000),
            jitter_percent: Limit(20),
            admission_ms: Limit(1000),
        }
    }
}

/// Rendering, work quanta, toasts and search.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Ui {
    fps: Fps,
    client_quantum_ms: Quantum,
    background_quantum_ms: Quantum,
    toast_records: Positive32,
    toast_bytes: Positive64,
    search_bytes: Positive64,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            fps: Limit(30),
            client_quantum_ms: Limit(2),
            background_quantum_ms: Limit(8),
            toast_records: Limit(128),
            toast_bytes: Limit(524_288),
            search_bytes: Limit(4096),
        }
    }
}

/// Every group as read, before the groups are checked against each other.
#[derive(Default, Deserialize)]
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

/// `monitor.limits`, consistent across its groups.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "Groups")]
pub struct Limits {
    memory: Memory,
    history: History,
    resources: Resources,
    queues: Queues,
    transfer: Transfer,
    payload: Payload,
    scheduling: Scheduling,
    discovery: Discovery,
    deadlines: Deadlines,
    retry: Retry,
    ui: Ui,
}

impl TryFrom<Groups> for Limits {
    type Error = String;

    fn try_from(groups: Groups) -> Result<Self, String> {
        let limits = Self {
            memory: groups.memory,
            history: groups.history,
            resources: groups.resources,
            queues: groups.queues,
            transfer: groups.transfer,
            payload: groups.payload,
            scheduling: groups.scheduling,
            discovery: groups.discovery,
            deadlines: groups.deadlines,
            retry: groups.retry,
            ui: groups.ui,
        };
        limits.violation().map_or(Ok(limits), Err)
    }
}

/// A setting's full name and its value, for a rule between settings.
#[derive(Clone, Copy)]
struct Named(&'static str, u64);

fn named<T: Into<u64>, const MIN: u64, const MAX: u64>(
    name: &'static str,
    limit: Limit<T, MIN, MAX>,
) -> Named {
    Named(name, limit.0.into())
}

fn at_most(lesser: Named, greater: Named) -> Option<String> {
    (lesser.1 > greater.1).then(|| {
        format!(
            "{} ({}) may not exceed {} ({})",
            lesser.0, lesser.1, greater.0, greater.1
        )
    })
}

fn below(lesser: Named, greater: Named) -> Option<String> {
    (lesser.1 >= greater.1).then(|| {
        format!(
            "{} ({}) must be less than {} ({})",
            lesser.0, lesser.1, greater.0, greater.1
        )
    })
}

impl Limits {
    /// The first rule these limits break, in this order: sub-budgets and
    /// reservations fit the totals that include them (each client's queues
    /// fit both the aggregate queue budget and the client's own); progress
    /// reserves leave room for ordinary work; the largest chunk fits one
    /// request, the ordinary event capacity and one delivery, and the largest
    /// metadata record fits the reserve, that capacity and one delivery; the
    /// per-client quantum fits the aggregate one; the initial retry delay fits
    /// its cap.
    fn violation(&self) -> Option<String> {
        let memory = &self.memory;
        let total = named("monitor.limits.memory.total_bytes", memory.total_bytes);
        let client = named("monitor.limits.memory.client_bytes", memory.client_bytes);
        let per_session = named(
            "monitor.limits.memory.history_bytes_per_session",
            memory.history_bytes_per_session,
        );
        let history = named(
            "monitor.limits.memory.history_bytes_total",
            memory.history_bytes_total,
        );
        let queued = named(
            "monitor.limits.memory.queue_bytes_total",
            memory.queue_bytes_total,
        );
        let queues = &self.queues;
        let request = named("monitor.limits.queues.request_bytes", queues.request_bytes);
        let events = named("monitor.limits.queues.event_bytes", queues.event_bytes);
        let reserve = named(
            "monitor.limits.queues.progress_bytes",
            queues.progress_bytes,
        );
        let ordinary = Named(
            "monitor.limits.queues.event_bytes - monitor.limits.queues.progress_bytes",
            events.1.saturating_sub(reserve.1),
        );
        let chunk = named(
            "monitor.limits.transfer.chunk_bytes",
            self.transfer.chunk_bytes,
        );
        let delivery = named(
            "monitor.limits.transfer.drain_payload_bytes",
            self.transfer.drain_payload_bytes,
        );
        let record = named(
            "monitor.limits.payload.metadata_record_bytes",
            self.payload.metadata_record_bytes,
        );
        let resources = &self.resources;
        [
            at_most(client, total),
            at_most(history, total),
            at_most(queued, total),
            at_most(
                named(
                    "monitor.limits.resources.stack_bytes",
                    resources.stack_bytes,
                ),
                total,
            ),
            at_most(
                named(
                    "monitor.limits.resources.release_bytes",
                    resources.release_bytes,
                ),
                total,
            ),
            at_most(per_session, history),
            at_most(per_session, client),
            at_most(request, queued),
            at_most(events, queued),
            at_most(request, client),
            at_most(events, client),
            below(reserve, events),
            below(
                named(
                    "monitor.limits.queues.progress_records",
                    queues.progress_records,
                ),
                named("monitor.limits.queues.event_records", queues.event_records),
            ),
            at_most(chunk, request),
            at_most(chunk, ordinary),
            at_most(chunk, delivery),
            at_most(record, reserve),
            at_most(record, ordinary),
            at_most(record, delivery),
            at_most(
                named(
                    "monitor.limits.ui.client_quantum_ms",
                    self.ui.client_quantum_ms,
                ),
                named(
                    "monitor.limits.ui.background_quantum_ms",
                    self.ui.background_quantum_ms,
                ),
            ),
            at_most(
                named("monitor.limits.retry.initial_ms", self.retry.initial_ms),
                named("monitor.limits.retry.cap_ms", self.retry.cap_ms),
            ),
        ]
        .into_iter()
        .flatten()
        .next()
    }
}
