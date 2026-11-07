# Restoration complete — `0x4a`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [session daemon](../../../GLOSSARY.md#sessions) completes the identified
[restoration](../../../GLOSSARY.md#restoration-and-presentation) sent to the
controlling [client](../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame) with required
`period: Counter`, `restoration_id: Counter`.

```json
{"period":"2","restoration_id":"3"}
```

## Order and guarantees

Core support is required. Matching [begin](event-restoration-begin.md) and zero
or more [Display](event-display.md) chunks precede this event.
An empty restoration has no Display frames, not an empty Display frame.
Live output follows using matching authority.

Receipt is not applied [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts).
The client must apply the complete current restoration before reporting
[Ready](request-ready.md). A newer begin or
[revocation](../../../GLOSSARY.md#attachment-and-control) invalidates old barriers;
superseded restoration receives no fabricated completion. Completion does not
select, acknowledge [attention](../../../GLOSSARY.md#status-and-attention)
or prove application consumption.

No [request ID](../TERMS.md#request-id)/defaults/unknown members. Invalid records follow
[fatal validation](core-v1.md#framing-and-validation). Required boundaries
cannot silently disappear under congestion; preserve or fail affected connection.
#63 owns application mechanics and #69 numeric transfer/deadline/exhaustion
policy. Related: [Control revoked](event-control-revoked.md).
