# Restoration begin — `0x48`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [session daemon](../../../GLOSSARY.md#sessions) begins a distinct
[restoration](../../../GLOSSARY.md#restoration-and-presentation) for the
controlling [client](../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame), all fields required:
`period: Counter`, `restoration_id: Counter`, `geometry: Geometry`.

```json
{"period":"2","restoration_id":"3","geometry":{"cols":80,"rows":24}}
```

## Order and guarantees

Core support is required. [Grant](event-control-granted.md) precedes initial
begin; a later restoration may occur in the same period, such as after resize.
New begin invalidates previous/incomplete
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts). Identities are not reused.
For monitor-mode connections geometry is the effective serialization size,
including passive promotion/background grants, not the outer host. Build a new
model/history separately; completion/application replaces both transactionally,
never union/dedup. Partial/superseded models never become current. The
[terminal contract](../terminal-integration.md) retains a coherent transition
frame and permits old cache only through explicit stale inspection.
Zero or more matching [Display](event-display.md) chunks precede matching
[Restoration complete](event-restoration-complete.md).
An empty restoration has no Display frames. Metadata may interleave without
reordering terminal [bytes](../TERMS.md#byte).

Begin is not completion, applied readiness,
[viewing](../../../GLOSSARY.md#status-and-attention) or
[attention](../../../GLOSSARY.md#status-and-attention) acknowledgement.
Superseded restoration must not get fabricated successful completion.
[Revocation](../../../GLOSSARY.md#attachment-and-control) invalidates authority
and prohibits later old-authority Display.
No [request ID](../TERMS.md#request-id)/defaults/unknown fields; invalid records follow
[fatal validation](core-v1.md#framing-and-validation).
Required boundaries are preserved or explicitly fail the affected connection.
[#63](https://github.com/jackgeek/climon2/issues/63) owns rendering/application;
#69 transfer budgets/deadlines/exhaustion.
