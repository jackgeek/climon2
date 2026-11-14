# Resize — `0x32`

**Status: agreed and implemented** ([#87](https://github.com/jackgeek/climon2/issues/87)), owned by [#67](https://github.com/jackgeek/climon2/issues/67).
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
The [#63 geometry contract](../terminal-integration.md#host-canvas-geometry-and-coherent-readiness)
sets selected monitor desire to host width by (height minus one status row),
with effective daemon geometry authoritative and blank local margins.
Opening/closing overlays never resizes; actual host resize still applies under
overlays. Too-small targets retain control/models but send no invalid resize or
application input. Coalesce unsent intent only; preserve all submitted outcomes.
#69 owns numeric limits/clamping bounds and progress budgets. A background
[monitor](../../../GLOSSARY.md#sessions) must not
originate resize just because it inherits control.

## Implemented behavior

The [session daemon](../../../GLOSSARY.md#sessions) accepts Resize that passes
the [authority checks](core-v1.md#implemented-authority-checks), even before
readiness. A geometry with a dimension above 65,535 is then
`invalid_geometry`. Every rejection has no effects, and the stream goes on.
The success is sent at once, before the size applies: the next daemon loop
step applies the controller's latest host size once, under the same session
geometry policy as an ordinary attach. A ready controller then receives
[Geometry](event-geometry.md) if the session geometry changed. Before
readiness, a changed host instead replaces the current restoration within
the same period: a new [Restoration begin](event-restoration-begin.md)
reports the new restoration ID and geometry, and the superseded restoration
gets no completion.
