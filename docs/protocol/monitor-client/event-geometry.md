# Geometry event

**Status:** agreed communication requirements; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [client](../../../GLOSSARY.md) reports actual geometry separately from
[resize](request-resize.md) acceptance. Only the selected
[controlling](../../../GLOSSARY.md) [session](../../../GLOSSARY.md) follows the
[monitor](../../../GLOSSARY.md)'s available size. Background sessions retain
their last size until selected; [hand-back](../../../GLOSSARY.md) alone does not trigger monitor resize.

[Terminal integration](https://github.com/jackgeek/climon2/issues/63) owns
columns/rows, clamping, available area, and
[restoration](../../../GLOSSARY.md#restoration-and-presentation) effects.
The [shared communication contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples), agreed in
[#66](https://github.com/jackgeek/climon2/issues/66), preserve causal geometry,
restoration, and result order in one per-instance
[response](../TERMS.md#response)/[event](../TERMS.md#event) stream.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
exact types, error codes, and executable examples. Do not copy the current private
[daemon](../../../GLOSSARY.md#sessions) geometry schema by assumption.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) owns numerical
bounds not intrinsic to the chosen geometry contract.
