# Open — `0x30`

**Status: agreed; monitor mode implemented, interactive mode not yet**
([#87](https://github.com/jackgeek/climon2/issues/87)), owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [client](../../../GLOSSARY.md#sessions) sends this first role request to the
[session daemon](../../../GLOSSARY.md#sessions) after
[hello](hello-request.md).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame); all fields required:

| Field | Type/value |
|---|---|
| `request_id` | [Counter](core-v1.md#shared-types). |
| `revision` | Numeric unsigned integer 1. |
| `required_capabilities` | Unique string array; all must be supported. |
| `optional_capabilities` | Unique string array, disjoint from required. |
| `mode` | `monitor` or `interactive`. |
| `geometry` | Null for [monitor](../../../GLOSSARY.md#sessions); Geometry for interactive. |

```json
{"request_id":"1","revision":1,"required_capabilities":["status-v1"],"optional_capabilities":["title-v1","notification-v1","progress-v1","focus-v1"],"mode":"monitor","geometry":null}
```

## Acceptance, order and failures

Validate advertised revision/support and
[capability](../../../GLOSSARY.md#sessions) dependencies before effects.
`daemon-wire-v1` is implicit; monitor mode also requires `monitor-v1`.
Unknown optional families are not selected; unknown required ones reject.
No release-number inference or extra negotiation exchange is allowed.
Exact-release validation is separate and mandatory.
Optional `focus-v1` selects dedicated non-user [Focus](request-focus.md);
if absent, expose the limitation without blocking otherwise supported work.
This #63 amendment adds no frozen-hello field or plugin negotiation.

Monitor mode registers an eligible
[passive monitor connection](../../../GLOSSARY.md#attachment-and-control),
not an [attachment](../../../GLOSSARY.md#attachment-and-control), with no
geometry or takeover. Interactive mode attaches and
[grants](../../../GLOSSARY.md#attachment-and-control)
[control](../../../GLOSSARY.md#attachment-and-control) using requested geometry
under daemon policy. See [succession](core-v1.md#membership-and-succession).
Monitor-mode display/restoration after acquisition or promotion uses effective
session geometry regardless of outer host or requested size under the
[canonical terminal contract](../terminal-integration.md#canonical-monitor-presentation-amendment-to-daemon-wire-revision-1).
Ordinary interactive direct/derived/native-history behavior is unchanged.

Success [Response](response.md) result is
`{"kind":"opened","connection_id":<Guid>,"selected_capabilities":<string array>}`.
The fresh logical connection identity combines with hello's session context;
it is not a physical-socket/session restriction.
Response precedes atomic [Initial metadata](event-initial-metadata.md);
interactive [grant](../../../GLOSSARY.md#attachment-and-control)/
[restoration](../../../GLOSSARY.md#restoration-and-presentation) follows metadata.
Success does not prove [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts).

Unsupported revision/capability, invalid state/argument/geometry, terminating
session or resource failure have explicit [errors](core-v1.md#errors-and-bounded-delivery).
Initial rejection has no effects, ends the attempted role stream, and may
include `known_final` if a command result is known. A later Open cannot change
role. No unknown fields/defaults; malformed records and bad IDs are fatal under
the shared rules. Numeric limits remain #69; geometry policy #63/#69.

## Implemented behavior

The [session daemon](../../../GLOSSARY.md#sessions) checks the revision first
(`unsupported_revision` unless 1), then the required families
(`unsupported_capability` for an unknown or unadvertised one). The selected
families are the requested ones it supports: every required family, plus the
optional families it advertises. It adds no family the request did not name,
since core and monitor support are implicit. Each successful Open gets a
fresh random `connection_id`.

A monitor-mode Open registers a passive monitor connection, answers success,
then sends the Initial metadata in the same daemon loop step. It never
changes the hello `state`. It is accepted during committed termination too,
which rejects new interactive work only, so a monitor can still learn the
outcome passively. A later Open on a registered connection is
`invalid_state` and keeps the registration.

Until [#87](https://github.com/jackgeek/climon2/issues/87) serves it, a valid
interactive Open is rejected without effects with `internal_error` and ends
its stream. Rejections carry no `known_final` yet: a known command result
arrives with [#88](https://github.com/jackgeek/climon2/issues/88).
