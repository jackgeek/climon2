# Restoration start

**Status:** agreed communication requirements; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [client](../../../GLOSSARY.md) reports a distinct
[restoration](../../../GLOSSARY.md) beginning to the [monitor](../../../GLOSSARY.md).
Identify its [control period](../../../GLOSSARY.md) and a separate
[restoration ID](../../../GLOSSARY.md). Several restorations can occur in one
control period, including after resize. A new one must not reuse an applied
restoration's identity or leave application input enabled prematurely.

Ordered [display updates](event-display.md) follow.
[Completion](event-restoration-complete.md) identifies the same restoration.
[Revocation](../../../GLOSSARY.md) invalidates it. [Plugins](../../../GLOSSARY.md) cannot hide these essential state changes.

The [terminal contract](../terminal-integration.md) selects exact ordered bytes
serialized at effective session geometry. Build a new model/history separately;
the completed restoration replaces both transactionally, never union/dedup.
Partial/superseded models never become current. Retain the last coherent
transition frame clipped/padded, without enabling input; old cache inspection
requires an explicit stale action.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) still owns
exact native declarations, identity fields, errors, and executable examples.
The [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples), agreed in
[#66](https://github.com/jackgeek/climon2/issues/66), require one causal
[response](../TERMS.md#response)/[event](../TERMS.md#event) stream.
The [grant](../../../GLOSSARY.md#attachment-and-control) precedes this start and its ordered updates
and completion. No lossy coalescing or dropped boundaries is permitted.
The [connection generation](../TERMS.md#connection-generation) remains distinct
from both restoration and control identities.
[Operational limits](../../monitor-operational-limits.md#deadlines-and-progress) specifies transfer
budgets, deadlines, and counter exhaustion.
[Wire Restoration begin](../daemon-client/event-restoration-begin.md) assigns
period, restoration ID and geometry. Application semantics are agreed above,
not deferred for redesign; native layout remains #71 and numerical budgets #69.
