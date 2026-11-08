# Report readiness

**Status: agreed semantics/wire mapping, not implemented; native declarations pending #71.**
The [monitor](../../../GLOSSARY.md#sessions) reports applied
[restoration](../../../GLOSSARY.md#restoration-and-presentation) through its
[client](../../../GLOSSARY.md#sessions) to the
[session daemon](../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md) and inherited
[communication contract](../README.md#shared-in-process-communication-contract).

[Wire Ready](../daemon-client/request-ready.md) validates current
[control period](../../../GLOSSARY.md#attachment-and-control),
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts),
and matching complete restoration sent and applied. Receipt of completion alone
is insufficient. A newer begin or
[revocation](../../../GLOSSARY.md#attachment-and-control) invalidates
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts).
Local queue acceptance is not daemon acceptance; no application input is enabled
by a stale barrier.
Application means transactionally replacing the current model and history.
It permits wire Ready/resize progress without selection or paint, including
background grants; selected application input additionally requires a coherent
current frame painted in usable space. See the
[terminal barriers](../terminal-integration.md#host-canvas-geometry-and-coherent-readiness).
Readiness alone does not set
[viewing](../../../GLOSSARY.md#status-and-attention) or acknowledge
[attention](../../../GLOSSARY.md#status-and-attention).

The wire page supplies exact fields, success/no-change result, errors and a
valid wire example. Those are not native layouts/calls.
[#71](https://github.com/jackgeek/climon2/issues/71) owns native declarations,
[#63](https://github.com/jackgeek/climon2/issues/63) owns application mechanics.
[Operational limits](../../monitor-operational-limits.md#deadlines-and-progress)
specifies numeric bounds/deadlines.
Related: [completion](event-restoration-complete.md), [viewing](request-viewing.md).
