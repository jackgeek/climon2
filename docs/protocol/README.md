# climon2 protocol reference

This reference is for future [plugin](../../GLOSSARY.md#protocol-and-plugin-concepts)
authors. It distinguishes implemented behavior, agreed plans, and unresolved
interfaces. The [monitor](../../GLOSSARY.md#sessions) is not implemented.
Plugin implementation is deferred. Title and terminal-signal detection and
[session status](../../GLOSSARY.md#status-and-attention) belong in
[session daemon](../../GLOSSARY.md#sessions) core;
the agreed [mechanics and state diagrams](../session-status.md) are not shipped
behavior.

Use the [domain glossary](../../GLOSSARY.md) for climon2 concepts and the
[technical terms](TERMS.md) for protocol vocabulary. Each operation page links
to its owning specification. Descriptive filenames do not assign
native function names or layouts. The daemon wire pages do assign exact
agreed codes and schemas, separately from current implementation.

## Status and scope

| Interface | Status | Reference |
|---|---|---|
| [Client](../../GLOSSARY.md#sessions) to session daemon: [hello handshake](../../GLOSSARY.md#sessions) | Implemented and frozen | [Request](daemon-client/hello-request.md) and [reply](daemon-client/hello-reply.md). |
| Client to session daemon: after hello | Current private protocol implemented; separate wire revision 1 agreed, not implemented | [Decision boundary](daemon-client/post-hello.md), [complete wire rules](daemon-client/core-v1.md). |
| Monitor to [client library](../../GLOSSARY.md#sessions) | Embedding and communication design agreed, not implemented; complete declarations unresolved | [Library embedding](library-embedding.md), [shared communication contract](#shared-in-process-communication-contract), and [interface schemas](https://github.com/jackgeek/climon2/issues/71). |
| [Client executable](../../GLOSSARY.md#sessions) to same-release library | Same-release loading agreed, not implemented | [Library embedding](library-embedding.md). |
| Monitor to [host](../../GLOSSARY.md#sessions) | Program-status advertising and toast bell agreed, not implemented | [Reports and bell](#monitor-to-host-status-and-bell). |

The embedding design selects a versioned [C ABI](TERMS.md#c-abi), but its
complete declarations and valid call examples are not yet published.
Communication uses tagged C-compatible records, not [JSON](TERMS.md#json),
a pipe protocol, or serialized [envelopes](TERMS.md#envelope). Complete
native [schemas](TERMS.md#schema) remain pending. No in-process
[framing](TERMS.md#frame), [frame](TERMS.md#frame) limit, or complete cross-release declarations
are assigned by these pages.
The existing daemon hello exchange is unaffected.

## Who communicates?

```text
MONITOR PROCESS
  monitor
    <-> version-matched client library
        one client thread per live session
        one loaded library shared by sessions on the same release
          <-> session daemon in its own process
              <-> command through its PTY
```

The [ownership decision](https://github.com/jackgeek/climon2/issues/58)
requires the monitor to load session-version libraries directly. It starts
clients eagerly, newest first, with bounded concurrent progress. Continuous
discovery adds clients for newly discovered sessions without changing selection.
The command remains behind the daemon's [PTY](TERMS.md#pseudo-terminal-pty).
The daemon remains the authority for terminal state and terminal-query replies.

The [agreed daemon wire](daemon-client/core-v1.md) is portable, independent of
today's local private sockets. Future monitor-to-monitor forwarding over
[TCP/IP](TERMS.md#tcpip) presents forwarded sessions as local at the downstream
monitor; future [WebSocket](TERMS.md#websocket) forwarding feeds xterm.js in a
browser, not a native library in that browser. Routing, multiplexing, discovery
and remote security remain unresolved. The
[artifact diagram](../runtime-artifacts.md) shows these planned consumers.
There is no initial application checksum; only a future negotiated integrity
revision could add one if chosen.

Future `climon2 exec --headless <command> [args...]` creates a
[headless session](../../GLOSSARY.md#sessions) without an interactive client and
returns success only after daemon-confirmed creation. Its zero-client survival
policy lasts until command completion or explicit
[Kill](../../GLOSSARY.md#attachment-and-control), including after later
[attachment](../../GLOSSARY.md#attachment-and-control) loss. CLI spelling and lifetime are
[agreed, unimplemented](../monitor-lifecycle.md#future-headless-creation-and-existing-cli-kill).
Preserve existing `climon2 kill <session>` and require automated headless
termination tests; no duplicate command is added. Ordinary sessions retain
[leave policy](../../GLOSSARY.md#attachment-and-control), and this exception does
not make all sessions persistent.

Ratatui powers the planned monitor's rendering. The agreed
[terminal integration contract](terminal-integration.md) selects exact ordered
terminal bytes through client-library records, parsed into the monitor's own
per-[session](../../GLOSSARY.md#sessions) `climon2-vt`, with planned read-only Rust
views for painting. Session bytes never go directly to the host. Monitor-mode
daemon presentation is canonically serialized at effective session
[geometry](TERMS.md#geometry),
not outer-host dimensions; ordinary attach is unchanged.
[Restoration](../../GLOSSARY.md#restoration-and-presentation) replaces
the whole model/history transactionally. Applied
[readiness](../../GLOSSARY.md#protocol-and-plugin-concepts) permits wire progress;
selected input additionally waits for a coherent current frame to be painted.
These #63 amendments are agreed, not implemented.

Each client has its own connection and per-session state. The
[embedding reference](library-embedding.md) specifies exact-release lookup,
revision negotiation, thread ownership, memory rules, and library lifetime.
The [shared communication contract](#shared-in-process-communication-contract)
selects bounded per-instance queues and explicit ownership, without callbacks
into monitor application code. Exact declarations remain open.
No client may repurpose process-global arguments, signals, or
[standard streams](TERMS.md#standard-streams) as though it owned the monitor.

Native client crashes or aborts can terminate the entire monitor and all its
clients. Ordinary errors and recoverable panics should remain local where
possible. There is no new session-survival guarantee:
the current [leave policy](../../GLOSSARY.md#attachment-and-control) can terminate
a session that unexpectedly loses its final
[attachment](../../GLOSSARY.md#attachment-and-control). The agreed wire
[succession rules](daemon-client/core-v1.md#membership-and-succession) try
remaining attachments, then eligible passive registrations, before
final-participant policy; they do not guarantee survival of losing every client.

## Monitor-to-host status and bell

The [selector specification](../monitor-selector.md) adds monitor-generated
[OSC](TERMS.md#osc) 7501 reports and a separate [BEL](TERMS.md#bel).
Session sequences are never forwarded for this purpose.
Reports use the upstream
[Program Status Protocol](https://www.superlogical.com/rex/docs/build/program-status):
[ASCII](TERMS.md#ascii) key/value pairs after `ESC ] 7501;`, colon-separated, terminated by
`ESC \`. Omit `id` to address its
[root record](https://www.superlogical.com/rex/docs/build/program-status#records-and-ids).
Every replacement repeats `app=climon2-monitor`; no session IDs, titles,
messages, progress or child records are exported.

Send initial aggregate state and changes only, without heartbeat or capability
query. Any connected `needs_attention` session means pending attention,
independent of toast visibility. Acknowledged, disconnected and final reports
do not qualify. These are not per-session messages or new daemon-wire operations.

| Monitor -> host operation | When | Reference |
|---|---|---|
| Root `blocked`, `kind=question` | Connected pending attention exists | [Attention report](monitor-host/report-attention.md). |
| Root `working` | Monitoring continues without pending attention | [Monitoring report](monitor-host/report-monitoring.md). |
| Root `clear` | Orderly exit cleanup | [Clear report](monitor-host/report-clear.md). |
| Bell | Each toast first begins drawing, if configured on | [Bell](monitor-host/bell.md). |

Upstream framing/field/record limits and unknown-key behavior apply.
These fixed reports fit the existing supported states; no new negotiation,
acknowledgement, [request IDs](TERMS.md#request-id),
[control periods](../../GLOSSARY.md#attachment-and-control) or retry/replay policy is added.
Unsupported hosts may ignore them. Writing does not prove host interpretation,
sound or user acknowledgement. The
[agreed lifecycle](../monitor-lifecycle.md#coordinated-shutdown) requires
restoration and root clear before final diagnostics; broken output makes these
best-effort with explicit failure, not a successful cleanup promise.
These signals do not amend daemon activity/attention [FSMs](TERMS.md#fsm) or acknowledge sessions.

## Startup and control

1. Discover live sessions in deterministic newest-first order.
2. Load each required session-version library and validate identity and interface
   support before instance creation or a daemon connection.
3. Start one client per live session, with bounded concurrency. A slow or broken
   session must not indefinitely prevent older sessions from getting clients.
4. Establish passive background connections that receive core title/status and
   structured notification/progress data
   without taking [control](../../GLOSSARY.md#attachment-and-control).
   [Open monitor mode](daemon-client/request-open.md) registers a
   [passive monitor connection](../../GLOSSARY.md#attachment-and-control),
   always eligible as a successor but not counted as an attachment until
   explicit acquisition or promotion. Registration alone never takes over a
   detached session; there is no observation-only option.
5. Collect current daemon status and its change time passively, with bounded
   progress. Choose the first candidate in the
   [status/time ordering](../session-status.md#reporting-ordering-and-initial-selection),
   not by creation time. Unknown status remains explicit uncertainty.
6. Acquire control for initial selection or explicit user selection/takeover.
   Enable application input only after the current complete
   [restoration](../../GLOSSARY.md#restoration-and-presentation) has been applied.

The [startup/discovery decision](https://github.com/jackgeek/climon2/issues/59)
is agreed, not implemented. Run an immediate discovery pass, then start the next
pass one second after the preceding pass completes, without overlap. Preserve
the two-second hello deadline and bounded [probes](../../GLOSSARY.md#sessions).
Reconcile by full [session ID](../../GLOSSARY.md#protocol-and-plugin-concepts),
reusing starting and retained clients; discovery uncertainty never overrides an
established client's state or tears it down.

Pin the chosen initial candidate while it starts. New arrivals
receive passive clients without displacing it. The later
[lifecycle amendment](../monitor-lifecycle.md#monitor-and-selection-lifetime)
preserves chosen identity through loss, completion and failed acquisition,
without automatically acquiring a replacement. Startup failures stay visible
on that candidate without silent fallback. Startup
selection succeeds only after control and the current complete restoration
have been applied. [Revocation](../../GLOSSARY.md#attachment-and-control) before
[readiness](../../GLOSSARY.md#protocol-and-plugin-concepts) leaves a non-controlling
[placeholder](../../GLOSSARY.md#attachment-and-control) requiring explicit user action, not repeated automatic takeover.
The monitor remains open on empty startup, all-unavailable or all-final states
and continues discovery. Explicit selection is never redirected because its
target fails.

## Session status and monitor recovery

The daemon owns independent
[activity/attention states](../../GLOSSARY.md#status-and-attention) and reports
their effective status plus its [UTC](TERMS.md#utc) change timestamp on connection
and on changes.
The monitor orders rows by status, then newest change time, with an internal
[GUID](TERMS.md#guid) tie-breaker. Later sorting must preserve selected/highlighted identity.
See the [canonical mechanics](../session-status.md) for both diagrams, the
snapshot strategy, exact ranking, and notification behavior. Both internal
working states report the existing `running` status; detector changes alone do
not update its timestamp.

### Terminal signals and metadata

The agreed revision interprets [OSC](TERMS.md#osc) 7501 program status,
OSC 133 shell lifecycle, and OSC 9/99/777 notification/progress signals in daemon
core before host-output filtering. This is planned behavior, not an implemented
parser or plugin contract. External sequence framing and validation belong to
their upstream references:

| Program/daemon interaction | Direction | Reference and local policy |
|---|---|---|
| OSC 7501 root/child reports and clearing | Program -> daemon | [Program Status Protocol](https://www.superlogical.com/rex/docs/build/program-status); [root-only effects and fallback](../session-status.md#explicit-program-status-and-fallback). |
| OSC 7501 [capability](../../GLOSSARY.md#sessions) query/reply | Program -> daemon -> program | Same upstream reference; daemon replies locally, and a query does not activate explicit detection. |
| OSC 133 prompt/command markers | Shell -> daemon | [Shell integration](https://learn.microsoft.com/en-us/windows/terminal/tutorials/shell-integration); [scope retirement and prompt exclusion](../session-status.md#shell-boundaries-and-prompt-rendering). |
| OSC 9 text notification | Program -> daemon | [iTerm2 sequences](https://iterm2.com/documentation-escape-codes.html); [notification policy](../session-status.md#notification-and-progress-signals). |
| OSC 9 progress/control operations | Program -> daemon | [ConEmu sequences](https://conemu.github.io/en/AnsiEscapeCodes.html#ConEmu_specific_OSC); progress is metadata, not attention. |
| OSC 99 logical notification and auxiliary operations | Program -> daemon | [kitty notifications](https://sw.kovidgoyal.net/kitty/desktop-notifications/); only completed notifications request attention. |
| OSC 777 `notify` | Program -> daemon | [foot syntax](https://codeberg.org/dnkl/foot/src/branch/master/doc/foot-ctlseqs.7.scd) and [parser](https://codeberg.org/dnkl/foot/src/branch/master/osc.c); [climon2 normalization](daemon-client/core-v1.md#source-normalization), not every upstream quirk. |

These are not new climon2 wire encodings. The daemon remains responsible for
terminal-query replies; raw sequences do not pass through to
[host](../../GLOSSARY.md#sessions) terminals.
Notification detection does not promise a full desktop-notification protocol
implementation; unsupported notification queries/control operations are ignored.
The [notification](monitor-client/event-notification.md) and
[progress](monitor-client/event-progress.md) interactions relay structured
metadata through clients to subscribers, including background/noncontrolling
ones. They are distinct from terminal display updates and do not authorize
screen mirroring. Their exact daemon wire
[schemas and capabilities](daemon-client/core-v1.md) are agreed, not implemented;
native declarations remain #71. [UI](TERMS.md#ui) terminal behavior is now
specified in the [terminal contract](terminal-integration.md); selector layout
is in the [selector specification](../monitor-selector.md), and lifecycle policy
in the [lifecycle specification](../monitor-lifecycle.md). All are unimplemented.

[Viewing](../../GLOSSARY.md#status-and-attention) is reported per
[attachment](../../GLOSSARY.md#attachment-and-control);
retained control alone is not viewing. Merely being visible does not
acknowledge attention. The planned [viewing operation](monitor-client/request-viewing.md)
and [status reports](monitor-client/event-session-status.md) map the agreed wire
without assigning native declarations. Menus clear this monitor's viewing
without releasing control/restoration. Only this monitor's own unobstructed
presentation suppresses its toasts; another viewer does not.
The [terminal rules](terminal-integration.md) also gate viewing on a live
viewport, usable painted canvas and reportable host focus. Nonmodal coverage
clears viewing but retains ready keyboard input; host focus-in/toast disappearance
never acknowledges. Application focus uses optional non-user core Focus.

The monitor retains a per-session `climon2-vt` model and bounded local scrollback
for disconnected read-only inspection. Overlay exactly `Reconnecting...` while
automatic passive recovery is scheduled or active. Permanent rejection instead
shows an explicit disconnected/error overlay. Connected revoked cache has no
inspection route; reconnect removes the overlay but leaves a non-controller
placeholder until explicit acquisition. No prefix+h/prefix+r binding is added.
Automatic exponential backoff covers uncertain transport loss, recoverable
startup failure (including missing libraries), and safe worker panics.
Permanent protocol/interface rejection, invalid configuration and definitive
completion stop retry. Each fresh attempt waits for prior instance cleanup/
join; never overlap replacement instances or replay indeterminate work.
Reconnect never automatically takes control; cached output/status is not proof
of a live or completed session.
Detailed [connection/retention rules](../session-status.md#connection-loss-and-retained-terminal-views)
preserve the existing leave policy and native-crash failure boundary.
The [selector Delete amendment](../monitor-selector.md#delete-terminate-and-remove)
reopens the map's prior killing/removal exclusion. A default-Cancel modal pins
immutable GUID identity. Confirmed removal waits for definitive termination and
local instance cleanup/join, then dismisses that GUID/cache for this run only.
Lost Kill responses keep the row and permit only passive outcome recovery, never
Kill replay. Discovery cannot re-add dismissed GUIDs in the same run. Selection/
highlight fallback after removal does not acquire control.

## Session menu titles

The menu shows the daemon's latest non-empty terminal title, falling back to
command plus arguments, never session IDs. IDs remain internal identity:
duplicate or changing titles cannot retarget selection, highlight, or
confirmation. Render titles and metadata literally, never as terminal commands.
Command, working directory, creation time, depth, and availability provide
distinguishing details; exact layout belongs to the selector decision.

The planned [title state/change interaction](monitor-client/event-title.md)
uses daemon core, not a plugin or plugin extension. When supported, it provides
the authoritative current title and gap-free ordered updates regardless of
control, menu visibility, or input readiness, without screen mirroring.
Cached titles must be marked stale/unavailable when delivery becomes uncertain.
An empty title restores the command/arguments fallback.
The permanent top row uses title/fallback as a label only, with
[display-cell](TERMS.md#display-cell)-safe
marquee solely for overflowing label text; status/control/error stay stationary
and full details remain in the selector. No host-title change is made.

Plugin implementation, configuration, and interface examples are deferred;
the earlier title-plugin release requirement is superseded. Older daemons
without title support remain usable by the monitor with command/arguments
fallback; lack of title support must not block takeover.

Users do not set terminal titles through climon2. User-assigned persisted session
names and renaming remain future work; a name will appear alongside a non-empty
terminal title, not replace or change it. Persisting renames would
require revisiting the [write-once metadata rule](../adr/0005-frozen-session-metadata-core.md).

## Retained control

Switching away retains a client and its existing control until
[revocation](../../GLOSSARY.md#attachment-and-control). Background clients receive
neither application input nor monitor resize requests. A revoked client shows a
[placeholder](../../GLOSSARY.md#attachment-and-control), not live screen mirroring,
and must not automatically take control back. Core title/status data continues
to flow regardless of control, as does planned structured notification/progress
metadata; future plugin implementation remains deferred.

## Shared in-process communication contract

**Status:** [communication design agreed in #66](https://github.com/jackgeek/climon2/issues/66),
not implemented. This section owns the common rules for every monitor-client
operation and delivery. The [embedding contract](library-embedding.md) remains
authoritative for loading and lifetime. Revision 1 is not published.

### Execution and representation

Each instance has bounded submission and delivery queues. A per-instance
thread-safe [communication handle](TERMS.md#communication-handle) lets the
monitor submit work and drain deliveries without blocking. The dedicated
monitor-created client thread executes connection work and operations; it also
creates and cleans up the instance. There are no calls into monitor application
code and no callback reentrancy. No lock-free implementation is required.

Submit, drain, and delivered-object release perform bounded local work: no
daemon [I/O](TERMS.md#io), terminal processing, waiting for queue space, or joining threads.
Copying accepted data may scale with its bounded payload; nonblocking does not
mean zero work. There is one monitor consumer per instance. The separate
thread-safe shutdown control handle remains serviceable independently of full
ordinary queues. Shutdown can interrupt ordinary execution order, but cannot
fabricate successful outcomes.

Records use a [discriminator](TERMS.md#discriminator), fixed-width values,
and length-delimited buffers across the C ABI, never Rust layouts or unbounded
C strings. Binary fields preserve every [byte](TERMS.md#byte), including
[NUL](TERMS.md#nul) and invalid [UTF-8](TERMS.md#unicode-and-utf-8).
[Native path](TERMS.md#native-path) and text encodings
remain for #71; [terminal representation/application](terminal-integration.md)
is agreed, while numeric chunk/transfer budgets remain #69.
Exact tags, widths, required/optional fields, defaults, unknown-field handling,
result codes, and declarations remain for #71. Unknown-field handling must not
silently convert invalid or unsupported core tags into extensions.

This in-process contract uses no checksum. Copies, immutable
deliveries, explicit lengths, and ownership address its relevant data-lifetime
risks; checksums provide neither memory safety nor authentication.
The [agreed daemon wire](daemon-client/core-v1.md#framing-and-validation) also
has no initial checksum; a future negotiated integrity revision is a separate
decision.

### Ownership, wakeup, and cleanup

Submit borrows arguments only during the call. Before returning local acceptance,
the library copies all accepted data. Rejected submissions retain no ownership
and produce no later [response](TERMS.md#response). Drain transfers an immutable library-owned
delivery object, including any response or [event](TERMS.md#event), which the monitor releases
through the same loaded library. No monitor allocator crosses the boundary.
Delivered-object release is thread-safe subject to coordinated instance lifetime.

All delivered objects must be released before instance destruction. Outstanding
objects explicitly prevent successful destruction rather than leaving dangling
data. Thread-safe handles do not permit concurrent destruction: stop new calls
and release deliveries before dedicated-thread cleanup completes. A closed
stream or final session event does not prove that the instance is cleaned up,
the thread returned, or the monitor joined it. Exact destruction/release
declarations remain for #71; shutdown coordination follows the
[agreed lifecycle](../monitor-lifecycle.md#coordinated-shutdown).
Prefix+x and catchable signals stop new input/acquisition/discovery/retries and
request every owned instance's shutdown concurrently through independent control
handles. Include passive, attached, partially started and failing instances.
Each attempts required atomic [Leave](daemon-client/request-leave.md), closes
and cleans workers on its owner thread. Drain/release objects, end concurrent
handle use and join every owned
thread before clean exit. Lost Leave remains unconfirmed; closure or a final
event cannot substitute. Deadline stalls fail-stop the whole process after
best-effort host restoration/root clear and diagnostics, never an individual
thread kill or running-code unload. Numerical deadlines remain #69.
Explicit exit succeeds only after required current Leave outcomes, local
cleanup/joins and host restoration succeed; it never inherits selected command
status. Fatal monitor error, required Leave failure/uncertainty, incomplete
cleanup or restoration failure uses the
[climon2 error code](../../GLOSSARY.md#installation). Catchable-signal exit uses
signal-style status after successful cleanup, otherwise that error code.
Previously resolved session-local failures alone do not poison a later clean
exit; prior transport loss remains recorded as unconfirmed, not accepted detach.

Each instance supplies a library-owned
[pollable notification descriptor](TERMS.md#pollable-notification-descriptor),
borrowed by the monitor until instance cleanup. It indicates that draining can
make progress or observe stream closure, not a record count. The monitor polls
it, but never directly reads, writes, or closes it; it calls interface drain.
The library manages race-free signaling and rearming so work or closure cannot
be missed. Cleanup coordination prevents polling a closed descriptor. This
avoids periodic polling and an extra monitor waiter thread per instance without
freezing platform-specific descriptor internals into the ABI.

### Admission, correlation, and authority

Local acceptance means validated and queued, not daemon acceptance or application
consumption. Submission explicitly distinguishes accepted, full, and closed,
and rejects malformed records, invalid length/null-buffer descriptions,
unsupported operation tags, and invalid [request IDs](TERMS.md#request-id).
Length validation does not make invalid native pointers safe.

Monitor-assigned accepted request IDs increase strictly within the instance
lifetime, without wrap. Reject an ID at or below the last accepted ID.
Locally rejected submissions do not consume an ID. The monitor serializes
per-instance submissions when allocating IDs; instances are independent.
A [high-water mark](TERMS.md#high-water-mark), not an unbounded used-ID set,
is sufficient. Exhaustion explicitly requires a new instance; exact width
remains for #71. IDs are not [idempotency keys](TERMS.md#idempotency-key) and
provide no retry mechanism.

Every accepted [request](TERMS.md#request) has one correlated eventual success/failure outcome
while communication remains healthy. No locally rejected request has a later
outcome. Execution follows accepted submission order except independent
shutdown. Runtime execution rechecks connection, generation, control, and
restoration authority that may have changed since admission. Known unexecuted
stale work gets an explicit asynchronous rejection; uncertain execution is
[indeterminate](TERMS.md#indeterminate-outcome). Never silently discard stale work or reassign it to new authority.

### Delivery order and bounded progress

Each instance has one [FIFO](TERMS.md#fifo) response-and-event stream preserving
causal control, restoration, and result order. Initial title/status and later
changes have no subscription gap. There is no ordering between instances.
Receiving restoration completion does not prove the monitor applied it.

Full queues apply [backpressure](TERMS.md#backpressure) to ordinary production
and socket reading as needed.
Required records cannot be dropped, overwritten, or reordered. In particular,
status transitions, attention episodes, responses, and control/restoration
boundaries cannot be lossily coalesced. Other clients and global monitor
actions remain responsive; shutdown and failure detection remain serviceable.

Bounds account for bytes, records, and drained-but-unreleased objects.
Draining without releasing must not permit indefinite growth. If the progress
policy in #69 cannot be met, fail explicitly instead of growing indefinitely.
Stream closure/failure reason is observable outside ordinary queue capacity;
already queued records remain ahead of the terminal indication when draining.
An instance stream's closure is distinct from recoverable daemon connection
loss. This contract does not require reconnect to close/reopen the stream.

### Capabilities and connection generations

After interface-revision and exact-release validation, the instance reports
[capabilities](../../GLOSSARY.md#sessions) actually available through both library
and daemon. Required support is checked before control acquisition and operation
submission. Unsupported operations fail explicitly. Optional title/status
support is reported unavailable/unknown, never fabricated as an empty title or
cleared attention. A supported empty title remains valid and selects the
command-name fallback. Optional core `focus-v1` enables
[application focus](monitor-client/request-focus.md); missing library/daemon
support is a visible focus limitation, not a reason to block an otherwise usable
session. It is not a plugin extension or release-number inference.
Lifecycle uses the existing agreed Leave/Kill/control contract; this amendment
assigns no new capability or operation schema.

Unknown core operation/event tags are invalid, not implicit extensions.
Future [namespaced](TERMS.md#namespaced-name) extensions require explicit name/revision negotiation of the
intersection including relay support; their [APIs](TERMS.md#api) and delivery implementation
remain deferred.

Each instance distinguishes a
[connection generation](TERMS.md#connection-generation) from request IDs,
[control periods](../../GLOSSARY.md#attachment-and-control),
[restoration IDs](../../GLOSSARY.md#protocol-and-plugin-concepts), and attention episodes. Unfinished work from
a lost connection is indeterminate, with no automatic retry/replay. Queued
authority-sensitive work for the old generation never crosses to the new
connection. Reconnect is passive; retained metadata and the monitor-owned
terminal model remain stale until refreshed. Application input requires fresh
control, restoration, and monitor-applied barriers. Confirmed final command
outcomes remain distinct from loss and stop reconnecting.

For each generation, explicitly delimit initial core metadata synchronization
from subsequent changes. Report current title, effective status, and stored
UTC change time, or an explicit unsupported indication. Supply reliable attention
episode and final-outcome identities; UTC timestamps are not unique identities.
This gap-free metadata barrier is independent of control, readiness, and
selection, and never makes stale terminal content fresh. Exact event forms,
timestamps, identities, and daemon mapping are specified by
[wire revision 1](daemon-client/core-v1.md); native representation remains #71.
The [lifecycle contract](../monitor-lifecycle.md#history-and-passive-recovery)
owns exponential passive recovery, recreation and cleanup. Numerical schedules
and budgets remain #69. Pause new retry scheduling during shutdown or confirmed
termination/removal; passive outcome recovery may establish an indeterminate
Kill result without replay.

### Semantic examples

These are valid behavioral timelines, not assigned calls or serialized records.
Every operation page inherits them. Executable C call examples and schema
fixtures cannot be published before #71 completes the declarations.

| Situation | Required observations |
|---|---|
| Library compatibility | A library supports the requested revision but reports the wrong session release. Reject it before creating an instance or daemon connection; retain the opened library until monitor exit. Supporting the revision does not waive exact-release validation. |
| Local admission | ID 10 is accepted and copied. ID 11 is rejected as full: no ownership or later outcome. Retrying local admission with ID 11 can succeed; submitting ID 10 again cannot. |
| Literal binary data | A supported binary submission contains hexadecimal `00 ff 41`; its accepted copy preserves all three bytes. Display deliveries preserve exact ordered terminal bytes too. Neither NUL nor invalid UTF-8 truncates or changes them. This does not imply input echo or direct host output. |
| Control and restoration | A [grant](../../GLOSSARY.md#attachment-and-control) precedes its restoration start, ordered updates, and completion in the single stream. The complete current restoration replaces the model/history; application permits Ready without selection/paint, while selected input additionally waits for coherent paint. Revocation invalidates readiness. |
| Passive metadata | A generation's initial title/status synchronization is delimited, then changes follow gap-free while unselected or revoked. Unchanged status retains its stored UTC time; a new attention episode uses its own identity, not timestamp uniqueness. |
| Resize and viewing | Only the selected [controller](../../GLOSSARY.md#attachment-and-control) receives monitor resize requests; local acceptance is not actual geometry. Report viewing once ready and unobstructed. Switching away or opening menus clears viewing without releasing control/restoration. Explicit selection/focus or menu close/cancel may separately acknowledge current pending attention once ready and viewing; opening/browsing never acknowledges. Cancel deferred acknowledgement if target context changes. Visibility alone never acknowledges a new request. |
| Menu notifications | Attention arriving while a menu is open toasts immediately over it without a daemon round trip. Still-pending attention never toasted becomes eligible on menu opening. Deduplicate per episode over this monitor lifetime, including reconnects. A final first learned viewed stays suppressed forever here; another client's viewing never suppresses this monitor. |
| Canonical geometry | Outer host 600x251 gives desire 600x250; daemon effective size 500x200 is used for every monitor-mode display/restoration byte. Paint that rectangle below the top row with blank margins; ordinary attach still uses its own direct/derived host policy. |
| Focus | Host focus loss clears viewing, not control/readiness. Dedicated non-user Focus stores current desired application focus even with reporting off; enabled reporting emits matching bytes once, never attention acknowledgement. Nonmodal toasts/scrollback retain keyboard focus; modal menus do not. |
| Attention jump/hints | Configured prefix + lowercase j chooses the newest currently connected needs_attention row at keypress, not a toast's saved target. Close overlays, acquire only if needed, apply/paint, then acknowledge when unobstructed/ready. No eligible row keeps selection and reports it. Planned show_key_hints hides implicit hints, not bindings or application text. |
| Succession | On any controller departure, most-recent-control existing attachment wins first; otherwise newest accepted passive registration is promoted before final-participant policy. Inheritance does not select, resize, send input or acknowledge. Promoted membership persists through switching/revocation. Committed termination overrides succession. |
| Core notification/progress | A recognized logical notification requests attention; repeated requests while pending are not new episodes. Progress metadata alone does not request attention or confirm command completion. Delivery remains independent of control and does not permit screen mirroring. |
| Deliberate detach | Local acceptance queues detach. Only daemon acceptance confirms deliberate detach; neither that outcome nor stream closure proves instance cleanup or a joined thread. No timeout or shutdown fabricates success. |
| Loss during work | An accepted unfinished request on the lost generation is indeterminate. Passive reconnect does not replay it or carry old-generation input forward. Retained terminal content stays stale until fresh control/restoration/application. |
| Stalled consumer | Capacity includes drained unreleased objects. Ordinary production backpressures; other instances and shutdown progress. Closure reason remains observable, with queued records drained before the terminal indication. Objects are released and polling/calls coordinated before cleanup; closure alone is not a join. |

### Required future evidence

These are specification requirements, not checks passed by current code:

- Automated public-ABI evidence with multiple same-release instances and
  simultaneous mixed-release libraries, including explicit unsupported/rejection
  behavior and ownership on accepted, rejected, and outstanding deliveries.
- Causal ordering and applied-restoration readiness, passive initial metadata
  and changes, reconnect generations, attention/final-outcome identities,
  arbitrary-byte integrity, stall/backpressure isolation, and truthful failure,
  detach, cleanup, and thread-completion reporting.
- Representative input/display latency, event throughput, memory use, and
  global responsiveness measurements under normal operation, load, and stalls.
  Numerical thresholds belong to #69.
- Exact valid executable examples and schema fixtures in #71 after #67, #63,
  #61, and #69 settle their contracts. Documentation-only checks are not
  evidence of a working ABI.

### Remaining decision boundaries

The queue, ownership, notification, admission, ordering, and generation rules
above are agreed; they are not placeholders for #71 to redesign. Agreed
contracts and remaining open decisions are distinguished below:

| Owner | Agreed contract or unresolved details |
|---|---|
| [Daemon wire #67](https://github.com/jackgeek/climon2/issues/67) | Agreed [wire revision 1](daemon-client/core-v1.md), not implemented. No remaining schema decision is delegated back to #67. |
| [Terminal integration #63](https://github.com/jackgeek/climon2/issues/63) | Agreed [terminal contract](terminal-integration.md), including canonical monitor geometry and optional core Focus; not implemented. Numeric transfer budgets remain #69 and complete native declarations #71. |
| [Lifecycle #61](https://github.com/jackgeek/climon2/issues/61) | Agreed [lifecycle contract](../monitor-lifecycle.md), not implemented: exponential passive recovery, concurrent truthful shutdown, bounded whole-process fail-stop, Delete/removal and headless lifetime. Numerical schedules remain #69; no general native-crash survival guarantee. |
| [Operational limits #69](https://github.com/jackgeek/climon2/issues/69) | Numeric byte/record/object and aggregate bounds, progress/stall deadlines, scheduling/admission/exhaustion policy, transfer budgets, and measurement thresholds. |
| [Interface schemas #71](https://github.com/jackgeek/climon2/issues/71) | Exact native declarations, record fields/tags/widths, handle and release operations, descriptor exposure, native capability reporting, timestamp/identity encodings, native error catalog, schema fixtures, and valid executable examples after the remaining decisions. It remains blocked; agreed wire focus-v1 and terminal semantics are not delegated back for redesign. |

## Compatibility, errors, and limits

The agreed library design uses a multi-instance, cross-release-safe embedding
interface and the communication contract above; complete declarations remain
to be specified.
Exact-release executable/library matching alone is not sufficient when a monitor
loads libraries from several releases simultaneously. Never substitute a
different installed release or
silently ignore a missing library.

Preserve structured operations and explicit outcomes; terminal output is not a
control interface. A grant is not [readiness](../../GLOSSARY.md#protocol-and-plugin-concepts),
input acceptance is not application consumption, and daemon-accepted
[detach](../../GLOSSARY.md#attachment-and-control) is not completed client cleanup.
Communication failure leaves unfinished outcomes
[indeterminate](TERMS.md#indeterminate-outcome); it does not authorize retries
or fabricated success.

Use bounded per-client and monitor-wide queues with
[backpressure](TERMS.md#backpressure), explicit failure reporting, and responsive
monitor actions. Numerical budgets, startup concurrency, deadlines, and
admission/exhaustion policies belong to
[Define monitor operational limits](https://github.com/jackgeek/climon2/issues/69).
No monitor-client frame maximum or request-ID width is currently assigned.

## Future plugin authors

Plugins are anticipated in the monitor and daemon, not the client library.
The client relays negotiated [extensions](../../GLOSSARY.md#protocol-and-plugin-concepts)
without interpreting plugin-owned [payloads](TERMS.md#payload) as core commands.
Support must be explicitly negotiated by
[namespaced name](TERMS.md#namespaced-name) and revision, including relay support.
Required support must be checked before control acquisition; missing optional
support disables that extension. Exact negotiation schemas remain unresolved.
These future extension requirements do not require implementing plugins now.
Core title/status and structured notification/progress delivery do not use
plugin negotiation.

Bidirectional extension [requests](TERMS.md#request), outcomes, and [events](TERMS.md#event) must work for passive,
background, and revoked clients as well as the selected
[controller](../../GLOSSARY.md#attachment-and-control). Receiving
plugin data does not require control: an attention status must reach the
monitor even while another attachment controls that session. This does not
permit core screen mirroring, application input, or resize without control.

Preserve core ordering, readiness, control, truthful outcomes, and resource
limits. Injected work needs its own correlation and outcome; removal before
execution requires explicit rejection. Replacement work must account truthfully
for the original outcome. [Responses](TERMS.md#response) and essential state changes cannot silently
disappear. The shared in-process contract preserves these guarantees without
assigning a plugin API.

Future daemon plugins need distinct [interception seams](TERMS.md#interception-seam)
for PTY traffic and structured daemon/socket traffic, observation before
filtering, and read-only terminal snapshots. The agreed OSC 7501, OSC 133,
OSC 9/99/777, input, and viewport strategies now belong to daemon core.
[BEL](TERMS.md#bel) and other future plugin strategies remain motivating
examples, not implemented hooks or frozen plugin schemas.
Plugin interfaces, examples, implementation, configuration, and
[sandboxing](TERMS.md#sandboxing) are deferred. The agreed core
[attention strategy and toast behavior](../session-status.md) are in scope;
additional detection strategies and plugin delivery are not prerequisites.

[QUIC](TERMS.md#quic), request-retry deduplication, outcome caches, and replay
policies remain out of scope. Passive connection retry is in scope; it is not
permission to retry indeterminate operations. [Request IDs](TERMS.md#request-id)
alone do not provide retry safety or exactly-once execution.

## Operation and event index

### Monitor-client: semantic requirements; interface schemas unresolved

All entries inherit the [shared communication contract](#shared-in-process-communication-contract)
and its semantic examples. They describe agreed but unimplemented in-process
interactions, not assigned calls or wire messages. Pages must gain complete
schemas and executable examples when #71 completes the interface.

| Operation or event | Direction | Purpose |
|---|---|---|
| [Request](monitor-client/request.md) | Monitor -> client; extensions either way | Identify and correlate work. |
| [Response](monitor-client/response.md) | Opposite its request | Report one explicit outcome. |
| [Event](monitor-client/event.md) | Client -> monitor; extensions either way | Report unsolicited changes. |
| [Compatibility check](monitor-client/request-hello.md) | Monitor <-> library | Validate release identity and embedding support. |
| [Open](monitor-client/request-open.md) | Monitor -> client | Verify daemon support and establish the intended connection role. |
| [Input](monitor-client/request-input.md) | Monitor -> client | Forward literal application bytes. |
| [Resize](monitor-client/request-resize.md) | Monitor -> client | Request controlling geometry. |
| [Take control](monitor-client/request-take-control.md) | Monitor -> client | Explicitly request takeover. |
| [Viewing](monitor-client/request-viewing.md) | Monitor -> client -> daemon | Report selected presentation independently of retained control. |
| [Readiness](monitor-client/request-ready.md) | Monitor -> client -> daemon | Report current complete restoration applied. |
| [Acknowledge attention](monitor-client/request-acknowledge.md) | Monitor -> client -> daemon | Explicitly acknowledge current pending attention. |
| [Application focus](monitor-client/request-focus.md) | Monitor -> client -> daemon | Optional non-user core focus state/forwarding report. |
| [Detach](monitor-client/request-detach.md) | Monitor -> client | Request deliberate detach. |
| [Kill](monitor-client/request-kill.md) | Monitor -> client | Confirm command termination and daemon cleanup through a standalone role. |
| [Extension request](monitor-client/request-extension.md) | Either endpoint, through client | Request negotiated plugin work regardless of control. |
| [Control](monitor-client/event-control.md) | Client -> monitor | Report grant or revocation. |
| [Restoration start](monitor-client/event-restoration-start.md) | Client -> monitor | Begin a distinct restoration. |
| [Display](monitor-client/event-display.md) | Client -> monitor | Deliver ordered presentation updates. |
| [Restoration completion](monitor-client/event-restoration-complete.md) | Client -> monitor | End the identified restoration. |
| [Geometry](monitor-client/event-geometry.md) | Client -> monitor | Report actual geometry. |
| [Title state/change](monitor-client/event-title.md) | Daemon core -> client -> monitor | Supply current daemon title and ongoing changes regardless of control. |
| [Session status](monitor-client/event-session-status.md) | Daemon core -> client -> monitor | Supply effective activity/attention status and its change timestamp. |
| [Notification metadata](monitor-client/event-notification.md) | Daemon core -> client -> subscriber | Relay a recognized logical notification independently of control. |
| [Progress metadata](monitor-client/event-progress.md) | Daemon core -> client -> subscriber | Relay progress information without implying attention or control. |
| [Session end](monitor-client/event-session-ended.md) | Client -> monitor | Report command/session completion. |
| [Client outcome](monitor-client/event-outcome.md) | Client -> monitor, when possible | Report client completion or failure. |
| [Extension event](monitor-client/event-extension.md) | Either endpoint, through client | Report negotiated plugin information regardless of control. |

### Daemon-client: current frozen hello

| Message | Direction | Purpose |
|---|---|---|
| [Hello request](daemon-client/hello-request.md) | Client -> daemon | Begin the frozen handshake. |
| [Hello reply](daemon-client/hello-reply.md) | Daemon -> client | Identify the session and advertise support/state. |

The [post-hello page](daemon-client/post-hello.md) distinguishes current private
messages from the agreed wire revision.

### Daemon-client: agreed wire revision 1, not implemented

Every page inherits the [complete shared wire rules](daemon-client/core-v1.md):
portable framing, strict JSON/binary types, IDs, capabilities, ordering,
acceptance/errors, bounded delivery and unresolved owners. Hello advertises
support; first Open/Kill validates before effects without a separate exchange.
There are ten request types and fourteen server types, including Response.
Examples are wire bodies, not native declarations or conformance evidence.

| Code | Message | Direction | Purpose |
|---|---|---|---|
| `0x30` | [Open](daemon-client/request-open.md) | Client -> daemon | Validate revision/families and enter monitor or interactive role. |
| `0x31` | [Input](daemon-client/request-input.md) | Client -> daemon | Forward literal nonempty user bytes with authority IDs. |
| `0x32` | [Resize](daemon-client/request-resize.md) | Client -> daemon | Request geometry under current authority. |
| `0x33` | [Acquire](daemon-client/request-acquire.md) | Client -> daemon | Explicitly acquire control, not acknowledgement or resize. |
| `0x34` | [Ready](daemon-client/request-ready.md) | Client -> daemon | Report complete current restoration applied. |
| `0x35` | [Set viewing](daemon-client/request-viewing.md) | Client -> daemon | Report this attachment's unobstructed presentation. |
| `0x36` | [Acknowledge attention](daemon-client/request-acknowledge.md) | Client -> daemon | Acknowledge currently pending attention without observed episode ID. |
| `0x37` | [Leave](daemon-client/request-leave.md) | Client -> daemon | Remove membership/registration and accept departure. |
| `0x38` | [Kill](daemon-client/request-kill.md) | Client -> daemon | Standalone termination with daemon-cleanup confirmation. |
| `0x39` | [Focus](daemon-client/request-focus.md) | Client -> daemon | Optional non-user core application focus (#63 amendment). |
| `0x40` | [Response](daemon-client/response.md) | Daemon -> client | Correlated explicit outcome/result/error. |
| `0x41` | [Initial metadata](daemon-client/event-initial-metadata.md) | Daemon -> client | Atomic current availability/state before gap-free changes. |
| `0x42` | [Title changed](daemon-client/event-title-changed.md) | Daemon -> client | Inert [Unicode](TERMS.md#unicode-and-utf-8) title. |
| `0x43` | [Status changed](daemon-client/event-status-changed.md) | Daemon -> client | Effective state, stored time and episode. |
| `0x44` | [Notification](daemon-client/event-notification.md) | Daemon -> client | Normalized source metadata, not an archive. |
| `0x45` | [Progress changed](daemon-client/event-progress-changed.md) | Daemon -> client | Retained normalized progress. |
| `0x46` | [Control granted](daemon-client/event-control-granted.md) | Daemon -> client | New authority, including hand-back/successor. |
| `0x47` | [Control revoked](daemon-client/event-control-revoked.md) | Daemon -> client | Invalidate old authority immediately. |
| `0x48` | [Restoration begin](daemon-client/event-restoration-begin.md) | Daemon -> client | Start a distinct restoration. |
| `0x49` | [Display](daemon-client/event-display.md) | Daemon -> client | Ordered nonempty binary terminal bytes. |
| `0x4a` | [Restoration complete](daemon-client/event-restoration-complete.md) | Daemon -> client | Sent barrier, not applied readiness. |
| `0x4b` | [Geometry](daemon-client/event-geometry.md) | Daemon -> client | Actual daemon geometry. |
| `0x4c` | [Command final](daemon-client/event-command-final.md) | Daemon -> client | Authoritative exit/signal and selected terminal status. |
| `0x4d` | [Fatal error](daemon-client/event-fatal-error.md) | Daemon -> client | Best-effort connection failure diagnostic. |

## Sources and unresolved decisions

- [Define monitor client ownership and control](https://github.com/jackgeek/climon2/issues/58):
  canonical thread-based hosting, eager clients, passive connections, and control;
  later core-status mechanics defer plugin implementation.
- [Define monitor startup and session discovery](https://github.com/jackgeek/climon2/issues/59):
  agreed startup/discovery integration, revised status/time ordering and core
  title/status reporting; not implemented, exact schemas still pending.
- [Define version-matched client library loading and lifetime](https://github.com/jackgeek/climon2/issues/65):
  agreed [multi-instance embedding, compatibility, and lifetime](library-embedding.md);
  not implemented, complete declarations still pending.
- [Define monitor-client communication contract](https://github.com/jackgeek/climon2/issues/66):
  agreed, unimplemented queues, tagged records, ownership, notification,
  correlation/order, validation, compatibility, bounded progress, core metadata,
  and future evidence requirements above; plugin implementation remains deferred.
- [Complete monitor-client interface schemas](https://github.com/jackgeek/climon2/issues/71):
  remains open, with numerical limits still unresolved; owns exact fields,
  calls/events, results, errors, and examples against the agreed lifecycle.
- [Freeze daemon-client communication contract](https://github.com/jackgeek/climon2/issues/67):
  agreed [revision-1 post-hello wire](daemon-client/core-v1.md), passive
  succession, metadata, outcomes and validation; no implementation claim.
- [Choose the monitor terminal integration contract](https://github.com/jackgeek/climon2/issues/63):
  agreed [terminal/input/geometry/restoration contract](terminal-integration.md),
  optional core Focus and #64/#59 input/config amendments; not implemented.
- [Define monitor exit and failure behavior](https://github.com/jackgeek/climon2/issues/61):
  agreed [lifecycle](../monitor-lifecycle.md), not implemented: recovery,
  concurrent shutdown/fail-stop, truthful outcomes, selector removal and
  headless zero-client lifetime.
- [Define monitor operational limits](https://github.com/jackgeek/climon2/issues/69):
  numerical bounds, progress, admission, and exhaustion.
- [Final automated monitor acceptance](https://github.com/jackgeek/climon2/issues/62):
  remains open; executable evidence must include the lifecycle obligations.

The implemented hello remains specified by
[current framing/messages](../../crates/climon2-core/src/protocol.rs),
[hello extensions](../../crates/climon2-core/src/hello.rs), and
[its ADR](../adr/0001-version-matched-client-and-frozen-hello.md).
The [protocol-docs skill](../../.agents/skills/protocol-docs/SKILL.md) owns
synchronized updates, terminology links, examples, and completeness requirements.
