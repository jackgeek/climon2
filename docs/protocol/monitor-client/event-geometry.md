# Geometry event

**Status:** agreed communication requirements; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [client](../../../GLOSSARY.md) reports actual geometry separately from
[resize](request-resize.md) acceptance. Only the selected
[controlling](../../../GLOSSARY.md) [session](../../../GLOSSARY.md) follows the
[monitor](../../../GLOSSARY.md)'s available size. Background sessions retain
their last size until selected; [hand-back](../../../GLOSSARY.md) alone does not trigger monitor resize.

The [agreed geometry contract](../terminal-integration.md#host-canvas-geometry-and-coherent-readiness)
uses effective daemon dimensions for parsing every monitor presentation,
independent of outer-host/desired size, including background grants. The selected
desire is host width by (height minus the permanent top row), subject to supported
limits. Place the effective rectangle top-left below chrome with blank margins;
never fake [PTY](../TERMS.md#pseudo-terminal-pty) reflow. Observe
[restoration](../../../GLOSSARY.md#restoration-and-presentation) application and
selected coherent paint separately from actual geometry.
The [shared communication contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples), agreed in
[#66](https://github.com/jackgeek/climon2/issues/66), preserve causal geometry,
restoration, and result order in one per-instance
[response](../TERMS.md#response)/[event](../TERMS.md#event) stream.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
exact native types, error codes, and executable examples.
[Wire Geometry](../daemon-client/event-geometry.md) reports actual positive
unsigned 32-bit columns/rows with period. This is separate from the current
private [daemon](../../../GLOSSARY.md#sessions) schema and does not choose
operational bounds or native layout.
[Operational limits](../../monitor-operational-limits.md#memory-history-and-admission) specifies numerical
bounds not intrinsic to the chosen geometry contract.
