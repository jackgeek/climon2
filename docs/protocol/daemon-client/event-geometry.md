# Geometry — `0x4b`

**Status: agreed and implemented** for interactive attachments
([#87](https://github.com/jackgeek/climon2/issues/87)) and monitor-mode
connections ([#89](https://github.com/jackgeek/climon2/issues/89)), owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [session daemon](../../../GLOSSARY.md#sessions) reports actual geometry to
the [client](../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame) with required
`period: Counter`, `geometry: Geometry`:

```json
{"period":"2","geometry":{"cols":100,"rows":30}}
```

[Geometry](core-v1.md#shared-types) has required positive unsigned 32-bit columns/rows, not pixel units.
No [restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts),
optional fields, defaults or unknown members are present.
Representable values do not promise operational bounds.

## Order and guarantees

Core support is required. Report actual daemon geometry, distinct from
[Resize](request-resize.md) acceptance. Immediate causal changes precede the
correlated success; deferred acceptance need not already have a geometry event.
Observe [restoration](../../../GLOSSARY.md#restoration-and-presentation)
and [Ready](request-ready.md) separately: geometry alone
is not applied [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts),
[viewing](../../../GLOSSARY.md#status-and-attention) or consumption.
Inherited/background control does not originate
[monitor](../../../GLOSSARY.md#sessions) resize.
Monitor-mode presentation uses these effective dimensions, not outer-host/
requested dimensions. The monitor positions the session rectangle top-left
below its permanent top row and blanks unused margins, without local
[PTY](../TERMS.md#pseudo-terminal-pty) reflow. Geometry carries no
restoration, and a monitor model is never reflowed in place: for
monitor-mode connections a [Restoration begin](event-restoration-begin.md)
at the reported geometry always follows Geometry, before any
[Display](event-display.md) at that geometry (developer decision on
[#76](https://github.com/jackgeek/climon2/issues/76), 2026-11-13).
The [#63 geometry contract](../terminal-integration.md#host-canvas-geometry-and-coherent-readiness)
keeps host-relative overlay layout independent of this acknowledgement.

Invalid records/wrong directions follow
[fatal validation](core-v1.md#framing-and-validation).
Required geometry records retain causal ordering or explicitly fail delivery;
numeric limits #69 and actual sizing/clamping/rendering/application #63 remain
separate decisions. Related: [Control granted](event-control-granted.md),
[Restoration begin](event-restoration-begin.md).

## Implemented behavior

After a ready [controller](../../../GLOSSARY.md#attachment-and-control)'s
[Resize](request-resize.md) applies, the
[session daemon](../../../GLOSSARY.md#sessions) sends Geometry under the
current period when the session geometry differs from the one the
controller last learned; a host change that leaves the session geometry as
it was sends none. Geometry follows the Resize success. Before readiness, a
changed geometry is reported by the replacement
Restoration begin instead. To a monitor-mode controller, a
[passive monitor connection](../../../GLOSSARY.md#attachment-and-control)
that its Acquire or a
[promotion](../../../GLOSSARY.md#attachment-and-control) made an attachment, the replacement
Restoration begin at the new geometry follows its Geometry at once, before
any Display, and its readiness ends until it confirms the new restoration.
