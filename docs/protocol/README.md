# climon2 protocol reference

This reference is for future [plugin](../../GLOSSARY.md#protocol-and-plugin-concepts)
authors. It distinguishes implemented behavior, agreed plans, and unresolved
interfaces. The [monitor](../../GLOSSARY.md#sessions) is not implemented.
Plugin implementation is deferred. Title and terminal-signal detection and
[session status](../../GLOSSARY.md#status-and-attention) belong in daemon core;
the agreed [mechanics and state diagrams](../session-status.md) are not shipped
behavior.

Use the [domain glossary](../../GLOSSARY.md) for climon2 concepts and the
[technical terms](TERMS.md) for protocol vocabulary. Each operation page links
to its owning specification. Descriptive filenames do not assign function names,
message names, field layouts, or encodings.

## Status and scope

| Interface | Status | Reference |
|---|---|---|
| [Client](../../GLOSSARY.md#sessions) to [session daemon](../../GLOSSARY.md#sessions): hello | Implemented and frozen | [Request](daemon-client/hello-request.md) and [reply](daemon-client/hello-reply.md). |
| Client to session daemon: after hello | Implemented as a version-matched private protocol; future frozen contract unresolved | [Decision boundary](daemon-client/post-hello.md). |
| Monitor to [client library](../../GLOSSARY.md#sessions) | Embedding design agreed, not implemented; communication and complete declarations unresolved | [Library embedding](library-embedding.md), [communication contract](https://github.com/jackgeek/climon2/issues/66), and [interface schemas](https://github.com/jackgeek/climon2/issues/71). |
| [Client executable](../../GLOSSARY.md#sessions) to same-release library | Same-release loading agreed, not implemented | [Library embedding](library-embedding.md). |

The embedding design selects a versioned [C ABI](TERMS.md#c-abi), but its
complete declarations and valid call examples are not yet published.
Communication does not yet have a frozen [schema](TERMS.md#schema) or
[framing](TERMS.md#frame). Do not infer a pipe protocol,
serialized [envelope](TERMS.md#envelope), frame limit, or complete cross-release
declarations from these pages.
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

Ratatui powers the planned monitor's rendering. The
[terminal integration decision](https://github.com/jackgeek/climon2/issues/63)
must define how session state reaches that renderer, including restoration,
overlays, resize, and fidelity. Choosing Ratatui does not choose the client
interface or display payload representation.

Each client has its own connection and per-session state. The
[embedding reference](library-embedding.md) specifies exact-release lookup,
revision negotiation, thread ownership, memory rules, and library lifetime.
Exact declarations, channel ownership, and callbacks or queues remain open.
No client may repurpose process-global arguments, signals, or
[standard streams](TERMS.md#standard-streams) as though it owned the monitor.

Native client crashes or aborts can terminate the entire monitor and all its
clients. Ordinary errors and recoverable panics should remain local where
possible. There is no new [session](../../GLOSSARY.md#sessions)-survival guarantee:
the existing [leave policy](../../GLOSSARY.md#attachment-and-control) can terminate
a session that unexpectedly loses its final attachment.

## Startup and control

1. Discover live sessions in deterministic newest-first order.
2. Load each required session-version library and validate identity and interface
   support before instance creation or a daemon connection.
3. Start one client per live session, with bounded concurrency. A slow or broken
   session must not indefinitely prevent older sessions from getting clients.
4. Establish passive background connections that receive core title/status and
   structured notification/progress data
   without taking control. The daemon contract must specify this connection mode;
   current ordinary attach is not a passive connection.
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
the two-second hello deadline and bounded probes. Reconcile by full session ID,
reusing starting and retained clients; discovery uncertainty never overrides an
established client's state or tears it down.

Pin the chosen initial candidate while it starts. New arrivals
receive passive clients without displacing it. Only definitive loss permits
automatic progression, at most once per candidate per pass. Other startup
failures stay visible on that candidate without silent fallback. Startup
selection succeeds only after control and the current complete restoration
have been applied. Revocation before readiness leaves a non-controlling
placeholder requiring explicit user action, not repeated automatic takeover.
After the first successful startup, selected-session loss and recovery belong
to the lifecycle decision.

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
| OSC 7501 capability query/reply | Program -> daemon -> program | Same upstream reference; daemon replies locally, and a query does not activate explicit detection. |
| OSC 133 prompt/command markers | Shell -> daemon | [Shell integration](https://learn.microsoft.com/en-us/windows/terminal/tutorials/shell-integration); [scope retirement and prompt exclusion](../session-status.md#shell-boundaries-and-prompt-rendering). |
| OSC 9 text notification | Program -> daemon | [iTerm2 sequences](https://iterm2.com/documentation-escape-codes.html); [notification policy](../session-status.md#notification-and-progress-signals). |
| OSC 9 progress/control operations | Program -> daemon | [ConEmu sequences](https://conemu.github.io/en/AnsiEscapeCodes.html#ConEmu_specific_OSC); progress is metadata, not attention. |
| OSC 99 logical notification and auxiliary operations | Program -> daemon | [kitty notifications](https://sw.kovidgoyal.net/kitty/desktop-notifications/); only completed notifications request attention. |
| OSC 777 `notify` | Program -> daemon | [Notification convention](https://sw.kovidgoyal.net/kitty/desktop-notifications/); other extension operations are not notifications. |

These are not new climon2 wire encodings. The daemon remains responsible for
terminal-query replies; raw sequences do not pass through to host terminals.
Notification detection does not promise a full desktop-notification protocol
implementation; unsupported notification queries/control operations are ignored.
The [notification](monitor-client/event-notification.md) and
[progress](monitor-client/event-progress.md) interactions relay structured
metadata through clients to subscribers, including background/noncontrolling
ones. They are distinct from terminal display updates and do not authorize
screen mirroring. Their exact climon2 schemas/capabilities and future UI remain
unresolved.

[Viewing](../../GLOSSARY.md#status-and-attention) is reported per attachment;
retained control alone is not viewing. Merely being visible does not
acknowledge attention. The planned [viewing operation](monitor-client/request-viewing.md)
and [status reports](monitor-client/event-session-status.md) still need complete
schemas.

The monitor retains a per-session `climon2-vt` model and bounded local scrollback
for disconnected inspection, and retries uncertain connections passively.
Reconnect never automatically takes control. Definitive completion stops
retrying; cached output/status is not proof of a live or completed session.
Detailed [connection/retention rules](../session-status.md#connection-loss-and-retained-terminal-views)
preserve the existing leave policy and native-crash failure boundary.

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

## Compatibility, errors, and limits

The agreed library design uses a multi-instance, cross-release-safe embedding
interface; complete declarations and communication remain to be specified.
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
No monitor-client frame maximum or [request-ID](TERMS.md#request-id) encoding is currently assigned.

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
background, and revoked clients as well as the selected controller. Receiving
plugin data does not require control: an attention status must reach the
monitor even while another attachment controls that session. This does not
permit core screen mirroring, application input, or resize without control.

Preserve core ordering, readiness, control, truthful outcomes, and resource
limits. Injected work needs its own correlation and outcome; removal before
execution requires explicit rejection. Replacement work must account truthfully
for the original outcome. [Responses](TERMS.md#response) and essential state changes cannot silently
disappear. The in-process representation of these guarantees is still open.

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

All entries describe planned in-process interactions, not implemented calls or
wire messages. Pages must gain complete schemas and valid examples when their
owning decisions settle the interface.

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
| [Detach](monitor-client/request-detach.md) | Monitor -> client | Request deliberate detach. |
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
messages from the future frozen contract and passive connection.

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
  in-process operations, core title/status/viewing, transport/representation,
  and validation; plugin implementation remains deferred.
- [Complete monitor-client interface schemas](https://github.com/jackgeek/climon2/issues/71):
  exact fields, calls/events, results, errors, and examples.
- [Freeze daemon-client communication contract](https://github.com/jackgeek/climon2/issues/67):
  future post-hello wire schemas, passive connections, and daemon extensions.
- [Choose the monitor terminal integration contract](https://github.com/jackgeek/climon2/issues/63):
  display/input representation, geometry, and restoration.
- [Define monitor exit and failure behavior](https://github.com/jackgeek/climon2/issues/61):
  client cleanup, monitor failure, session survival, and recovery.
- [Define monitor operational limits](https://github.com/jackgeek/climon2/issues/69):
  numerical bounds, progress, admission, and exhaustion.

The implemented hello remains specified by
[current framing/messages](../../crates/climon2-core/src/protocol.rs),
[hello extensions](../../crates/climon2-core/src/hello.rs), and
[its ADR](../adr/0001-version-matched-client-and-frozen-hello.md).
The [protocol-docs skill](../../.agents/skills/protocol-docs/SKILL.md) owns
synchronized updates, terminology links, examples, and completeness requirements.
