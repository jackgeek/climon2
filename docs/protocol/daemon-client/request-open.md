# Open — `0x30`

**Status: agreed and implemented** (monitor mode in
[#86](https://github.com/jackgeek/climon2/issues/86), interactive mode in
[#87](https://github.com/jackgeek/climon2/issues/87)), owned by [#67](https://github.com/jackgeek/climon2/issues/67).
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
Ordinary interactive [direct](../../../GLOSSARY.md#restoration-and-presentation)/[derived](../../../GLOSSARY.md#restoration-and-presentation)/native-history behavior is unchanged.

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
changes the hello `state`, and registering never takes control. From then on
the registration may inherit control: a controller's departure that leaves
no attachment promotes the newest registration, and the registration's own
[Acquire](request-acquire.md#implemented-behavior) converts it into an
attachment (see [Control granted](event-control-granted.md#implemented-behavior)).
It is accepted during committed termination too,
which rejects new interactive work only, so a monitor can still learn the
outcome passively. A later Open on a registered connection is
`invalid_state` and keeps the registration.

An interactive-mode Open attaches at once and is granted control. In the
same daemon loop step the daemon answers success, sends the Initial metadata,
then [Control granted](event-control-granted.md) with reason `attach` and the
[Restoration begin](event-restoration-begin.md) of its first restoration. The
restoration's [Display](event-display.md) chunks and
[Restoration complete](event-restoration-complete.md) follow as the
connection drains. The requested geometry is the attachment's host size:
each dimension must be at most 65,535, or the Open is rejected with
`invalid_geometry` and ends its stream. The granted geometry is the session
geometry under the same daemon policy as an ordinary attach, which the
[daemon README](../../../crates/climon2-daemon/README.md#ownership-and-lifecycle)
describes: a larger host gets
[Derived presentation](../../../GLOSSARY.md#restoration-and-presentation). The attachment
takes control from any current
[controller](../../../GLOSSARY.md#attachment-and-control), private-protocol attachments
included,
and the hello `state` counts it like any attachment. During committed
termination an interactive Open is rejected with `session_terminating` and
ends its stream. Once the daemon knows how the command ended (see
[Command final](event-command-final.md#implemented-behavior)), every
rejection of an Open carries that result as `known_final`.
