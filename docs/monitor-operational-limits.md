# Monitor operational limits

**Status: agreed future specification, not implemented behavior, except that
the monitor and `exec` validate the `monitor.limits` configuration and the
monitor reports its effective values
([#82](https://github.com/jackgeek/climon2/issues/82)). That validation does
not yet check that the mandatory current model and recent page fit
([#167](https://github.com/jackgeek/climon2/issues/167)). Operating-system
limits are left to admission
([#120](https://github.com/jackgeek/climon2/issues/120)). The client library
rejects native instance limits it cannot honour, bounds worker cleanup by
`cleanup_ms` and keeps its failure diagnostic within `event_bytes`
([#85](https://github.com/jackgeek/climon2/issues/85)). For passive monitor
connections it also enforces its request and event queues, outstanding
operations and their outcome reserves, deliveries, drain ceilings,
`wire_frame_bytes`, `metadata_record_bytes`, `title_bytes`, `start_ms` and
`leave_ms` ([#93](https://github.com/jackgeek/climon2/issues/93)); nothing
else enforces the limits yet.** This resolves
[Define monitor operational limits](https://github.com/jackgeek/climon2/issues/69).
It supplies requirements and evidence, not executable code or a benchmark suite.

The [monitor](../GLOSSARY.md#sessions) eagerly schedules a
[client](../GLOSSARY.md#sessions) for every discovered
[live session](../GLOSSARY.md#sessions). Finite resources cannot guarantee
unlimited simultaneous clients. Exhaustion must be visible, bounded and fair;
it never authorizes silent omission, client eviction or fabricated success.

Read the [glossary](../GLOSSARY.md), [protocol overview](protocol/README.md),
[embedding contract](protocol/library-embedding.md),
[daemon wire](protocol/daemon-client/core-v1.md),
[terminal integration](protocol/terminal-integration.md),
[status mechanics](session-status.md), [selector](monitor-selector.md), and
[lifecycle](monitor-lifecycle.md) together. Complete native declarations are
specified in [native revision 1](protocol/monitor-client/core-v1.md);
[final acceptance](https://github.com/jackgeek/climon2/issues/62) incorporates
the implementation obligations below.

## Configuration and communication of limits

Operational settings live in the existing
[climon2-home](../GLOSSARY.md#installation) [JSONC](protocol/TERMS.md#jsonc)
configuration, under `monitor.limits`. Read and validate one snapshot before
changing the [host](../GLOSSARY.md#sessions) terminal. No live reload is implied.
Tables below name fields relative to `monitor.limits`; omitted fields use their
listed defaults. Byte fields use [bytes](protocol/TERMS.md#byte), duration fields
milliseconds, other counts integers, and jitter integer percentages.
[MiB](protocol/TERMS.md#mib) in explanatory tables is not a configuration suffix.

Null, wrong types, duplicate known fields, overflow and inconsistent settings
are explicit startup errors. Unknown/unrelated fields retain the existing
ignore rule. Convert integers to their native types with checked bounds.
Byte/count budgets are positive; `resources.max_workers_per_client` may be zero.
History rows range from 200 through 100,000, duration fields from 1 through
3,600,000 ms, [frame](protocol/TERMS.md#frame) ceilings through positive `u32`,
and render ceilings from
1 through 120 fps. Processing quanta range from 1 through 50 ms, with the
per-client quantum no larger than the aggregate quantum. Retry initial delay
must not exceed its cap; jitter ranges from 0 through 50%.

Validate sub-budgets against the total, reserves against their containing queues,
chunk/record maxima against usable capacity, and the mandatory current model and
recent-page allocation requirements against history/client/aggregate limits.
There must be capacity for both ordinary work and the progress reserve.
Counts must fit the interface and applicable operating-system limits.
Changing a setting never makes an allocation-aware engine unnecessary.
These are ceilings, not instructions to preallocate every client's maximum.

Report effective configured values, qualified reservations and exhaustion
diagnostics explicitly. Classification and peer visibility are:

| Class | Examples | How applicable values are learned |
|---|---|---|
| Fixed existing contract | Hello deadline; discovery cadence; activity timing; [counter](protocol/TERMS.md#counter) ranges; supported session geometry | Published frozen/agreed contracts; configuration cannot override them. |
| Configurable local ceiling on daemon wire | Request/receive frame ceiling, text/parser budgets | Read the local snapshot. Honor the daemon's advertised request/server frame limits in hello; do not add unagreed hello fields. |
| Configurable native instance contract | Queue bytes/records, payloads, outstanding operations/deliveries | Monitor supplies validated limits at creation; the library reports accepted/effective limits before ordinary work. [Revision 1](protocol/monitor-client/core-v1.md#startup-limits-and-resources) assigns declarations, qualified resources and failure codes. |
| Configurable monitor-local budget | Aggregate memory, local history, rendering, discovery, admission, retry and deadlines | Startup configuration and explicit local diagnostics, not daemon negotiation. |
| Qualified resource requirement | Stacks, workers, descriptors, shared release resources, engine allocations | Release/engine qualification plus measured requirements; reserve before admitting work. |

A library must reject unsupported limits explicitly before opening an instance.
Do not silently raise caller limits or apply unknown release-specific defaults.
The native interface is tagged records, not serialized
[frames](protocol/TERMS.md#frame); it gets no invented frame limit or
[JSON](protocol/TERMS.md#json) parser.
Wire frame lengths count type plus body, excluding the four-byte prefix.
Outgoing frames fit both the local ceiling and the daemon's advertised maximum.
If the daemon's permitted server frames cannot fit the client's receive budget,
report an explicit resource/compatibility failure before ordinary Open rather
than promising an unsupported healthy stream. Smaller transfer chunks do not
change the daemon's advertisement.

An illustrative override is:

```json
{
  "monitor": {
    "limits": {
      "memory": {
        "total_bytes": 536870912,
        "client_bytes": 67108864,
        "history_bytes_per_session": 16777216,
        "history_bytes_total": 268435456,
        "queue_bytes_total": 67108864
      },
      "history": { "max_rows": 10000 },
      "retry": {
        "initial_ms": 1000,
        "cap_ms": 30000,
        "jitter_percent": 20
      }
    }
  }
}
```

The monitor and `exec` validate this configuration, but no limit is enforced
yet; the status note names the checks still to come. It does not modify daemon
`scrollback.max_rows`.

## Memory, history and admission

| Field | Default |
|---|---:|
| `memory.total_bytes` | 536870912 (512 MiB) |
| `memory.client_bytes` | 67108864 (64 MiB per client/session, including its monitor model) |
| `memory.history_bytes_per_session` | 16777216 (16 MiB) |
| `memory.history_bytes_total` | 268435456 (256 MiB) |
| `memory.queue_bytes_total` | 67108864 (64 MiB) |
| `history.max_rows` | 10000 |
| `resources.stack_bytes` | 2097152 (2 MiB per owner/worker thread) |
| `resources.max_workers_per_client` | 2 |
| `resources.descriptors_per_client` | 8 |
| `resources.descriptors_per_load` | 4 |
| `resources.descriptor_headroom` | 64 |
| `resources.release_bytes` | 67108864 (64 MiB per newly opened release) |

Count owned allocation capacity, not only logical length: current/alternate
screens, local history, auxiliary cell data, parser/assembly state, retained
metadata and identities, requests, events, immutable deliveries,
[restoration](../GLOSSARY.md#restoration-and-presentation) staging and
renderer/search/toast state. Ownership transfer moves accounting;
it does not duplicate it or let borrowed objects escape it. Copying does need
capacity for both copies until the old allocation is released.
All sub-budgets are included in the total, not added on top.

Reserve thread/worker, descriptor and release requirements before admission,
using the greater of configured reservation and qualified actual requirements.
Reservations also consume aggregate capacity. Shared release resources count
once; per-instance state counts separately. Successfully opened library handles,
including later-rejected ones, and their reservations remain until process exit.
A release without bounded qualified requirements is explicitly unsupported or
resource-blocked; it is not assumed to use the default reservation.
Qualified requirements are measured and recorded per platform, because a
release's macOS and Linux libraries can differ, for example in notification
descriptors.
Qualified worker counts above the configured maximum require explicit rejection,
not silently raising that ceiling. Greater reservations apply to actual
stack/descriptor/shared-memory requirements, not permission to exceed a limit.
Operating-system descriptor/thread limits and headroom independently constrain
admission. Account partially started instances until cleanup is complete.

The total is an enforceable tracked-allocation/reservation budget, **not a hard
whole-process resident-memory guarantee**. Report total process memory separately,
including mapping residency, native allocations, allocator overhead and stack
behavior. Trusted native initializers can crash or exceed a reservation; they
are not sandboxed by configuration. Measurements must expose that limitation.

Local history has both row and allocation ceilings. Discard oldest history
truthfully as it exceeds either ceiling, not required live terminal updates.
Under aggregate pressure, shorten unselected disconnected/final histories first,
least recently inspected first. The inspected history is shortened only after
other eligible caches. Keep the newest page of already-captured normal-screen
history: a page is the session's last accepted terminal height in rows, or fewer
rows if fewer exist. Preserve current normal/alternate-screen state separately.
Never completely erase that page merely to admit another session.
No history is fabricated for a passive client that never received display data.

The protected page/current model become admission obligations. Shortening history
does not remove a row, release
[control](../GLOSSARY.md#attachment-and-control), or turn partial restoration into
[readiness](../GLOSSARY.md#protocol-and-plugin-concepts).
Label truncation and stale/error state accurately. An oversized update must fail
safely before exceeding allocation bounds, preserving the last safely captured
page rather than silently changing characters or checking only after allocation.
If existing mandatory state cannot fit, explicitly fail the affected client and
clean it up; if shared accounting/retention cannot remain safe, coordinate
monitor failure. Neither path permits discarding the protected cache.

Every discovered live identity gets a visible row and a client-start obligation.
Insufficient admission resources leave it **resource-blocked, awaiting client**,
with an actionable diagnostic and automatic fair retry. Existing clients remain
usable. Resource-blocked is local availability information, not a new daemon
activity/attention status. No arbitrary hidden session cap, silent omission or live-client eviction
is allowed. If all required identities/metadata cannot be retained, report
incomplete discovery and coordinate shutdown; do not present a complete list.
Explicit selector Delete remains the only run-local dismissal route.

### Allocation-aware terminal prerequisite

[ADR 0007](adr/0007-daemon-owned-virtual-terminal.md) documents an unchanged
engine with unbounded combining marks per cell and an infallible current VT
interface. Row limits alone cannot establish a memory bound.
This decision explicitly reopens those narrow assumptions for future monitor
conformance. Qualification must provide bounded, allocation-aware parser/model
updates, including auxiliary data, resize and transactional restoration copies.
Checking size afterward, truncating accepted characters or relying on fewer
processes is not sufficient. No engine patch, replacement or daemon behavior
change is delivered here.

#76 adds `climon2-vt`'s monitor `Model` on the unchanged engine. It charges
each token's worst-case growth, from vte's own parse of the token, before
the engine sees it. It admits history storage in the engine's chunks,
stages restorations beside the current model and never reflows in place
([ADR 0007](adr/0007-daemon-owned-virtual-terminal.md#monitor-models-bounded-by-allocation-on-the-unchanged-engine)).
Before it rejects live output or a restoration, its oldest history yields a
chunk at a time, never the chunk holding the newest page, and
`Model::shorten_history()` lets the monitor shorten history the same way.
Its tests show no stream passing a model's limit, including adversarial
ones; the owner accepted that evidence, with the measured reserve, as the
risk proof. The owner also decided that a monitor-mode geometry change is
always followed by a restoration at that geometry (a wire amendment that
[#89](https://github.com/jackgeek/climon2/issues/89) makes), and how
per-model cost is to meet PERF-01
([#166](https://github.com/jackgeek/climon2/issues/166)). #83's read-only
history view labels truncation: rows beyond the row ceiling or released under
pressure, and, as the owner decided, a full history that output could have
overrun once growth was capped or refused, because the engine does not report
the rows it discards there
([ADR 0007](adr/0007-daemon-owned-virtual-terminal.md#read-only-views-for-painting)).
The monitor's own budgets and pressure policies above remain unimplemented.

## Queues and transfers

| Field | Default per client |
|---|---:|
| `queues.request_bytes` | 262144 |
| `queues.request_records` | 64 |
| `queues.event_bytes` | 1048576, jointly for queued events and outstanding deliveries |
| `queues.event_records` | 1024, including outstanding deliveries |
| `queues.progress_bytes` | 65536, reserved inside the event allowance |
| `queues.progress_records` | 32, reserved inside the event allowance |
| `queues.deliveries` | 8 |
| `queues.operations` | 16 ordinary accepted operations |
| `transfer.chunk_bytes` | 65536 display/input payload bytes |
| `transfer.drain_records` | 64 |
| `transfer.drain_payload_bytes` | 65536 per immutable delivery |

Record headers, auxiliary allocations and retained capacity count against byte
allowances; transfer payload maxima do not waive overhead. A drain stops before
either record or payload ceiling, never splits a metadata record, and never
splits a display/input chunk inside a required encoding boundary.
The aggregate queue budget includes UI queues and all client deliveries; per-client
limits do not imply unconditional aggregate capacity.

Reserve space for mandatory outcomes/control/status/restoration-boundary progress
before accepting ordinary work. The reserve is not an alternate stream and
cannot overtake earlier terminal bytes. Enforce both bytes and records; releasing
neither a delivery nor its payload cannot be hidden by repeated drains.
Independent shutdown control remains callable even when ordinary queues are full.

Full/malformed/unsupported submissions are synchronous explicit rejections and
retain no caller ownership; they create no later outcome. Locally rejected
[request IDs](protocol/TERMS.md#request-id) are not consumed. Accepted requests receive exactly one response
while healthy; response loss/failure makes unfinished work explicitly
[indeterminate](protocol/TERMS.md#indeterminate-outcome), never replayable by
assumption. Maintain at most one pending
[Kill](../GLOSSARY.md#attachment-and-control) per immutable session
[GUID](protocol/TERMS.md#guid).

[Backpressure](protocol/TERMS.md#backpressure) affects only the congested
connection while others, selector input and exit progress. No required title,
status/episode transition, outcome, control boundary, restoration boundary or
terminal bytes may silently disappear or be lossily coalesced. Congestion beyond
the progress deadline explicitly fails the affected connection.
Discarding obsolete paints is different from discarding required records.

Do not add an unbounded pending-input queue. Visibly reject unaccepted input when
submit is full, distinguishing already-accepted earlier chunks. Continue global
prefix/selector/exit parsing. Selection/control/restoration changes never
redirect stale buffered input to another client or make it automatically replay.
[Selected-only input/resize](protocol/terminal-integration.md) and restoration/
paint readiness still apply.

## Payloads and parser state

| Field | Default |
|---|---:|
| `payload.wire_frame_bytes` | 1048576, local request/receive ceiling |
| `payload.metadata_record_bytes` | 16384, including variable fields |
| `payload.title_bytes` | 4096 [UTF-8](protocol/TERMS.md#unicode-and-utf-8) bytes |
| `payload.notification_title_bytes` | 4096 UTF-8 bytes |
| `payload.notification_body_bytes` | 8192 UTF-8 bytes |
| `payload.assembly_records` | 32 incomplete notification assemblies |
| `payload.assembly_bytes` | 65536 total incomplete assembly allocation |
| `deadlines.assembly_ms` | 5000 from assembly creation |

Validate lengths before copying or allocating, including nested assembly state
and library-owned deliveries. [UTF-8](protocol/TERMS.md#unicode-and-utf-8) limits
count bytes, not characters; binary application/display bytes stay exact.
The existing 4,096-byte terminal-token body bound remains distinct from transfer
chunks and structured/multipart notification limits. No new raw terminal
sequence support is implied.

Oversized required metadata cannot become a truncated healthy title/status:
explicitly reject/fail the affected stream and label cached metadata uncertain.
Notification assembly overflow/expiry reports a diagnostic and no fabricated
completed notification. Individual fragments must satisfy existing token limits;
the completed notification must also fit its text and metadata-record ceilings.
Charge serialized daemon parsing allocations to the client budget; this does not
prescribe a serialized monitor-client parser.

## Discovery, loading and start scheduling

| Field | Default |
|---|---:|
| `scheduling.probes` | 8 |
| `scheduling.loads` | 2 release loads |
| `scheduling.starts` | 8 client starts |
| `scheduling.newer_start_burst` | 8 |
| `discovery.pass_ms` | 30000 |
| `discovery.stall_ms` | 5000 |
| `discovery.batch_records` | 128 |
| `discovery.batch_bytes` | 1048576 |
| `discovery.metadata_file_bytes` | 262144 |
| `discovery.metadata_depth` | 32 |

Preserve the agreed immediate first pass, next pass one second after the preceding
pass finishes, and non-overlapping passes, including selector refresh.
Every hello probe keeps its frozen two-second deadline.
Enumeration, file parsing, probes and result collection are incremental and
off the UI thread; no whole-directory unbounded temporary collection is allowed.
File/depth/parse failures produce per-entry diagnostics, never metadata deletion.

A pass deadline or lack of actual scan/probe progress reports incomplete
discovery, retaining known rows and structured uncertainty. Unchecked metadata,
disappearance or failed probes do not prove command completion or override an
established client. Continue with bounded scan/probe scheduling state next pass,
fair to older entries even while the directory changes. Progress/no-progress
reporting must distinguish a completed empty pass from an incomplete one.

Use separate bounded load/start lanes. Share one load attempt per release;
sessions waiting on it do not each consume a running-start slot.
First attempts start newest-first; after eight newer starts, give the oldest
waiting eligible candidate a turn. Retries share bounded capacity and must not
monopolize first starts. Memory/descriptor/thread admission precedes execution.
Continuous arrivals and one newer stalled release must not indefinitely prevent
older eligible clients of already loaded releases from starting. Progress and
cleanup deadlines also apply to discovery workers. Load workers have none: the
system loader serializes loading and a load cannot be cancelled, so a load that
never returns is a native fault, like a crash, and can block later loads
([library embedding](protocol/library-embedding.md#loading-and-instance-lifetime)).
Initial selection remains pinned under the status/lifecycle rules, independently
of scheduling fairness; failure never silently selects/acquires another session.

## Deadlines and progress

| Field | Default |
|---|---:|
| `deadlines.load_ms` | 10000 total load/validation |
| `deadlines.start_ms` | 10000 through passive initial metadata synchronization |
| `deadlines.operation_ms` | 5000 per ordinary accepted operation |
| `deadlines.kill_ms` | 10000 for Kill outcome |
| `deadlines.restoration_stall_ms` | 5000 without transfer/application progress |
| `deadlines.restoration_total_ms` | 60000 |
| `deadlines.delivery_stall_ms` | 5000 without drain/release progress |
| `deadlines.leave_ms` | 5000 for required Leave response |
| `deadlines.cleanup_ms` | 10000 for local instance cleanup/join |
| `deadlines.shutdown_ms` | 10000 for coordinated monitor shutdown |
| `deadlines.final_host_ms` | 1000 additional best-effort final restoration/diagnostics |

Use a [monotonic clock](protocol/TERMS.md#monotonic-clock).
Total deadlines start when the corresponding work starts/is locally accepted;
waiting resource-blocked rows consume bounded scheduling state, not infinite
active-start slots. A phase transition starts only its own applicable timer.
Unrelated title/heartbeat traffic and repeated signals do not reset deadlines.
Restoration progress means bytes successfully transferred/applied toward that
restoration; delivery progress means actual draining/releasing that relieves
congestion. Assembly has a fixed lifetime, not a fragment-reset timer.
Silence on a healthy connected session has no generic output timeout.

Timeout distinguishes known rejection/unexecuted work from indeterminate work.
Kill's deadline does not replace the daemon's five-second hang-up grace or prove
termination/cleanup. A Leave response proves only daemon acceptance, not connection
closure, workers, delivery release or join.

Shutdown is concurrent for passive, attached, partial and failing instances.
One global shutdown deadline bounds local phase timers; repeated exit/signals
never restart it. Keep draining/releasing while cleanup is possible.
If cleanup cannot be proven by the applicable deadline, attempt host restoration,
root attention clear and diagnostics within the final host allowance, then
fail-stop the entire monitor with the climon2 error code. Never kill an individual
thread, free a still-borrowed delivery, unload running code, or claim false detach/
join. A monitor-owned delivery leak/shared allocator failure is monitor-fatal;
client-local failure stays local only with provably safe cleanup.
Native abort/fault and uncatchable signals retain their lack of cleanup guarantees.
An ordinary non-headless session may end under existing leave policy on client
loss; this specification adds no universal session-survival promise.

## Passive retry

| Field | Default |
|---|---:|
| `retry.initial_ms` | 1000 |
| `retry.cap_ms` | 30000 |
| `retry.jitter_percent` | 20 |
| `retry.admission_ms` | 1000 |

For recoverable loss/startup/panic failure, double the base delay from initial to
cap, using overflow-safe saturation. Choose fresh uniform jitter from minus to
plus the percentage for each delay, then clamp the actual delay to the cap.
There is no lifetime attempt-count cutoff while recovery remains valid.
Keep at most one scheduled/active attempt per session and one shared load attempt
per release; retry records themselves consume memory.
Capacity release can wake admission; repeated reconsideration is rate-limited to
once per admission interval, not a busy loop.

Wait for old deliveries/handles, workers and owner-thread cleanup/join before a
replacement. Permanent protocol/interface rejection, invalid configuration,
definitive final outcome and shutdown stop automatic attempts.
Recovery is passive: it refreshes metadata and reconciles state, never
automatically takes control or replays input, Kill or other indeterminate work.
Cleanup that cannot finish follows fail-stop, not another retry.
Daemon-initiated passive succession is still valid, not a replayed takeover.

## UI, sampling and responsiveness

| Field | Default |
|---|---:|
| `ui.fps` | 30 |
| `ui.client_quantum_ms` | 2 |
| `ui.background_quantum_ms` | 8 per UI turn |
| `ui.toast_records` | 128 queued episodes |
| `ui.toast_bytes` | 524288 |
| `ui.search_bytes` | 4096 UTF-8 bytes |

Rotate bounded client/background work fairly. Bound each indivisible
parse/copy/render step too: checking a timer after an unbounded call cannot
establish a quantum. Supported-load evidence targets no more than 100 ms p99
application-input dispatch, selector/global-action response and attention
presentation latency; a measured stall of 250 ms or more requires diagnosis.
Exit initiation is responsive independently of eventual cleanup.
These are measurement targets under declared configuration/hardware, not
hard real-time operating-system guarantees. Nondefault scheduling configurations
must disclose their effect on these targets.

Toast state has no unbounded secondary queue. Distinct accepted episodes remain
ordered; congestion past the delivery deadline explicitly fails the affected
notification stream/connection and reports undelivered notifications.
The selector's accepted visual timings are 400 ms border drawing, 2000 ms title
hold, 300 ms text fade and 400 ms erasure. Use elapsed time and skip obsolete
animation paints; never accumulate frames or block input/exit. Over-limit search/
paste text is rejected visibly, without leaking to an application.

Snapshot accounting permits one retained viewport character baseline and one
candidate comparison snapshot per session where the daemon detector uses them;
account title and auxiliary allocation too. Bound both against the qualified
per-session detector/model requirements. Daemon-owned snapshots consume daemon
memory, reported separately; the monitor's aggregate budget does not cap another
process. Do not copy a viewport per output
byte or block daemon connection handling during comparison.
Preserve hardcoded 10,000 ms inactivity, fresh random 500-1500 ms sampling clamped
to the remaining running deadline, and all other agreed detector timing.
Budgeting does not invent plugin configuration or alter the detector machines.

## Counters and explicit results

Preserve wire [counter](protocol/TERMS.md#counter) values 1 through
18,446,744,073,709,551,615, without wrap/reuse.
Native request IDs strictly increase over the instance lifetime;
[revision 1](protocol/monitor-client/core-v1.md#shared-values) assigns matching
`uint64_t` representations and fresh-attempt generation scopes.
A rejected local submission consumes no ID.
Before effects, reject work that requires an exhausted new counter.

Retire/replace the relevant correlation connection/instance only after safe
cleanup. New connection/instance domains cannot reset session-wide control,
restoration, attention or other daemon histories. If a session-wide domain cannot
advance, report `resource_limit` and stop affected new work, not restart the
command or invent an identifier. An exhausted delivery stream becomes explicitly
unavailable; protected cache and authoritative known outcomes survive.

Use existing wire operational/fatal codes as appropriate: `resource_limit` for
exhausted resources, `invalid_argument` for invalid arguments, `frame_too_large`
for oversized frames, `io_error` for transport failure and `cleanup_failed` for
failed daemon cleanup. Fatal error delivery on broken transport is best effort.
[Native errors](protocol/monitor-client/core-v1.md#errors-terminal-evidence-and-cleanup)
assign distinct local timeout, queue-full, unsupported-limit, cleanup and
indeterminate results, never success-shaped fallbacks.
Report affected GUID/release, exhausted budget, effective values and the evidence
known: rejection, acceptance, daemon response, closure, final outcome, delivery/
handle release, worker completion and thread join. Do not log session content.

## Required implementation evidence

Automated boundary tests must exercise exactly at/one over byte, record, delivery,
operation, frame, text, assembly, search, toast and allocation limits. Include
UTF-8 boundaries, zero/overflow/invalid configuration, reserved-capacity behavior,
counter exhaustion, draining without release, allocation capacity versus length,
ownership-transfer/copy peaks and safely rejected unsupported release limits.
No trivial tests of constants substitute for observable behavior.

Test protected one-page retention at differing geometries and with less/no received
history; current/alternate state, combining/auxiliary growth, resize, authoritative
restoration replacement and staging peaks; selected-cache shortening only after
other eligible histories; explicit truncation/staleness without fabricated content.
Prove rejection before unsafe allocation, not an after-the-fact RSS observation.

Test resource-blocked visible rows, no silent omission/client eviction, identity
exhaustion failure, operating-system resource failures, mixed-release reservations,
partially started ownership, continuous discovery and fair continuation.
Stall newer starts/releases while older clients, selector and exit progress.
Exercise incomplete/empty discovery, oversized metadata, non-overlapping cadence,
bounded probes, deadline boundaries and no silent staleness cleanup.

Test full queues and missing release, required title/status traffic independently
of control, ordered restoration boundaries, readiness/paint/input/resize gates,
toast backlog, indeterminate work and no replay. Test passive retry saturation/
jitter/cap, no overlapping instances, permanent stopping and capacity-triggered
admission without busy looping. Preserve the detector's fixed activity timing.

Test accepted Leave versus closure/final outcome/worker cleanup/join separately;
lost responses, pending Kill, concurrent shutdown, repeated signals, worker/
loader/discovery stalls and monitor-held delivery leaks. Verify bounded
whole-process failure and truthful host-restoration/exit results, including native
crash exposure and ordinary leave policy. Incomplete cleanup is never successful
detach; protected history does not authorize unloading running code.

Require reproducible CPU/memory, startup/load and sustained-display measurements
for 1, 10 and 100 live sessions, one release and ten mixed releases where
applicable, at 80x24 and 200x60, plus 500x200 boundary geometry.
Cover passive idle, controlling background output, cold/warm loading, restoration/
switching, selected sustained display, reconnect storms, continuous arrivals and
stalled newer starts. Measure snapshot comparison/allocation and retained
per-session/aggregate history, queues, threads and shared libraries.
Declare OS/hardware, configuration, workload/rate/duration, admission failures,
CPU, total process memory and attributable components, startup/readiness latency,
input/selector/exit latency distribution and backlogs.

Compare standalone clients doing equivalent work. A session count is an evidence
point, not an admission cap or arbitrary-hardware promise. If defaults cannot
admit a scenario, report that outcome and repeat with disclosed sufficient
configuration where supported. Do not infer lower total use merely from fewer
processes. Acceptance records limitations honestly; this specification claims
none of this evidence has already been delivered.
