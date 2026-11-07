# Resize — `0x32`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [client](../../../GLOSSARY.md#sessions) requests geometry from the
[session daemon](../../../GLOSSARY.md#sessions) as current
[controller](../../../GLOSSARY.md#attachment-and-control).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame), all fields required:
`request_id`, `period`, `restoration_id` are
[Counters](core-v1.md#shared-types); `geometry` is [Geometry](core-v1.md#shared-types).
Columns/rows are positive unsigned 32-bit integer tokens, not pixel sizes or
promised operational bounds.

```json
{"request_id":"3","period":"1","restoration_id":"1","geometry":{"cols":100,"rows":30}}
```

## Acceptance, order and failures

Validate current period/[restoration](../../../GLOSSARY.md#restoration-and-presentation)
and control, even before [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts).
Success [Response](response.md) result is
`{"kind":"resize_accepted"}`, not final geometry or applied restoration.
Observe actual [Geometry](event-geometry.md) and any new
[Restoration begin](event-restoration-begin.md) separately.
Immediate causal changes precede success; deferred acceptance does not pretend
the geometry is already reported. A newer restoration invalidates readiness.

Stale period/restoration, missing membership/control, invalid geometry,
termination and resource/I/O failures use the shared
[error catalog](core-v1.md#errors-and-bounded-delivery).
Malformed records/IDs are fatal. No unknown fields/defaults; use the advertised
request limit. Core support is required.
[#63](https://github.com/jackgeek/climon2/issues/63)/#69 own actual sizing,
clamping, application and resize policy. A background
[monitor](../../../GLOSSARY.md#sessions) must not
originate resize just because it inherits control.
