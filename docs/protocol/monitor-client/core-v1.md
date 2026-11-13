# Native client interface revision 1

**Status: developer-confirmed specification for [#71](https://github.com/jackgeek/climon2/issues/71).
Implemented for the bootstrap and the immutable description
([#75](https://github.com/jackgeek/climon2/issues/75)), the instance
lifecycle calls ([#85](https://github.com/jackgeek/climon2/issues/85)) and
passive monitor connections ([#93](https://github.com/jackgeek/climon2/issues/93)).**
The shipped [client library](../../../GLOSSARY.md#sessions)
(`libclimon2_client.dylib` on macOS, `libclimon2_client.so` on Linux)
returns revision 1's description and rejects other revisions as specified
below. Its requirements are provisional bounded values
until [#121](https://github.com/jackgeek/climon2/issues/121) records qualified
ones. All eleven [function-table](../TERMS.md#function-table) calls follow
the rules below, with one gap: snapshot reports no shared-release allocation
and undercounts instance allocation, because it does not yet count each
instance's diagnostics dispatcher
([#180](https://github.com/jackgeek/climon2/issues/180)). `submit` accepts
monitor-mode `CM1_OPEN` and `CM1_LEAVE`; interactive Open
([#94](https://github.com/jackgeek/climon2/issues/94)) and every other request
report `CM1_UNSUPPORTED_OPERATION`. An instance opens a
[passive monitor connection](../../../GLOSSARY.md#attachment-and-control) to
its [session daemon](../../../GLOSSARY.md#sessions) over
[daemon wire revision 1](../daemon-client/core-v1.md), delivers Response,
Initial metadata, Title and diagnostic records, and attempts
[Leave](../daemon-client/request-leave.md) once on shutdown.
[Implemented behavior](#implemented-behavior) records the choices this page
leaves open. Interactive roles, [Kill](../daemon-client/request-kill.md) and
the status, notification, progress, control, display and final records are
not implemented. The [C ABI](../TERMS.md#c-abi)
[header](client-v1.h) is authoritative for declarations, numeric tags and field
order. This page owns their semantics. The [examples](examples.c) compile but
do not link to a shipped library. [Declaration checks](check.py) are not runtime
compatibility evidence.

Read the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
[technical terms](../TERMS.md), [embedding](../library-embedding.md),
[daemon wire](../daemon-client/core-v1.md), [terminal contract](../terminal-integration.md),
[lifecycle](../../monitor-lifecycle.md), and
[operational limits](../../monitor-operational-limits.md) together.

## Representation and compatibility

Revision 1 targets the supported platform C calling conventions (macOS arm64
and x86_64, and x86-64 GNU/Linux), with 64-bit pointers, natural alignment and
no packing. All calls use the platform C calling convention. No Rust layouts,
C enums, `bool`, `size_t`, variadic calls, callbacks or unwinding cross this
interface.
The header fixes every field and tag; the platform fixes padding/alignment.
Never serialize these structs or compare their padding.

The bootstrap symbol is `climon2_client_get_interface`. Its signature is frozen
independently of the negotiated table. `requested_revision=1` returns
`CM1_SUCCESS` and a nonnull immutable `cm1_description` supporting precisely 1.
Unsupported revisions return `CM1_UNSUPPORTED_REVISION` and null. Null output
storage returns `CM1_INVALID_ARGUMENT`; containable bootstrap failure returns
`CM1_INTERNAL_ERROR`. No other bootstrap results are permitted. Bootstrap opens
no [daemon](../../../GLOSSARY.md#sessions) connection and creates no instance.
Description, table, [capability](../../../GLOSSARY.md#sessions) tokens and release
text live until process exit; all eleven function pointers are nonnull.
`requirements` are bounded, qualified
resource requirements, not guesses based on a release number.

Validate requested revision and exact release before create; create also checks
`expected_release` against its own identity. A supported revision never permits
using a different release's library for a [session](../../../GLOSSARY.md#sessions).
One load is shared by same-release instances; distinct release libraries have
independent state and
allocator ownership. Retain every opened library until process exit, including
later-rejected handles. [Loader rules](../library-embedding.md#artifact-and-dependency-lookup)
prevent private dependency cross-binding.

Published revision-1 layouts and semantics are frozen. Incompatible changes,
including added operation/event variants, require a new revision. New releases
may continue supporting 1. Capability selection changes available optional
behavior, not struct layouts. Unknown core tags, invalid active fields and
nonzero named `reserved` fields reject explicitly. Validate only the active
union member, not padding or inactive storage. Zero-initialized records are
convenient construction, not permission to omit required active fields.
There is no generic extension tag or unknown-field skipping.

Future [plugin](../../../GLOSSARY.md#protocol-and-plugin-concepts) declarations,
name/revision negotiation and opaque relay require a separately specified
interface revision. Revision 1 publishes none; they are not prerequisites for
current core support. Unknown optional capability names cannot enable plugin
traffic, and an unknown core tag cannot be treated as plugin data.

## Shared values

Lengths and counted-list counts are `uint64_t`. Tags, presence/availability
values, count limits and durations are `uint32_t`; flag bytes are `uint8_t` with
only 0 and 1 valid. Nonnull output pointers designate writable caller storage.
Any nonempty buffer/list requires a nonnull readable pointer covering its entire
length/count. Empty buffers/lists use null and zero. Check lengths, multiplication
overflow and operational ceilings before allocating or copying. Validating
lengths cannot make invalid native addresses or wrong-library handles safe.

| Header type | Required interpretation |
|---|---|
| `cm1_bytes` | Arbitrary bytes for Input/Display; validated [UTF-8](../TERMS.md#unicode-and-utf-8) elsewhere. No trailing terminator is counted. Text can contain embedded [NUL](../TERMS.md#nul); it is inert, never a C string or terminal instruction. |
| `cm1_tokens` | Counted `cm1_bytes` array of nonempty case-sensitive tokens. Unique; required/optional lists disjoint. |
| `cm1_guid` | 16 [GUID](../TERMS.md#guid) bytes in canonical textual order: `00112233-4455-6677-8899-aabbccddeeff` becomes `00 11 22 ... ff`, without host-endian UUID field swapping. |
| `cm1_utc` | Exactly 24 [ASCII](../TERMS.md#ascii) bytes, valid [UTC](../TERMS.md#utc) `YYYY-MM-DDTHH:MM:SS.mmmZ`, real calendar date and seconds 00..59. No trailing NUL. Preserve daemon-stored effective-change time, not receipt time. |
| `cm1_path` | `encoding=CM1_NATIVE_UNIX`; absolute, nonempty length-delimited native Unix bytes starting `/`, without NUL. Invalid UTF-8 is preserved. Other encodings reject. |
| `cm1_geometry` | Positive `uint32_t` columns/rows, not pixels. Supported sizing/clamping remains with the terminal/operational contracts. |
| `cm1_authority` | Nonzero generation, period and restoration ID. Never substitute one identity for another. |
| `cm1_optional_text` / `cm1_optional_path` / `cm1_optional_command` | `present` is 0/1. Absent value is inactive and ignored; present value follows its own constraints. A present empty text is valid. |
| `cm1_title` | `CM1_UNAVAILABLE`/`CM1_UNSET` require null/zero `value`; `CM1_VALUE` permits empty or nonempty UTF-8. Unavailable is not supported-unset or supported-empty. |
| `cm1_available_status` / `cm1_available_progress` | `available` is 0/1. Available requires a valid value; unavailable value is inactive, never a fabricated idle/clear/no-progress value. |

All [counters](../TERMS.md#counter) are nonzero `uint64_t`, maximum
18446744073709551615; never wrap. Public request IDs increase strictly per
instance's accepted submissions. Local rejection does not advance the high-water
mark or produce a later response. Period/restoration/episode scopes and ordering
are the daemon's [shared types](../daemon-client/core-v1.md#shared-types), mapped
losslessly to integers. No new native period, restoration or episode counter
is invented.

The caller assigns a [connection generation](../TERMS.md#connection-generation)
in startup, strictly increasing per immutable
[session](../../../GLOSSARY.md#sessions) across attempts during this monitor run.
Every delivery reports it. One instance permits at most one Open attempt;
passive reconnect creates a fresh instance after old cleanup/join. Generation
exhaustion explicitly prevents further attempts, never wraps. Accepted old work
cannot cross into a new attempt. Kill's auxiliary connection is correlated by
its public request ID, never represented as new authority on the opened stream.
Public IDs are mapped to distinct per-wire-connection IDs.

`cm1_status.state` maps in order to idle/running/needs_attention/acknowledged/
completed/failed. `episode_present=1` with nonzero `episode_id` is required only
for needs_attention/acknowledged; otherwise presence is 0 and ID is 0.
The episode survives acknowledgement, coalesces while pending, and differs from
a timestamp. Unknown/unsupported status is `available=0` plus explicit capability
information, not another status enum. A wire value outside the agreed enum is
a protocol error, never normalized to idle or cleared.

`cm1_progress.state` maps to none/normal/indeterminate/error/paused.
`percent_present` is 0/1; present percent is 0..100. Normal requires it;
none/indeterminate forbid it; error/paused permit either. Absent percent is 0.
100 is not command completion.

`cm1_command_result.kind` is `CM1_EXIT` or `CM1_SIGNAL`. `value` is raw unsigned
32-bit exit code, or positive source-host signal number. Exit requires absent
`signal_name`; signal permits an absent or UTF-8 name. Exit 143 differs from
signal 15. Final identity is session GUID plus fixed category `final outcome`,
not result kind or a new counter. If status and result coexist, completed
requires exit zero; failed requires nonzero exit or signal. Never revive a final
session or accept inconsistent repeats as new final events.

## Startup, limits and resources

Create copies `cm1_startup` and all nested buffers before returning. Session ID,
generation, nonempty expected release, and metadata/home/runtime absolute paths
are required. Metadata, hello session identity, daemon release and expected release
must agree before role effects. These paths are explicit per-instance inputs;
do not substitute process environment, cwd, signals or standard streams.
Creation performs local resource setup only: no daemon I/O or internal workers.
It returns all instance/communication/control handles or none; partial local
setup is rolled back before returning failure.

`cm1_settings` is the caller's fully resolved validated snapshot.
Enabled keys are bytes 0..31; disabled keys require byte zero.
Take-control cannot be 27; enabled escape and take-control cannot coincide.
The library never interprets submitted Input as shortcuts; executable/monitor
wrappers own routing. This snapshot does not permit the library to own the host.

`cm1_diagnostics.log_path` is an optional absolute native path. Absent means no
library file logging; failures still use explicit error records/results.
`filter` is required nonempty UTF-8 using the existing diagnostic filter grammar
([current logger](../../../crates/climon2-core/src/diagnostics.rs)).
Validate it even when file logging is disabled. Diagnostic files are
instance-scoped and content-free; no process-global subscriber or stderr writes.
The current session-log cap remains unchanged.

All `cm1_limits` fields are required effective values, never zero-as-default.
The caller resolves configuration defaults and checks aggregate admission.
Create rejects values it cannot honor with `CM1_UNSUPPORTED_LIMIT`; it never
raises or silently lowers them; `drain_payload_bytes` below
`metadata_record_bytes` is one such value, since a delivery must hold a whole
record. `effective_limits` equals the accepted snapshot.
The library owns allocations only within its remaining share of `client_bytes`;
the caller subtracts/reserves monitor-owned model/history/staging capacity before
supplying that ceiling. No double-counted independent 64 MiB allowances.

| Header fields | Owning configuration under `monitor.limits` |
|---|---|
| `client_bytes` | Remaining admitted library share of `memory.client_bytes`. |
| `request_bytes`, `request_records`, `event_bytes`, `event_records`, `progress_bytes`, `progress_records`, `deliveries`, `operations` | Same-named `queues` fields; progress is reserved inside events, not added on top. |
| `chunk_bytes`, `drain_records`, `drain_payload_bytes` | Same-named `transfer` fields. |
| `wire_frame_bytes`, `metadata_record_bytes`, `title_bytes`, `notification_title_bytes`, `notification_body_bytes` | Same-named `payload` fields. |
| `max_workers` | `resources.max_workers_per_client`; zero is permitted. |
| `operation_ms`, `kill_ms`, `restoration_stall_ms`, `restoration_total_ms`, `delivery_stall_ms`, `leave_ms`, `cleanup_ms`, `start_ms` | Same-named `deadlines` fields. |

Wire-frame ceilings apply to local daemon parsing/sending, not native framing.
The daemon's advertised receive/server limits must fit before Open. Text/metadata
limits cover capability/release/error fields too; oversized required records
explicitly fail instead of truncating to a healthy value. Notification assembly,
application-source parsing and assembly deadlines belong to the daemon, not
this native client's startup. Aggregate memory/history, discovery/load/admission,
retry, render, host/shutdown and process-memory measurements remain caller-owned.
The caller enforces startup and whole-process cleanup deadlines even if run stalls.
The library reports its internal deadlines without claiming it can interrupt
stalled native code.

`cm1_requirements` gives qualified release allocations, minimum instance local
allocations, owner/worker stack reservations, required worker count and
instance/load descriptors. Include auxiliary Kill resources in the bounds.
These are bounded requirements for any supported revision-1 operation, not
guarantees against native initializer faults. Reserve the greater of configured
and qualified stacks/descriptors/release memory, respecting worker limits and
aggregate accounting, before create. Pre-load qualification/reservation remains
required by #69;
bootstrap data is a post-load validation, not permission to load an unqualified
release first. Requirements incompatible with admitted
limits reject explicitly. Release resources count once and remain until exit.
Each platform's library reports that platform's qualified requirements. On
Linux the [notification descriptor](../TERMS.md#pollable-notification-descriptor) is one `eventfd`, created close-on-exec and
nonblocking; it stays readable until the library clears it and counts once in
`instance_descriptors`.

`snapshot` supplies bounded nonblocking current/peak instance and shared-release
allocation, queue-byte measurements, record/delivery/operation/worker/descriptor
counts, and monotonic actual accepted-input/delivered-display/drain/release counters.
Counter exhaustion fails explicitly rather than silently wrapping.
Event bytes/records include queued and drained-unreleased deliveries; all
allocation capacity counts, not just payload length.
Shared-release allocation is deduplicated by loaded release, never charged once
per instance. Stacks/reservations are accounted from qualified requirements and
actual worker counts as required by #69.
Snapshot is observational, not proof of daemon acceptance, readiness, cleanup
or application consumption.
The monitor combines it with its own measurements required by
[operational evidence](../../monitor-operational-limits.md#required-implementation-evidence).

## Function calls and ownership

All table calls return `cm1_call`: success has `code=CM1_SUCCESS`, zero origin
and empty error buffers. Failure has a nonzero code, nonzero origin, nonempty
stable UTF-8 error code and diagnostic message. Error text is borrowed until
the next table call to that library on the same calling thread; copy it first.
No error allocation is freed by the caller. Failed calls clear output structs,
except run preserves its initialized report
of known terminal/cleanup evidence even on failure.
Notification failure writes -1. They retain no submitted buffers. A failed
release/destroy leaves its object valid and owned.

| Call | Thread and semantics |
|---|---|
| `create(startup, out)` | Dedicated caller-created owner thread, once. Copy/validate and return three handles plus effective limits; no role effects. |
| `run(instance, out)` | Owner thread, once. Execute accepted work/connection I/O until shutdown, final outcome, Leave, or failure. Return a separate worker-cleanup report. Does not destroy instance/handles/deliveries or join the owner thread. |
| `destroy(instance)` | Owner thread, after run returns with complete worker cleanup. `CM1_BUSY` if run/workers/owned handles/deliveries remain. Success frees instance and borrowed notification; no later use. |
| `submit(communication, request)` | Thread-safe, caller serializes IDs. Bounded validation/copy/admission only; no daemon I/O, waits or terminal processing. Success is queued, not daemon success. |
| `drain(communication, out)` | One monitor consumer, nonblocking. One bounded immutable delivery, empty, or terminal; no joining or direct descriptor I/O. |
| `release_delivery(delivery)` | Thread-safe same-library release exactly once, after all readers finish. Makes that delivery's records/buffers invalid. |
| `shutdown(control)` | Thread-safe nonblocking idempotent shutdown intent, independent of queue space. Does not prove Leave, cleanup or join. May precede run. |
| `release_communication(communication)` | After stream terminal, all deliveries released and concurrent submit/drain/snapshot/notification/poll use ended. Releases caller handle; no later calls. Busy if stream/deliveries still active. |
| `release_control(control)` | After stream terminal and all concurrent shutdown calls ended. Does not destroy the instance. Busy if stream still active. |
| `notification(communication, out_fd)` | Thread-safe bounded lookup of borrowed nonnegative platform pollable descriptor. Caller polls only, never reads/writes/closes. Library signaling/rearming cannot miss work/closure. |
| `snapshot(communication, out)` | Thread-safe bounded local measurement, available through terminal until handle release. No I/O/wait/join. |

Wrong-owner-thread calls return `CM1_WRONG_THREAD`. No concurrent destruction
or handle release with permitted uses is allowed. Handles are not cross-library
or cross-instance interchangeable; stale/wrong pointers remain caller misuse,
not a promised safe rejection mechanism.

`cm1_drained.kind=CM1_DELIVERY` returns a nonnull delivery and nonempty immutable
record array with `count` within accepted drain bounds; terminal is null.
`CM1_EMPTY` has null pointers/count zero. `CM1_TERMINAL` has no delivery/records,
count zero and a nonnull immutable `cm1_terminal` valid until destroy.
Terminal remains repeatedly observable outside ordinary queue capacity only
after earlier queued records have drained. Drained unreleased objects remain
charged and may make further drain return `CM1_FULL`; they never disappear.

Submit returns synchronous full/closed/malformed/unsupported/ID rejection
without ownership or later response. Reserve mandatory outcome capacity before
acceptance. Accepted requests execute FIFO except independent shutdown. Recheck
authority at execution. Known unexecuted cancellation/staleness is rejected;
unknown effects are indeterminate, never replayed. Each accepted request has one
outcome while healthy; terminal failure additionally makes any still-unanswered
accepted IDs explicitly indeterminate in the caller's bounded pending ledger.
Never fabricate individual responses after an unrecoverable delivery failure.

## Requests and result mapping

`cm1_request` contains `tag`, reserved zero, positive `request_id` and precisely
the named active argument member below. Every request is monitor/caller -> client.
Core capability support is `daemon-wire-v1`; monitor Open additionally requires
`monitor-v1`. Required/optional tokens are unique, disjoint and dependency-valid.
The library intersects its support with daemon advertisements; unsupported
required support rejects before effects, unknown optional support is not selected.
After Open, acquisition/optional operations check the selected support, never
release versions. Lack of title/status/progress/focus is explicit and does not
fabricate supported empty/clear values.

| Native tag / args | Exact fields and allowed state | Wire / success result |
|---|---|---|
| `CM1_OPEN` / `open` | `mode` monitor/interactive, geometry zero/zero for monitor and positive for interactive, required/optional lists. Once per instance; no prior role. | [Open `0x30`](../daemon-client/request-open.md); `CM1_OPENED` / `opened` with fresh connection GUID and effective selected tokens. |
| `CM1_INPUT` / `input` | Current authority and nonempty exact user bytes; selected ready/applied/painted target only. No shortcuts, injected input or host replies. | [Input `0x31`](../daemon-client/request-input.md); `CM1_INPUT_ACCEPTED` / `accepted_bytes` equals full payload. |
| `CM1_RESIZE` / `resize` | Current authority and positive desired geometry; selected controller only, including restoration before readiness. | [Resize `0x32`](../daemon-client/request-resize.md); `CM1_RESIZE_ACCEPTED`, no active result member. |
| `CM1_ACQUIRE` / `generation` | Current opened generation; passive or attached, explicit selection/takeover only. No geometry/episode. | [Acquire `0x33`](../daemon-client/request-acquire.md); `CM1_ACQUIRED` / `authority` changed flag plus period/restoration. |
| `CM1_READY` / `authority` | Matching current complete restoration transactionally applied; paint/selection not required. | [Ready `0x34`](../daemon-client/request-ready.md); `CM1_READY_RESULT` / `authority` changed plus period/restoration. |
| `CM1_VIEWING` / `flag` | Current authority plus 0/1 `value`; true requires ready/unobstructed selected presentation; false may clear during restoration/background. | [Viewing `0x35`](../daemon-client/request-viewing.md); `CM1_VIEWING_RESULT` / `flag` changed/value; accepted_bytes zero. |
| `CM1_ACKNOWLEDGE` / `authority` | Current controller ready and latest viewing true. No observed episode. Explicit user selection/menu-close context, never visibility/focus-in alone. | [Acknowledge `0x36`](../daemon-client/request-acknowledge.md); `CM1_ACKNOWLEDGEMENT` / `changed`, including no-change success. |
| `CM1_LEAVE` / `generation` | Current opened connection; no period/readiness requirement. Atomic registration/membership departure including promotion race. | [Leave `0x37`](../daemon-client/request-leave.md); `CM1_LEFT` / `attachment_detached` distinguishes passive deregistration from attachment detach. |
| `CM1_KILL` / `required` | This immutable session; standalone or alongside Open, through a separate hello-validated first-role connection. No takeover/role replacement; at most one pending Kill. | [Kill `0x38`](../daemon-client/request-kill.md); `CM1_TERMINATION` / `termination` with present result and complete daemon cleanup. |
| `CM1_FOCUS` / `flag` | Current ready authority plus desired 0/1 state, selected focus or valid retained-controller focus-out; selected `focus-v1`. Never user Input or acknowledgement. | [Focus `0x39`](../daemon-client/request-focus.md); `CM1_FOCUS_RESULT` / `flag` changed/value and accepted_bytes 0 or 3. |

Shutdown's internal Leave has no public request ID and is recorded separately
in terminal evidence. If an explicit Leave is already executing, join that
departure attempt; never submit/replay another Leave after uncertain execution.
Kill-only run ends after its result; a Kill on an opened instance ends that
auxiliary request, without repurposing the opened role.

`cm1_response` carries public `request_id`, one of `CM1_OK`, `CM1_REJECTED`,
`CM1_FAILED`, `CM1_INDETERMINATE`, explicit `result_tag`, error, and matching
active result. Success requires the matching table result and empty error.
Rejected guarantees no operation execution; failed does not. Non-success requires
an error and may preserve a matching known partial result; otherwise
`CM1_RESULT_NONE` means no active result.
Initial Open/Kill rejection may use `CM1_KNOWN_FINAL` / `known_final`.
Failed/indeterminate Kill may use termination with optional command and
cleanup incomplete/unknown. Success never has absent command or incomplete
cleanup. Other unknown/mismatched result variants are protocol failures.
Authoritative results learned through Initial metadata, known-final rejection,
Kill or Final share the same final deduplication identity and stop completion
retry; neither metadata nor a partial Kill result proves daemon cleanup.

## Events and ordering

`cm1_record` has native `tag`, reserved zero, startup generation and exactly the
active `data` member below. All events are client -> monitor. The generation
applies to responses too, including Kill auxiliary results. No event has a
public request ID except the correlated Response payload.

| Tag / data | Required fields / source |
|---|---|
| `CM1_RESPONSE` / `response` | Correlated outcome above; [wire `0x40`](../daemon-client/response.md). |
| `CM1_INITIAL_METADATA` / `metadata` | Title/status/progress availability and optional known command; atomic [wire `0x41`](../daemon-client/event-initial-metadata.md). |
| `CM1_TITLE` / `title` | Supported unset/value title only, `title-v1`; [wire `0x42`](../daemon-client/event-title-changed.md). |
| `CM1_STATUS` / `status` | Effective nonfinal state/time/episode, `status-v1`; [wire `0x43`](../daemon-client/event-status-changed.md). Final transition is not duplicated here. |
| `CM1_NOTIFICATION` / `notification` | Source OSC9/99/777, optional source_id/title/body, optional nonzero episode; `notification-v1` plus `status-v1`; [wire `0x44`](../daemon-client/event-notification.md). Source ID is not episode ID; title/body cannot both be absent/empty. |
| `CM1_PROGRESS` / `progress` | Normalized valid current progress, `progress-v1`; [wire `0x45`](../daemon-client/event-progress-changed.md). |
| `CM1_GRANTED` / `granted` | Nonzero period/restoration, effective geometry, reason attach/take-control/hand-back/successor; [wire `0x46`](../daemon-client/event-control-granted.md). |
| `CM1_REVOKED` / `revoked` | Period/restoration and reason takeover/session-ending; [wire `0x47`](../daemon-client/event-control-revoked.md). Membership persists. |
| `CM1_RESTORATION_BEGIN` / `restoration` | Period/restoration plus effective geometry; [wire `0x48`](../daemon-client/event-restoration-begin.md). |
| `CM1_DISPLAY` / `display` | Period/restoration and nonempty exact binary bytes; [wire `0x49`](../daemon-client/event-display.md). |
| `CM1_RESTORATION_COMPLETE` / `barrier` | Matching period/restoration; [wire `0x4a`](../daemon-client/event-restoration-complete.md). Sent, not applied/painted. |
| `CM1_GEOMETRY` / `geometry` | Period and effective geometry, no restoration field; [wire `0x4b`](../daemon-client/event-geometry.md). |
| `CM1_FINAL` / `final` | Authoritative command and terminal status if selected, unavailable otherwise; [wire `0x4c`](../daemon-client/event-command-final.md). |
| `CM1_DIAGNOSTIC` / `diagnostic` | Explicit origin/code/message; [wire fatal `0x4d`](../daemon-client/event-fatal-error.md) when deliverable, or content-free library diagnostic. A fatal failure also persists in terminal evidence. |

Open success precedes exactly one Initial metadata barrier, then gap-free
selected-family changes. Interactive grant/restoration follows metadata.
Passive metadata does not refresh a cached terminal. Grant precedes restoration
begin, ordered display chunks, matching complete; metadata may interleave.
A newer begin/revocation invalidates previous readiness and forbids later
old-authority display. Monitor-mode bytes use effective geometry even on
background grants/promotion. Apply whole model/history transactionally before
Ready; selected Input additionally waits for coherent paint.

Immediate causal changes precede their correlated success response. Resize
acceptance need not have a final geometry yet. Real accepted application Input
clears pending/acknowledged attention at the daemon; local submission does not.
Focus never does. Required records are never lossily coalesced. Backpressure
includes drained-unreleased objects and cannot freeze other instances or
independent shutdown.

## Errors, terminal evidence and cleanup

Synchronous `cm1_call.code` values are `CM1_SUCCESS` through
`CM1_RELEASE_MISMATCH` in the header. Non-success diagnoses respectively
unsupported revision, malformed arguments, unsupported operation/capability,
invalid public ID, capacity full, closed submission/stream, outstanding ownership,
wrong thread, unsupported limit, exhausted resources, containable internal
failure, failed local cleanup, local deadline expiry and exact-release mismatch.
Corresponding library error strings are `unsupported_revision`,
`invalid_argument`, `unsupported_operation` (or `unsupported_capability`),
`invalid_request_id`, `queue_full`, `closed`, `busy`, `wrong_thread`,
`unsupported_limit`, `resource_limit`, `internal_error`, `cleanup_failed`,
`timeout`, `release_mismatch`.
Rejected submission never consumes its ID. Full can be retried only as local
admission of the same still-unaccepted work and still-current authority, never
replay of accepted work.

Asynchronous daemon errors preserve the
[complete wire catalog](../daemon-client/core-v1.md#errors-and-bounded-delivery)
with daemon origin; framing/transport failure uses transport origin.
Native execution errors additionally use `stale_generation`, `cancelled`,
`release_mismatch`, `invalid_configuration`, `unsupported_limit`, `timeout`,
`delivery_stalled`, `worker_panic`, `cleanup_failed`, `internal_error`, and,
with library origin, `invalid_state`, `resource_limit`, `io_error`,
`unsupported_revision` and `unsupported_capability`
([decided on #93](https://github.com/jackgeek/climon2/issues/93)).
Malformed core responses/events are `invalid_record` with explicit failure,
not success-shaped defaults. Unknown error strings stay explicit non-success;
text is not a retry classification. Stable origin/code and known partial effects
remain separate from diagnostic prose.

`cm1_terminal` is not another command-final event. It reports this generation's
client outcome stopped/failed, retry none/passive/permanent, Leave evidence,
attachment-detached flag, connection-closed flag and optional failure error.
Stopped has no error; failed requires one. Retry passive is only for safely
recoverable startup/transport/contained-worker failure after cleanup/join;
permanent rejection/configuration stops retry. Definitive command completion
uses retry none. Never infer command result from terminal reason.

Leave evidence is not-required/confirmed/rejected/failed/indeterminate.
Confirmed requires daemon acceptance; attachment_detached is 1 only for a
confirmed attachment departure, 0 for confirmed passive deregistration or all
other states. Not-required requires no remaining possible registration/attachment
(including never-opened and authoritative final closure).
Connection-closed concerns all owned daemon connections, including auxiliary
Kill. Closure never retroactively confirms Leave.
Required Leave rejection/failure/uncertainty makes client outcome failed with
the specific Leave error, even if worker cleanup succeeds. Shutdown uses retry
none; its failure never starts a new passive instance.

Run publishes terminal evidence after ordinary queued deliveries, then returns
`cm1_run_report` with separate workers-cleanup complete/incomplete/unknown.
Terminal does not assert that workers have finished. Failed cleanup returns a
non-success call with a separate error; copy call diagnostics before another call.
On successful run, cleanup is complete; failed client/Leave outcomes may still
be reported even though local worker cleanup succeeded.
Run's report error buffers have the synchronous lifetime; drained records'
buffers live until delivery release; terminal buffers live until destroy.

After terminal, the monitor drains/releases all objects, ends polling/concurrent
calls and releases communication/control handles. The owner thread waits for
this release handshake after run, then destroys the instance. Successful destroy
proves local object release, not daemon acceptance or owner-thread completion.
The caller joins that thread separately. Outstanding handles/deliveries return
busy without freeing state.
Incomplete/unknown worker cleanup prevents successful destroy/replacement and
uses the bounded fail-stop policy; a failed run never erases that ownership.

Shutdown may interrupt FIFO ordinary execution but never fabricate outcomes.
It closes admission, rejects known-unexecuted work and attempts required atomic
Leave once before closing, including promotion races. Uncertain Leave/Kill/input
is never replayed. The notification resource remains valid until destroy.
Native faults/abort can terminate all instances; incomplete cleanup past the
agreed deadline uses whole-process fail-stop, never thread killing, forced unload,
replacement while old code runs or false joins.

## Public call walkthroughs

The per-page C fragments compile inside a caller context with validated
`api`/`communication` and current nonzero `generation`/`period`/`restoration_id`.
Event examples are constructed record values, not invented daemon requests.
[`examples.c`](examples.c) supplies full startup/Open/Input and release functions.
Its paths stand for real validated installation/metadata; it does not create a
session or bypass admission. Copy returned diagnostic text before the next call.

| Step | Valid caller observations and obligations |
|---|---|
| Load/check | On a load worker, load the absolute exact-release library with eager/local resolution, retain its handle, resolve the frozen bootstrap, and check revision/release/qualified resources. `example_bootstrap` illustrates validation after resolution; the direct symbol call stands for the resolved entry point. |
| Create/publish | On the dedicated owner thread, `example_create` returns three handles. Publish communication/control to the monitor through caller-owned synchronized handoff. No daemon I/O/workers have started. |
| Open/run | `example_open` submits ID 1; accepted buffers are copied. Owner calls `example_owner_finish` (run) while monitor polls/drains. Open response then Initial metadata establishes support; explicit acquisition/restoration/application precedes Input. |
| Input/drain | `example_input` submits literal `00 ff 41` under current authority only after selected paint. `example_drain` returns a batch/empty/terminal. Process records before `example_release_batch`; no record pointers survive release. Handle full/rejected admission explicitly. |
| Shutdown | `example_shutdown` requests independent intent even with full queues. Continue drain/release until terminal; preserve Leave/closure/client evidence separately. Run may still be cleaning workers. |
| Release/destroy/join | After all concurrent uses/polling end and deliveries release, `example_release_handles_after_terminal` releases handles, preserving ownership on failure. Signal this completed handoff to the owner. Run report must show complete worker cleanup before `example_destroy_after_release`; join the owner afterward. Retain dylib handles until process exit. |

For same/mixed-release usage, suppose sessions A/B use release `0.1.0`, while C
uses `0.2.0`. Load/validate two descriptions, each supporting revision 1.
Create A and B on separate owner threads using the *same* first description's
table, and C on a third thread using the second table. Each has independent
startup/settings/limits/IDs/generation/queues and its own communication/control
handles. Release A/B deliveries only with the first table and C deliveries only
with the second. A stalled A must not stop B/C or independent shutdown.
Both loaded handles remain through exit, even after all three instances are
destroyed/joined. A revision-compatible `0.2.0` library reporting the wrong release
for A is rejected before create, not substituted or unloaded. This is a valid
public-interface timeline, not a runtime test or a release prerequisite for plugins.

## Implemented behavior

The [Library-layer tests](../../../crates/climon2-e2e/README.md#library-layer-passive-monitor-connections-testspassivers-93)
are the evidence for this section
([#93](https://github.com/jackgeek/climon2/issues/93)).

- **Description.** `capabilities` lists exactly the families the library
  relays: `daemon-wire-v1`, `monitor-v1` and `title-v1`. `focus-v1` joins with
  [#94](https://github.com/jackgeek/climon2/issues/94), `status-v1` with
  [#95](https://github.com/jackgeek/climon2/issues/95), and `notification-v1`
  and `progress-v1` with [#98](https://github.com/jackgeek/climon2/issues/98).
  A required family outside the description is `CM1_UNSUPPORTED_OPERATION`
  with `unsupported_capability`; an optional one is not requested.
- **Admission.** `submit` checks the tag (`CM1_OPEN` to `CM1_FOCUS`), reserved
  zero and a nonzero ID before the operation's arguments. A token list is null
  and zero or a pointer and a positive count; tokens are nonempty UTF-8, each
  named once across both lists. A request's footprint, `sizeof(cm1_request)`
  plus `sizeof(cm1_bytes)` and the bytes of each token, must fit
  `request_bytes` before any token is read. A request counts against
  `request_records` and `request_bytes` until it executes and against
  `operations` until its Response is queued. Each outstanding request
  reserves one record and `metadata_record_bytes` of `event_records` and
  `event_bytes` for its outcome; without room, `submit` is `CM1_FULL`
  (`queue_full`).
- **Records.** A record is charged `sizeof(cm1_record)` plus the bytes it owns:
  copied text and token arrays. The library's own error text is static and
  costs nothing. While the event queue is full, the library stops reading the
  connection, so daemon records wait. A delivery holds the oldest records, at
  most `drain_records`, stopping before `drain_payload_bytes`; a record never
  exceeds it alone, because create refuses `drain_payload_bytes` below
  `metadata_record_bytes`. `drain` is `CM1_FULL` while `deliveries` deliveries are
  outstanding. Every received [frame](../TERMS.md#frame), type byte included,
  must fit `metadata_record_bytes`, and every title `title_bytes`; either
  breach fails the connection with library `resource_limit`.
- **Open.** The session metadata, read up to `client_bytes`, must name the
  startup's session (`invalid_configuration`) and release
  (`release_mismatch`); an unreadable file is library `io_error`. Hello, within
  `start_ms` and its 2-second limit, must name the same session, protocol 1
  (`unsupported_revision`) and release, and advertise revision 1 validly
  (`unsupported_revision`) with `monitor-v1` and every required family
  (`unsupported_capability`) and server frames within `wire_frame_bytes`
  (`unsupported_limit`). These refusals have library origin; an unreachable
  socket is transport `io_error`, and a reply that is not a hello transport
  `invalid_record`. A refused Open is never sent: it is `CM1_REJECTED` with the
  refusal and the stream ends. The Open sent names the required families and
  the optional ones both sides support; the daemon's selection must contain
  every required family and nothing unrequested (`invalid_record`).
- **Failures.** Transport origin covers the socket, framing and the wire's
  rules: `io_error`, `truncated_frame`, the wire's framing codes,
  `invalid_record` and `invalid_request_id`. Daemon origin covers the daemon's
  own errors, copied. Library origin covers `timeout`, `cancelled`,
  `invalid_state` (a second Open, or Leave without a role or while another
  Leave is unanswered), `stale_generation`, `resource_limit` and the Open
  refusals above. A failure ends the connection, queues one diagnostic when it
  fits and fails terminal evidence with its error.
- **Deadlines and loss.** `start_ms` bounds an Open until its Initial metadata,
  and `leave_ms` each Leave; on expiry that request is `CM1_INDETERMINATE`
  `timeout` and the connection fails. No other Response is fabricated: a
  request lost with the connection stays unanswered for the caller's pending
  ledger. Requests still queued when the stream ends are `CM1_REJECTED`
  `cancelled`.
- **Leave.** A Leave waits for the Initial metadata. The daemon's answer ends
  the stream, except a rejection, which keeps the registration. Shutdown
  rejects queued requests, waits for an Open under way, then attempts its own
  Leave once; an Open whose hello completes after shutdown is never sent and
  is `CM1_REJECTED` `cancelled`.
- **Terminal evidence.** Retry is none once shutdown is requested, once a
  Leave the caller asked for is sent, answered or lost, and once the
  command's final result is known, from a known-final rejection or the
  Initial metadata's command result; otherwise permanent for the
  library's `unsupported_revision`, `unsupported_capability`,
  `invalid_configuration`, `release_mismatch` and `unsupported_limit`
  refusals and the daemon's `unsupported_revision` and
  `unsupported_capability`; passive otherwise. Leave is not required when no
  Leave was attempted, since closing the connection ends a passive
  registration, and indeterminate when one was lost or late.
- **Signals.** Sending never raises `SIGPIPE` in the embedding process.

## Required runtime evidence

Later implementation must exercise actual packaged libraries through this header:

- Multiple same-release instances and simultaneous mixed releases, with distinct
  private dependency behavior detecting cross-binding; independent settings,
  paths, errors, resources and progress.
- Bootstrap missing/unsupported/mismatched releases before creation/connection;
  unknown tags/reserved values, bad IDs, null/length/overflow/text/path validation,
  unsupported required/optional capabilities and unsupported limit rejection.
- Copy-on-acceptance and no retained rejected buffers; literal `00 ff 41`;
  immutable batch lifetime, same-library allocator release, retained diagnostic
  lifetimes, outstanding-object/handle busy destruction and borrowed descriptor
  wakeup/closure races on both supported architectures.
- Every operation/result/event mapping, arbitrary exits versus signals,
  partial Kill/known-final results, loss versus final outcome, atomic metadata,
  UTC/episode identities across passive fresh-instance recovery and no replay.
- Causal grant/restoration/geometry/result ordering, applied versus painted Ready,
  Focus state versus forwarding, Input attention clearing, viewing/explicit
  acknowledgement, passive succession/Leave races and repeated shutdown.
- Full queues, retained deliveries, progress reserves, cancellation/indeterminate
  pending work, run cleanup/handle release/destroy/join, stalled/native-fault
  whole-process failure, independent instances and all #69 measurements.

[Declaration checks](check.py) compile C examples/layout assertions for both
targets and validate reference reachability. They deliberately do not use a mock
library as production compatibility evidence. [Final acceptance #62](https://github.com/jackgeek/climon2/issues/62)
assigns the agreed [acceptance matrix](../../monitor-acceptance.md).
Actual-monitor conformance remains an unexecuted release gate.
