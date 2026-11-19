# Status changed — `0x43`

**Status: agreed and implemented** for heuristic detection
([#91](https://github.com/jackgeek/climon2/issues/91)) and for explicit and
fallback detection from program status and shell markers
([#92](https://github.com/jackgeek/climon2/issues/92)), and relayed by the
[client library](../../../GLOSSARY.md#sessions) ([#95](https://github.com/jackgeek/climon2/issues/95)), owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [session daemon](../../../GLOSSARY.md#sessions) sends effective
[session status](../../../GLOSSARY.md#status-and-attention) to the
[client](../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame) with sole required
field `status: Status`. All nested members are required.

```json
{"status":{"state":"acknowledged","changed_at":"2026-11-06T12:00:01.000Z","episode_id":"1"}}
```

See [shared types](core-v1.md#shared-types) for states, exact [UTC](../TERMS.md#utc) encoding and
episode constraints. Only `needs_attention`/`acknowledged` have nonnull episode.
Time is not identity. Coalesced pending requests reuse their episode;
acknowledgement preserves it; clear removes it.

## Order, guarantees and errors

Requires selected `status-v1`. After [Initial metadata](event-initial-metadata.md)
deliver changes gap-free regardless of control,
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts) or menus.
Immediate causal changes precede the correlated operation success.
Final transition is carried by [Command final](event-command-final.md), **not**
a duplicate Status changed event. No late event revives terminal state.

Effective state changes update stored time; hidden detector changes do not.
Follow the [unchanged detector mechanics](../../session-status.md).
Delivery does not itself acknowledge or clear
[attention](../../../GLOSSARY.md#status-and-attention),
[grant](../../../GLOSSARY.md#attachment-and-control) control or prove
application consumption. Local toast decisions use each
[monitor](../../../GLOSSARY.md#sessions)'s own
[viewing](../../../GLOSSARY.md#status-and-attention).

No [request ID](../TERMS.md#request-id)/defaults/unknown members. Malformed or unselected events are
invalid, not implicit extensions. Apply
[validation and bounded failure](core-v1.md#errors-and-bounded-delivery);
never lossily coalesce required status/episode transitions. Numeric limits #69.
Related: [Acknowledge](request-acknowledge.md).

## Implemented behavior

The [session daemon](../../../GLOSSARY.md#sessions) runs both
[detector machines](../../session-status.md), starting in heuristic mode, and sends a
Status changed to every
[passive monitor connection](../../../GLOSSARY.md#attachment-and-control) and interactive
[attachment](../../../GLOSSARY.md#attachment-and-control) that selected
`status-v1`, whatever its control, once per effective change, after the
event that caused it ran to completion: an inactivity expiry that requests
attention reports `needs_attention`, never an intermediate `idle`. Changes
between the two working states, activity hidden under attention, and
coalesced requests send nothing and keep the stored change time. An
accepted Input's change precedes its [Response](response.md), as does an
[Acknowledge](request-acknowledge.md)'s. The final transition goes only in
[Command final](event-command-final.md#implemented-behavior). Episode
identifiers start at 1 and never wrap or repeat. When a request needs one
after the last [counter](core-v1.md#shared-types) value was issued, the
session-wide episode domain cannot advance: the daemon reports
`resource_limit`, as the
[operational limits](../../monitor-operational-limits.md#counters-and-explicit-results)
require, by rejecting every later [Acknowledge](request-acknowledge.md#implemented-behavior)
with that code, and logs it. Accepted Input holding only focus reports
(`ESC [ I`, `ESC [ O`), which a host terminal sends on its own, is not user
input and clears nothing. Root [OSC](../TERMS.md#osc) 7501 program-status reports and OSC 133
shell markers in the command's output also change activity and attention
([#92](https://github.com/jackgeek/climon2/issues/92)), as the
[status mechanics](../../session-status.md#explicit-program-status-and-fallback)
specify: each change they cause is reported here the same way, in the order
the program wrote them. Each recognized
[Notification](event-notification.md#implemented-behavior)
([#98](https://github.com/jackgeek/climon2/issues/98)) requests attention
the same way; its Status changed, if any, precedes the Notification.
