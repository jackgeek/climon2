# Daemon hello reply

**Status:** implemented and frozen, with additive optional advertisements.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The daemon answers a valid [hello request](hello-request.md) with type `0x01`
and one UTF-8 JSON object under the same framing and deadline.

## Required fields

| Field | Type | Meaning |
|---|---|---|
| `session_id` | String containing a valid session ID | Identifies the session. |
| `climon2_version` | String containing a valid installed version name | Exact daemon release. |
| `protocol` | Unsigned 32-bit integer | Daemon protocol; currently 1. |

```json
{"session_id":"550e8400-e29b-41d4-a716-446655440000","climon2_version":"1.2.3","protocol":1}
```

A prober determines liveness by matching the expected session ID. It does not
require the daemon release or protocol to match its own. A version-matched client
checks session ID, protocol, and exact release before continuing.

## Optional fields

The current daemon advertises `capabilities` and `state`:

```json
{"session_id":"550e8400-e29b-41d4-a716-446655440000","climon2_version":"1.2.3","protocol":1,"capabilities":["attach","kill","state-v1","client-startup-v1"],"state":{"lifecycle":"attached","attachments":2,"controlled":true}}
```

`capabilities` is an array of strings. Tokens are case-sensitive; unknown tokens
are ignored. Absence grants no support. Current operation checks require:

| Operation/data | Required tokens |
|---|---|
| Attach | `attach` and `client-startup-v1` |
| Kill | `kill` and `client-startup-v1` |
| State advertisement | `state-v1` |

These tokens do not advertise a plugin system or the future helper extension
contract.

`state` is a point-in-time object:

| Required state field | Type | Meaning |
|---|---|---|
| `lifecycle` | String | `starting`, `attached`, `detached`, or `terminating`. |
| `attachments` | Unsigned 32-bit integer | Accepted attachments, excluding probes and kill requests. |
| `controlled` | Boolean | Whether the session has a controller; not which client it is. |

Starting/detached state has zero attachments and no controller. Attached state
has at least one attachment and a controller. Terminating state may retain
attachments, but zero attachments cannot have a controller.

## Acceptance and failures

Unknown optional reply fields are ignored. Malformed required fields fail the
handshake. Malformed optional capabilities/state are reported as unusable to
the caller for diagnostics, independently of other optional fields; they never
make an otherwise matching session stale. They are not silently treated as
valid support.

A snapshot can become outdated immediately. It does not grant control or promise
future successful attachment. Use the [post-hello decision boundary](post-hello.md)
for the unresolved future frozen interface.

**Sources:** [implementation](../../../crates/climon2-core/src/protocol.rs),
[hello advertisements](../../../crates/climon2-core/src/hello.rs), and
[hello tests](../../../crates/climon2-core/tests/hello.rs).
