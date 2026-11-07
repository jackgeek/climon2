# Resize

**Status:** agreed communication requirements; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [monitor](../../../GLOSSARY.md) requests geometry only for the selected
[controlling](../../../GLOSSARY.md) [session](../../../GLOSSARY.md), identifying
its [control period](../../../GLOSSARY.md). A controlling resize may establish
initial geometry before [restoration](../../../GLOSSARY.md) is ready; this does
not enable application input. Reject stale-period or revoked resize explicitly.

Background [clients](../../../GLOSSARY.md#sessions) receive no monitor resize requests.
[Daemon](../../../GLOSSARY.md#sessions) acceptance does
not prove final geometry or applied restoration; observe
[geometry](event-geometry.md) and restoration separately.

The [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples) apply. Local acceptance
only queues a copied record. Recheck
[connection generation](../TERMS.md#connection-generation) and current authority
at execution: known unexecuted stale work gets an explicit asynchronous
rejection; uncertain execution is
[indeterminate](../TERMS.md#indeterminate-outcome). Never carry old-generation
resize into a reconnected [controller](../../../GLOSSARY.md#attachment-and-control).

The [terminal geometry contract](../terminal-integration.md#host-canvas-geometry-and-coherent-readiness)
sets desire to host width by (height minus one top row), with effective daemon
geometry authoritative and blank right/bottom margins. Overlay open/close never
resizes; actual host resize still updates selected control under an overlay.
Too-small space reports an explicit unavailable canvas without invalid resize
or application input. Retain last coherent frame clipped/padded until replacement
is applied/painted. Coalesce only unsent intent, never submitted outcomes.
The [communication decision](https://github.com/jackgeek/climon2/issues/66) owns
the shared rules.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
exact types, results, errors, and executable examples. Numerical budgets belong to
[operational limits](https://github.com/jackgeek/climon2/issues/69).

The [agreed wire Resize](../daemon-client/request-resize.md) uses current period
and [restoration IDs](../../../GLOSSARY.md#protocol-and-plugin-concepts) with requested geometry. Its acceptance result is distinct
from actual Geometry/restoration events; no native layout is implied.
