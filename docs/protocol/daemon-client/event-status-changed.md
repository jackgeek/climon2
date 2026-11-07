# Status changed — `0x43`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
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
