# Ready — `0x34`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [client](../../../GLOSSARY.md#sessions) reports applied
[restoration](../../../GLOSSARY.md#restoration-and-presentation) to the
[session daemon](../../../GLOSSARY.md#sessions), as current
[controller](../../../GLOSSARY.md#attachment-and-control).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame) with required
`request_id`, `period`, `restoration_id`, all [Counters](core-v1.md#shared-types):

```json
{"request_id":"5","period":"1","restoration_id":"1"}
```

## Acceptance, order and failures

Core support is required. The matching complete restoration must have been
sent and applied by the client; for a [monitor](../../../GLOSSARY.md#sessions)
this includes application to its presentation.
Applied means transactionally replacing the complete current model/history,
not painting it. It permits Ready/resize progress for background grants or before
selected paint; selected application input additionally requires a coherent
current frame painted in usable space under the
[terminal barriers](../terminal-integration.md#host-canvas-geometry-and-coherent-readiness).
Receipt of [Restoration complete](event-restoration-complete.md) alone is insufficient.
Success [Response](response.md) result is
`{"kind":"ready","changed":<bool>,"period":<Counter>,"restoration_id":<Counter>}`.
This barrier does not itself select, report
[viewing](../../../GLOSSARY.md#status-and-attention), acknowledge
[attention](../../../GLOSSARY.md#status-and-attention),
or prove application consumption.

New restoration/[revocation](../../../GLOSSARY.md#attachment-and-control)
invalidates earlier [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts).
Stale period/restoration, missing
[attachment](../../../GLOSSARY.md#attachment-and-control)/control, incomplete readiness,
termination and operational failures use the
[error catalog](core-v1.md#errors-and-bounded-delivery).
No optional fields/defaults/unknown members; malformed records or IDs are
fatal. Request bounds/deadlines remain #69, application mechanics #63.
Related: [Input](request-input.md), [Set viewing](request-viewing.md).
