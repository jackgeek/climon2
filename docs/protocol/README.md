# climon2 protocol reference

This first draft is for future [plugin authors](../../GLOSSARY.md). It describes
what is implemented, what has been agreed, and what still needs a decision.
It is not a claim that [monitor](../../GLOSSARY.md) or plugin support already exists.

Use the [domain glossary](../../GLOSSARY.md) for climon2 concepts and the
[technical terms](TERMS.md) for unfamiliar protocol vocabulary.
Every message page links back here and to its owning specification.

## Status and scope

| Interface | Status | What this draft covers |
|---|---|---|
| [Client](../../GLOSSARY.md) to [session daemon](../../GLOSSARY.md): hello | Implemented and frozen | [Request](TERMS.md#request), reply, [capabilities](../../GLOSSARY.md), and [session](../../GLOSSARY.md)-state advertisement. |
| Client to session daemon: after hello | Implemented as a version-matched private protocol; future frozen contract unresolved | [Decision boundary](daemon-client/post-hello.md), not a promised plugin interface. |
| Monitor to [client helper](../../GLOSSARY.md) | Agreed, not implemented | Hybrid [framing](TERMS.md#frame), shared [envelopes](TERMS.md#envelope), operation meanings, and [extension](../../GLOSSARY.md) rules. Several complete message [schemas](TERMS.md#schema) remain unspecified. |
| [Client executable](../../GLOSSARY.md) to same-release library | Agreed, not implemented | [Separate library decision](https://github.com/jackgeek/climon2/issues/65#issuecomment-6612616128); this is not the plugin message channel. |

The helper resolution does not assign every argument field, [event](TERMS.md#event) name, result
schema, or error code. [Schema completion](https://github.com/jackgeek/climon2/issues/71)
tracks these gaps; pages identify them instead of inventing wire
examples. A page marked **schema pending** is not sufficient to implement that
message. Filenames are navigation labels, not newly assigned wire names.

## Who communicates?

The command runs behind a [PTY](TERMS.md#pseudo-terminal-pty).

```text
monitor
  <-> client library inside the session's version-matched client helper
      <-> session daemon
          <-> command through its PTY
```

The [client helper](../../GLOSSARY.md) is isolated from the monitor and other
helpers. Its executable loads its own release's library. The monitor does not
load session [client libraries](../../GLOSSARY.md). The daemon remains the authority for session
terminal state and terminal-query replies.

Future plugins are anticipated in the monitor and daemon, not in the client.
The client relays negotiated extensions between their endpoints.

## Monitor-helper framing

**Agreed, not implemented.** Each direction uses a private pipe: the helper
reads [stdin](TERMS.md#standard-streams) and writes [stdout](TERMS.md#standard-streams). [Stderr](TERMS.md#standard-streams) is diagnostic text, not protocol traffic.
There is no helper PTY or terminal-output scraping.

```text
4-byte big-endian length | 1-byte frame kind | frame body
```

The length includes the kind and body, but not the four-[byte](TERMS.md#byte) prefix. It must be
between 1 and 1,048,576 bytes inclusive.

| Kind | Body |
|---|---|
| `0x01` | One [UTF-8](TERMS.md#unicode-and-utf-8) [JSON](TERMS.md#json) object. |
| `0x02` | Arbitrary binary bytes declared by the preceding JSON object. |

Both kinds belong to the required [baseline](TERMS.md#baseline).
Lengths count bytes, not characters. Arbitrary terminal input/output is not
converted lossily to JSON text.

### Binary payloads

A message with binary data has a top-level declaration:

```json
{"binary":{"length":2}}
```

This is a header fragment, not a complete message. It requires an immediately
following binary frame with exactly two body bytes. For example, body bytes
`00 ff` preserve a null byte and a byte that is not valid UTF-8 by itself.

- A JSON header and its binary frame form one logical message.
- Nothing may interleave between them, including plugin-injected messages.
- Process the message only after its complete declared [payload](TERMS.md#payload) arrives.
- A binary body is at most 1,048,575 bytes.
- A declared zero-length payload still requires its binary frame.
- Standalone binary frames, wrong lengths, unknown kinds, and [EOF](TERMS.md#eof) inside a pair
  fail the channel.
- Larger transfers need ordered bounded chunks defined by the owning message
  or extension. A transfer is not permission to allocate an unbounded snapshot.

Operating-system read boundaries do not define frames. One read may contain
part of a frame or several frames.

## Monitor-helper message structure

The baseline protocol is 1. JSON messages use `kind` to distinguish
[requests](monitor-helper/request.md),
[responses](monitor-helper/response.md), and
[events](monitor-helper/event.md).

Requests name an operation using `op`. Events name a notification using
`event`. A response matches the originating request using `id`.
Message-specific content lives in `args`, `result`, `error`, or `data`.

[Request IDs](TERMS.md#request-id) are positive decimal strings, strictly
increasing within each originating direction. For example, `"1"` then `"2"`.
Leading zeros and reuse are invalid. A receiver can track a [high-water mark](TERMS.md#high-water-mark)
rather than keep every historical ID. Responses may complete out of order.

[Control-period](../../GLOSSARY.md) and [restoration](../../GLOSSARY.md) counters are also [canonical decimal strings](TERMS.md#canonical-decimal-string).
Their maximum magnitude, length, and exhaustion policy are
[not yet settled](https://github.com/jackgeek/climon2/issues/69).
No default numerical limit should be inferred from this draft.

## Startup and readiness

1. Launch the exact-release helper using the existing client invocation and an
   explicit helper startup operation/settings snapshot.
2. Exchange [hello](monitor-helper/request-hello.md) and verify helper identity
   and required support. Launching or saying hello does not attach.
3. Send [open](monitor-helper/request-open.md). The helper checks daemon identity
   and required support before attaching.
4. Observe control and restoration events. A [grant](../../GLOSSARY.md) is not [readiness](../../GLOSSARY.md).
5. Apply the current complete restoration before sending application input.

The channel stays bound to one verified session. Successive restorations must
have distinct [restoration IDs](../../GLOSSARY.md) even within the same [control period](../../GLOSSARY.md).
[Revocation](../../GLOSSARY.md) invalidates readiness.

The current [startup envelope](../../GLOSSARY.md) only implements attach and [kill](../../GLOSSARY.md). The
helper-specific operation spelling and complete startup schema are pending;
do not guess an operation value from a filename or this sequence.

## Compatibility, errors, and limits

- Preserve baseline message meanings across releases. Negotiate optional
  capabilities and extensions explicitly; support is not inferred from versions.
- Accept unknown optional object fields without assigning them new meanings.
- Reject malformed framing/JSON, duplicate object keys, missing required
  fields, invalid required types, duplicate IDs, and impossible state events.
- A well-formed unknown request gets `unsupported_operation`. Unknown core
  events fail the channel.
- Request rejection and channel failure are different. A rejected request is
  not successful work; channel failure leaves unfinished outcomes [indeterminate](TERMS.md#indeterminate-outcome).
- Silence, EOF, a final message, and process exit are not interchangeable
  evidence of readiness, [detach](../../GLOSSARY.md), or cleanup.
- Use independent bounded queues and [backpressure](TERMS.md#backpressure). Required updates are not
  silently dropped. One slow helper must not disable other helpers or monitor
  commands.

The 1 [MiB](TERMS.md#mib) frame maximum is agreed. Queue budgets, parser limits, outstanding
request counts, deadlines, and admission/exhaustion policies remain with
[Define monitor operational limits](https://github.com/jackgeek/climon2/issues/69).

## Future plugin authors

Negotiate extension support using its [namespaced name](TERMS.md#namespaced-name) and explicit revision.
Effective support is the intersection of monitor support, daemon support, and helper relay support.
Discover required support before [attachment](../../GLOSSARY.md). Missing optional support disables
that extension; missing required support rejects open before [takeover](../../GLOSSARY.md).

Extensions can use [requests](monitor-helper/request-extension.md) and
[events](monitor-helper/event-extension.md) in either direction, with JSON and
declared binary payloads. The client preserves plugin-owned payloads without
interpreting them as core commands. Support is fixed for the connection.

Conforming plugins preserve core schema, ordering, readiness, control, outcome,
and resource rules. Removing a request before execution requires explicit
rejection. Injected work gets its own ID and outcome. Replacing a request with
several requires one truthful final outcome for the original. Responses and
essential state events cannot silently disappear.

Future daemon plugins need distinct [interception seams](TERMS.md#interception-seam) for PTY traffic and
structured daemon/socket traffic, plus observation before filtering and
read-only terminal snapshots. These are requirements, not implemented hooks.

An attention plugin might observe [OSC 7501](TERMS.md#osc), [BEL](TERMS.md#bel),
[OSC 9](TERMS.md#osc), terminal idleness, or activity, then notify a monitor
plugin. Detection strategies, [toast notifications](TERMS.md#toast), subscriptions, plugin loading, hook
ordering, and [sandboxing](TERMS.md#sandboxing) are not specified here.

Status extensions may define notifications while revoked. They do not authorize
live core screen mirroring, input, or resize while revoked.

[QUIC](TERMS.md#quic) transports, cross-stream ordering, reconnect deduplication, and replay
handling are future work. Baseline IDs do not provide retry safety or exactly-once
execution. See [idempotency key](TERMS.md#idempotency-key).

## Message index

### Monitor-helper: agreed, not implemented

| Message or operation | Direction | Purpose |
|---|---|---|
| [Request envelope](monitor-helper/request.md) | Monitor -> helper; extensions can go either way | Identify and correlate work. |
| [Response envelope](monitor-helper/response.md) | Opposite its request | Report one explicit outcome. |
| [Event envelope](monitor-helper/event.md) | Helper -> monitor; extensions can go either way | Report unsolicited changes. |
| [Hello request](monitor-helper/request-hello.md) | Monitor -> helper | Establish compatibility before attachment. |
| [Open request](monitor-helper/request-open.md) | Monitor -> helper | Check daemon identity/support, then attach. |
| [Input request](monitor-helper/request-input.md) | Monitor -> helper | Forward literal application bytes. |
| [Resize request](monitor-helper/request-resize.md) | Monitor -> helper | Request [controlling](../../GLOSSARY.md) geometry. |
| [Take-control request](monitor-helper/request-take-control.md) | Monitor -> helper | Explicitly request takeover. |
| [Detach request](monitor-helper/request-detach.md) | Monitor -> helper | Request deliberate detach. |
| [Extension request](monitor-helper/request-extension.md) | Either endpoint, through helper | Request plugin-defined work. |
| [Control event](monitor-helper/event-control.md) | Helper -> monitor | Report grant or revocation. |
| [Restoration-start event](monitor-helper/event-restoration-start.md) | Helper -> monitor | Begin a distinct restoration. |
| [Display event](monitor-helper/event-display.md) | Helper -> monitor | Deliver ordered [presentation](../../GLOSSARY.md) updates. |
| [Restoration-completion event](monitor-helper/event-restoration-complete.md) | Helper -> monitor | End the identified restoration. |
| [Geometry event](monitor-helper/event-geometry.md) | Helper -> monitor | Report actual geometry. |
| [Session-end event](monitor-helper/event-session-ended.md) | Helper -> monitor | Report command/session completion. |
| [Helper outcome/failure event](monitor-helper/event-outcome.md) | Helper -> monitor, when possible | Report terminal helper outcome. |
| [Extension event](monitor-helper/event-extension.md) | Either endpoint, through helper | Report plugin-defined information. |

### Daemon-client: current frozen hello

| Message | Direction | Purpose |
|---|---|---|
| [Hello request](daemon-client/hello-request.md) | Client performing a [probe](../../GLOSSARY.md) or attaching -> daemon | Begin the frozen handshake. |
| [Hello reply](daemon-client/hello-reply.md) | Daemon -> client performing a probe or attaching | Identify the session and advertise support/state. |

The [post-hello page](daemon-client/post-hello.md) explains why the existing
private messages are not yet a frozen plugin reference.

## Sources and unresolved decisions

- [Freeze monitor-helper communication contract](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936):
  canonical agreed helper behavior; some exact schemas remain unstated.
- [Complete monitor-helper message schemas](https://github.com/jackgeek/climon2/issues/71):
  exact helper fields, event names, result schemas, and error mappings.
- [Freeze daemon-client communication contract](https://github.com/jackgeek/climon2/issues/67):
  future post-hello wire schemas and end-to-end daemon extension support.
- [Choose the monitor terminal integration contract](https://github.com/jackgeek/climon2/issues/63):
  display/input representation, geometry, and chunk application.
- [Define monitor exit and failure behavior](https://github.com/jackgeek/climon2/issues/61):
  session survival, recovery, and visible failure behavior.
- [Define monitor operational limits](https://github.com/jackgeek/climon2/issues/69):
  numerical bounds and exhaustion policies.
- [Current framing and messages](../../crates/climon2-core/src/protocol.rs),
  [hello extensions](../../crates/climon2-core/src/hello.rs), and
  [handshake ADR](../adr/0001-version-matched-client-and-frozen-hello.md):
  implemented daemon handshake. That ADR's future-monitor PTY/private-protocol
  prescriptions have been deliberately reopened by the monitor decisions.

Every protocol change must update this overview, affected message pages,
examples, compatibility guidance, and affected terminology in the same change. The
[protocol-docs skill](../../.agents/skills/protocol-docs/SKILL.md) owns these
maintenance and completeness requirements.
