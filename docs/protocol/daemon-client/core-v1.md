# Daemon wire revision 1

**Status: developer-agreed specification in [#67](https://github.com/jackgeek/climon2/issues/67), partly implemented.**
This is the maintained contract, not evidence that any release conforms.
The [#63 terminal contract](../terminal-integration.md) adds optional core
[Focus](request-focus.md) and canonical monitor-mode presentation below;
these amendments are also agreed, and Focus is implemented.
The [private protocol](post-hello.md) is not this revision;
[session daemons](../../../GLOSSARY.md#sessions) now speak both, and the
first [frame](../TERMS.md#frame) after [hello](../../../GLOSSARY.md#sessions)
selects one.
Use the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md) alongside each message page.

An encoder and validator for the [framing](../TERMS.md#frame), the shared
types and all ten [request](../TERMS.md#request) and fourteen server
messages of this revision, including the binary [Input](request-input.md)
and [Display](event-display.md) records, now exists in
[`climon2-core`](../../../crates/climon2-core/README.md). Its tests decode
every [JSON](../TERMS.md#json) and binary example published on the message
pages and re-encode it to identical [bytes](../TERMS.md#byte).

The [session daemon](../../../GLOSSARY.md#sessions) implements this revision
for [passive monitor connections](../../../GLOSSARY.md#attachment-and-control)
([#86](https://github.com/jackgeek/climon2/issues/86)), ordinary
interactive [attachments](../../../GLOSSARY.md#attachment-and-control)
([#87](https://github.com/jackgeek/climon2/issues/87)) and standalone
[Kill](../../../GLOSSARY.md#attachment-and-control)
([#88](https://github.com/jackgeek/climon2/issues/88)): the
[hello advertisement](hello-reply.md#revision-1-advertisement), framing and
validation with their connection-fatal codes, Open in both modes, Leave,
Input, Resize, Ready, Acquire between attachments, Focus, Kill, Response,
Fatal error, Initial metadata (the title and a known command result; status
and progress reported unavailable), Title changed, Control granted and
revoked, Restoration begin and complete, Display, Geometry and
[Command final](event-command-final.md#implemented-behavior) without status,
with the private protocol's takeover,
[hand-back](../../../GLOSSARY.md#attachment-and-control) and
[leave policy](../../../GLOSSARY.md#attachment-and-control). Until it serves them, it
rejects the other
requests without effects, as each request's page describes. Acquire by a
passive monitor connection, Set viewing, Acknowledge, the status,
notification and progress messages, promotion of passive registrations,
canonical monitor-mode presentation and bounded delivery remain
unimplemented, and no
[client](../../../GLOSSARY.md#sessions) speaks this revision yet. The
[daemon's contract tests](../../../crates/climon2-daemon/README.md#daemon-wire-revision-1-acceptance-scenarios)
are the evidence for the implemented parts.

## Transport and startup

The [client](../../../GLOSSARY.md#sessions) communicates with the
[session daemon](../../../GLOSSARY.md#sessions) over an ordered logical stream.
Core traffic is independent of its carrier. Today's implementation uses local
private sockets with same-[UID](../TERMS.md#uid) checks and a private
[runtime directory](../../../GLOSSARY.md#sessions); these are local deployment
requirements, not universal future transport guarantees.

Future [monitor](../../../GLOSSARY.md#sessions)-to-monitor forwarding over
[TCP/IP](../TERMS.md#tcpip) will present forwarded
[sessions](../../../GLOSSARY.md#sessions) as local sessions at the downstream
monitor. Future [WebSocket](../TERMS.md#websocket) forwarding will feed an
xterm.js browser. Routing, multiplexing, discovery, remote authentication and
security remain deliberately unresolved. No forwarding implementation or
native library running inside a browser is implied.

The frozen [hello handshake](../../../GLOSSARY.md#sessions) remains type `0x01`,
protocol 1, with its existing required fields, limits, and unknown-optional-field
behavior. [Hello reply](hello-reply.md#revision-1-advertisement)
adds `daemon-wire-v1` and a `daemon_wire_v1` limits object.
Missing or malformed support makes this revision unavailable, not the session
stale. Existing hello tokens are not new wire promises.

Hello advertises support; the first [Open](request-open.md) or
[Kill](request-kill.md) validates revision and required
[capabilities](../../../GLOSSARY.md#sessions) before effects. There is no
redundant negotiation exchange. Exact-release client/library/daemon matching
remains mandatory and separate from support negotiation. Release numbers never
imply support. Initial role rejection has no effects and ends that attempted
role stream; later Open/Kill cannot repurpose an existing role.
Until the private protocol is retired, the first frame after hello selects
the protocol by its type, as the [post-hello boundary](post-hello.md)
describes.

## Framing and validation

Every later [frame](../TERMS.md#frame) is
`u32be length | u8 type | body`. Length counts the type [byte](../TERMS.md#byte)
and body, not the four-byte prefix. Input and Display have the binary headers
on their pages; all other bodies are complete [UTF-8](../TERMS.md#unicode-and-utf-8)
[JSON](../TERMS.md#json) objects. Partial reads/writes do not define messages.
All control records occupy one bounded frame; binary payload chunking is
explicit. No application checksum is included. Only a future negotiated
integrity revision could add one if chosen.

JSON has no duplicate or unknown members, including nested records. Every field
listed by a message or variant is required, including nullable fields when null;
there are no undocumented fields or defaults. Unsigned numeric fields use
decimal integer tokens without signs, fractions, or exponents. Capability
arrays contain unique case-sensitive strings. Required and optional arrays
must not overlap; selected ordering has no meaning.

The hello advertisement separately bounds request and server frame lengths,
including type and body. Actual numeric values, queue budgets, deadlines, and
exhaustion policy are in [operational limits](../../monitor-operational-limits.md).
Illustrative advertisements do not assign defaults; the operational specification
separately assigns configurable local frame ceilings. Representable geometry is not an
operational size limit; application, clamping, rendering and resize semantics
are in the [terminal contract](../terminal-integration.md#host-canvas-geometry-and-coherent-readiness),
with numeric supported limits/policy remaining #69.

Unknown types, wrong directions, malformed/unidentifiable records, invalid
lengths and bad IDs are connection-fatal. An otherwise identifiable but
impermissible operation receives `invalid_state`. Invalid wire data is never
silently treated as an extension. An invalid application terminal sequence
may instead be ignored by its source decoder; that is not wire validation.

## Shared types

These are wire shapes, not [C ABI](../TERMS.md#c-abi) declarations.
Technical vocabulary is defined in [TERMS](../TERMS.md); domain identities in
the [glossary](../../../GLOSSARY.md#protocol-and-plugin-concepts).

| Type | Exact representation |
|---|---|
| [`Counter`](../TERMS.md#counter) | [Canonical decimal string](../TERMS.md#canonical-decimal-string), 1 through 18446744073709551615, no wrap/reuse. Binary: eight-byte [big endian](../TERMS.md#big-endian). |
| `Guid` | Lowercase canonical hyphenated [GUID](../TERMS.md#guid). New fields only; frozen hello's accepted syntax does not change. |
| [`Geometry`](../TERMS.md#geometry) | `{"cols":<positive u32>,"rows":<positive u32>}` in terminal columns/rows. |
| `Utc` | Valid [UTC](../TERMS.md#utc) `YYYY-MM-DDTHH:MM:SS.mmmZ`, actual calendar date, seconds 00–59. Not ordering or identity. |
| `CommandResult` | `{"kind":"exit","code":<u32>}` or `{"kind":"signal","number":<positive u32>,"name":<string or null>}`. |
| `Status` | `{"state":<enum>,"changed_at":<Utc>,"episode_id":<Counter or null>}`. |
| `Progress` | `{"state":<enum>,"percent":<integer 0..100 or null>}`. |
| `Available<T>` ([availability](../TERMS.md#availability)) | `{"available":<bool>,"value":<T or null>}`. Unavailable requires null. Available status/progress require a value; available title permits null (unset), empty string, or nonempty string. |
| `Error` | `{"code":<nonempty stable code>,"message":<diagnostic string>}`. Unknown codes remain explicit non-successes; never imply retry. Text is not terminal output. |

`Status.state` is `idle`, `running`, `needs_attention`, `acknowledged`,
`completed`, or `failed`. Only the two
[attention](../../../GLOSSARY.md#status-and-attention) states have a nonnull episode.
[Episodes](../../../GLOSSARY.md#status-and-attention) increase per session,
are nonzero, coalesce while pending, survive
acknowledgement, and become null when cleared. A subsequent request after
acknowledgement starts a new episode. Effective-state change time is stored
daemon time, not connection time. The initial effective state uses the daemon's
initialization UTC time; only an effective-state change replaces it.
Reconnect and hidden detector changes do not update it.
See the unchanged [detector mechanics](../../session-status.md).

`Progress.state` is `none`, `normal`, `indeterminate`, `error`, or `paused`.
`normal` requires percent; `none`/`indeterminate` require null; `error`/`paused` allow a
number or null. Only the current normalized value is retained, not an archive.

Command results preserve raw unsigned 32-bit normal exits, including future
Windows values. Normal exit 143 and termination by signal 15 are different
results. Signal numbers describe the source host, not local commands to send.
Session daemons keep that distinction on this revision
([#88](https://github.com/jackgeek/climon2/issues/88)); only their
[private protocol](post-hello.md) still folds a signal into 128 plus its
number.

## Capability families

| Token | Promise |
|---|---|
| `daemon-wire-v1` | Common revision, interactive control, [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts)/[viewing](../../../GLOSSARY.md#status-and-attention), leave, final results and kill. |
| `monitor-v1` | Passive monitor entry, successor registration and promotion. |
| `title-v1` | Current title and ordered changes. |
| `status-v1` | Effective status/time/episodes and explicit attention acknowledgement, distinct from application Focus. |
| `notification-v1` | Normalized notification events; requires `status-v1`. |
| `progress-v1` | Retained normalized progress and changes. |
| `focus-v1` | Optional non-user core [application Focus](request-focus.md), with truthful state/forwarding outcomes; #63 amendment. |

Core support is implicit in a revision-1 request; monitor support is implicit
in monitor mode. Unknown optional requested families are not selected.
Unknown required families reject before effects. Required and selected
families must satisfy dependencies. Events from unselected optional families
must not be sent. Initial metadata explicitly reports unavailable families;
unsupported title is not a fabricated supported empty title.
Missing `focus-v1` keeps a session usable with an explicit focus limitation.
It is advertised in the existing optional hello capability array and selected
by Open; no new frozen-hello field or plugin negotiation is introduced.

## Correlation and context

Every request has a positive `request_id: Counter`, strictly increasing per
originating direction/logical connection. A well-formed received request
consumes its ID even if the operation is rejected.
[Public-interface request IDs](../monitor-client/request.md) are mapped, not
assumed equal: locally rejected public submissions do not consume public IDs.
Responses and events consume no IDs. IDs provide no retry safety.

Hello supplies the session identity; successful Open supplies a fresh logical
`connection_id: Guid`. Later records use that stream context instead of
repeating GUIDs. This does not assume one session per physical socket;
future physical multiplexing/routing is a separate layer.

Each received request gets one correlated [Response](response.md) while
communication remains healthy. Known unexecuted work is rejected explicitly;
lost responses leave unfinished outcomes [indeterminate](../TERMS.md#indeterminate-outcome).
There is no automatic replay/retry after reconnect. Old-generation work never
crosses to new authority.

## Membership and succession

[Passive monitor connections](../../../GLOSSARY.md#attachment-and-control)
always register as eligible
[successors](../../../GLOSSARY.md#attachment-and-control), but are not
[attachments](../../../GLOSSARY.md#attachment-and-control) until explicit
Acquire or promotion. There is no observer-only option or eligibility flag.
Registration alone never takes over a detached session.

Interactive Open attaches immediately and
[grants](../../../GLOSSARY.md#attachment-and-control) control using requested
geometry under daemon policy. On any
[controller](../../../GLOSSARY.md#attachment-and-control) departure, including
deliberate detach or loss, first
[hand back](../../../GLOSSARY.md#attachment-and-control) to the most recently
controlling existing attachment; otherwise promote the newest accepted passive
registration. Remove the departing connection before succession, including
promotion races. Promote before final-participant
[leave policy](../../../GLOSSARY.md#attachment-and-control).

Inheritance does not change monitor selection, resize, send input or acknowledge
attention. Promotion creates ordinary attachment membership, which persists
on switching away or [revocation](../../../GLOSSARY.md#attachment-and-control).
Old authority is invalid immediately: receipt of revocation need not precede
a new grant. Committed termination overrides succession and rejects new
interactive work; Leave and repeated standalone Kill remain possible.
This intentionally amends #58's final-attachment assumptions; it does not
guarantee survival of a native monitor crash.

`c2 exec --headless <command> [args...]` starts a
[headless session](../../../GLOSSARY.md#sessions) without an interactive client
and returns success only after daemon-confirmed creation. Zero-client survival
lasts for that session's lifetime, even after later attachments disappear,
until command completion or explicit Kill. That CLI spelling and lifetime are
[implemented](../../monitor-lifecycle.md#headless-creation-and-existing-cli-kill)
on the current private protocol: creation uses a private same-release daemon
startup argument, not a wire message, and the frozen [hello reply](hello-reply.md)
reports a headless session without attachments as `detached`. Existing
`c2 kill <session>` ends headless sessions. Under this revision,
succession stays as above for every session. The headless
[lifetime policy](../../../GLOSSARY.md#sessions) changes only what happens with
no attachment: from creation on, and after succession leaves no participant,
the session keeps running. The leave policy of
[ordinary sessions](../../../GLOSSARY.md#sessions) is unchanged, and not every
session becomes persistent.

The [lifecycle amendment](../../monitor-lifecycle.md) also agrees concurrent
shutdown and automatic exponential passive recovery. It does not change any
wire code, field, control/readiness rule or status state machine.
Required atomic [Leave](request-leave.md) proves daemon departure only, not
connection closure, final client outcome, worker cleanup or thread join.
Lost responses remain indeterminate and are never replayed, including Kill.
Monitor removal waits for definitive termination then local cleanup/join,
not merely discovery disappearance or a queued request. Cleanup stalls use
bounded whole-monitor fail-stop; numerical budgets remain #69.

## Ordering and authority

Successful Open response precedes an atomic
[Initial metadata](event-initial-metadata.md) capture/subscription, followed
gap-free by changes. Interactive grant/
[restoration](../../../GLOSSARY.md#restoration-and-presentation) follows that report.
Metadata works while passive, background or revoked, independently of
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts).
It does not refresh stale terminal content or authorize screen mirroring.

Grant precedes [Restoration begin](event-restoration-begin.md), zero or more
matching [Display](event-display.md) chunks, then
[Restoration complete](event-restoration-complete.md).
Live Display uses the same authority IDs. Metadata may interleave without
changing terminal-byte order. A newer begin or revocation invalidates previous
readiness; superseded restoration gets no fabricated completion. No
old-authority Display follows its invalidating boundary.

Resize, Ready, Set viewing and Acknowledge validate the current
[control period](../../../GLOSSARY.md#attachment-and-control) and
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts).
Ready requires complete [restoration](../../../GLOSSARY.md#restoration-and-presentation)
sent and applied by the client.
Viewing true additionally requires readiness; false can clear viewing during
restoration. Later immediate causal changes precede their correlated success
response. Deferred resize acceptance does not pretend actual geometry has
already been reported.

Monitor-mode Display/restoration is always serialized at effective session
geometry, including promotion/background grants, not host/requested geometry.
The existing Grant/Restoration begin/Geometry fields suffice with this new
[canonical policy](../terminal-integration.md#canonical-monitor-presentation-amendment-to-daemon-wire-revision-1);
no presentation-size field is added. Ordinary interactive direct/derived/
native-history behavior is unchanged.
Applied model readiness permits Ready and resize progress without selection or
paint; selected application input additionally waits for a coherent painted
current target. Focus validates current authority and readiness, but focus-out
may come from a retained background controller.

[Command final](event-command-final.md) is authoritative, distinct from
[EOF](../TERMS.md#eof). It carries terminal Status when selected, not a second
Status changed event. Zero normal exit is completed; all other exits/signals
are failed. Final identity is session GUID plus the fixed category `final outcome`, not
exit-versus-signal kind; there is one final result per session and no second
final counter. Inconsistent repeated results cannot authorize another toast.
When terminal status and result are both present, completed requires normal
exit zero and failed requires nonzero exit or signal. No later event
revives terminal activity/attention.

### Implemented authority checks

The session daemon checks Input, [Resize](request-resize.md),
[Ready](request-ready.md) and Focus in this order and rejects the first
unmet condition without effects; the stream goes on:

1. Focus only: the connection's Open selected `focus-v1`, otherwise
   `unsupported_capability`.
2. The connection has a role, otherwise `invalid_state`.
3. It is an attachment, otherwise `not_attached`: neither a passive monitor
   connection nor a standalone Kill's connection is one.
4. The session is not in committed termination, otherwise
   `session_terminating`.
5. It is the [controller](../../../GLOSSARY.md#attachment-and-control),
   otherwise `not_controller`.
6. The request's control period is the current one, otherwise
   `stale_period`.
7. Its restoration ID is the current one of that period, otherwise
   `stale_restoration`: a replacement restoration supersedes the earlier one.
8. The current restoration is far enough along: Resize needs nothing more,
   Ready needs a complete restoration, whose Restoration complete was sent,
   and Input and Focus need readiness. Otherwise `not_ready`.

Each request then checks its own arguments. Periods and restoration IDs are
session-wide and only increase: every grant starts a new period with its
first restoration, and a replacement restoration within a period, after the
controller's host changed before readiness, gets a new restoration ID.
Private-protocol attachments share the same control, so the numbers one
connection sees can skip.

## Viewing, acknowledgement and local notifications

[Viewing](../../../GLOSSARY.md#status-and-attention) means this monitor's own
unobstructed selected ready live presentation, with host focus when reportable.
Menus clear viewing and application keyboard focus without releasing
control/restoration. Nonmodal notifications covering application cells clear
viewing but retain keyboard focus and ready key input; scrollback does not
change keyboard focus. Host focus loss clears viewing, not control/readiness;
focus-in and notification disappearance never automatically acknowledge.
Without host focus reports, assumed focus is explicitly best-effort.
No aggregate viewing broadcast/schema exists.
Another client/monitor viewing never suppresses this monitor's
[toasts](../TERMS.md#toast). Per-attachment viewing is still reported to the
daemon to validate acknowledgement.

[Acknowledge](request-acknowledge.md) is distinct, with no observed episode ID.
It requires the current controller, matching period/restoration, ready state,
and latest reported viewing true; it affects the currently pending episode.
No pending episode gives explicit no-change success.
Explicit ordinary attach, user focus/selection, or menu close/cancel may acknowledge
current pending attention once ready and unobstructed. Opening/browsing menus
never acknowledges. Cancel deferred acknowledgement if local target context
changes before submission. Visibility alone never auto-acknowledges new requests.
Host focus-in and non-user Focus are not these explicit selection actions.

Attention arriving in a menu toasts immediately over it, without waiting for
menu close or a daemon round trip. Still-pending previously suppressed attention
becomes eligible when the menu opens. Deduplicate once per episode in this
monitor lifetime, including reconnects. Acknowledged/cleared is not pending.
A final result first learned while viewing stays suppressed forever in that
monitor; first learned unviewed notifies once. Later menu opening cannot
resurrect a suppressed completion toast. UTC timestamps are not episode IDs.
The [global lowercase j jump and hint configuration](../terminal-integration.md#attention-jump-and-configurable-hints)
amend #64/#59, without changing status or notification identities.
Dedicated [Focus](request-focus.md) never clears or acknowledges attention.

## Source normalization

Title/notification text is inert [Unicode](../TERMS.md#unicode-and-utf-8) JSON metadata, not
[OSC](../TERMS.md#osc) or percent-encoded terminal presentation. Invalid UTF-8
is replaced only at the text boundary; binary Input/Display bytes remain exact.
No raw notification sequences are relayed for host interpretation.

Notification records are small normalized events, not a replayed payload history
or full desktop-notification protocol implementation. Pending episode identity
is retained separately. OSC 9 text notifications use the
[iTerm2 reference](https://iterm2.com/documentation-escape-codes.html);
progress/control forms do not request attention.

For [OSC 99](https://sw.kovidgoyal.net/kitty/desktop-notifications/), optional
`i` supplies source ID; `p` selects title (default) or body; chunks concatenate;
`d` completes by default. Emit only a complete logical notification with title
or body, not queries/control/auxiliary operations. Both missing/empty is ignored.
UTF-8 replacement is a climon2 normalization choice, not an upstream recovery
guarantee.

For OSC 777 recognize `777;notify;title[;body]`, splitting the first title/body
delimiter and retaining later semicolons in body. Title-only and empty-title/
nonempty-body are accepted; both empty is ignored. This is a climon2 convention
across differing upstream implementations, not a claim to reproduce foot's
leading-delimiter tokenization or invalid-UTF-8 rejection.
Primary syntax references: [foot documentation](https://codeberg.org/dnkl/foot/src/branch/master/doc/foot-ctlseqs.7.scd)
and [foot `osc_notify`](https://codeberg.org/dnkl/foot/src/branch/master/osc.c).

Current progress source is OSC 9;4. State numbers normalize as
0 none, 1 normal, 2 error, 3 indeterminate, 4 paused.
Abbreviated OSC 9;4 without a state is accepted as clear.
Invalid/out-of-range application sequences are ignored without changing the
indicator. Normal requires a percentage. A last-numeric cache is reset by
none and updated by numeric normal/error/paused. Indeterminate ignores a supplied
number and retains the cache. Error without percentage reports unknown, not
invented zero; paused without percentage uses the cache or unknown. Only the
normalized current Progress is exposed. Unchanged values emit no event;
100 is not command completion.

These choices explicitly normalize differences between
[ConEmu](https://conemu.github.io/en/AnsiEscapeCodes.html#ConEmu_specific_OSC),
[iTerm2](https://iterm2.com/documentation-escape-codes.html), and
[Windows Terminal](https://learn.microsoft.com/en-us/windows/terminal/tutorials/progress-bar-sequences):
the latter calls state 4 warning and ignores percentage for state 3.
No full OSC 7501 root/child export or heartbeat is added; the
[detector state machines](../../session-status.md) remain unchanged.

## Errors and bounded delivery

Operational codes are `unsupported_revision`, `unsupported_capability`,
`invalid_state`, `invalid_argument`, `invalid_geometry`, `not_attached`,
`not_controller`, `stale_period`, `stale_restoration`, `not_ready`,
`not_viewing`, `session_terminating`, `resource_limit`, `io_error`,
`cleanup_failed`, and `internal_error`.
They report, respectively, unsupported revision/family, impermissible state,
invalid operation argument/geometry, missing membership/authority, obsolete
authority IDs, missing readiness/viewing, committed termination, exhausted
resources, transport failure, failed daemon cleanup, or internal failure.

Connection-fatal codes are `invalid_frame_length`, `frame_too_large`,
`truncated_frame`, `invalid_json`, `invalid_record`, `invalid_request_id`,
`unknown_message_type`, `wrong_direction`, and `internal_error`.
They distinguish invalid/oversized/truncated framing, invalid JSON/record/ID,
unknown type, wrong direction, and internal failure.
Fatal delivery is best effort; a broken stream cannot guarantee its own error
message or fabricate successful confirmation.

Queues are bounded by bytes, records and outstanding deliveries.
[Backpressure](../TERMS.md#backpressure) must preserve required healthy-stream
records, or explicitly fail the affected connection under the
[agreed operational policy](../../monitor-operational-limits.md#queues-and-transfers).
Draining without releasing cannot bypass bounds. Other connections/control
must progress independently. No required-record loss is excused by congestion.

## Remaining owners and evidence

[#71](https://github.com/jackgeek/climon2/issues/71) publishes complete
[native declarations, calls, errors and examples](../monitor-client/core-v1.md).
Map the wire to the public interface; do not copy JSON or wire IDs as an ABI.
The [#63 terminal contract](../terminal-integration.md) owns rendering/application/
resize semantics and optional Focus; [operational limits](../../monitor-operational-limits.md) supplies numeric limits, deadlines
and policy; [agreed lifecycle](../../monitor-lifecycle.md) recovery, concurrent
shutdown, cleanup/fail-stop, selector removal and headless lifetime.

Future namespaced/revisioned opaque plugin relay must preserve IDs, truthful
outcomes, authority/readiness and required records, including while revoked;
it must not permit core mirroring. There are no unnegotiated plugin types,
[API](../TERMS.md#api) declarations or examples in revision 1.

Required future public wire evidence includes frozen hello/additive support;
partial I/O, truncation, framing/JSON/IDs/directions/fields; capability
intersection; ordinary attach/resize/input/escape/detach/kill parity;
passive/successor candidates, departures and races; stale authority/readiness;
arbitrary bytes; atomic metadata/status/episode/time and source normalization;
menus/focus/local toast scope/deduplication; exit/signal versus disconnect;
kill cleanup failures; bounded stalled consumers and independent control.
The daemon's contract tests already cover the hello advertisement, framing,
JSON, ID, direction and truncation failures, capability intersection,
passive registration, Leave, atomic Initial metadata and title changes, and
for interactive attachments the record order under partial I/O, binary
Input header failures, stale authority and readiness, arbitrary bytes, paste
closure, query authority, inert titles, Focus, and takeover, hand-back and
leave-policy parity. They also cover exit versus signal in Command final,
known results in Initial metadata and first-role rejections, no final
without evidence, and standalone Kill: its answer after termination and
cleanup, Kills of both protocols joining one grace, and a partial result;
the rest remains future.
Native same/mixed-release evidence remains required by #65/#66/#71.
The [lifecycle evidence obligations](../../monitor-lifecycle.md#required-automated-implementation-evidence)
add Leave/promotion races, lost response versus closure/outcome/join, concurrent
cleanup/fail-stop and passive recovery. These are future tests, not
conformance/performance claims. Headless creation and existing CLI kill of
headless sessions already have executed evidence on the current protocol
(HDL-01 in the [acceptance matrix](../../monitor-acceptance.md)).
The [acceptance matrix](../../monitor-acceptance.md) is agreed in #62;
native declarations are agreed in #71. Numerical budgets are
agreed in [operational limits](../../monitor-operational-limits.md), not implemented.
Forwarding/security implementation remains deferred.
