# Daemon hello reply

**Status:** implemented and frozen, with additive optional advertisements.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [daemon](../../../GLOSSARY.md) answers a valid [hello request](hello-request.md) with type `0x01`
and one [UTF-8](../TERMS.md#unicode-and-utf-8) [JSON](../TERMS.md#json) object under the same [framing](../TERMS.md#frame) and deadline.

## Required fields

| Field | Type | Meaning |
|---|---|---|
| `session_id` | String containing a valid [session ID](../../../GLOSSARY.md) | Identifies the [session](../../../GLOSSARY.md). |
| `climon2_version` | String containing a valid installed version name | Exact daemon release. |
| `protocol` | [Unsigned 32-bit integer](../TERMS.md#unsigned-integer) | Daemon protocol; currently 1. |

```json
{"session_id":"550e8400-e29b-41d4-a716-446655440000","climon2_version":"1.2.3","protocol":1}
```

A [client](../../../GLOSSARY.md) performing a [probe](../../../GLOSSARY.md) determines liveness by matching the expected session ID. It does not
require the daemon release or protocol to match its own. A version-matched client
checks session ID, protocol, and exact release before continuing.

## Optional fields

Every daemon advertises at least `capabilities` and `state`, as here; the
current daemon also adds its [revision-1 advertisement](#revision-1-advertisement):

```json
{"session_id":"550e8400-e29b-41d4-a716-446655440000","climon2_version":"1.2.3","protocol":1,"capabilities":["attach","kill","state-v1","client-startup-v1"],"state":{"lifecycle":"attached","attachments":2,"controlled":true}}
```

`capabilities` is an array of strings. Tokens are case-sensitive; unknown tokens
are ignored. Missing [capabilities](../../../GLOSSARY.md) provide no support. Current operation checks require:

| Operation/data | Required tokens |
|---|---|
| Attach | `attach` and `client-startup-v1` |
| [Kill](../../../GLOSSARY.md) | `kill` and `client-startup-v1` |
| State advertisement | `state-v1` |

These tokens do not advertise a [plugin](../../../GLOSSARY.md) system or the future [helper](../../../GLOSSARY.md) [extension](../../../GLOSSARY.md)
contract.

`state` is a point-in-time object:

| Required state field | Type | Meaning |
|---|---|---|
| `lifecycle` | String | `starting`, `attached`, `detached`, or `terminating`. |
| `attachments` | Unsigned 32-bit integer | Accepted [attachments](../../../GLOSSARY.md), excluding probes, kill requests and [passive monitor connections](../../../GLOSSARY.md#attachment-and-control). |
| `controlled` | Boolean | Whether the session has a [controller](../../../GLOSSARY.md); not which client it is. |

Starting/detached state has zero attachments and no controller. Attached state
has at least one attachment and a controller. Terminating state may retain
attachments, but zero attachments cannot have a controller.

An [ordinary session](../../../GLOSSARY.md#sessions) is `starting` until its
first attachment, and `detached` after an accepted final
[detach](../../../GLOSSARY.md#attachment-and-control). A
[headless session](../../../GLOSSARY.md#sessions) is never `starting`: it is
`detached` whenever it has no attachment, from the moment it is created and
after any later departure, including an unexpected loss. In both cases
`detached` means the session keeps running without a client. No lifecycle
value was added for headless sessions.

## Acceptance and failures

Unknown optional reply fields are ignored. Malformed required fields fail the
handshake. Malformed optional [capabilities](../../../GLOSSARY.md)/state are reported as unusable to
the caller for diagnostics, independently of other optional fields; they never
make an otherwise matching session stale. They are not silently treated as
valid support.

A snapshot can become outdated immediately. It does not [grant](../../../GLOSSARY.md) control or promise
future successful attachment. Use the [post-hello decision boundary](post-hello.md)
to distinguish the private implementation from the agreed wire revision.

This implemented `state` object does not contain the planned
[activity/attention status or its UTC change timestamp](../../session-status.md).
Those core reports use [separate wire schemas](core-v1.md); do not infer them from
the current lifecycle advertisement or silently repurpose its existing fields.

**Sources:** [implementation](../../../crates/climon2-core/src/protocol.rs),
[hello advertisements](../../../crates/climon2-core/src/hello.rs),
[hello tests](../../../crates/climon2-core/tests/hello.rs), and the daemon's
headless-session [lifecycle tests](../../../crates/climon2-daemon/tests/lifecycle.rs).

## Revision 1 advertisement

**Status:** agreed and implemented; the daemon advertises the families it
implements so far.

The [approved contract](https://github.com/jackgeek/climon2/issues/67) adds
`daemon-wire-v1` to capabilities and optional `daemon_wire_v1` with required
`max_request_frame_bytes` and `max_server_frame_bytes`: positive unsigned
32-bit integer limits counting type plus body, not the four-byte prefix.
The entire advertisement must fit inside frozen hello's existing limit.
A daemon that speaks revision 1 still sends the frozen tokens and `state`,
as in this complete reply.
These example numbers are **illustrative advertisements, not agreed defaults**;
[Operational limits](../../monitor-operational-limits.md#configuration-and-communication-of-limits)
assigns configurable local ceilings and how they interact with this advertisement.

```json
{"session_id":"550e8400-e29b-41d4-a716-446655440000","climon2_version":"1.2.3","protocol":1,"capabilities":["attach","kill","state-v1","client-startup-v1","daemon-wire-v1","monitor-v1","title-v1","status-v1","notification-v1","progress-v1","focus-v1"],"state":{"lifecycle":"attached","attachments":2,"controlled":true},"daemon_wire_v1":{"max_request_frame_bytes":1048576,"max_server_frame_bytes":1048576}}
```

Missing/malformed advertisement makes revision 1 unavailable, not the session
stale. The [hello parser](../../../crates/climon2-core/src/hello.rs) already
reports this advertisement as valid, absent or malformed, and
[its tests](../../../crates/climon2-core/tests/hello.rs) cover each case.
The [session daemon](../../../GLOSSARY.md#sessions) sends it through
[`climon2-core`](../../../crates/climon2-core/README.md), after the frozen
tokens and `state`: `daemon-wire-v1`, `monitor-v1` and `title-v1`, with
`max_request_frame_bytes` and `max_server_frame_bytes` both 1,048,576, the
release constant from the
[operational limits](../../monitor-operational-limits.md#payloads-and-parser-state).
It advertises only the families it implements; `status-v1`,
`notification-v1`, `progress-v1` and `focus-v1` follow with their
implementations. The [post-hello boundary](post-hello.md#choosing-a-protocol)
explains how the first frame after hello then chooses a protocol.
Existing optional-field behavior and required fields remain frozen;
this is not permission to apply post-hello strict unknown-field rejection to
hello. Optional family meanings are in the
[core reference](core-v1.md#capability-families), including notification's status
dependency. Existing `attach`/`kill` tokens do not promise these new schemas.
[Open](request-open.md) or standalone [Kill](request-kill.md) selects revision
and validates required support before effects, without another exchange.
Exact-release matching remains mandatory and separate.
The [#63 amendment](../terminal-integration.md) adds optional core `focus-v1`
in the same capability array, selecting [Focus](request-focus.md) through Open.
Missing support is a visible application-focus limitation without blocking an
otherwise usable session; it is not a plugin extension or release-number claim.
No new hello field, required field change, or frozen limit change is introduced.
