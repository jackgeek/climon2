# climon2 protocol reference

This reference is for future [plugin](../../GLOSSARY.md#protocol-and-plugin-concepts)
authors. It distinguishes implemented behavior, agreed plans, and unresolved
interfaces. The [monitor](../../GLOSSARY.md#sessions) is not implemented.

Use the [domain glossary](../../GLOSSARY.md) for climon2 concepts and the
[technical terms](TERMS.md) for protocol vocabulary. Each operation page links
to its owning specification. Descriptive filenames do not assign function names,
message names, field layouts, or encodings.

## Status and scope

| Interface | Status | Reference |
|---|---|---|
| [Client](../../GLOSSARY.md#sessions) to [session daemon](../../GLOSSARY.md#sessions): hello | Implemented and frozen | [Request](daemon-client/hello-request.md) and [reply](daemon-client/hello-reply.md). |
| Client to session daemon: after hello | Implemented as a version-matched private protocol; future frozen contract unresolved | [Decision boundary](daemon-client/post-hello.md). |
| Monitor to [client library](../../GLOSSARY.md#sessions) | In-process hosting agreed; embedding and communication interfaces reopened, not implemented | [Library loading and lifetime](https://github.com/jackgeek/climon2/issues/65), [communication contract](https://github.com/jackgeek/climon2/issues/66), and [interface schemas](https://github.com/jackgeek/climon2/issues/71). |
| [Client executable](../../GLOSSARY.md#sessions) to same-release library | Planned; interface unresolved | [Library decision](https://github.com/jackgeek/climon2/issues/65). |

The in-process interface does not yet have a frozen [schema](TERMS.md#schema),
[framing](TERMS.md#frame), calling convention, or valid call example. Do not infer a pipe protocol,
serialized [envelope](TERMS.md#envelope), frame limit, or cross-release interface from these pages.
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

Each client has its own connection and per-session state. Exact multi-instance
library calls, channel ownership, memory ownership, callbacks or queues, native
path handling, shutdown, and cross-release compatibility remain open decisions.
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
   support before it can connect or take control.
3. Start one client per live session, with bounded concurrency. A slow or broken
   session must not indefinitely prevent older sessions from getting clients.
4. Establish passive background connections that relay negotiated plugin data
   without taking control. The daemon contract must specify this connection mode;
   current ordinary attach is not a passive connection.
5. Acquire control for initial selection or explicit user selection/takeover.
   Enable application input only after the current complete
   [restoration](../../GLOSSARY.md#restoration-and-presentation) has been applied.

The [startup/discovery decision](https://github.com/jackgeek/climon2/issues/59)
is reopened to integrate eager clients and continuous discovery with its retained
eligibility, ordering, uncertainty, and selection rules.

Switching away retains a client and its existing control until
[revocation](../../GLOSSARY.md#attachment-and-control). Background clients receive
neither application input nor monitor resize requests. A revoked client shows a
[placeholder](../../GLOSSARY.md#attachment-and-control), not live screen mirroring,
and must not automatically take control back. Negotiated plugin data continues
to flow regardless of control.

## Compatibility, errors, and limits

The communication and library decisions must specify a multi-instance,
cross-release-safe embedding interface. Exact-release executable/library
matching alone is not sufficient when a monitor loads libraries from several
releases simultaneously. Never substitute a different installed release or
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
filtering, and read-only terminal snapshots. An attention plugin might observe
[OSC](TERMS.md#osc) 7501, [BEL](TERMS.md#bel), OSC 9, terminal idleness, or activity.
These are motivating examples, not implemented hooks or frozen plugin schemas.
Detection strategies, [toast notifications](TERMS.md#toast), plugin loading,
hook ordering, and [sandboxing](TERMS.md#sandboxing) remain outside this map.

[QUIC](TERMS.md#quic), reconnect deduplication, outcome caches, and replay policies
also remain out of scope. [Request IDs](TERMS.md#request-id) alone do not provide
retry safety or exactly-once execution.

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
| [Detach](monitor-client/request-detach.md) | Monitor -> client | Request deliberate detach. |
| [Extension request](monitor-client/request-extension.md) | Either endpoint, through client | Request negotiated plugin work regardless of control. |
| [Control](monitor-client/event-control.md) | Client -> monitor | Report grant or revocation. |
| [Restoration start](monitor-client/event-restoration-start.md) | Client -> monitor | Begin a distinct restoration. |
| [Display](monitor-client/event-display.md) | Client -> monitor | Deliver ordered presentation updates. |
| [Restoration completion](monitor-client/event-restoration-complete.md) | Client -> monitor | End the identified restoration. |
| [Geometry](monitor-client/event-geometry.md) | Client -> monitor | Report actual geometry. |
| [Session end](monitor-client/event-session-ended.md) | Client -> monitor | Report command/session completion. |
| [Client outcome](monitor-client/event-outcome.md) | Client -> monitor, when possible | Report client completion or failure. |
| [Extension event](monitor-client/event-extension.md) | Either endpoint, through client | Report negotiated plugin information regardless of control. |

### Daemon-client: current frozen hello

| Message | Direction | Purpose |
|---|---|---|
| [Hello request](daemon-client/hello-request.md) | Client -> daemon | Begin the frozen handshake. |
| [Hello reply](daemon-client/hello-reply.md) | Daemon -> client | Identify the session and advertise support/state. |

The [post-hello page](daemon-client/post-hello.md) distinguishes current private
messages from the future frozen contract and passive plugin connection.

## Sources and unresolved decisions

- [Define monitor client ownership and control](https://github.com/jackgeek/climon2/issues/58):
  canonical thread-based hosting, eager clients, passive plugin traffic, and control.
- [Define monitor startup and session discovery](https://github.com/jackgeek/climon2/issues/59):
  integrate continuous discovery and eager client startup.
- [Define version-matched client library loading and lifetime](https://github.com/jackgeek/climon2/issues/65):
  multi-instance embedding, cross-release compatibility, and resource lifetime.
- [Define monitor-client communication contract](https://github.com/jackgeek/climon2/issues/66):
  in-process operations, transport/representation, validation, and plugin relay.
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
