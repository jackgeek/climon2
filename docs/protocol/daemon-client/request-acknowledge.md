# Acknowledge attention — `0x36`

**Status: agreed, not implemented; rejected until served**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [client](../../../GLOSSARY.md#sessions) asks the
[session daemon](../../../GLOSSARY.md#sessions) to acknowledge current pending
[attention](../../../GLOSSARY.md#status-and-attention).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame), with required
`request_id`, `period`, `restoration_id`, all [Counters](core-v1.md#shared-types).
There is **no observed episode ID**.

```json
{"request_id":"7","period":"1","restoration_id":"1"}
```

## Acceptance, order and failures

Requires selected `status-v1`, current
[controller](../../../GLOSSARY.md#attachment-and-control), matching
period/[restoration](../../../GLOSSARY.md#restoration-and-presentation),
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts),
and latest reported [viewing](../../../GLOSSARY.md#status-and-attention) true.
It affects the currently pending episode, not a caller-observed one.
Success [Response](response.md) result is
`{"kind":"acknowledgement","changed":<bool>}`.
No pending episode is explicit `changed:false` success. An immediate
[Status changed](event-status-changed.md) precedes a changed success.
Acceptance proves acknowledgement, not command consumption or completion.

Explicit ordinary attach, focus/selection and menu close/cancel may submit once
ready and unobstructed. Cancel deferred submission if local target context
changes. Opening/browsing menus never acknowledges; visibility alone never
auto-acknowledges later attention.
Here focus means an explicit user selection action, not host focus-in or
non-user [Focus](request-focus.md). The
[global lowercase j jump](../terminal-integration.md#attention-jump-and-configurable-hints)
may select/acquire only if needed and then acknowledge once applied, painted,
ready and unobstructed. Hiding a toast does not auto-acknowledge.

Unsupported family, no [attachment](../../../GLOSSARY.md#attachment-and-control)/
control/readiness/viewing, stale authority,
termination and operational failures use the
[error catalog](core-v1.md#errors-and-bounded-delivery).
No optional fields/defaults/unknown members; malformed records or IDs are fatal.
Bounds remain #69. Related: [Set viewing](request-viewing.md),
[local rules](core-v1.md#viewing-acknowledgement-and-local-notifications).

## Implemented behavior

Until [#91](https://github.com/jackgeek/climon2/issues/91) serves Acknowledge, the
[session daemon](../../../GLOSSARY.md#sessions) rejects it as impermissible
with `invalid_state`, before the connection has a role, on a
[passive monitor connection](../../../GLOSSARY.md#attachment-and-control) and
on an interactive [attachment](../../../GLOSSARY.md#attachment-and-control)
alike.
No attention is acknowledged; the stream goes on and any role stays.
This interim rejection comes before the `status-v1` check above.
