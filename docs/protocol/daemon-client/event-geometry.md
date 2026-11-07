# Geometry — `0x4b`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
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

Invalid records/wrong directions follow
[fatal validation](core-v1.md#framing-and-validation).
Required geometry records retain causal ordering or explicitly fail delivery;
numeric limits #69 and actual sizing/clamping/rendering/application #63 remain
separate decisions. Related: [Control granted](event-control-granted.md),
[Restoration begin](event-restoration-begin.md).
